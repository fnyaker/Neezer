import test from "node:test";
import assert from "node:assert/strict";

const O = await import("../src/lib/onboarding.js");

function memory(seed = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => map.set(key, String(value)),
    removeItem: (key) => map.delete(key),
    dump: () => Object.fromEntries(map),
  };
}

const artist = (id, name, genre = null) => ({
  deezer_id: id,
  id,
  name,
  genres: genre ? [genre] : [],
});
const forest = Array.from({ length: 20 }, (_, i) =>
  artist(String(100 + i), `Artiste ${i}`, ["Pop", "Rap", "Rock"][i % 3])
);

test("idOf lit l'identifiant d'un artiste et passe une chaîne telle quelle", () => {
  assert.equal(O.idOf({ deezer_id: "42", id: "7" }), "42");
  assert.equal(O.idOf({ id: "7" }), "7");
  assert.equal(O.idOf({ entity_id: "u-1" }), "u-1");
  assert.equal(O.idOf("42"), "42");
  assert.equal(O.idOf(null), "");
  assert.equal(O.idOf(undefined), "");
});

test("genreOf lit le premier genre, replié, et accepte `genre`", () => {
  assert.equal(O.genreOf({ genres: ["Pop", "Rap"] }), "pop");
  assert.equal(O.genreOf({ genres: ["  Électro  "] }), "électro");
  assert.equal(O.genreOf({ genres: [] }), null);
  assert.equal(O.genreOf({ genre: "Rock" }), "rock");
  assert.equal(O.genreOf({}), null);
  assert.equal(O.genreOf(null), null);
});

test("insertSimilar place au plus deux proches juste après la tuile aimée", () => {
  const list = forest.slice(0, 6);
  const similar = [artist("900", "Proche A", "Pop"), artist("901", "Proche B", "Rap")];
  const out = O.insertSimilar(list, 2, similar);
  assert.equal(out.inserted, 2);
  assert.deepEqual(
    out.items.map((a) => a.deezer_id),
    ["100", "101", "102", "900", "901", "103", "104", "105"]
  );
  assert.equal(out.items[3].via, "related");
  assert.equal(out.items[3].parent, "102");
  assert.equal(list.length, 6, "l'entrée n'est pas modifiée");
});

test("insertSimilar ne dépasse jamais deux proches par like", () => {
  const list = forest.slice(0, 4);
  const similar = [
    artist("900", "A", "Pop"),
    artist("901", "B", "Rap"),
    artist("902", "C", "Rock"),
  ];
  const out = O.insertSimilar(list, 1, similar);
  assert.equal(out.inserted, 2);
  assert.deepEqual(
    out.items.map((a) => a.deezer_id),
    ["100", "101", "900", "901", "102", "103"]
  );
});

test("insertSimilar refuse l'artiste lui-même, les doublons, les déjà aimés", () => {
  const list = forest.slice(0, 4);
  const similar = [
    artist("101", "Lui-même"),
    artist("102", "Déjà dans la grille"),
    artist("900", "Déjà aimé"),
    artist("901", "Bon"),
  ];
  const out = O.insertSimilar(list, 0, similar, { liked: new Set(["900"]) });
  assert.equal(out.inserted, 1);
  assert.deepEqual(out.items.map((a) => a.deezer_id), ["100", "901", "101", "102", "103"]);
});

test("insertSimilar respecte le plafond de douze tuiles insérées", () => {
  const list = forest.slice(0, 4);
  const twelve = Array.from({ length: 12 }, (_, i) => ({
    ...artist(String(800 + i), `Inséré ${i}`, i % 2 ? "Pop" : "Rap"),
    via: "related",
    parent: "100",
  }));
  const full = [...list.slice(0, 2), ...twelve, ...list.slice(2)];
  assert.equal(O.budgetLeft(full), 0);
  const blocked = O.insertSimilar(full, 0, [artist("950", "Nouveau", "Rock")]);
  assert.equal(blocked.inserted, 0);
  assert.equal(blocked.items.length, full.length);

  const eleven = full.filter((a) => a.via === "related").slice(0, 11);
  const nearly = [...list.slice(0, 2), ...eleven, ...list.slice(2)];
  const one = O.insertSimilar(nearly, 0, [artist("951", "Un", "Rock"), artist("952", "Deux", "Pop")]);
  assert.equal(one.inserted, 1);
});

