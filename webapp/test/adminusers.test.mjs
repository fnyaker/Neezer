import test from "node:test";
import assert from "node:assert/strict";

import {
  relativeTime,
  expiryLabel,
  absoluteTime,
  shortDate,
  inviteUrl,
  inviteRow,
  accountRow,
  stateLabel,
  stateKey,
  roleLabel,
  lastSeenLabel,
  mergeUsers,
  filterRows,
  guardMessage,
  countLabel,
  clockTime,
} from "../src/lib/adminusers.js";

const NOW = Date.UTC(2026, 10, 14, 12, 0, 0);

test("relativeTime speaks French from a fixed now", () => {
  assert.equal(relativeTime(null, NOW), "—");
  assert.equal(relativeTime(new Date(NOW - 30_000).toISOString(), NOW), "à l'instant");
  assert.equal(relativeTime(new Date(NOW - 5 * 60_000).toISOString(), NOW), "il y a 5 min");
  assert.equal(relativeTime(new Date(NOW - 3 * 3_600_000).toISOString(), NOW), "il y a 3 h");
  assert.equal(relativeTime(new Date(NOW - 2 * 86_400_000).toISOString(), NOW), "il y a 2 j");
  assert.equal(relativeTime(new Date(NOW - 75 * 86_400_000).toISOString(), NOW), "il y a 2 mois");
  assert.equal(relativeTime(new Date(NOW - 800 * 86_400_000).toISOString(), NOW), "il y a 2 ans");
});

test("expiryLabel counts the time left, then says expired", () => {
  assert.equal(expiryLabel(new Date(NOW + 30 * 60_000).toISOString(), NOW), "expire dans 30 min");
  assert.equal(expiryLabel(new Date(NOW + 6 * 3_600_000).toISOString(), NOW), "expire dans 6 h");
  assert.equal(expiryLabel(new Date(NOW + 6 * 86_400_000).toISOString(), NOW), "expire dans 6 j");
  assert.equal(expiryLabel(new Date(NOW - 1).toISOString(), NOW), "expirée");
  assert.equal(expiryLabel(null, NOW), "—");
});

test("dates are readable and never invent a value", () => {
  assert.match(absoluteTime("2026-11-14T10:22:00Z"), /2026/);
  assert.match(absoluteTime("2026-11-14T10:22:00Z"), / à /);
  assert.match(shortDate("2026-11-14T10:22:00Z"), /2026/);
  assert.equal(absoluteTime(null), "—");
  assert.equal(shortDate("not-a-date"), "—");
  assert.match(clockTime(new Date(NOW).toISOString()), /^\d{2}:\d{2}$/);
});

test("inviteUrl uses the deployment prefix and hides the token in the fragment", () => {
  const loc = {
    origin: "https://neezer.example",
    pathname: "/app/",
    search: "?svc=deezer",
    hash: "#/settings/users",
  };
  assert.equal(
    inviteUrl("AbC123.dEf456", loc),
    "https://neezer.example/app/#/invite/AbC123.dEf456",
  );
  assert.equal(inviteUrl("", loc), "");
  assert.equal(
    inviteUrl("tok", { origin: "https://x", pathname: "/" }),
    "https://x/#/invite/tok",
  );
});

test("an invite row carries no secret and names its state", () => {
  const row = inviteRow(
    {
      id: "AbC",
      note: "  pour Léa ",
      created: new Date(NOW - 3_600_000).toISOString(),
      expires: new Date(NOW + 6 * 86_400_000).toISOString(),
      state: "pending",
      created_by: "alice",
      used_by: null,
      used_at: null,
    },
    NOW,
  );
  assert.equal(row.kind, "invite");
  assert.equal(row.name, "pour Léa");
  assert.equal(stateLabel(row, NOW), "Invité");
  assert.equal(stateKey(row, NOW), "invite");
  assert.equal(roleLabel(row), "Membre");
  assert.equal(lastSeenLabel(row, NOW), `Expire le ${shortDate(row.expires)}`);
  for (const key of ["token", "token_hash", "mac"]) assert.equal(key in row, false);
});

test("invite states follow used > revoked > expired > pending", () => {
  const base = {
    id: "i",
    note: "x",
    created: new Date(NOW - 10 * 86_400_000).toISOString(),
    expires: new Date(NOW - 86_400_000).toISOString(),
    created_by: null,
  };
  assert.equal(stateLabel(inviteRow({ ...base, state: "used" }, NOW), NOW), "Utilisée");
  assert.equal(stateLabel(inviteRow({ ...base, state: "revoked" }, NOW), NOW), "Révoquée");
  assert.equal(stateLabel(inviteRow({ ...base, state: "pending" }, NOW), NOW), "Expirée");
  assert.equal(stateKey(inviteRow({ ...base, state: "pending" }, NOW), NOW), "expired");
});

