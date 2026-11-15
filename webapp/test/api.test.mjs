// lib/api.js on a failure the server has classified as Deezer's.
//
// A page only Deezer has, asked while Deezer is out, answered 404 and the page
// said "introuvable"; made a 503, it would have been retried three times with
// ~3 s of backoff for the same instant refusal. The server now marks such an
// answer (`X-NS-Error`) and does the reaching itself — every call retries
// inside its own budget, the circuit breaker probes Deezer once its cooldown is
// over — so the client asks ONCE and says what happened. An unmarked 503 (a
// server restarting) is still retried: that one is worth another try.
//
// Time is node's mock clock, so the backoff costs nothing.

import test, { mock } from "node:test";
import assert from "node:assert/strict";

globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
globalThis.window = { addEventListener() {}, removeEventListener() {}, location: { hash: "" } };

let calls = 0;
let answer = null;
let seen = [];
let dbOpens = 0;
globalThis.fetch = async (url, opts = {}) => {
  calls++;
  seen.push({ url, opts });
  return answer();
};
globalThis.indexedDB = {
  open() {
    dbOpens++;
    throw new Error("IndexedDB is not available here");
  },
};

function response(status, body, headers = {}) {
  const h = new Map(Object.entries(headers).map(([k, v]) => [k.toLowerCase(), v]));
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: "",
    headers: { get: (k) => h.get(String(k).toLowerCase()) ?? null },
    json: async () => body,
    clone() {
      return this;
    },
    text: async () => JSON.stringify(body),
  };
}

mock.timers.enable({ apis: ["setTimeout"] });
const { api, apiError, isDeezerUnreachable, onServerError } = await import("../src/lib/api.js");
const { isCacheable } = await import("../src/lib/apicache.js");

// Run a request to its end, moving the mock clock through any backoff: a
// client that retried must show up as a call count, not as a test that hangs.
async function drive(promise) {
  let done = false;
  const p = promise.then(
    () => ((done = true), null),
    (e) => ((done = true), e)
  );
  for (let i = 0; i < 50 && !done; i++) {
    for (let j = 0; j < 20; j++) await Promise.resolve();
    mock.timers.tick(10_000);
  }
  return p;
}

const reported = [];
onServerError((status, path) => reported.push([status, path]));

test("a page only Deezer has, while Deezer is out: asked once, said as such", async () => {
  calls = 0;
  answer = () =>
    response(
      503,
      { error: "Deezer est injoignable", code: "deezer-unreachable", retry_in: 42 },
      { "X-NS-Error": "deezer-unreachable", "Retry-After": "42" }
    );
  const err = await drive(api.album("600"));
  assert.equal(calls, 1, "the client retried what the server had already tried");
  assert.ok(isDeezerUnreachable(err));
  assert.equal(err.status, 503);
  assert.equal(err.retryIn, 42);
  // The Deezer health watcher still hears of it (it raises the notice).
  assert.deepEqual(reported.at(-1), [503, "/album/600"]);
});

test("a disabled proxy is final too, and is not 'unreachable'", async () => {
  calls = 0;
  answer = () =>
    response(503, { error: "Deezer proxy disabled", code: "deezer-disabled" }, { "X-NS-Error": "deezer-disabled" });
  const err = await drive(api.playlist("123"));
  assert.equal(calls, 1);
  assert.equal(err.code, "deezer-disabled");
  assert.equal(isDeezerUnreachable(err), false);
});

test("an unmarked 503 (a server restarting) is still retried", async () => {
  calls = 0;
  answer = () => response(503, { error: "busy" });
  const err = await drive(api.album("600"));
  assert.equal(calls, 4);
  assert.equal(err.status, 503);
  assert.equal(err.code, undefined);
});

async function flush() {
  for (let i = 0; i < 30; i++) await Promise.resolve();
  mock.timers.tick(10_000);
  for (let i = 0; i < 30; i++) await Promise.resolve();
}

function last() {
  return seen[seen.length - 1];
}