test("insertSimilar ne casse pas la fenêtre de diversité du serveur", () => {
  const list = Array.from({ length: 8 }, (_, i) => artist(String(200 + i), `P${i}`, "Pop"));
  const out = O.insertSimilar(list, 4, [artist("900", "Pop encore", "Pop")], {
    diversity: { window: 6, max_same_genre: 4 },
  });
  assert.equal(out.inserted, 0, "la fenêtre est déjà au plafond Pop");

  const mixed = [artist("300", "A", "Pop"), artist("301", "B", "Rap"), artist("302", "C", "Rock")];
  const ok = O.insertSimilar(mixed, 0, [artist("901", "Pop 2", "Pop")], {
    diversity: { window: 6, max_same_genre: 3 },
  });
  assert.equal(ok.inserted, 1);
});

test("insertSimilar alterne deux genres quand c'est possible", () => {
  const list = [artist("100", "A", "Pop"), artist("101", "B", "Rock")];
  const similar = [artist("900", "Pop 1", "Pop"), artist("901", "Pop 2", "Pop"), artist("902", "Rap", "Rap")];
  const out = O.insertSimilar(list, 0, similar, { diversity: { window: 12, max_same_genre: 4 } });
  assert.deepEqual(
    out.items.map((a) => a.deezer_id),
    ["100", "900", "902", "101"]
  );
});

test("insertSimilar ne touche pas une liste vide ou un index hors bornes", () => {
  assert.deepEqual(O.insertSimilar([], 0, [artist("1", "A")]), { items: [], inserted: 0 });
  const list = forest.slice(0, 2);
  assert.equal(O.insertSimilar(list, 9, [artist("900", "A")]).inserted, 0);
  assert.equal(O.insertSimilar(list, -1, [artist("900", "A")]).inserted, 0);
  assert.equal(O.insertSimilar(list, 0, []).inserted, 0);
});

test("removeSimilar retire le bloc du parent et garde une tuile aimée", () => {
  const base = O.insertSimilar(forest.slice(0, 3), 0, [
    artist("900", "A", "Pop"),
    artist("901", "B", "Rap"),
  ]).items;
  const out = O.removeSimilar(base, "100");
  assert.deepEqual(out.map((a) => a.deezer_id), ["100", "101", "102"]);

  const kept = O.removeSimilar(base, "100", new Set(["901"]));
  assert.deepEqual(kept.map((a) => a.deezer_id), ["100", "901", "101", "102"]);
  assert.equal(kept[1].via, "grid");
  assert.equal(kept[1].parent, null);

  const other = O.removeSimilar(base, "999");
  assert.equal(other.length, base.length);
});

test("reconcilePage met à jour sur place, ajoute les nouvelles et déduplique", () => {
  const current = [
    { deezer_id: "1", name: "Ancien nom" },
    { deezer_id: "9", name: "Inséré", via: "related", parent: "1" },
    { deezer_id: "2", name: "Deux" },
  ];
  const page = [
    { deezer_id: "1", name: "Nouveau nom", picture: "p" },
    { deezer_id: "3", name: "Trois" },
    { deezer_id: "1", name: "Doublon" },
  ];
  const out = O.reconcilePage(current, page);
  assert.deepEqual(
    out.map((a) => [a.deezer_id, a.name]),
    [
      ["1", "Nouveau nom"],
      ["9", "Inséré"],
      ["2", "Deux"],
      ["3", "Trois"],
    ]
  );
  assert.equal(out[1].via, "related");
  assert.equal(out[0].picture, "p");
});

test("cacheList borne ce qui part en sessionStorage", () => {
  const items = Array.from({ length: 200 }, (_, i) => artist(String(i), `A${i}`));
  assert.equal(O.cacheList(items, 120).length, 120);
  assert.equal(O.cacheList(items, 0).length, 200);
  assert.equal(O.cacheList(null).length, 0);
});