test("account rows have stable booleans and labels", () => {
  const row = accountRow({ id: "1", name: "Alice", admin: 1, disabled: 0 });
  assert.equal(row.kind, "user");
  assert.equal(row.admin, true);
  assert.equal(row.disabled, false);
  assert.equal(stateLabel(row), "Actif");
  assert.equal(roleLabel(row), "Administrateur");
  assert.equal(lastSeenLabel({ ...row, last_login: null }, NOW), "—");
  assert.equal(stateLabel({ ...row, admin: false, disabled: true }), "Désactivé");
});

test("keyset pages append, dedupe and never mix two searches", () => {
  const first = mergeUsers(
    null,
    {
      users: [
        { id: "1", name: "Alice", admin: false, disabled: false },
        { id: "2", name: "Bob", admin: false, disabled: true },
      ],
      next: "2",
      total: 3,
    },
    false,
  );
  assert.deepEqual(first.rows.map((row) => row.name), ["Alice", "Bob"]);
  assert.equal(first.next, "2");
  assert.equal(first.total, 3);

  const second = mergeUsers(
    first,
    {
      users: [
        { id: "2", name: "Bob", admin: false, disabled: true },
        { id: "3", name: "Camille", admin: false, disabled: false },
      ],
      next: null,
      total: 3,
    },
    true,
  );
  assert.deepEqual(second.rows.map((row) => row.id), ["1", "2", "3"]);
  assert.equal(second.next, null);
  assert.equal(second.total, 3);

  const other = mergeUsers(
    second,
    { users: [{ id: "9", name: "Dora", admin: false, disabled: false }], next: null, total: 1 },
    false,
  );
  assert.deepEqual(other.rows.map((row) => row.name), ["Dora"]);
  assert.equal(other.total, 1);
});

test("state filters keep accounts and invitations apart", () => {
  const rows = [
    accountRow({ id: "1", name: "Alice", admin: false, disabled: false }),
    accountRow({ id: "2", name: "Bob", admin: false, disabled: true }),
    inviteRow(
      {
        id: "i",
        note: "Léa",
        state: "pending",
        created: new Date(NOW - 3_600_000).toISOString(),
        expires: new Date(NOW + 86_400_000).toISOString(),
      },
      NOW,
    ),
  ];
  assert.deepEqual(filterRows(rows, "all").map((row) => row.name), ["Alice", "Bob", "Léa"]);
  assert.deepEqual(filterRows(rows, "active").map((row) => row.name), ["Alice"]);
  assert.deepEqual(filterRows(rows, "disabled").map((row) => row.name), ["Bob"]);
  assert.deepEqual(filterRows(rows, "invited").map((row) => row.name), ["Léa"]);
  assert.deepEqual(filterRows(rows, "invited", "lea").map((row) => row.name), ["Léa"]);
  assert.deepEqual(filterRows(rows, "all", "lea").map((row) => row.name), ["Alice", "Bob", "Léa"]);
});

test("server refusals become French administrative sentences", () => {
  assert.equal(
    guardMessage({ status: 409, body: { code: "last_admin", error: "x" } }),
    "Impossible : c'est le dernier administrateur actif.",
  );
  assert.equal(
    guardMessage({ status: 403, body: { code: "self_delete", error: "x" } }),
    "Vous ne pouvez pas supprimer votre propre compte.",
  );
  assert.match(
    guardMessage({ status: 409, body: { code: "server_account_holder", error: "x" } }),
    /titulaire/,
  );
  assert.equal(
    guardMessage({ status: 403, body: { code: "forbidden", error: "x" } }),
    "Votre compte n'a plus les droits d'administration.",
  );
  assert.equal(
    guardMessage({ status: 409, body: { code: "invite_used", error: "x" } }),
    "Cette invitation a déjà servi.",
  );
  assert.match(
    guardMessage({ status: 429, body: { code: "rate_limited", reason: "invite_cap" } }),
    /Trop d'invitations en attente/,
  );
  assert.equal(
    guardMessage({ status: 0, message: "network", offline: true }),
    "Le serveur ne répond pas (réseau).",
  );
  assert.equal(guardMessage({ status: 400, body: { error: "Note trop longue." } }), "Note trop longue.");
  assert.equal(guardMessage(null), "Cette action a échoué.");
});

test("counts read naturally in French", () => {
  assert.equal(countLabel(0, "compte"), "0 compte");
  assert.equal(countLabel(1, "compte"), "1 compte");
  assert.equal(countLabel(2, "compte"), "2 comptes");
  assert.equal(countLabel(1, "fichier", "fichiers"), "1 fichier");
});