test("the collaborative playlist calls carry their counters", async () => {
  seen = [];
  answer = () => response(200, { ok: true });

  await api.playlistVersion("p1");
  assert.equal(last().url, "/api/playlist/p1/version");
  assert.equal(last().opts.method, undefined);

  await api.playlistMembers("p1");
  assert.equal(last().url, "/api/playlist/p1/members");

  await api.invitePlaylistMember("p1", "bob", "editor", 4);
  assert.equal(last().url, "/api/playlist/p1/members");
  assert.equal(last().opts.method, "POST");
  assert.deepEqual(JSON.parse(last().opts.body), {
    username: "bob",
    role: "editor",
    members_version: 4,
  });

  await api.setPlaylistMemberRole("p1", "u 1", "viewer", 5);
  assert.equal(last().url, "/api/playlist/p1/members/u%201");
  assert.equal(last().opts.method, "PATCH");
  assert.deepEqual(JSON.parse(last().opts.body), { role: "viewer", members_version: 5 });

  await api.removePlaylistMember("p1", "u1", 6);
  assert.equal(last().url, "/api/playlist/p1/members/u1");
  assert.equal(last().opts.method, "DELETE");
  assert.deepEqual(JSON.parse(last().opts.body), { members_version: 6 });

  await api.leavePlaylist("p1");
  assert.equal(last().url, "/api/playlist/p1/leave");
  assert.equal(last().opts.method, "POST");
  assert.deepEqual(JSON.parse(last().opts.body), {});
});

test("the existing playlist mutations take a version without breaking old calls", async () => {
  seen = [];
  answer = () => response(200, { ok: true });

  await api.editPlaylist("p1", { title: "Route 66" }, 7);
  assert.deepEqual(JSON.parse(last().opts.body), { title: "Route 66", version: 7 });
  await api.editPlaylist("p1", { title: "Route 66" });
  assert.deepEqual(JSON.parse(last().opts.body), { title: "Route 66" });

  await api.addToPlaylist("p1", ["t1"], 9);
  assert.deepEqual(JSON.parse(last().opts.body), { tracks: ["t1"], version: 9 });
  await api.addToPlaylist("p1", ["t1"]);
  assert.deepEqual(JSON.parse(last().opts.body), { tracks: ["t1"] });

  await api.removeFromPlaylist("p1", ["t1"], 9);
  assert.deepEqual(JSON.parse(last().opts.body), { tracks: ["t1"], version: 9 });

  await api.removePlaylistIndexes("p1", [1, 2], 9);
  assert.deepEqual(JSON.parse(last().opts.body), { indexes: [1, 2], version: 9 });

  await api.reorderPlaylist("p1", ["a", "b"], 9);
  assert.equal(last().url, "/api/playlist/p1/order");
  assert.deepEqual(JSON.parse(last().opts.body), { tracks: ["a", "b"], version: 9 });

  await api.deletePlaylist("p1");
  assert.equal(last().opts.body, undefined);
  await api.deletePlaylist("p1", 9);
  assert.deepEqual(JSON.parse(last().opts.body), { version: 9 });
});

