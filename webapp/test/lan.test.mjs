// Réglages → « Appareils et amis » (lot LN8): the pure words and list edits the
// screen is built from, and the exact shape of each LAN call.
//
// No DOM here: `node --test` with a `window`/`localStorage` shim, the way
// test/sources.test.mjs loads a module that reads stores. The fetch stub
// records method, path and body so a route that changes shape (a PATCH turning
// into a PUT, a field renamed) fails here instead of in front of the person.
//
// The numbers this file pins, and where they come from:
//   - the feature probe costs ONE request per page load, and a network blip is
//     not remembered (a later mount asks again);
//   - the friend picker reads `/me/playlists` with ZERO requests once the SPA's
//     offline cache has it (one `peek`, no `GET`).
import test from "node:test";
import assert from "node:assert/strict";

globalThis.window = {
  addEventListener() {},
  location: { href: "http://x/", origin: "http://x", hash: "" },
  matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
};
globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
globalThis.sessionStorage = globalThis.localStorage;

const lan = await import("../src/lib/lan.js");
const { api } = await import("../src/lib/api.js");

// -- a fetch that answers what the test says, and remembers what it was asked

let calls = [];
let answers = [];
let failing = false;
function stubFetch({ fail = false } = {}) {
  calls = [];
  answers = [];
  failing = fail;
  globalThis.fetch = async (url, opts = {}) => {
    const path = String(url);
    // lib/net.js probes /api/me when a request fails; it is not this screen's
    // request and counting it would blur what is measured here.
    const counted = !/\/api\/me$/.test(path);
    if (counted) {
      calls.push({
        url: path,
        method: (opts.method || "GET").toUpperCase(),
        body: opts.body ? JSON.parse(opts.body) : null,
      });
    }
    if (failing) throw new TypeError("network");
    const next = answers.shift() || { status: 200, body: {} };
    return {
      ok: next.status >= 200 && next.status < 300,
      status: next.status,
      statusText: next.status === 404 ? "Not Found" : "OK",
      headers: { get: () => null },
      json: async () => next.body ?? {},
      clone() {
        return { text: async () => JSON.stringify(next.body ?? {}) };
      },
    };
  };
}
// `navigator.onLine` is what lib/api.js reads to decide whether a failed call
// is worth retrying. Node's own `navigator` has no `onLine`, which is exactly
// the browser case that retries: no shim, so the module under test is the one
// the person runs.
stubFetch();

test("names, states and dates read in French", () => {
  assert.equal(lan.platformLabel("desktop"), "Ordinateur");
  assert.equal(lan.platformLabel("android"), "Téléphone Android");
  assert.equal(lan.platformLabel("ios"), "iPhone ou iPad");
  assert.equal(lan.platformLabel("browser"), "browser");
  assert.equal(lan.platformLabel(null), "Appareil");

  assert.equal(lan.platformIcon("desktop"), "monitor");
  assert.equal(lan.platformIcon("android"), "smartphone");
  assert.equal(lan.platformIcon("ios"), "smartphone");

  assert.equal(lan.deviceState({ current: true }), "current");
  assert.equal(lan.deviceState({ revoked: "2026-11-01T10:00:00Z" }), "revoked");
  assert.equal(lan.deviceState({}), "enrolled");

  const NOW = Date.UTC(2026, 10, 18, 12, 0, 0);
  assert.equal(lan.seenLabel(new Date(NOW - 20_000).toISOString(), NOW), "vu à l'instant");
  assert.equal(lan.seenLabel(new Date(NOW - 5 * 60_000).toISOString(), NOW), "vu il y a 5 min");
  assert.equal(lan.seenLabel(new Date(NOW - 3 * 3_600_000).toISOString(), NOW), "vu il y a 3 h");
  assert.equal(lan.seenLabel(new Date(NOW - 2 * 86_400_000).toISOString(), NOW), "vu il y a 2 j");
  assert.equal(lan.seenLabel(new Date(NOW - 400 * 86_400_000).toISOString(), NOW), "vu il y a 1 an");
  assert.equal(lan.seenLabel(null, NOW), "jamais vu");
  assert.equal(lan.seenLabel("not-a-date", NOW), "date inconnue");

  const device = {
    platform: "android",
    created: "2026-11-12T09:00:00Z",
    last_seen: new Date(NOW - 120_000).toISOString(),
  };
  const detail = lan.deviceDetail(device, NOW);
  assert.match(detail, /^Téléphone Android · inscrit le 12 nov\. 2026 · vu il y a 2 min$/);
  // The screen renders one SEGMENT per fact and breaks between them, so a
  // narrow column can never cut "vu il y a 2 min" in two.
  assert.deepEqual(lan.deviceSegments(device, NOW), [
    { kind: "platform", text: "Téléphone Android" },
    { kind: "enrolled", text: "inscrit le 12 nov. 2026" },
    { kind: "seen", text: "vu il y a 2 min" },
  ]);
  assert.deepEqual(lan.deviceSegments(null, NOW), []);
  assert.match(lan.deviceDetail({ ...device, revoked: "2026-11-17T09:00:00Z" }, NOW), /· révoqué$/);
});

