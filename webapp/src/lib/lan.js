// Réglages → « Appareils et amis »: the server's half of the LAN exchange
// (docs/agents/lan-nodes.md, *Server*, lot LN8).
//
// The SPA is NOT a node — a browser can neither browse mDNS nor speak raw QUIC
// — so nothing here ever talks to a peer. What is configured here does reach
// the devices: my enrolled devices (rename, revoke), my trusted friends and
// what each of them may fetch, and, for an administrator, the feature switch
// and the lifetime of a device credential (CLAUDE.md rule 6).
//
// The routes are the note's: `GET /api/lan/status` (the feature probe),
// `GET /api/lan/devices` (a bare array), `PATCH|DELETE /api/lan/devices/<id>`,
// `GET /api/lan/friends` (`{shares_to, trusted_by}`), `POST /api/lan/friends`,
// `PATCH|DELETE /api/lan/friends/<account uuid>`, `GET|PUT /api/lan/settings`
// (admin only). A refusal is `{error, code}` and its French sentence is the
// `message` `failureCause` already turns into words, so a failed call reads
// the same here as everywhere else in Réglages.

import { api, failureCause } from "./api.js";
import { user } from "./stores.js";

const BASE = "/api";

// The server's own bounds (neezer/lan.py): one name is 1–64 characters and a
// credential lives 7–90 days. Mirrored, never invented: the server checks
// again, this only keeps an impossible value off the wire.
export const NAME_MAX = 64;
export const CREDENTIAL_DAYS_MIN = 7;
export const CREDENTIAL_DAYS_MAX = 90;
// `/me/playlists` — the friend picker reads it from the SPA's own offline cache
// (lib/apicache.js), so opening the picker costs no request once the library
// has been visited.
const PLAYLISTS = "/me/playlists";

// -- the wire ----------------------------------------------------------------

/** One JSON request. Returns the body, or null for the routes that answer 204. */
async function send(path, method, payload) {
  let res;
  try {
    res = await fetch(BASE + path, {
      method,
      credentials: "include",
      headers: { "Content-Type": "application/json" },
      body: payload === undefined ? undefined : JSON.stringify(payload),
    });
  } catch {
    throw { status: 0, message: "network", offline: true };
  }
  if (res.status === 401) {
    // The session ended under the request: the app's own behaviour (lib/api.js
    // does exactly this for its GETs), so the shell goes back to the login.
    user.set(null);
    throw { status: 401, message: "unauthorized" };
  }
  if (res.status === 204) return null;
  let data = null;
  try {
    data = await res.json();
  } catch {
    data = null;
  }
  if (!res.ok) {
    throw { status: res.status, message: (data && data.error) || res.statusText, code: data && data.code };
  }
  return data;
}

/** The French cause of a failed call, for a line under the control. */
export function cause(err) {
  return failureCause(err);
}

// -- the feature probe -------------------------------------------------------

let probeCache = null;

/**
 * Whether this server has the LAN exchange at all, and its state.
 *
 * `supported` is a 200 on `/api/lan/status`; an older server answers 404 (no
 * such route) and the whole section is hidden, which is rule 7. A definitive
 * answer is remembered for the session (one request per page load, never one
 * per Réglages visit); a network failure is NOT remembered — the section is
 * simply not shown, and the next mount tries again.
 */
export async function lanFeature({ refresh = false } = {}) {
  if (probeCache && !refresh) return probeCache;
  let answer;
  try {
    answer = await api.get("/lan/status");
  } catch (err) {
    const definitive = err && err.status && err.status !== 0;
    // 404: an older server. 401/403: nothing is lent to this session (a remote
    // control never sees the trust chain). Both mean "not here", and both are
    // cached; an offline blip is not.
    const state = { supported: false, definitive: !!definitive, error: err };
    if (definitive) probeCache = state;
    return state;
  }
  probeCache = {
    supported: true,
    definitive: true,
    enabled: answer?.enabled !== false,
    credential_days: answer?.credential_days ?? 30,
    keys: Array.isArray(answer?.keys) ? answer.keys : [],
    subject: answer?.subject || "",
  };
  return probeCache;
}

/** Tests only: forget the probe of this page load. */
export function resetFeature() {
  probeCache = null;
}

// -- devices -----------------------------------------------------------------

/** My enrolled devices, newest first (the server orders them). */
export function devices() {
  return api.get("/lan/devices");
}

