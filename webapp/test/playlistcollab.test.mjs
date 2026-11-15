// The collaborative-playlist rules the screen and the sheet decide on, without
// a browser: the role vocabulary, what a 409 says and which GET it asks for,
// the single-flight resync, and what the 30 s probe may do to the page.
//
// Run with `npm test` (node --test). The component's own state machine (the
// epoch queue, the optimistic rows) is exercised in Chromium by the bench; what
// is pinned here is the vocabulary and the decisions that were inline in two
// components and could not be tested.
import test from "node:test";
import assert from "node:assert/strict";

import {
  conflictOf,
  conflictNotice,
  isSharedPlaylist,
  memberError,
  probeDecision,
  resyncOnce,
  roleLabel,
} from "../src/lib/actions.js";

const conflict = (scope, extra = {}) => ({
  status: 409,
  message: "conflict",
  body: {
    error: "conflict",
    scope,
    version: 17,
    members_version: 4,
    last_edited_by: "Léa",
    last_edited_at: "2026-11-14T10:22:31",
    ...extra,
  },
});

test("the three roles speak the words of the sheet", () => {
  assert.equal(roleLabel("owner"), "Propriétaire");
  assert.equal(roleLabel("editor"), "Peut modifier");
  assert.equal(roleLabel("viewer"), "Lecture seule");
  assert.equal(roleLabel("admin"), "Administrateur");
  assert.equal(roleLabel(null), "");
  assert.equal(roleLabel("public"), "");
});

test("a playlist is shared exactly when its role is not owner", () => {
  const list = [
    { id: "1", role: "owner" },
    { id: "2", role: "editor" },
    { id: "3", role: "viewer" },
    { id: "4" },
    null,
  ];
  assert.deepEqual(list.filter(isSharedPlaylist).map((p) => p.id), ["2", "3"]);
  const owned = list.filter((p) => p && !isSharedPlaylist(p));
  assert.deepEqual(owned.map((p) => p.id), ["1", "4"]);
  // The two groups partition the list: no row can appear twice.
  assert.equal(owned.length + list.filter(isSharedPlaylist).length, list.filter(Boolean).length);
});

test("only a real version conflict is a conflict", () => {
  assert.equal(conflictOf(conflict("content")).scope, "content");
  assert.equal(conflictOf({ status: 409, body: { error: "owner" } }), null);
  assert.equal(conflictOf({ status: 409, body: { error: "playlist limit" } }), null);
  assert.equal(conflictOf({ status: 403, body: { error: "forbidden" } }), null);
  assert.equal(conflictOf({ status: 500, message: "boom" }), null);
  assert.equal(conflictOf(null), null);
});

test("a content conflict names who moved it; a members one does not", () => {
  assert.equal(conflictNotice(conflict("content").body), "Modifiée par Léa — rechargée.");
  assert.equal(
    conflictNotice(conflict("content", { last_edited_by: null }).body),
    "La playlist a changé entre-temps — rechargée."
  );
  // The name in a members conflict is the last CONTENT editor, not the person
  // who changed the roles: it must not be shown as the author of that change.
  assert.equal(
    conflictNotice(conflict("members").body),
    "La liste des collaborateurs a changé — rechargée."
  );
  assert.equal(conflictNotice(null), "La playlist a changé entre-temps — rechargée.");
});

test("a membership failure is explained in French, with the account the person typed", () => {
  const at = (status, body) => ({ status, body });
  assert.equal(
    memberError(at(404, { error: "unknown user" }), "bob"),
    "Aucun compte ne s'appelle « bob » sur ce serveur."
  );
  assert.equal(
    memberError(at(404, { error: "not found" }), "bob"),
    "Cette playlist ne vous est plus accessible."
  );
  assert.equal(memberError(at(400, { error: "self" })), "C'est votre propre compte.");
  assert.equal(memberError(at(400, { error: "invalid role" })), "Rôle invalide.");
  assert.equal(memberError(at(400, { error: "invalid username" })), "Indiquez un nom d'utilisateur.");
  assert.equal(
    memberError(at(403, { error: "forbidden" })),
    "Vous n'avez plus le droit de gérer les collaborateurs."
  );
  assert.equal(
    memberError(at(409, { error: "owner" })),
    "Cette personne est déjà propriétaire de la playlist."
  );
  assert.equal(
    memberError(at(409, { error: "member limit" })),
    "Cette playlist a atteint sa limite de 100 collaborateurs."
  );
  assert.equal(
    memberError(conflict("members")),
    "La liste des collaborateurs a changé — rechargée."
  );
  assert.equal(
    memberError(at(429, { error: "rate limited", retry_after: 42 })),
    "Trop d'invitations d'un coup — réessayez dans 42 s."
  );
  assert.equal(
    memberError(at(429, { error: "rate limited" })),
    "Trop d'invitations d'un coup — réessayez dans 1 s."
  );
  assert.equal(
    memberError({ status: 0, offline: true }),
    "Hors ligne : la modification n'a pas été enregistrée."
  );
  assert.equal(memberError(at(500, { error: "boom" })), "Impossible de modifier les collaborateurs.");
  assert.equal(memberError(null), "");
});

test("two 409s during one reload share ONE GET", async () => {
  let runs = 0;
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  const run = async () => {
    runs++;
    await gate;
    return { members_version: 5 };
  };
  const first = resyncOnce("pl-1:members", run);
  const second = resyncOnce("pl-1:members", run);
  assert.equal(first, second, "the second caller waits for the same answer");
  assert.equal(runs, 0, "the reload starts on a microtask, not synchronously");
  release();
  assert.deepEqual(await first, { members_version: 5 });
  assert.equal(runs, 1);
  // Settled: a LATER conflict is a new situation and asks again.
  assert.deepEqual(await resyncOnce("pl-1:members", async () => ({ members_version: 6 })), {
    members_version: 6,
  });
  assert.equal(runs, 1);
});

test("a failed reload frees the slot instead of pinning the failure", async () => {
  let runs = 0;
  await assert.rejects(
    resyncOnce("pl-2:content", async () => {
      runs++;
      throw { status: 404, message: "not found" };
    })
  );
  await resyncOnce("pl-2:content", async () => {
    runs++;
    return { playlist: { version: 9 } };
  });
  assert.equal(runs, 2);
});

test("the probe reloads only when nobody is editing, otherwise it offers the pill", () => {
  const seen = { version: 17, members_version: 4 };
  const moved = { version: 18, members_version: 4 };
  assert.deepEqual(probeDecision(seen, seen), { members: false, reload: false, stale: false });
  assert.deepEqual(probeDecision(seen, moved), { members: false, reload: true, stale: false });
  assert.deepEqual(probeDecision(seen, moved, { editing: true }), {
    members: false,
    reload: false,
    stale: true,
  });
  assert.deepEqual(probeDecision(seen, moved, { busy: true }), {
    members: false,
    reload: false,
    stale: true,
  });
  // A members-only move is a small GET, never a reload of the tracklist.
  assert.deepEqual(probeDecision(seen, { version: 17, members_version: 5 }), {
    members: true,
    reload: false,
    stale: false,
  });
  assert.deepEqual(probeDecision(null, null), { members: false, reload: false, stale: false });
});
