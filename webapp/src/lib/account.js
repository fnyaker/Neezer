import { REMOTE } from "./remote/mode.js";

const SAVED = "auth.user";
const PREFIX = "neezer:";
const SWITCH = "neezer:account-switched";

const PERSONAL = [
  "player.session",
  "player.pos",
  "recent.tracks",
  "playlist.lastUsed",
  "podcast.progress",
];

const SHARED_PERSONAL = [
  "search.recent",
  "flow.session",
  "flow.energy",
  "flow.styles",
  "discover.level",
  "explorer.discover.level",
  "genre.recent",
];

function area(name) {
  try {
    const v = globalThis[name];
    return v && typeof v.getItem === "function" ? v : null;
  } catch {
    return null;
  }
}

function origin() {
  try {
    if (typeof location !== "undefined" && location && location.origin) return String(location.origin);
    const w = globalThis.window;
    if (w && w.location && w.location.origin) return String(w.location.origin);
  } catch {
    return "app";
  }
  return "app";
}

export function savedUser() {
  const ls = area("localStorage");
  if (!ls) return null;
  try {
    const raw = ls.getItem(SAVED);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

export function accountOwner(user) {
  if (!user || typeof user !== "object") return null;
  if (user.id != null && String(user.id)) return origin() + "/id:" + String(user.id);
  const name = user.name ? String(user.name) : "";
  return name ? origin() + "/" + name : null;
}

export function ownerPrefix(owner) {
  return PREFIX + owner + ":";
}

const bootOwner = accountOwner(savedUser());
let owner = bootOwner;
const listeners = new Set();

export function savedOwner() {
  return bootOwner;
}

export function activeOwner() {
  return owner;
}

export function scopedKey(key) {
  return owner && !REMOTE ? ownerPrefix(owner) + key : null;
}

export function onOwnerChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function writeSaved(user) {
  const ls = area("localStorage");
  if (!ls) return;
  try {
    ls.setItem(SAVED, JSON.stringify(user));
  } catch {}
}

function removeSaved() {
  const ls = area("localStorage");
  if (!ls) return;
  try {
    ls.removeItem(SAVED);
  } catch {}
}

function flag(name) {
  const st = area("sessionStorage") || area("localStorage");
  if (!st) return null;
  try {
    return st.getItem(name);
  } catch {
    return null;
  }
}

function setFlag(name, on) {
  const st = area("sessionStorage") || area("localStorage");
  if (!st) return;
  try {
    if (on) st.setItem(name, "1");
    else st.removeItem(name);
  } catch {}
}

function adoptLegacy() {
  const ls = area("localStorage");
  if (!ls || !owner) return;
  for (const key of PERSONAL) {
    try {
      const scoped = ownerPrefix(owner) + key;
      if (ls.getItem(scoped) !== null) continue;
      const legacy = ls.getItem(key);
      if (legacy === null) continue;
      ls.setItem(scoped, legacy);
      ls.removeItem(key);
    } catch {}
  }
}

function purgeLocal(ownerName) {
  const ls = area("localStorage");
  if (!ls || !ownerName) return;
  const prefix = ownerPrefix(ownerName);
  const doomed = [];
  try {
    const n = Number(ls.length) || 0;
    for (let i = 0; i < n; i++) {
      const key = ls.key(i);
      if (key && key.startsWith(prefix)) doomed.push(key);
    }
    for (const key of doomed) ls.removeItem(key);
    for (const key of PERSONAL) ls.removeItem(key);
    for (const key of SHARED_PERSONAL) ls.removeItem(key);
  } catch {}
}

export function switchAccount(user) {
  const next = accountOwner(user);
  const prev = owner;
  if (next === prev) {
    if (user) writeSaved(user);
    return { prev, next, action: null };
  }
  owner = next;
  if (user) writeSaved(user);
  else removeSaved();
  if (prev) purgeLocal(prev);
  if (next) adoptLegacy();
  const pending = flag(SWITCH) === "1";
  let action = null;
  if (prev && next) action = "reload";
  else if (prev && !next) action = "defer";
  else if (!prev && next && pending) action = "reload";
  if (action === "defer") setFlag(SWITCH, true);
  if (action === "reload") setFlag(SWITCH, false);
  for (const fn of listeners) {
    try {
      fn(next, prev);
    } catch {}
  }
  return { prev, next, action };
}

adoptLegacy();