test("normalizeQuery et searchProblem appliquent le minimum de deux caractères", () => {
  assert.equal(O.normalizeQuery("  Aya   Nakamura  "), "Aya Nakamura");
  assert.equal(O.normalizeQuery(null), "");
  assert.equal(O.searchProblem(""), "too_short");
  assert.equal(O.searchProblem(" a "), "too_short");
  assert.equal(O.searchProblem("ay"), null);
  assert.equal(O.searchProblem("  Aya  "), null);
});

test("le compteur, l'aide et les messages de fin sont ceux du plan", () => {
  assert.equal(O.artistCountLabel(0), "0 artiste");
  assert.equal(O.artistCountLabel(1), "1 artiste");
  assert.equal(O.artistCountLabel(3), "3 artistes");
  assert.equal(O.artistCountLabel(1200), "1\u202f200 artistes");
  assert.equal(O.progressHint(0), "Choisissez-en au moins un.");
  assert.equal(O.progressHint(1), "Encore 2 pour un bon départ.");
  assert.equal(O.progressHint(3), "C'est un bon départ.");
  assert.equal(O.progressHint(5), "Vous pouvez continuer ou terminer.");
  assert.equal(O.finishBlocked({ liked: 0 }), "min");
  assert.equal(O.finishBlocked({ liked: 1, online: false }), "offline");
  assert.equal(O.finishBlocked({ liked: 1, pending: 1 }), "pending");
  assert.equal(O.finishBlocked({ liked: 1, queued: 1 }), "pending");
  assert.equal(O.finishBlocked({ liked: 1 }), null);
  assert.equal(O.finishMessage("min"), "Choisissez au moins un artiste pour terminer.");
  assert.equal(O.finishMessage("offline"), "Hors ligne : reconnectez-vous pour terminer.");
  assert.equal(O.finishMessage("pending"), "Enregistrement de vos choix…");
  assert.equal(O.finishMessage(null), "");
});

test("feedCause nomme la panne sans code brut", () => {
  assert.equal(O.feedCause("deezer-unreachable"), "Deezer ne répond pas.");
  assert.equal(O.feedCause("disabled"), "Le catalogue Deezer est désactivé sur ce serveur.");
  assert.equal(O.feedCause("held"), "Le catalogue est en pause après plusieurs échecs.");
  assert.equal(O.feedCause("absent"), "Aucun catalogue n'est configuré sur ce serveur.");
  assert.equal(O.feedCause("error"), "Le serveur n'a pas pu préparer la sélection.");
  assert.equal(O.feedCause(null), "La sélection n'a pas pu être préparée.");
});

test("« Passer pour l'instant » attend deux tentatives et un serveur vide", () => {
  const empty = { state: "unavailable", building: false };
  assert.equal(O.canPassArtists({ attempts: 0, feed: empty, count: 0 }), false);
  assert.equal(O.canPassArtists({ attempts: 1, feed: empty, count: 0 }), false);
  assert.equal(O.canPassArtists({ attempts: 2, feed: empty, count: 0 }), true);
  assert.equal(O.canPassArtists({ attempts: 2, feed: empty, count: 1 }), false);
  assert.equal(O.canPassArtists({ attempts: 2, feed: { state: "building", building: true }, count: 0 }), false);
  assert.equal(O.canPassArtists({ attempts: 2, feed: { state: "ready", building: false }, count: 0 }), false);
  assert.equal(O.canPassArtists({ attempts: 2, feed: null, count: 0 }), false);
});

test("l'étape de l'onboarding vient du serveur et se traduit en chemin", () => {
  assert.equal(O.stepFor("artists"), "artists");
  assert.equal(O.stepFor("account"), "account");
  assert.equal(O.stepFor("done"), "done");
  assert.equal(O.stepFor(undefined), "done");
  assert.equal(O.stepFor("n'importe quoi"), "done");
  assert.equal(O.pathForStep("artists"), "/onboarding/artists");
  assert.equal(O.pathForStep("account"), "/onboarding/accounts");
  assert.equal(O.pathForStep("done"), "/");
  assert.equal(O.stepFromPath("/onboarding"), "root");
  assert.equal(O.stepFromPath("/onboarding/"), "root");
  assert.equal(O.stepFromPath("/onboarding/artists"), "artists");
  assert.equal(O.stepFromPath("/onboarding/accounts/"), "accounts");
  assert.equal(O.stepFromPath("/onboarding/autre"), "unknown");
  assert.equal(O.stepFromPath("/library"), null);
});