test("what a friend is allowed to see, in one line", () => {
  assert.equal(lan.sharedSummary({ all: true, playlists: [] }), "Tout partager");
  assert.equal(lan.sharedSummary({ all: false, playlists: [] }), "Aucune liste partagée");
  assert.equal(lan.sharedSummary({ all: false, playlists: [{ id: "a" }] }), "1 liste partagée");
  assert.equal(
    lan.sharedSummary({ all: false, playlists: [{ id: "a" }, { id: "b" }] }),
    "2 listes partagées",
  );
});

test("the credential lifetime takes only 7–90 whole days", () => {
  assert.equal(lan.daysValue("30"), 30);
  assert.equal(lan.daysValue(7), 7);
  assert.equal(lan.daysValue(90), 90);
  assert.equal(lan.daysValue(6), null);
  assert.equal(lan.daysValue(91), null);
  assert.equal(lan.daysValue("12.5"), null);
  assert.equal(lan.daysValue(""), null);
  assert.equal(lan.daysLabel(30), "30 jours");
  assert.equal(lan.daysLabel(1), "1 jour");
});

test("the server key is named by its first eight hex digits", () => {
  assert.equal(lan.keyLabel({ keys: [{ kid: "56475aa75463474c" }] }), "n° 56475aa7");
  assert.equal(lan.keyLabel({ keys: [] }), "");
  assert.equal(lan.keyLabel(null), "");
});

test("an optimistic edit is a pure list change the screen can roll back", () => {
  const devices = [
    { id: "a", name: "PC" },
    { id: "b", name: "Téléphone" },
  ];
  assert.deepEqual(
    lan.renameDeviceIn(devices, "a", "Bureau").map((d) => d.name),
    ["Bureau", "Téléphone"],
  );
  // The input list is never mutated: the caller keeps the old value to restore.
  assert.equal(devices[0].name, "PC");
  assert.deepEqual(lan.withoutDevice(devices, "a").map((d) => d.id), ["b"]);

  const friends = {
    shares_to: [{ user_id: "u1", all: false, playlists: [{ id: "p1", title: "Route" }] }],
    trusted_by: [{ user_id: "u2", name: "Alice" }],
  };
  const scoped = lan.withScope(friends, "u1", { all: true });
  assert.equal(scoped.shares_to[0].all, true);
  assert.equal(friends.shares_to[0].all, false);
  // `trusted_by` (who shares WITH me) is never touched by my own scope edit.
  assert.deepEqual(scoped.trusted_by, friends.trusted_by);
  assert.deepEqual(lan.withoutFriend(friends, "u1").shares_to, []);
  assert.deepEqual(lan.withoutFriend(friends, "u1").trusted_by, friends.trusted_by);
});

test("a playlist tick is one id added or removed, order kept", () => {
  assert.deepEqual(lan.togglePlaylist([], "p1"), ["p1"]);
  assert.deepEqual(lan.togglePlaylist(["p1", "p2"], "p3"), ["p1", "p2", "p3"]);
  assert.deepEqual(lan.togglePlaylist(["p1", "p2"], "p1"), ["p2"]);
  assert.deepEqual(lan.sharedIds({ playlists: [{ id: 7, title: "x" }] }), ["7"]);
  assert.deepEqual(lan.sharedPairs({ playlists: [{ id: "p1" }, { id: "gone" }] }, [{ id: "p1", title: "Route" }]), [
    { id: "p1", title: "Route" },
    { id: "gone", title: "Liste supprimée" },
  ]);
});

// -- the wire ---------------------------------------------------------------

test("the feature probe is one request, and a 404 hides everything", async () => {
  lan.resetFeature();
  stubFetch();
  answers.push({ status: 404, body: {} });
  const answer = await lan.lanFeature();
  assert.equal(answer.supported, false);
  assert.equal(answer.definitive, true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "/api/lan/status");
  // A definite "no" is remembered: a second Réglages visit costs 0 requests.
  await lan.lanFeature();
  assert.equal(calls.length, 1);
});

test("the probe reads the switch, the lifetime and the key", async () => {
  lan.resetFeature();
  stubFetch();
  answers.push({
    status: 200,
    body: {
      enabled: false,
      subject: "00112233445566778899aabbccddeeff",
      keys: [{ kid: "56475aa75463474c", pub: "ab" }],
      credential_days: 30,
      revocation_serial: 7,
    },
  });
  const answer = await lan.lanFeature();
  assert.equal(answer.supported, true);
  assert.equal(answer.enabled, false);
  assert.equal(answer.credential_days, 30);
  assert.equal(lan.keyLabel(answer), "n° 56475aa7");
});