/** Rename one device. `id` is the server's row id, or "self". */
export function renameDevice(id, name) {
  return send("/lan/devices/" + encodeURIComponent(id), "PATCH", { name: String(name).trim() });
}

/** Revoke one device ("self" allowed): its keys leave every peer's next list. */
export function revokeDevice(id) {
  return send("/lan/devices/" + encodeURIComponent(id), "DELETE");
}

// -- trusted friends ---------------------------------------------------------

/** `{shares_to: [...], trusted_by: [...]}` — see neezer/lan.py#friends_json. */
export function friends() {
  return api.get("/lan/friends");
}

/** Add a trusted friend by ACCOUNT name (folded server-side). */
export function addFriend(name) {
  return send("/lan/friends", "POST", { name: String(name).trim() });
}

/**
 * What one friend may fetch: `all` (everything I have locally) or the listed
 * playlists. Both fields are optional on the route; the screen sends the one
 * it changed, so a switch flick and a checkbox tick stay one small request.
 */
export function setFriendScope(userId, patch) {
  const body = {};
  if (patch && typeof patch.all === "boolean") body.all = patch.all;
  if (patch && Array.isArray(patch.playlists)) body.playlists = patch.playlists.map(String);
  return send("/lan/friends/" + encodeURIComponent(userId), "PATCH", body);
}

export function removeFriend(userId) {
  return send("/lan/friends/" + encodeURIComponent(userId), "DELETE");
}

/**
 * The person's playlists for the friend picker, from the offline cache when the
 * library has already been visited: `peek` reads IndexedDB with no network at
 * all, and only an empty cache costs one GET.
 */
let playlistCache = null;
export async function playlistChoices() {
  if (playlistCache) return playlistCache;
  let data = await api.peek(PLAYLISTS);
  if (data == null) data = await api.get(PLAYLISTS);
  const lists = (data?.playlists || [])
    .filter((p) => p && p.id)
    .map((p) => ({ id: String(p.id), title: String(p.title ?? "") }));
  playlistCache = lists;
  return lists;
}

/** Tests only: forget the picker's copy of the library. */
export function resetPlaylists() {
  playlistCache = null;
}

// -- administration (rule 6) -------------------------------------------------

/** `{enabled, credential_days}` — admin only; a non-admin must not ask. */
export function settings() {
  return api.get("/lan/settings");
}

/** Switch the feature, or set the lifetime of a device credential (7–90 days). */
export function saveSettings(fields) {
  return send("/lan/settings", "PUT", fields);
}

/**
 * Sign with a NEW server LAN key (the previous one stays known for 90 days, so
 * the credentials already issued keep verifying until they expire). Answers
 * `{kid}`; the caller re-reads the probe to show it.
 */
export function renewKey() {
  return send("/lan/rotate", "POST");
}

// -- words the screen says ---------------------------------------------------

/** The server's platform keys, in the person's words. */
export function platformLabel(platform) {
  switch (platform) {
    case "desktop":
      return "Ordinateur";
    case "android":
      return "Téléphone Android";
    case "ios":
      return "iPhone ou iPad";
    default:
      return platform ? String(platform) : "Appareil";
  }
}

/**
 * The glyph of one platform (Icon.svelte names): a screen for a computer, a
 * phone for a handset. `ios` does not say whether it is an iPhone or an iPad,
 * so it gets the phone too rather than a glyph that would guess wrong.
 */
export function platformIcon(platform) {
  return platform === "desktop" ? "monitor" : "smartphone";
}

/** "current" | "revoked" | "enrolled" — what a device row's state is called. */
export function deviceState(device) {
  if (!device) return "enrolled";
  if (device.revoked) return "revoked";
  if (device.current) return "current";
  return "enrolled";
}

function minutes(ms) {
  return Math.round(ms / 60000);
}

/** "vu à l'instant", "vu il y a 5 min", "vu il y a 3 j", "jamais vu". */
export function seenLabel(iso, now = Date.now()) {
  if (!iso) return "jamais vu";
  const at = Date.parse(iso);
  if (Number.isNaN(at)) return "date inconnue";
  const m = minutes(now - at);
  if (m < 1) return "vu à l'instant";
  if (m < 60) return `vu il y a ${m} min`;
  const h = Math.round(m / 60);
  if (h < 24) return `vu il y a ${h} h`;
  const d = Math.round(h / 24);
  if (d < 31) return `vu il y a ${d} j`;
  const months = Math.round(d / 30);
  if (months < 12) return `vu il y a ${months} mois`;
  return `vu il y a ${Math.round(months / 12)} an${Math.round(months / 12) > 1 ? "s" : ""}`;
}