test("the onboarding, invite and admin routes use the server's paths and bodies", async () => {
  seen = [];
  answer = () => response(200, { ok: true });

  await api.onboardingState();
  assert.equal(last().url, "/api/onboarding");

  await api.onboardingArtists({ after: 24, limit: 12 });
  assert.equal(last().url, "/api/onboarding/artists?after=24&limit=12");
  await api.onboardingArtists();
  assert.equal(last().url, "/api/onboarding/artists");
  await api.onboardingArtists({ cursor: 48 });
  assert.equal(last().url, "/api/onboarding/artists?after=48");
  await api.onboardingArtists({ q: "Aya", limit: 20 });
  assert.equal(last().url, "/api/onboarding/search?q=Aya&limit=20");

  await api.onboardingSearch("Aya Nakamura");
  let u = new URL(last().url, "http://x");
  assert.equal(u.pathname, "/api/onboarding/search");
  assert.equal(u.searchParams.get("q"), "Aya Nakamura");

  await api.onboardingLike({ deezer_id: "42", id: "42" });
  assert.equal(last().url, "/api/onboarding/artists/like");
  assert.equal(last().opts.method, "POST");
  assert.deepEqual(JSON.parse(last().opts.body), { deezer_id: "42" });
  await api.onboardingLike("42", ["7", "8"]);
  assert.deepEqual(JSON.parse(last().opts.body), { deezer_id: "42", seen: ["7", "8"] });

  await api.onboardingUnlike("42");
  assert.equal(last().url, "/api/onboarding/artists/42/like");
  assert.equal(last().opts.method, "DELETE");

  await api.onboardingRelated("42");
  assert.equal(last().url, "/api/onboarding/artists/42/related");

  await api.onboardingArtistsDone();
  assert.equal(last().url, "/api/onboarding/artists/done");
  assert.equal(last().opts.method, "POST");
  await api.onboardingFeedRetry();
  assert.equal(last().url, "/api/onboarding/feed/retry");
  assert.equal(last().opts.method, "POST");
  await api.onboardingAccountDone();
  assert.equal(last().url, "/api/onboarding/account/done");
  assert.equal(last().opts.method, "POST");

  await api.onboardingAccountOptions();
  assert.equal(last().url, "/api/onboarding/account/options");
  await api.onboardingAccountLink({ provider: "deezer", arl: "secret" });
  assert.equal(last().url, "/api/onboarding/account/link");
  assert.deepEqual(JSON.parse(last().opts.body), { provider: "deezer", arl: "secret" });
  await api.onboardingAccountImport({ source: "spotify", url: "https://x" });
  assert.equal(last().url, "/api/onboarding/account/import");
  assert.deepEqual(JSON.parse(last().opts.body), { source: "spotify", url: "https://x" });

  await api.inviteInspect("tok");
  assert.equal(last().url, "/api/invites/inspect");
  assert.deepEqual(JSON.parse(last().opts.body), { token: "tok" });
  const redeem = { token: "tok", username: "elodie", password: "p", password_confirm: "p" };
  await api.inviteRedeem(redeem);
  assert.equal(last().url, "/api/invites/redeem");
  assert.deepEqual(JSON.parse(last().opts.body), redeem);

  await api.adminUsers();
  assert.equal(last().url, "/api/admin/users?limit=50");
  await api.adminUsers({ q: "cam", after: "42", limit: 25 });
  u = new URL(last().url, "http://x");
  assert.equal(u.pathname, "/api/admin/users");
  assert.equal(u.searchParams.get("q"), "cam");
  assert.equal(u.searchParams.get("after"), "42");
  assert.equal(u.searchParams.get("limit"), "25");

  await api.adminUser("u 1");
  assert.equal(last().url, "/api/admin/users/u%201");
  await api.adminUserPatch("42", { admin: true });
  assert.equal(last().opts.method, "PATCH");
  assert.deepEqual(JSON.parse(last().opts.body), { admin: true });
  await api.adminUserDelete("42");
  assert.equal(last().url, "/api/admin/users/42");
  assert.equal(last().opts.method, "DELETE");

  await api.adminInvites();
  assert.equal(last().url, "/api/admin/invites?limit=50");
  await api.adminInvites({
    state: "pending",
    limit: 10,
    afterCreated: "2026-11-14T10:00:00",
    afterId: "AbC",
  });
  u = new URL(last().url, "http://x");
  assert.equal(u.pathname, "/api/admin/invites");
  assert.equal(u.searchParams.get("state"), "pending");
  assert.equal(u.searchParams.get("limit"), "10");
  assert.equal(u.searchParams.get("after_created"), "2026-11-14T10:00:00");
  assert.equal(u.searchParams.get("after_id"), "AbC");

  await api.adminInviteCreate({ note: "Léa", expires_in: 604800 });
  assert.equal(last().url, "/api/admin/invites");
  assert.equal(last().opts.method, "POST");
  assert.deepEqual(JSON.parse(last().opts.body), { note: "Léa", expires_in: 604800 });
  await api.adminInviteRevoke("i 1");
  assert.equal(last().url, "/api/admin/invites/i%201/revoke");
  assert.equal(last().opts.method, "POST");
  await api.adminInviteRenew("i1");
  assert.equal(last().url, "/api/admin/invites/i1/renew");
  assert.equal(last().opts.method, "POST");
});

test("an episode's chapters are one bare GET", async () => {
  seen = [];
  const payload = {
    episode_id: "e1",
    archived: true,
    scanned: true,
    source: "id3",
    duration: 3600,
    chapters: [{ index: 1, start: 0, end: 60, title: "Intro" }],
  };
  answer = () => response(200, payload);
  const data = await api.episodeChapters("e 1");
  assert.equal(last().url, "/api/podcast/episode/e%201/chapters");
  assert.equal(last().opts.method, undefined);
  assert.deepEqual(data, payload);
});