test("la garde d'onboarding ne renvoie jamais vers la page courante", () => {
  assert.equal(O.onboardingTarget("artists", "/"), "/onboarding/artists");
  assert.equal(O.onboardingTarget("artists", "/library"), "/onboarding/artists");
  assert.equal(O.onboardingTarget("artists", "/onboarding"), "/onboarding/artists");
  assert.equal(O.onboardingTarget("artists", "/onboarding/artists"), null);
  assert.equal(O.onboardingTarget("artists", "/onboarding/accounts"), "/onboarding/artists");
  assert.equal(O.onboardingTarget("account", "/onboarding/accounts"), null);
  assert.equal(O.onboardingTarget("account", "/onboarding/artists"), null);
  assert.equal(O.onboardingTarget("account", "/onboarding"), "/onboarding/accounts");
  assert.equal(O.onboardingTarget("account", "/"), "/onboarding/accounts");
  assert.equal(O.onboardingTarget("done", "/onboarding/artists"), "/");
  assert.equal(O.onboardingTarget("done", "/onboarding"), "/");
  assert.equal(O.onboardingTarget("done", "/"), null);
  assert.equal(O.onboardingTarget(undefined, "/onboarding/accounts"), "/");
  for (const step of ["artists", "account", "done"]) {
    for (const path of ["/", "/library", "/onboarding", "/onboarding/artists", "/onboarding/accounts"]) {
      assert.notEqual(O.onboardingTarget(step, path), path, `${step} sur ${path}`);
    }
  }
});

test("le store de session porte une version, un âge et se relit", () => {
  const storage = memory();
  assert.equal(O.readStore("list", { storage }), null);
  assert.equal(O.writeStore("list", { items: [1] }, { storage, now: 1000 }), true);
  assert.equal(
    O.storeKey("list"),
    "nsOnboarding.list"
  );
  assert.deepEqual(O.readStore("list", { storage, now: 1500 }), { items: [1] });
  assert.equal(O.readStore("list", { storage, maxAge: 100, now: 5000 }), null);
  O.dropStore("list", { storage });
  assert.equal(O.readStore("list", { storage }), null);
  storage.setItem("nsOnboarding.broken", "{pas du json");
  assert.equal(O.readStore("broken", { storage }), null);
  storage.setItem("nsOnboarding.old", JSON.stringify({ v: 0, at: Date.now(), value: 1 }));
  assert.equal(O.readStore("old", { storage }), null);
  const refused = {
    getItem: () => null,
    setItem: () => {
      throw new Error("quota");
    },
    removeItem: () => {},
  };
  assert.equal(O.writeStore("x", 1, { storage: refused }), false);
});

test("la file des likes hors ligne garde un choix par artiste", () => {
  let queue = [];
  queue = O.enqueueLike(queue, "42", true);
  queue = O.enqueueLike(queue, "43", true);
  queue = O.enqueueLike(queue, "42", false);
  assert.deepEqual(queue, [
    { id: "43", liked: true },
    { id: "42", liked: false },
  ]);
  assert.deepEqual(O.enqueueLike(null, "", true), []);
});