test("an offline blip is not remembered: the next mount asks again", async () => {
  lan.resetFeature();
  // lib/api.js retries a GET three times on a network failure (its own budget,
  // shared by every screen): every attempt fails here, not just the first.
  stubFetch({ fail: true });
  const first = await lan.lanFeature();
  assert.equal(first.supported, false);
  assert.equal(first.definitive, false);
  stubFetch();
  answers.push({ status: 200, body: { enabled: true, credential_days: 30, keys: [] } });
  const second = await lan.lanFeature();
  assert.equal(second.supported, true);
  assert.equal(calls.length, 1);
  assert.equal(calls.filter((c) => c.url === "/api/lan/status").length, 1);
});

test("each mutation is the route, the method and the body the server validates", async () => {
  stubFetch();
  answers.push({ status: 200, body: { id: "abc", name: "Bureau" } });
  await lan.renameDevice("abc", "  Bureau  ");
  answers.push({ status: 204 });
  await lan.revokeDevice("abc");
  answers.push({ status: 200, body: { user_id: "u1", name: "Alice", all: false, playlists: [] } });
  await lan.addFriend(" Alice ");
  answers.push({ status: 200, body: { user_id: "u1", all: true, playlists: [] } });
  await lan.setFriendScope("u1", { all: true });
  answers.push({ status: 200, body: { user_id: "u1", all: false, playlists: [] } });
  await lan.setFriendScope("u1", { playlists: ["p1", "p2"] });
  answers.push({ status: 204 });
  await lan.removeFriend("u1");
  answers.push({ status: 200, body: { enabled: true, credential_days: 14 } });
  await lan.saveSettings({ credential_days: 14 });
  answers.push({ status: 200, body: { kid: "ffeeddccbbaa9988" } });
  await lan.renewKey();

  assert.deepEqual(
    calls.map((c) => `${c.method} ${c.url}`),
    [
      "PATCH /api/lan/devices/abc",
      "DELETE /api/lan/devices/abc",
      "POST /api/lan/friends",
      "PATCH /api/lan/friends/u1",
      "PATCH /api/lan/friends/u1",
      "DELETE /api/lan/friends/u1",
      "PUT /api/lan/settings",
      "POST /api/lan/rotate",
    ],
  );
  assert.deepEqual(calls[0].body, { name: "Bureau" });
  assert.deepEqual(calls[2].body, { name: "Alice" });
  assert.deepEqual(calls[3].body, { all: true });
  assert.deepEqual(calls[4].body, { playlists: ["p1", "p2"] });
  assert.deepEqual(calls[6].body, { credential_days: 14 });
});

test("a refusal keeps the server's French sentence and its code", async () => {
  stubFetch();
  answers.push({ status: 404, body: { error: "Aucun compte de ce nom.", code: "unknown_account" } });
  await assert.rejects(lan.addFriend("personne"), (err) => {
    assert.equal(err.status, 404);
    assert.equal(err.code, "unknown_account");
    // What the screen shows under the field: the same words as everywhere else.
    assert.equal(lan.cause(err), "HTTP 404 : Aucun compte de ce nom.");
    return true;
  });
});

test("a rate limit and an expired session are told apart", async () => {
  stubFetch();
  answers.push({ status: 429, body: { error: "Trop d'ajouts d'amis, réessayez dans une heure.", code: "rate_limited" } });
  await assert.rejects(lan.addFriend("Alice"), (err) => {
    assert.equal(err.status, 429);
    assert.match(lan.cause(err), /^HTTP 429 : Trop d'ajouts/);
    return true;
  });
  stubFetch();
  answers.push({ status: 401, body: {} });
  await assert.rejects(lan.devices(), (err) => {
    assert.equal(err.status, 401);
    return true;
  });
});

test("the friend picker costs ZERO requests once the library is cached", async () => {
  lan.resetPlaylists();
  stubFetch();
  // `api.peek` is the SPA's offline copy of /me/playlists (lib/apicache.js).
  const realPeek = api.peek;
  let peeks = 0;
  api.peek = async (path) => {
    peeks++;
    assert.equal(path, "/me/playlists");
    return { playlists: [{ id: "p1", title: "Route" }, { id: "p2", title: "Nuit" }], folders: [] };
  };
  try {
    const choices = await lan.playlistChoices();
    assert.deepEqual(choices, [
      { id: "p1", title: "Route" },
      { id: "p2", title: "Nuit" },
    ]);
    assert.equal(calls.length, 0);
    assert.equal(peeks, 1);
    // A second friend's picker reuses the list: still 0 requests, 0 peeks.
    await lan.playlistChoices();
    assert.equal(calls.length, 0);
    assert.equal(peeks, 1);

    // Cold cache: exactly one GET, then kept.
    lan.resetPlaylists();
    api.peek = async () => null;
    answers.push({ status: 200, body: { playlists: [{ id: "p9", title: "Seule" }] } });
    assert.deepEqual(await lan.playlistChoices(), [{ id: "p9", title: "Seule" }]);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, "/api/me/playlists");
  } finally {
    api.peek = realPeek;
  }
});