test("a 409 exposes its authoritative counters for the resync", async () => {
  calls = 0;
  answer = () =>
    response(409, {
      error: "conflict",
      scope: "members",
      version: 17,
      members_version: 5,
      last_edited_by: "Léa",
      last_edited_at: "2026-11-14T10:22:31",
    });
  const err = await drive(api.setPlaylistMemberRole("p1", "u1", "viewer", 4));
  assert.equal(calls, 1);
  assert.equal(err.status, 409);
  assert.equal(err.message, "conflict");
  const info = apiError(err);
  assert.equal(info.error, "conflict");
  assert.equal(info.scope, "members");
  assert.equal(info.version, 17);
  assert.equal(info.members_version, 5);
  assert.equal(info.last_edited_by, "Léa");
  assert.equal(info.last_edited_at, "2026-11-14T10:22:31");
  assert.equal(apiError({ status: 500 }), null);
  assert.equal(apiError(null), null);
});

test("a 409 or a 429 never reaches the offline cache", async () => {
  answer = () => response(200, { ok: true });
  await api.version();
  await flush();
  dbOpens = 0;

  answer = () => response(409, { error: "conflict", scope: "content", version: 2 });
  await drive(api.editPlaylist("p1", { title: "x" }, 1));
  await flush();
  assert.equal(dbOpens, 0);

  answer = () => response(429, { error: "Trop de tentatives.", code: "rate_limited" });
  await assert.rejects(api.playlist("p1"), (err) => err.status === 429);
  await flush();
  assert.equal(dbOpens, 0);
});

test("live and personal paths are kept out of the offline cache", () => {
  assert.equal(isCacheable("/playlist/p1/version"), false);
  assert.equal(isCacheable("/playlist/p1/version?since=1"), false);
  assert.equal(isCacheable("/admin/users?limit=50"), false);
  assert.equal(isCacheable("/admin/invites"), false);
  assert.equal(isCacheable("/invites/inspect"), false);
  assert.equal(isCacheable("/onboarding"), false);
  assert.equal(isCacheable("/onboarding/artists?after=24"), false);
  assert.equal(isCacheable("/podcast/episode/e1/chapters"), false);
  assert.equal(isCacheable("/me/playlists"), true);
  assert.equal(isCacheable("/playlist/p1"), true);
  assert.equal(isCacheable("/playlist/p1/members"), true);
});

test("the client contract exports every method the lots rely on", () => {
  for (const name of [
    "playlistVersion",
    "playlistMembers",
    "invitePlaylistMember",
    "setPlaylistMemberRole",
    "removePlaylistMember",
    "leavePlaylist",
    "episodeChapters",
    "onboardingState",
    "onboardingArtists",
    "onboardingSearch",
    "onboardingLike",
    "onboardingUnlike",
    "onboardingRelated",
    "onboardingArtistsDone",
    "onboardingFeedRetry",
    "onboardingAccountOptions",
    "onboardingAccountLink",
    "onboardingAccountImport",
    "onboardingAccountDone",
    "inviteInspect",
    "inviteRedeem",
    "adminUsers",
    "adminUser",
    "adminUserPatch",
    "adminUserDelete",
    "adminInvites",
    "adminInviteCreate",
    "adminInviteRevoke",
    "adminInviteRenew",
  ]) {
    assert.equal(typeof api[name], "function", name);
  }
});

test("a refusal drops the cached copy of a page", async () => {
  const { switchAccount } = await import("../src/lib/account.js");
  switchAccount({ name: "bench" });
  dbOpens = 0;
  answer = () => response(404, { error: "not found" });
  await assert.rejects(api.playlist("private"), (err) => err.status === 404);
  assert.equal(dbOpens, 1);

  dbOpens = 0;
  answer = () =>
    response(
      503,
      { error: "Deezer est injoignable", code: "deezer-unreachable" },
      { "X-NS-Error": "deezer-unreachable" }
    );
  await assert.rejects(api.playlist("private"), (err) => err.code === "deezer-unreachable");
  assert.equal(dbOpens, 0);
});