test("les phases d'un import de playlist se disent en français", () => {
  assert.equal(O.importProgress(null).label, "");
  assert.equal(O.importProgress({ state: "reading" }).label, "Lecture de la playlist…");
  const matching = O.importProgress({ state: "matching", done: 42, total: 118 });
  assert.equal(matching.label, "Recherche des titres : 42 sur 118");
  assert.equal(matching.percent, 36);
  assert.equal(O.importProgress({ state: "saving" }).label, "Ajout à votre bibliothèque…");
  const done = O.importProgress({ state: "done", matched: 118, total: 120 });
  assert.equal(done.label, "118 titres trouvés sur 120.");
  assert.equal(done.percent, 100);
  const savedJob = O.importProgress({ state: "saved", matched: 1 });
  assert.equal(savedJob.label, "Ajouté : 1 titre.");
  assert.equal(savedJob.done, true);
  assert.equal(savedJob.saved, true);
  assert.equal(
    O.importProgress({ state: "error", error: "Cette playlist est vide" }).label,
    "L'import a échoué : Cette playlist est vide."
  );
  assert.equal(O.isImportRunning({ state: "matching" }), true);
  assert.equal(O.isImportRunning({ state: "done" }), false);
  assert.equal(O.isImportRunning(null), false);
});

test("les titres non trouvés se comptent et se listent sans requête", () => {
  const job = {
    state: "done",
    items: [
      { line: { title: "A" }, track: { id: "1" } },
      { line: { title: "B" }, track: null },
      { line: { title: "C" }, track: null },
    ],
  };
  assert.equal(O.unmatchedCount(job), 2);
  assert.deepEqual(O.unmatchedItems(job).map((it) => it.line.title), ["B", "C"]);
  assert.equal(O.unmatchedItems(job, 1).length, 1);
  assert.equal(O.unmatchedCount(null), 0);
  assert.equal(O.importNote({ state: "done", error: "Deezer ne répondait pas" }), "Deezer ne répondait pas");
  assert.equal(O.importNote({ state: "error", error: "x" }), "");
});

test("l'état d'un compte lié dit la vérité (jamais un import fini par défaut)", () => {
  assert.deepEqual(O.accountSummary(null), { text: "Non relié", tone: "idle" });
  assert.equal(O.accountSummary({ status: "ok" }).text, "Compte relié");
  assert.equal(
    O.accountSummary({ status: "ok", running: { running: true, phase: "import", done: 4, total: 10 } }).text,
    "Import en cours : 4 sur 10"
  );
  assert.equal(
    O.accountSummary({ status: "ok", running: { running: true, phase: "connect" } }).text,
    "Connexion au compte…"
  );
  assert.equal(
    O.accountSummary({ status: "auth" }).text,
    "Le compte refuse la connexion : reliez-le à nouveau."
  );
  assert.equal(
    O.accountSummary({ status: "network" }).text,
    "Le fournisseur ne répond pas : l'import reprendra plus tard."
  );
  assert.equal(
    O.accountSummary({ status: "ok", last: { added: { artists: 42, tracks: 118 } } }).text,
    "Dernier import : 42 artistes, 118 titres."
  );
  assert.equal(
    O.accountSummary({ status: "ok", last: { added: { artists: 1 } } }).text,
    "Dernier import : 1 artiste."
  );
  assert.equal(O.accountSummary({ status: "ok", last: { added: {} } }).text, "Dernier import : rien de nouveau.");
  assert.equal(
    O.accountSummary({ status: "ok", last: { added: {}, reason: "auth" } }).tone,
    "error"
  );
});

test("les échecs de liaison et d'import gardent la cause du serveur", () => {
  assert.equal(O.linkFailure({ body: { error: "Deezer refuse cet ARL." } }), "Deezer refuse cet ARL.");
  assert.equal(O.linkFailure({ status: 429 }), "Trop de tentatives. Réessayez dans quelques minutes.");
  assert.equal(O.linkFailure({ offline: true }), "Vous êtes hors ligne : la liaison attend le retour du réseau.");
  assert.equal(O.linkFailure({ status: 503 }), "Le fournisseur ne répond pas. Réessayez dans un instant.");
  assert.equal(O.linkFailure({}), "La liaison a échoué. Réessayez dans un instant.");
  assert.equal(O.importFailure({ body: { error: "Lien non reconnu" } }), "Lien non reconnu");
  assert.equal(O.importFailure({ status: 429 }), "Un import est déjà en cours. Attendez qu'il finisse.");
  assert.equal(O.saveFailure({ body: { error: "Aucun titre à enregistrer" } }), "Aucun titre à enregistrer");
  assert.equal(O.saveFailure({ status: 400 }), "Aucun titre de cet import n'est disponible.");
});
