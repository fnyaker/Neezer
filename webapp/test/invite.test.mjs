import test from "node:test";
import assert from "node:assert/strict";

const I = await import("../src/lib/invite.js");

const TOKEN = "a".repeat(22) + "." + "b".repeat(24);

function memory(seed = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => map.set(key, String(value)),
    removeItem: (key) => map.delete(key),
    dump: () => Object.fromEntries(map),
  };
}

test("foldName replie sans toucher aux bords pendant la frappe", () => {
  assert.equal(I.foldName(" Élodie "), " elodie ");
  assert.equal(I.foldName("ÉLODIE"), "elodie");
});

test("foldUsername replie la casse, les accents et les bords", () => {
  assert.equal(I.foldUsername("Élodie"), "elodie");
  assert.equal(I.foldUsername("ALICE"), "alice");
  assert.equal(I.foldUsername("Jean-Luc"), "jean-luc");
  assert.equal(I.foldUsername("  Zoé  "), "zoe");
  assert.equal(I.foldUsername("e\u0301"), "e");
  assert.equal(I.foldUsername(null), "");
  assert.equal(I.foldUsername(undefined), "");
});

test("usernameProblem accepte la forme 3–32 et refuse le reste", () => {
  assert.equal(I.usernameProblem("elodie"), null);
  assert.equal(I.usernameProblem("Élodie"), null);
  assert.equal(I.usernameProblem("a.b-c_d"), null);
  assert.equal(I.usernameProblem("abc"), null);
  assert.equal(I.usernameProblem("a".repeat(32)), null);
  assert.equal(I.usernameProblem(""), "too_short");
  assert.equal(I.usernameProblem("ab"), "too_short");
  assert.equal(I.usernameProblem("a".repeat(33)), "too_long");
  assert.equal(I.usernameProblem("elodie!"), "charset");
  assert.equal(I.usernameProblem("elodie smith"), "charset");
  assert.equal(I.usernameProblem("@elodie"), "charset");
  assert.equal(I.usernameProblem("-elodie"), "edge");
  assert.equal(I.usernameProblem("elodie."), "edge");
  assert.equal(I.usernameProblem("_elodie"), "edge");
});

test("refusedCharacters ne signale que ce qui reste après repli", () => {
  assert.deepEqual(I.refusedCharacters("Élodie"), []);
  assert.deepEqual(I.refusedCharacters("élodie @"), [" ", "@"]);
  assert.deepEqual(I.refusedCharacters("a!a?"), ["!", "?"]);
});

