import test from "node:test";
import assert from "node:assert/strict";

function memoryStorage() {
  const rows = new Map();
  return {
    getItem: (k) => (rows.has(k) ? rows.get(k) : null),
    setItem: (k, v) => rows.set(k, String(v)),
    removeItem: (k) => rows.delete(k),
    key: (i) => [...rows.keys()][i] ?? null,
    get clear() {
      return () => rows.clear();
    },
    get length() {
      return rows.size;
    },
  };
}

globalThis.localStorage = memoryStorage();
globalThis.sessionStorage = memoryStorage();
globalThis.localStorage.setItem("auth.user", JSON.stringify({ name: "a", admin: true }));
globalThis.localStorage.setItem(
  "player.session",
  JSON.stringify({ queue: [{ id: "t1" }], index: 0 })
);
globalThis.localStorage.setItem("search.recent", JSON.stringify(["orage secret"]));
globalThis.localStorage.setItem("flow.styles", JSON.stringify(["Techno"]));

const { activeOwner, accountOwner, ownerPrefix, scopedKey, switchAccount } = await import(
  "../src/lib/account.js"
);

const A = accountOwner({ name: "a" });
const B = accountOwner({ name: "b" });

const keysOf = () => {
  const out = [];
  for (let i = 0; i < globalThis.localStorage.length; i++) out.push(globalThis.localStorage.key(i));
  return out;
};

test("the remembered account owns its queue and recents", () => {
  assert.equal(activeOwner(), A);
  assert.equal(scopedKey("player.session"), ownerPrefix(A) + "player.session");
  assert.equal(
    JSON.parse(globalThis.localStorage.getItem(scopedKey("player.session"))).queue[0].id,
    "t1"
  );
  assert.equal(globalThis.localStorage.getItem("player.session"), null);
  assert.equal(scopedKey("player.session").includes(ownerPrefix(B)), false);
});

test("two accounts with the same name but different ids own different keys", () => {
  const first = accountOwner({ id: "11111111-1111-1111-1111-111111111111", name: "bob" });
  const second = accountOwner({ id: "22222222-2222-2222-2222-222222222222", name: "bob" });
  assert.notEqual(first, second);
  assert.ok(first.includes("11111111"));
  assert.ok(second.includes("22222222"));
  assert.ok(accountOwner({ name: "bob" }).endsWith("/bob"));
});

test("a switch deletes the account that left, not the one arriving", () => {
  globalThis.localStorage.setItem(scopedKey("recent.tracks"), JSON.stringify([{ id: "t9" }]));
  const out = switchAccount({ name: "b" });
  assert.equal(out.action, "reload");
  assert.equal(activeOwner(), B);
  assert.equal(
    keysOf().filter((k) => k.startsWith(ownerPrefix(A))).length,
    0
  );
  assert.equal(globalThis.localStorage.getItem("search.recent"), null);
  assert.equal(globalThis.localStorage.getItem("flow.styles"), null);
  assert.equal(scopedKey("player.session"), ownerPrefix(B) + "player.session");
  assert.equal(globalThis.localStorage.getItem(scopedKey("player.session")), null);
});

test("logging out keeps the login screen, the next sign-in restarts the page", () => {
  const out = switchAccount(null);
  assert.equal(out.action, "defer");
  assert.equal(activeOwner(), null);
  assert.equal(scopedKey("recent.tracks"), null);
  assert.equal(globalThis.localStorage.getItem("auth.user"), null);

  const back = switchAccount({ name: "c" });
  assert.equal(back.action, "reload");
  assert.equal(accountOwner({ name: "c" }), activeOwner());
});