/** The short date an enrolment happened on ("12 nov. 2026"). */
export function enrolledLabel(iso) {
  if (!iso) return "";
  const at = Date.parse(iso);
  if (Number.isNaN(at)) return "";
  return new Date(at).toLocaleDateString("fr-FR", { day: "numeric", month: "short", year: "numeric" });
}

/**
 * One device's second line, FACT by fact: the platform, when it enrolled, when
 * it was last seen. Each `kind` lets the screen decide where a fact is worth
 * its line — the enrolment date is the first to go in a narrow column — and
 * every segment is rendered unbreakable, so "vu il y a 1 min" is never cut.
 */
export function deviceSegments(device, now = Date.now()) {
  if (!device) return [];
  const out = [{ kind: "platform", text: platformLabel(device.platform) }];
  const since = enrolledLabel(device.created);
  if (since) out.push({ kind: "enrolled", text: `inscrit le ${since}` });
  out.push({
    kind: "seen",
    text: deviceState(device) === "revoked" ? "révoqué" : seenLabel(device.last_seen, now),
  });
  return out;
}

/** The same line as one string, for tests and any plain-text use. */
export function deviceDetail(device, now = Date.now()) {
  return deviceSegments(device, now)
    .map((fact) => fact.text)
    .join(" · ");
}

/** What a friend is allowed to see, in one line. */
export function sharedSummary(friend) {
  if (!friend) return "";
  if (friend.all) return "Tout partager";
  const n = (friend.playlists || []).length;
  if (n === 0) return "Aucune liste partagée";
  return `${n} liste${n > 1 ? "s" : ""} partagée${n > 1 ? "s" : ""}`;
}

/** "30 jours", "1 jour" — the credential's lifetime. */
export function daysLabel(days) {
  const n = Number(days);
  if (!Number.isFinite(n)) return "—";
  return `${n} jour${n > 1 ? "s" : ""}`;
}

/** The lifetime a typed value means, or null when it is outside 7–90. */
export function daysValue(raw) {
  const n = Number(raw);
  if (!Number.isInteger(n)) return null;
  if (n < CREDENTIAL_DAYS_MIN || n > CREDENTIAL_DAYS_MAX) return null;
  return n;
}

/** The current server key, short enough for a line ("n° 56475aa7"). */
export function keyLabel(feature) {
  const kid = feature && Array.isArray(feature.keys) ? feature.keys[0]?.kid : "";
  return kid ? `n° ${String(kid).slice(0, 8)}` : "";
}

// -- optimistic list edits (pure: the screen rolls back by keeping the old value)

/** The same list with one device renamed. */
export function renameDeviceIn(list, id, name) {
  return (list || []).map((d) => (d.id === id ? { ...d, name } : d));
}

/** The same list with one device removed (a revoke is optimistic). */
export function withoutDevice(list, id) {
  return (list || []).filter((d) => d.id !== id);
}

/** The same `friends` payload with one friend's scope replaced. */
export function withScope(payload, userId, patch) {
  if (!payload) return payload;
  const shares = (payload.shares_to || []).map((f) => {
    if (f.user_id !== userId) return f;
    const next = { ...f };
    if (typeof patch.all === "boolean") next.all = patch.all;
    if (Array.isArray(patch.playlists)) next.playlists = patch.playlists.map(String);
    return next;
  });
  return { ...payload, shares_to: shares };
}

/** The same `friends` payload without one friend (shares_to only). */
export function withoutFriend(payload, userId) {
  if (!payload) return payload;
  return { ...payload, shares_to: (payload.shares_to || []).filter((f) => f.user_id !== userId) };
}

/** The playlist ids a friend shares, as a list the picker can tick. */
export function sharedIds(friend) {
  return ((friend && friend.playlists) || []).map((p) => String(p.id));
}

/** One playlist ticked or unticked: the next `playlists` array. */
export function togglePlaylist(ids, id) {
  const key = String(id);
  const list = (ids || []).map(String);
  return list.includes(key) ? list.filter((x) => x !== key) : [...list, key];
}

/** The `{id, title}` pairs of the playlists a friend shares, in list order. */
export function sharedPairs(friend, choices) {
  const byId = new Map((choices || []).map((p) => [String(p.id), p.title]));
  return sharedIds(friend).map((id) => ({ id, title: byId.get(id) ?? "Liste supprimée" }));
}