test("usernameMessage explique chaque refus", () => {
  assert.equal(I.usernameMessage("elodie"), "");
  assert.equal(I.usernameMessage("ab"), "Il faut au moins 3 caractères.");
  assert.equal(I.usernameMessage("a".repeat(33)), "32 caractères au plus.");
  assert.equal(
    I.usernameMessage("-elodie"),
    "Le nom doit commencer et finir par une lettre ou un chiffre."
  );
  assert.match(
    I.usernameMessage("élodie @"),
    /^Ces caractères ne sont pas autorisés : l'espace, « @ »\./
  );
  assert.match(I.usernameMessage("elodie@"), /^Ce caractère n'est pas autorisé : « @ »\./);
  assert.match(I.usernameMessage("elodie@"), /a–z, 0–9/);
});

test("passwordProblem applique 12–128 sans imposer de composition", () => {
  assert.equal(I.passwordProblem("court"), "short");
  assert.equal(I.passwordProblem("a".repeat(11)), "short");
  assert.equal(I.passwordProblem("a".repeat(12)), null);
  assert.equal(I.passwordProblem("une phrase de passe simple"), null);
  assert.equal(I.passwordProblem("a".repeat(129)), "long");
  assert.equal(I.passwordProblem("elodieelodie", "elodie"), "same_as_username");
  assert.equal(I.passwordProblem("xxeidoleeidole", "elodie"), "same_as_username");
  assert.equal(I.passwordProblem("ÉLODIEELODIE", "elodie"), "same_as_username");
});

test("passwordMessage reprend la règle du serveur", () => {
  assert.equal(I.passwordMessage("court"), "12 caractères minimum.");
  assert.equal(I.passwordMessage("a".repeat(129)), "128 caractères au plus.");
  assert.equal(
    I.passwordMessage("elodieelodie", "elodie"),
    "Le mot de passe ne peut pas contenir ton nom d'utilisateur."
  );
  assert.equal(I.passwordMessage("une phrase de passe simple"), "");
});

test("passwordScore monte avec la longueur et ne note pas la composition", () => {
  assert.equal(I.passwordScore(""), 0);
  assert.equal(I.passwordScore("court"), 1);
  const base = "abcdefghijklmnopqrstuvwxyz0123456789";
  let previous = 0;
  for (let length = 0; length <= 36; length++) {
    const score = I.passwordScore(base.slice(0, length));
    assert.ok(score >= previous, `score ${score} < ${previous} à ${length}`);
    previous = score;
  }
  assert.equal(I.passwordScore("abcdefghijkl"), 2);
  assert.equal(I.passwordScore("abcdefghijklmnop"), 3);
  assert.equal(I.passwordScore("abcdefghijklmnopqrstuvwx"), 4);
  assert.ok(I.passwordScore("une phrase de passe tres simple") >= 3);
  assert.equal(I.passwordScore("aaaaaaaaaaaaaaaaaaaa"), 2);
  assert.equal(I.passwordScore("elodieelodieelodie", "elodie"), 2);
});

test("passwordLabel donne un mot à chaque cran", () => {
  assert.equal(I.passwordLabel(""), "Trop court");
  assert.equal(I.passwordLabel("court"), "Trop court");
  assert.equal(I.passwordLabel("abcdefghijkl"), "Correct");
  assert.equal(I.passwordLabel("abcdefghijklmnop"), "Bon");
  assert.equal(I.passwordLabel("abcdefghijklmnopqrstuvwx"), "Très bon");
});

test("tokenShapeOk n'accepte que le format id.mac du serveur", () => {
  assert.equal(I.tokenShapeOk(TOKEN), true);
  assert.equal(I.tokenShapeOk("a".repeat(22)), false);
  assert.equal(I.tokenShapeOk("a".repeat(32)), false);
  assert.equal(I.tokenShapeOk("a".repeat(21) + "." + "b".repeat(24)), false);
  assert.equal(I.tokenShapeOk("a".repeat(22) + "." + "b".repeat(23)), false);
  assert.equal(I.tokenShapeOk(TOKEN + "\n"), false);
  assert.equal(I.tokenShapeOk(""), false);
  assert.equal(I.tokenShapeOk(null), false);
});

test("expiryLabel date le lien sans mentir", () => {
  const text = I.expiryLabel("2026-11-21T18:00:00");
  assert.match(text, /^Expire le /);
  assert.match(text, /2026/);
  assert.match(text, /21/);
  assert.equal(I.expiryLabel(""), "");
  assert.equal(I.expiryLabel(null), "");
  assert.equal(I.expiryLabel("pas une date"), "");
});

test("landingFor mène à l'onboarding d'un compte neuf, à l'accueil sinon", () => {
  assert.equal(I.landingFor({ onboarding: "artists" }), "/onboarding");
  assert.equal(I.landingFor({ onboarding: "accounts" }), "/onboarding");
  assert.equal(I.landingFor({ onboarding: "done" }), "/");
  assert.equal(I.landingFor({}), "/");
  assert.equal(I.landingFor(null), "/");
});

test("newRequestId produit des clés stables et bornées", () => {
  const seen = new Set();
  for (let i = 0; i < 50; i++) {
    const id = I.newRequestId();
    assert.equal(typeof id, "string");
    assert.ok(id.length > 0 && id.length <= 128);
    assert.match(id, /^[A-Za-z0-9-]+$/);
    assert.ok(!seen.has(id));
    seen.add(id);
  }
});

test("le brouillon garde le jeton, le nom et la clé — jamais le mot de passe", () => {
  const storage = memory();
  const saved = I.rememberDraft(
    { token: TOKEN, name: "elodie", request_id: "cle-1", password: "un secret" },
    storage
  );
  assert.equal(saved, true);
  const raw = JSON.stringify(storage.dump());
  assert.ok(raw.includes(TOKEN));
  assert.ok(raw.includes("cle-1"));
  assert.ok(!raw.includes("un secret"));
  assert.deepEqual(I.readDraft(TOKEN, storage), { request_id: "cle-1", name: "elodie" });
});

test("readDraft ignore un autre jeton, un brouillon périmé ou illisible", () => {
  const storage = memory();
  I.rememberDraft({ token: TOKEN, name: "elodie", request_id: "cle-1" }, storage);
  assert.equal(I.readDraft("c".repeat(22) + "." + "d".repeat(24), storage), null);

  const realNow = Date.now;
  try {
    Date.now = () => realNow() + I.DRAFT_TTL + 1;
    assert.equal(I.readDraft(TOKEN, storage), null);
  } finally {
    Date.now = realNow;
  }

  const broken = memory({ [I.DRAFT_KEY]: "{pas du json" });
  assert.equal(I.readDraft(TOKEN, broken), null);
  const unkeyed = memory({ [I.DRAFT_KEY]: JSON.stringify({ token: TOKEN, name: "x" }) });
  assert.equal(I.readDraft(TOKEN, unkeyed), null);
  assert.equal(I.rememberDraft({ token: TOKEN }, storage), false);
  assert.equal(I.rememberDraft({ token: TOKEN, request_id: "x" }, null), false);
});

test("clearDraft efface le brouillon", () => {
  const storage = memory();
  I.rememberDraft({ token: TOKEN, name: "elodie", request_id: "cle-1" }, storage);
  I.clearDraft(storage);
  assert.equal(storage.dump()[I.DRAFT_KEY], undefined);
  assert.equal(I.readDraft(TOKEN, storage), null);
});
