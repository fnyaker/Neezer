// The web bughunt's shared rules (plan-bughunt §4, §8): the closed screen table,
// the outcome vocabulary, the fingerprint, the sibling dedup, the destructive
// list, and the two writers (JSON registry + Markdown report).
//
// One rule per exported name, in the order the plan states them. Nothing here
// knows about a browser session: run.mjs and stress.mjs own the page.
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve as resolvePath } from "node:path";
import { fileURLToPath } from "node:url";

/** The repository root, from this file's place in webapp/test/bughunt. */
export const REPO = resolvePath(dirname(fileURLToPath(import.meta.url)), "../../..");

// ---------------------------------------------------------------------------
// §8.2 — the closed list of screens. One line per route INSTANCE and per
// Settings section. `ready` is a CSS/Playwright selector that only matches once
// that screen (or that section) has painted; `openScreen` waits on it.
//
// {artist} {album} {big} {writer} {q} {show} {year} are filled by idsInto from
// the bench server's own READY payload (never guessed).
// ---------------------------------------------------------------------------
export const SCREENS = [
  { hash: "#/", name: "Accueil", ready: 'main [role="radiogroup"]' },
  { hash: "#/search", name: "Rechercher", ready: "main [role=searchbox], main input[type=search]" },
  { hash: "#/search/{q}", name: "Rechercher — résultats", ready: "main [role=tab]" },
  { hash: "#/library", name: "Bibliothèque", ready: "main [role=tablist]" },
  // `button`/`select`, not `[role=button]`: a NATIVE element carries its role
  // implicitly, and the attribute selector never matches it (measured: the
  // Titres tab was declared unreachable because of it).
  { hash: "#/library/artists", name: "Bibliothèque — Artistes", ready: "main [role=tabpanel] input[type=search], main [role=tabpanel] button" },
  { hash: "#/library/albums", name: "Bibliothèque — Albums", ready: "main [role=tabpanel] button, main [role=tabpanel] select" },
  { hash: "#/library/tracks", name: "Bibliothèque — Titres", ready: "main [role=tabpanel] button, main [role=tabpanel] table" },
  { hash: "#/artist/{artist}", name: "Artiste", ready: "main h1" },
  { hash: "#/album/{album}", name: "Album", ready: "main h1" },
  { hash: "#/playlist/{big}", name: "Playlist (4 000 titres)", ready: "main h1" },
  { hash: "#/credits/{writer}", name: "Crédits", ready: "main h1" },
  { hash: "#/podcasts", name: "Podcasts", ready: "main h1" },
  { hash: "#/podcast/{show}", name: "Émission", ready: "main h1" },
  { hash: "#/genres", name: "Genres", ready: "main h1" },
  { hash: "#/releases", name: "Nouveautés", ready: "main h1" },
  { hash: "#/releases/artists", name: "Nouveautés — Tes artistes", ready: "main h1" },
  { hash: "#/releases/discovery", name: "Nouveautés — Découvertes", ready: "main h1" },
  { hash: "#/recap", name: "Récap", ready: "main h1" },
  { hash: "#/recap/{year}", name: "Récap — année", ready: "main h1" },
  { hash: "#/import", name: "Importer", ready: "main h1" },
  { hash: "#/settings", name: "Réglages", ready: "main h1" },
  ...["playback", "appearance", "viz", "downloads", "discovery", "sources", "lan", "account", "users", "server", "help"].map(
    (id) => ({
      hash: "#/settings/" + id,
      name: "Réglages — " + id,
      // Animations carries the 50-world picker, which mounts only when its
      // catalogue import lands (WorldPicker.svelte): `main h1` is painted long
      // before the worlds exist.
      ready: id === "viz" ? "main .wp .chip" : "main h1",
      section: id,
    })
  ),
];

/** Routes the SPA declares but this pass does not walk, with why (§8.2). */
export const OUT_OF_PASS = [
  { route: "/party/:id", why: "demande un second client (l'invité a son propre moteur)" },
  { route: "/rc/:token", why: "demande un lien de contrôle et un second appareil" },
  { route: "/shared/playlist/:token", why: "demande un lien partagé public" },
  { route: "/invite/:token", why: "demande une invitation à usage unique" },
  { route: "/onboarding", why: "demande un compte invité neuf (le banc n'en a pas)" },
  { route: "/viz", why: "écran d'affichage (projecteur), hors coquille" },
];

/**
 * Route patterns of App.svelte that a SCREENS hash answers for, when the
 * pattern is not a screen of its own: /import/:url is the same screen as
 * /import (an import link pasted in the bar). Anything else that exists in the
 * source and is not here makes the anti-drift guard fail.
 */
export const ROUTE_ALIASES = {
  "/import/:url": "#/import",
};

/** Fill {artist} {album} … from the bench server's READY payload (§8.2). */
export function idsInto(screen, ids) {
  const hash = screen.hash.replace(/\{(\w+)\}/g, (_, key) => {
    if (key === "artist") return ids.artist;
    if (key === "album") return ids.album;
    if (key === "big") return ids.big;
    if (key === "writer") return encodeURIComponent(ids.writer);
    if (key === "q") {
      if (!ids.q) throw new Error("idsInto: {q} needs ids.q (the bench album's first title)");
      return encodeURIComponent(ids.q);
    }
    if (key === "show") {
      if (!ids.podcast?.show) throw new Error("idsInto: {show} needs --podcasts");
      return ids.podcast.show;
    }
    if (key === "year") {
      if (!ids.recap?.current) throw new Error("idsInto: {year} needs --recap");
      return String(ids.recap.current);
    }
    throw new Error(`idsInto: unknown placeholder {${key}}`);
  });
  return hash;
}

// ---------------------------------------------------------------------------
// §4.1 — the outcome classes, in priority order (the first that applies wins).
// ---------------------------------------------------------------------------
export const OUTCOMES = [
  "exception",
  "blank",
  "freeze",
  "console-error",
  "navigated",
  "opened-layer",
  "state-changed",
  "nothing",
  "blocked-destructive",
  "unreachable",
  "skipped-session",
];

/** ARIA roles a person can operate (run.mjs clicks what a screen reader names). */
export const INTERACTIVE_ROLES = new Set([
  "button",
  "link",
  "tab",
  "radio",
  "checkbox",
  "switch",
  "menuitem",
  "menuitemcheckbox",
  "menuitemradio",
  "textbox",
  "searchbox",
  "combobox",
  "slider",
  "spinbutton",
  "treeitem",
]);

// ---------------------------------------------------------------------------
// §4.5 — destructive actions. Matched on the element's accessible name (the
// app's own French labels, read out of the sources named in the plan).
// `when`: "passe" (clicked up to the confirmation, then cancelled),
//         "fin" (never in the pass; once at the end of its section / in session),
//         "jamais" (clicked up to its sheet, never past it).
// ---------------------------------------------------------------------------
export const DESTRUCTIVE = [
  { match: /^Déconnexion$/i, when: "fin", why: "session", confirm: "aucune (logout direct)" },
  { match: /^Se déconnecter$/i, when: "fin", why: "session", confirm: "aucune (logout direct)" },
  { match: /^Délier$/i, when: "fin", why: "compte lié", confirm: "aucune (unlink direct)" },
  { match: /^Supprimer le compte/i, when: "passe", confirm: "feuille + saisie du nom" },
  { match: /^Supprimer la playlist/i, when: "passe", confirm: "window.confirm" },
  { match: /^Supprimer la transcription/i, when: "passe", confirm: "role=group dans la feuille" },
  { match: /^Vider la file/i, when: "passe", reversible: true, confirm: "aucune (réversible)" },
  { match: /^Vider le cache/i, when: "passe", reversible: true, confirm: "aucune (réversible)" },
  { match: /^Réinitialiser/i, when: "passe", reversible: true, confirm: "aucune (réversible)" },
  { match: /^Recharger l'application/i, when: "fin", why: "hardReload", confirm: "recharge par construction" },
  { match: /^Mettre à jour/i, when: "jamais", confirm: "feuille puis installation" },
];

/** The destructive rule for a name, or null. */
export function destructiveFor(name) {
  return DESTRUCTIVE.find((d) => d.match.test(name.trim())) || null;
}

// ---------------------------------------------------------------------------
// §4.6 — action verbs. A `nothing` (or an `unreachable`) on one of these is
// bloquant/sérieux; on a decoration it is mineur.
// ---------------------------------------------------------------------------
export const VERBS =
  /^(Lire|Ouvrir|Ajouter|Retirer|Supprimer|Appliquer|Choisir|Enregistrer|Fermer|Chercher|Télécharger|Suivant|Précédent|Lecture|Réduire|Partager|Importer|Créer|Renommer|Déplacer|Vider|Recharger|Se connecter|Se déconnecter|Lancer|Reprendre|Mettre|Délier|Déconnexion|Répéter|Aléatoire|Favori|Paroles|Exporter|Inviter|Copier|Coller|Envoyer|Valider|Confirmer|Annuler|Retour|Trier|Filtrer|Basculer|Activer|Désactiver|Modifier|Éditer|Monter|Descendre)/i;

/** Is this accessible name an action? (`« Rien en lecture »` is not.) */
export function namedAction(name) {
  return VERBS.test((name || "").trim());
}

/**
 * §4.2 — what is not an error. Matched on URL HOST or PATH, never on the whole
 * URL string (a `?next=api.deezer.com` must not match Deezer's host).
 */
export const ALLOW = [
  { why: "401 /api/me avant que le cookie ne soit posé (boot)", test: (u, s) => s === 401 && u.pathname === "/api/me" },
  { why: "404 /api/cover (le banc ne sert pas de pochette)", test: (u, s) => s === 404 && u.pathname.startsWith("/api/cover/") },
  { why: "404 /api/stream (le banc ne sert pas d'audio)", test: (u, s) => s === 404 && u.pathname.startsWith("/api/stream/") },
  { why: "404 /sw.js en aperçu statique", test: (u, s) => s === 404 && u.pathname === "/sw.js" },
  { why: "Deezer, que le banc n'atteint jamais", test: (u) => u.hostname === "api.deezer.com" || u.hostname === "dzcdn.net" || u.hostname.endsWith(".dzcdn.net") },
];

/** Is this failed request an allowed one? */
export function allowedRequest(url, status) {
  let u;
  try {
    u = new URL(url);
  } catch {
    return false;
  }
  return ALLOW.some((a) => a.test(u, status));
}

/**
 * The bench runs tools/perf_api.py --serve with the Deezer proxy OFF, so every
 * route that needs Deezer answers 503 with `"code":"deezer-disabled"`. That is
 * the bench's state, not a screen's defect: the shell says it in a notice. A
 * 503 without that code stays an error (the plan's §4.2 rule, applied to the
 * one state the bench itself creates).
 */
export async function deezerOffAnswer(response) {
  if (response.status() !== 503) return false;
  try {
    const body = await response.text();
    return body.includes("deezer-disabled") || body.includes("deezer-unreachable");
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// §4.3 — the fingerprint, and its normalisation.
// ---------------------------------------------------------------------------

/** One accessible name, flattened (whitespace runs become one space). */
export function flat(s) {
  return (s || "").replace(/\s+/g, " ").trim();
}

/** `[^a-z0-9]+` → `-`, accents folded, at most 40 chars, `-2` on collision. */
export function slug(s, max = 40) {
  const base = (s || "")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, max)
    .replace(/-+$/g, "");
  return base || "sans-nom";
}

/**
 * The aria snapshot, pass by pass (§4.3), plus two families the plan's list
 * implies but does not spell: the bench's UUID keys (a playlist id is a UUID,
 * not 32 hex) and its seven-digit ids (8000 tracks go past 3000099). Both are
 * library identifiers; leaving them would make two passes differ.
 */
export function normalizeAria(snapshot) {
  const out = [];
  for (const raw of snapshot.split("\n")) {
    if (/^\s*-\s+slider\b/.test(raw)) continue;
    let line = raw;
    line = line.replace(/\d+:\d+(?::\d+)?/g, "#");
    line = line.replace(/\b\d{4}-\d{2}-\d{2}\b/g, "#");
    line = line.replace(/\b\d{1,2} (?:janv|févr|mars|avr|mai|juin|juil|août|sept|oct|nov|déc)\b/gi, "#");
    line = line.replace(/\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi, "#");
    line = line.replace(/\b[0-9a-f]{32}\b/gi, "#");
    line = line.replace(/\b[123]\d{6}\b/g, "#");
    line = line.replace(/[ \t]+$/gm, "");
    if (!line.trim()) continue;
    out.push(line);
  }
  return out.join("\n");
}

/** Hash of a normalised snapshot: two equal hashes mean two equal screens. */
export function ariaHash(normalized) {
  return createHash("sha1").update(normalized).digest("hex").slice(0, 12);
}

/** `#/artist/1000000` → `#/artist/#`; ids and uuids become `#` (§4.1 navigated). */
export function normalizeHash(hash) {
  const [path, query] = String(hash || "").split("?");
  const segs = path.split("/").map((s) => {
    if (!s) return s;
    if (/^\d{4,}$/.test(s)) return "#";
    if (/^[0-9a-f]{32}$/i.test(s)) return "#";
    if (/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s)) return "#";
    return s;
  });
  return segs.join("/") + (query ? "?" + query : "");
}

/** §4.4 — 3 samples per group of ≥6 same-role, same-child-roles siblings. */
export const SIBLING_SAMPLES = 3;
export const SIBLING_MIN = 6;

/**
 * Parse an aria snapshot into nodes {indent, role, name, state, parent, childRoles}.
 * The YAML shape is one node per `- role "name"` line, children indented deeper.
 * Pure text nodes (`- text: …`) are not roles; they are counted in the parent's
 * text but never grouped as siblings.
 */
export function parseSnapshot(snapshot) {
  const lines = snapshot.split("\n");
  const nodes = [];
  const stack = [];
  for (const line of lines) {
    const m = line.match(/^(\s*)-\s+([a-z]+)(?:\s+"((?:[^"\\]|\\.)*)")?\s*(.*)$/);
    if (!m) continue;
    const indent = m[1].length;
    const role = m[2];
    if (!INTERACTIVE_ROLES.has(role) && !/^[a-z]+$/.test(role)) continue;
    const node = {
      id: nodes.length,
      indent,
      role,
      name: flat((m[3] || "").replace(/\\"/g, '"')),
      state: (m[4] || "").trim(),
      parent: null,
      childRoles: [],
    };
    while (stack.length && stack[stack.length - 1].indent >= indent) stack.pop();
    if (stack.length) {
      node.parent = stack[stack.length - 1];
      stack[stack.length - 1].childRoles.push(role);
    }
    stack.push(node);
    nodes.push(node);
  }
  return nodes;
}

/**
 * §4.4 — the sibling rule, decided on the DOM's repeated structure.
 *
 * The aria tree alone cannot tell a list from a toolbar (every leaf under
 * `main` shares "no child roles"), so a group is a LIST only when an ancestor
 * of its members repeats: ≥ 6 siblings of the same tag and class (a card, a
 * row, a list item). The 3 first members of each group are clicked, the rest
 * are `duplicates` pointing at the first — and every member is judged on its
 * own ancestor chain, so a card's inner « Lire » belongs to the same sampled
 * group as the card.
 */
export async function markDomDuplicates(page, els, scope = null) {
  const groups = new Map();
  let resolved = 0;
  let keyed = 0;
  for (const el of els) {
    const r = await resolve(page, el, scope);
    if (!r) continue;
    resolved += 1;
    const key = await r.loc
      .evaluate((node, min) => {
        for (let n = node, depth = 0; n && n !== document.body && depth < 4; n = n.parentElement, depth++) {
          const parent = n.parentElement;
          if (!parent || parent === document.body) continue;
          const cls = (n.className || "").toString().split(/\s+/).filter(Boolean)[0] || "";
          const same = [...parent.children].filter(
            (c) => c.tagName === n.tagName && ((c.className || "").toString().split(/\s+/).filter(Boolean)[0] || "") === cls
          ).length;
          if (same >= min) return `${n.tagName}.${cls}|${depth}|${parent.tagName}`;
        }
        return null;
      }, SIBLING_MIN)
      .catch(() => null);
    if (!key) continue;
    keyed += 1;
    const groupKey = `${key}|${el.role}`;
    if (!groups.has(groupKey)) groups.set(groupKey, []);
    groups.get(groupKey).push(el);
  }
  if (process.env.BUGHUNT_DEBUG) console.log(`  dup resolved=${resolved}/${els.length} keyed=${keyed}`, [...groups].map(([k, v]) => `${k}×${v.length}`).join(" "));
  for (const group of groups.values()) {
    if (group.length < SIBLING_MIN) continue;
    for (let i = SIBLING_SAMPLES; i < group.length; i++) group[i].duplicateOf = group[0].signature;
  }
  return els;
}

/** §4.7 — an element's stable signature, `role|slug(normalised-name)`. */
export function signature(role, name) {
  return `${role}|${slug(flat(name))}`;
}

// ---------------------------------------------------------------------------
// §4.6 — severity, decided by the harness (B3 may confirm or downgrade).
// ---------------------------------------------------------------------------
export function severity(row) {
  if (!row) return null;
  const { outcome } = row;
  const name = row.element?.name || "";
  if (row.disabled) return null; // a disabled control is not clickable, by design
  if (row.deezerOff) return null; // the action needs Deezer, off on the bench by design
  if (outcome === "exception" || outcome === "blank" || outcome === "freeze") return "bloquant";
  if (outcome === "nothing") return namedAction(name) ? "bloquant" : "mineur";
  if (outcome === "blocked-destructive")
    return row.confirm === "no-confirm" ? "bloquant" : row.cancelBroken ? "sérieux" : null;
  if (outcome === "console-error") return "sérieux";
  if (outcome === "unreachable") return row.vanished ? null : namedAction(name) ? "sérieux" : "mineur";
  if (outcome === "skipped-session" || outcome === "duplicate") return null;
  return null;
}

// ---------------------------------------------------------------------------
// §4.9 — the bug list: same (screen, outcome, normalised name) is one bug.
// ---------------------------------------------------------------------------
export function bugKey(row) {
  return `${slug(row.screen.name, 30)}|${row.outcome}|${slug(row.element.name, 40)}`;
}

/** Fold rows into bugs, most severe first, then by screen order. */
export function collectBugs(rows, screens) {
  const order = new Map(screens.map((s, i) => [s.name, i]));
  const rank = { bloquant: 0, sérieux: 1, mineur: 2 };
  const bugs = new Map();
  for (const row of rows) {
    if (!row.severity) continue;
    const key = bugKey(row);
    if (!bugs.has(key)) {
      bugs.set(key, {
        key,
        severity: row.severity,
        screen: row.screen.name,
        element: `${row.element.role} « ${row.element.name || "(sans nom)"} »`,
        outcome: row.outcome,
        occurrences: 0,
        steps: stepsFor(row),
        evidence: row.evidence,
        detail: row.detail,
      });
    } else {
      const b = bugs.get(key);
      b.occurrences += 1;
      if (rank[row.severity] < rank[b.severity]) b.severity = row.severity;
    }
  }
  const list = [...bugs.values()];
  list.forEach((b, i) => (b.id = i + 1));
  list.sort(
    (a, b) =>
      rank[a.severity] - rank[b.severity] ||
      (order.get(a.screen) ?? 99) - (order.get(b.screen) ?? 99) ||
      a.key.localeCompare(b.key)
  );
  list.forEach((b, i) => (b.id = i + 1));
  return list;
}

/** Three steps a person can replay, from what the harness did. */
export function stepsFor(row) {
  if (row.screen.name === "Scénarios")
    return ["rejouer `cd webapp && node test/bughunt/stress.mjs`", `le verdict « ${row.element.name} » est FAIL`, row.detail || ""];
  const open = `ouvrir ${row.screen.path || row.screen.name}`;
  const act =
    row.action === "type"
      ? `taper « zzz » dans ${row.element.role} « ${row.element.name} » puis Échap`
      : `cliquer ${row.element.role} « ${row.element.name || "(sans nom)"} »`;
  const what = {
    exception: "une exception non capturée part pendant le clic",
    blank: "le contenu principal devient vide et la route ne change pas",
    freeze: "une tâche de plus d'une seconde bloque la page",
    "console-error": "une erreur console ou une requête en échec part",
    navigated: "la route change",
    "opened-layer": "une couche s'ouvre",
    "state-changed": "l'état de l'écran change",
    nothing: "rien ne se passe (1 200 ms d'observation)",
    "blocked-destructive": "l'action destructive part sans demander confirmation",
    unreachable: "l'élément existe mais n'est ni visible ni atteignable",
    "skipped-session": "l'élément termine la session et n'est pas cliqué dans la passe",
  }[row.outcome];
  return [open, act, what];
}

// ---------------------------------------------------------------------------
// §4.8 — the Markdown report, exact template order.
// ---------------------------------------------------------------------------
function pct(clicked, found) {
  return found ? ((100 * clicked) / found).toFixed(1) : "0.0";
}

export function summaryLine(report) {
  const { counts } = report;
  const parts = [
    `screens=${counts.screens}`,
    `elements=${counts.elementsFound}`,
    `clicked=${counts.clicked}`,
    `coverage=${pct(counts.clicked, counts.elementsFound)}% (${counts.clicked}/${counts.elementsFound})`,
    `duplicates=${counts.duplicates}`,
    `nothing=${counts.nothing}`,
    `named_nothing=${counts.namedNothing}`,
    `blank=${counts.blank}`,
    `exceptions=${counts.exceptions}`,
    `freezes=${counts.freezes}`,
    `seconds=${counts.seconds}`,
  ];
  if (counts.stopped) parts.push(`stopped=${counts.stopped}`);
  if (counts.screenshotsCapped) parts.push("screenshots_capped=1");
  return "bughunt: " + parts.join(" ");
}

// Backslashes first, then the table's pipes: escaping only `|` would let a
// literal `\|` in a name break the cell (CodeQL js/incomplete-sanitization).
const esc = (s) => String(s ?? "").replace(/\\/g, "\\\\").replace(/\|/g, "\\|").replace(/\n/g, " ");

export function writeJson(dir, report) {
  mkdirSync(dir, { recursive: true });
  const path = join(dir, "bughunt-web.json");
  writeFileSync(path, JSON.stringify(report, null, 2));
  return path;
}

export function writeMarkdown(report) {
  const { meta, counts, scenarios, bugs, screens, rows } = report;
  const L = [];
  L.push("# Chasse aux bugs — application web");
  L.push("");
  L.push(
    `\`${meta.date}\` · tête \`${meta.head}\` · ${meta.server} · ${meta.viewport} · outil \`${meta.tool}\``
  );
  L.push(summaryLine(report));
  L.push("");
  L.push("## Ce que le propriétaire a demandé d'abord");
  L.push("");
  L.push("| Scénario | Verdict | Nombres mesurés | Preuve |");
  L.push("|---|---|---|---|");
  for (const s of scenarios) {
    L.push(`| ${esc(s.title)} | ${s.verdict.toUpperCase()} | \`${esc(s.numbers)}\` | ${s.evidence ? `\`${s.evidence}\`` : "—"} |`);
  }
  L.push("");
  L.push("## Bugs (dédupliqués, le plus grave d'abord)");
  L.push("");
  if (!bugs.length) {
    L.push("Aucun bug de gravité bloquante, sérieuse ou mineure n'a été relevé par cette passe.");
  } else {
    L.push("| # | Gravité | Écran | Élément | Ce qui s'est passé | Étapes | Preuve |");
    L.push("|---|---|---|---|---|---|---|");
    for (const b of bugs) {
      L.push(
        `| ${b.id} | ${b.severity} | ${esc(b.screen)} | ${esc(b.element)} | ${esc(b.detail || b.outcome)}${b.occurrences > 1 ? ` (×${b.occurrences})` : ""} | ${esc(b.steps.join(" ; "))} | ${b.evidence ? `\`${b.evidence}\`` : "—"} |`
      );
    }
  }
  L.push("");
  L.push("## Écrans");
  for (const s of screens) {
    const own = rows.filter((r) => r.screen.name === s.name);
    L.push("");
    L.push(`### ${s.name} — \`${s.hash}\` — ${s.elements ?? own.filter((r) => r.outcome !== "duplicate").length} éléments`);
    L.push("");
    if (!own.length) {
      L.push("_Aucun élément exposé._");
      continue;
    }
    L.push("| Élément (rôle + nom) | Action | Résultat | ms | Preuve / reproduire |");
    L.push("|---|---|---|---|---|");
    for (const r of own) {
      const el = `${r.element.role} « ${esc(r.element.name || "(sans nom)")} »`;
      const result = r.outcome === "nothing" ? "**NOTHING**" : r.outcome;
      const evidence = r.evidence ? `\`${r.evidence}\`` : "";
      const detail = r.detail ? `${evidence}${evidence ? " · " : ""}${esc(r.detail)}` : evidence;
      L.push(`| ${el} | ${r.action} | ${result}${r.confirm ? ` (${r.confirm})` : ""} | ${r.ms} | ${detail} |`);
    }
  }
  const unreachable = rows.filter((r) => r.outcome === "unreachable");
  L.push("");
  L.push("## Éléments non cliqués");
  L.push("");
  if (counts.duplicates) L.push(`- \`duplicates=${counts.duplicates}\` (règle du frère, §4.4, et coquille persistante comptée une fois)`);
  if (unreachable.length)
    L.push(`- \`unreachable=${unreachable.length}\` : ${unreachable.map((r) => `${r.screen.name} « ${r.element.name} »`).join(", ")}`);
  const out = report.outOfPass || [];
  if (out.length) L.push(`- hors passe : ${out.map((o) => `\`${o.route}\` (${o.why})`).join(", ")}`);
  if (report.layersOutOfDepth?.length)
    L.push(`- hors profondeur : ${report.layersOutOfDepth.map((l) => `« ${l} »`).join(", ")}`);
  if (!counts.duplicates && !unreachable.length && !out.length) L.push("_Rien : tous les éléments trouvés ont été cliqués._");
  L.push("");
  L.push("## Hors passe (routes non couvertes, et pourquoi)");
  L.push("");
  for (const o of out) L.push(`- \`${o.route}\` — ${o.why}`);
  L.push("");
  return L.join("\n");
}

// ---------------------------------------------------------------------------
// §8.2 — the anti-drift guard. Reads the SPA's own sources and compares with
// SCREENS; a route or a Settings section in the source and not in the table is
// a hard stop (exit 2, no click).
// ---------------------------------------------------------------------------
export function declaredRoutes() {
  const src = readFileSync(join(REPO, "webapp/src/App.svelte"), "utf8");
  const block = src.match(/const routes = \{([\s\S]*?)\n  \};/);
  if (!block) throw new Error("anti-drift: could not read `const routes` from App.svelte");
  return [...block[1].matchAll(/^\s*"(\/[^"]*)"\s*:/gm)].map((m) => m[1]);
}

export function declaredSections() {
  const src = readFileSync(join(REPO, "webapp/src/routes/Settings.svelte"), "utf8");
  const block = src.match(/\$:\s*SECTIONS = \[([\s\S]*?)\n  \];/);
  if (!block) throw new Error("anti-drift: could not read `$: SECTIONS` from Settings.svelte");
  return [...block[1].matchAll(/\bid:\s*"([a-z]+)"/g)].map((m) => m[1]);
}

/** Does a route pattern match a SCREENS hash? `:param?` is optional. */
function patternMatches(pattern, hash) {
  const path = hash.replace(/^#/, "");
  const segs = pattern.split("/").filter(Boolean);
  const hsegs = path.split("/").filter(Boolean);
  for (let i = 0; i < segs.length; i++) {
    const seg = segs[i];
    const optional = seg.endsWith("?");
    const base = optional ? seg.slice(0, -1) : seg;
    const isParam = base.startsWith(":");
    if (i >= hsegs.length) return optional;
    if (isParam) continue;
    if (base !== hsegs[i]) return false;
  }
  return hsegs.length === segs.length;
}

// ---------------------------------------------------------------------------
// §8.3 — the page half: observation, fingerprint, enumeration, click, walk.
// Everything here takes a Playwright page (and, for the walk, a `ctx`).
// ---------------------------------------------------------------------------

/** The observation window after a click, in ms (§8.7: named observation_ms). */
export const OBSERVATION_MS = 1200;

/**
 * A `nothing` is confirmed once more, 800 ms later: a slow paint (a playback
 * title that lands while the player settles) otherwise turns the same element
 * into `nothing` on one pass and `state-changed` on the next. Measured: 5
 * `nothing` lines differed between two passes at 1 200 ms alone; the criterion
 * is the same bug list, so silence is proven over 2 000 ms.
 */
export const NOTHING_CONFIRM_MS = 800;

/** What a screen's fingerprint must carry (§4.3). */
export async function fingerprint(page) {
  const state = await page.evaluate(() => {
    const main = document.querySelector("main");
    const host = document.querySelector(".route-host");
    const layers = new Set();
    for (const el of document.querySelectorAll("[role=dialog],[role=menu],.sheet,[aria-modal]")) {
      if (el.getClientRects().length) layers.add(el);
    }
    return {
      hash: location.hash,
      textLen: (main?.innerText || "").trim().length,
      // `main` also holds the permanent shell (top bar, notices), so its text
      // can never be < 20: the routed box is the one whose emptiness is the
      // owner's symptom (measured: a blanked route still had 120 chars in
      // `main` from the search bar).
      routeTextLen: (host?.innerText || "").trim().length,
      nodes: host ? host.querySelectorAll("*").length + 1 : 0,
      layers: layers.size,
    };
  });
  const aria = normalizeAria(await page.locator("body").ariaSnapshot());
  return { ...state, hash: normalizeHash(state.hash), rawHash: state.hash, aria, hashOfAria: ariaHash(aria) };
}

/** Install the page observers once. Returns the event log ctx.events carries. */
export async function installObservation(page) {
  const events = {
    pageerrors: [],
    consoleErrors: [],
    httpErrors: [],
    failed: [],
    dialogs: [],
    writes: [],
    deezerOff: [],
    newPages: [],
    downloads: [],
  };
  page.context().on("page", async (p) => {
    if (p === page) return;
    events.newPages.push({ at: Date.now() });
    await p.close().catch(() => {});
  });
  page.on("download", async (d) => {
    events.downloads.push({ at: Date.now(), url: d.url() });
    await d.cancel().catch(() => {});
  });
  page.on("pageerror", (e) => events.pageerrors.push({ at: Date.now(), msg: e.message }));
  page.on("console", (m) => {
    if (m.type() !== "error") return;
    const text = m.text();
    // A resource that failed to load is already counted by its RESPONSE (which
    // names the URL); counting the console echo too would double every 404.
    if (/^Failed to load resource/i.test(text)) return;
    events.consoleErrors.push({ at: Date.now(), msg: text });
  });
  page.on("response", async (r) => {
    const s = r.status();
    if (s < 400) return;
    if (allowedRequest(r.url(), s)) return;
    if (await deezerOffAnswer(r)) {
      events.deezerOff.push({ at: Date.now(), url: r.url() });
      return;
    }
    events.httpErrors.push({ at: Date.now(), status: s, url: r.url() });
  });
  page.on("requestfailed", (r) => {
    const err = r.failure()?.errorText || "";
    if (allowedRequest(r.url(), 0)) return;
    // A navigation after the 1 200 ms window cancels what was in flight; the
    // harness itself creates this abort, it is not the screen failing.
    if (err === "net::ERR_ABORTED") return;
    events.failed.push({ at: Date.now(), url: r.url(), err });
  });
  page.on("request", (r) => {
    if (r.method() === "GET") return;
    events.writes.push({ at: Date.now(), method: r.method(), url: r.url() });
  });
  page.on("dialog", async (d) => {
    events.dialogs.push({ at: Date.now(), type: d.type(), msg: d.message() });
    await d.dismiss().catch(() => {});
  });
  await page.addInitScript(() => {
    window.__bh = { longtasks: [], lastMut: 0, els: [] };
    // The player's <audio> elements are never in the DOM (`new Audio()`), so
    // they are captured at creation — the same seam webapp/test/player/run.mjs uses.
    const Native = window.Audio;
    window.__bh.els = [];
    window.Audio = function (...a) {
      const el = new Native(...a);
      window.__bh.els.push(el);
      return el;
    };
    window.Audio.prototype = Native.prototype;
    try {
      new PerformanceObserver((l) => l.getEntries().forEach((e) => window.__bh.longtasks.push({ at: e.startTime, d: e.duration }))).observe({
        entryTypes: ["longtask"],
      });
    } catch {
      /* no longtask observer */
    }
    const mo = new MutationObserver(() => {
      window.__bh.lastMut = performance.now();
    });
    addEventListener("DOMContentLoaded", () => mo.observe(document.body, { childList: true, subtree: true, characterData: true }));
  });
  return events;
}

/** Slice of the event log since `mark` (a length snapshot), inside [t0, t1]. */
function since(list, from, t0, t1) {
  return list.slice(from).filter((e) => e.at >= t0 && e.at <= t1);
}

/** Wait until the routed content stops changing (200 ms of quiet, ≤ cap). */
export async function quiet(page, quietMs = 200, capMs = 2000) {
  await page
    .waitForFunction(
      ({ quietMs, capMs }) => {
        const bh = window.__bh;
        if (!bh) return true;
        const now = performance.now();
        return now - bh.lastMut >= quietMs || now > (bh.openedAt || 0) + capMs;
      },
      { quietMs, capMs },
      { timeout: capMs + 500 }
    )
    .catch(() => {});
}

/**
 * Walk the routed box to its end and back, so everything lazily mounted exists
 * before enumeration: a panel under `content-visibility: auto` is 0×0 and
 * absent from the aria tree until it comes near the viewport (measured: the
 * 50-world picker, 0×0 at y=4991, invisible to `ariaSnapshot`). The plan's
 * inventory did the same one scroll per screen.
 */
export async function revealAll(page) {
  await page
    .evaluate(async () => {
      const main = document.querySelector("main");
      if (!main) return;
      // A folded panel is not in the accessibility tree at all (a closed
      // `<details>`): open every one first. Measured on Réglages → Animations:
      // the "Monde de cet écran" picker hides 50 worlds.
      for (const summary of main.querySelectorAll("details:not([open]) > summary")) {
        summary.click();
        await new Promise((r) => setTimeout(r, 120));
      }
      const step = Math.max(200, main.clientHeight * 0.8);
      const max = Math.min(main.scrollHeight, 40000);
      for (let y = 0; y <= max; y += step) {
        main.scrollTop = y;
        await new Promise((r) => requestAnimationFrame(() => r()));
      }
      main.scrollTop = 0;
      await new Promise((r) => requestAnimationFrame(() => r()));
    })
    .catch(() => {});
  await quiet(page, 120, 800);
}

/** Open a screen by hash, wait for its marker; reload once if it never paints. */
export async function openScreen(page, base, screen, ids, timeout = 8000) {
  const hash = screen.concreteHash || idsInto(screen, ids);
  await page.evaluate((h) => {
    window.__bh.openedAt = performance.now();
    if (location.hash !== h) location.hash = h;
  }, hash);
  let ok = await page.waitForSelector(screen.ready, { timeout, state: "attached" }).then(() => true).catch(() => false);
  if (!ok) {
    await page.reload();
    await page.goto(base + "/app/" + hash);
    ok = await page.waitForSelector(screen.ready, { timeout, state: "attached" }).then(() => true).catch(() => false);
  }
  await quiet(page, 200, 1500);
  return ok;
}

/** The document-order occurrence of each (role,name), for stable nth picks. */
function withOccurrences(nodes) {
  const seen = new Map();
  for (const n of nodes) {
    const key = `${n.role}|${n.name}`;
    const i = seen.get(key) || 0;
    seen.set(key, i + 1);
    n.occurrence = i;
  }
  return nodes;
}

/**
 * §8.3 — enumerate what a screen reader names, plus the DOM union inside main
 * for what the tree does not name. Sibling rule applied; each element carries
 * its resolver inputs, its signature and (if suppressed) its duplicate_of.
 */
export async function enumerate(page, screen) {
  const snapshot = await page.locator("body").ariaSnapshot();
  const nodes = parseSnapshot(snapshot).filter((n) => INTERACTIVE_ROLES.has(n.role));
  const withOcc = withOccurrences(nodes);
  const out = [];
  for (const n of withOcc) {
    out.push({
      role: n.role,
      name: n.name,
      state: n.state,
      via: "aria",
      selector: `role=${n.role}[name=${JSON.stringify(n.name)}]`,
      signature: signature(n.role, n.name),
      occurrence: n.occurrence,
      duplicateOf: n.duplicateOf ? signature(n.duplicateOf.role, n.duplicateOf.name) : null,
      disabled: /\[disabled\]/.test(n.state),
      inLayer: false,
    });
  }
  // The DOM union: inside main`, an element a person can operate that carries no
  // ARIA role at all (the tree does not name it). The shell outside main is
  // already in the tree through its named controls.
  const dom = await page.evaluate(() => {
    const path = (el) => {
      const parts = [];
      for (let n = el; n && n.nodeType === 1 && parts.length < 6; n = n.parentElement) {
        if (n === document.body) break;
        if (n.id) {
          parts.unshift("#" + CSS.escape(n.id));
          break;
        }
        const cls = (n.className || "").toString().split(/\s+/).filter(Boolean)[0];
        let part = n.tagName.toLowerCase() + (cls ? "." + CSS.escape(cls) : "");
        const parent = n.parentElement;
        if (parent) {
          const same = [...parent.children].filter((c) => c.tagName === n.tagName);
          if (same.length > 1) part += `:nth-of-type(${same.indexOf(n) + 1})`;
        }
        parts.unshift(part);
      }
      return parts.join(" > ");
    };
    const out = [];
    for (const el of document.querySelectorAll("main [onclick], main [tabindex='0']")) {
      if (el.closest("svg")) continue;
      if (el.matches("button, a[href], input, select, textarea, [role]")) continue;
      if (!el.getClientRects().length) continue;
      const name = (el.getAttribute("aria-label") || el.innerText || "").replace(/\s+/g, " ").trim().slice(0, 120);
      out.push({ tag: el.tagName.toLowerCase(), name, selector: path(el), disabled: el.hasAttribute("disabled") || el.getAttribute("aria-disabled") === "true" });
    }
    return out;
  });
  for (const d of dom) {
    out.push({
      role: d.tag,
      name: d.name,
      state: "",
      via: "dom",
      selector: d.selector,
      signature: signature(d.tag, d.name),
      occurrence: 0,
      duplicateOf: null,
      disabled: d.disabled,
      inLayer: false,
    });
  }
  await markDomDuplicates(page, out, null);
  return out;
}

/**
 * Resolve an enumerated element to a locator, scoped to a layer when given.
 *
 * Exact name first — that is what the snapshot announced. A composed name can
 * differ from the DOM's by an invisible suffix (a hidden count, an aria-label
 * the snapshot flattened), and an exact miss is not an absent element: the
 * substring pass is the same element a screen reader announces. It is only
 * reached when nothing carries the exact name.
 */
export async function resolve(page, el, scope = null) {
  const root = scope || page.locator("body");
  if (el.via === "aria") {
    let loc = root.getByRole(el.role, { name: el.name, exact: true });
    let count = await loc.count();
    if (count > (el.occurrence || 0)) return { loc: loc.nth(el.occurrence || 0), count };
    if (el.name) {
      loc = root.getByRole(el.role, { name: el.name, exact: false });
      count = await loc.count();
      if (count > (el.occurrence || 0)) return { loc: loc.nth(el.occurrence || 0), count };
    } else {
      // An unnamed control is still a control: the nth of its role is what the
      // snapshot listed. Its `nothing` is a mineur bug (an unnamed target).
      loc = root.getByRole(el.role);
      count = await loc.count();
      if (count > (el.occurrence || 0)) return { loc: loc.nth(el.occurrence || 0), count };
    }
  }
  if (el.selector) {
    const loc = (scope ? scope.locator(el.selector) : page.locator(el.selector)).first();
    if ((await loc.count()) > 0) return { loc, count: 1 };
  }
  return null;
}

/** The visible layer (dialog/menu/sheet), last in DOM order — the newest one. */
export async function newestLayer(page) {
  const loc = page.locator('[role=dialog],[role=menu],.sheet').filter({ visible: true });
  const n = await loc.count();
  return n ? loc.nth(n - 1) : null;
}

async function layerKey(layer) {
  return layer
    .evaluate((el) => {
      const label = el.getAttribute("aria-label") || el.getAttribute("aria-labelledby");
      const byId = label && document.getElementById(label);
      const text = ((byId && byId.innerText) || el.getAttribute("aria-label") || el.querySelector("h2,h1,.title")?.innerText || "").trim();
      const names = [...el.querySelectorAll("button,a[href],[role=button],[role=menuitem]")].map((b) => (b.getAttribute("aria-label") || b.innerText || "").trim()).slice(0, 40);
      return text.slice(0, 80) + "|" + names.join(",");
    })
    .catch(() => "");
}

/** Fallback content comparison for an aria snapshot diff detail. */
function firstDiff(before, after) {
  if (before === after) return "";
  const a = before.split("\n");
  const b = after.split("\n");
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (a[i] !== b[i]) return `${(a[i] || "∅").trim()} → ${(b[i] || "∅").trim()}`.slice(0, 200);
  }
  return "";
}

/**
 * §4.1 — classify one click, in priority order. `observed` is the slice of the
 * event log in the observation window; `before`/`after` are fingerprints.
 */
export function classify(before, after, observed) {
  if (observed.pageerrors.length) return { outcome: "exception", detail: observed.pageerrors[0].msg };
  if (after.hash === before.hash && after.routeTextLen < 20 && after.nodes < 3) {
    return { outcome: "blank", detail: `contenu de la route ${after.routeTextLen} caractères, ${after.nodes} nœuds` };
  }
  const freeze = observed.longtasks.find((t) => t.d > 1000);
  if (freeze) return { outcome: "freeze", detail: `longtask ${Math.round(freeze.d)} ms` };
  const http = observed.httpErrors[0] || observed.failed[0];
  const consoleError = observed.consoleErrors[0];
  if (http) return { outcome: "console-error", detail: `${http.status || "réseau"} ${http.url || ""}`.trim() };
  if (consoleError) return { outcome: "console-error", detail: consoleError.msg.slice(0, 160) };
  if (observed.newPages?.length) return { outcome: "navigated", detail: "ouvre un nouvel onglet" };
  if (observed.downloads?.length) return { outcome: "navigated", detail: "lance un téléchargement" };
  if (after.hash !== before.hash) return { outcome: "navigated", detail: `${before.hash} → ${after.hash}` };
  if (after.layers > before.layers) return { outcome: "opened-layer", detail: `${before.layers} → ${after.layers} couche(s)` };
  if (after.aria !== before.aria) return { outcome: "state-changed", detail: firstDiff(before.aria, after.aria) };
  return { outcome: "nothing", detail: `aucun changement en ${OBSERVATION_MS} ms` };
}

/** A row, shaped like §4.9. */
function makeRow(screen, el, action, outcome, detail, ms, extra = {}) {
  return {
    run_id: "",
    screen: { name: screen.name, path: screen.hash },
    element: {
      role: el.role,
      name: el.name,
      selector: el.selector,
      via: el.via,
      signature: el.signature,
      duplicate_of: el.duplicateOf || null,
    },
    action,
    outcome,
    detail,
    ms,
    severity: null,
    bug_id: null,
    evidence: null,
    confirm: null,
    ...extra,
  };
}

/** Restore the screen after one click (§8.7: hash first, reload only if needed). */
async function restore(page, screen, ctx) {
  // The full-screen player is not a layer: close it by its own button.
  const reduce = page.getByRole("button", { name: "Réduire", exact: true });
  if ((await reduce.count()) > 0 && (await reduce.first().isVisible().catch(() => false))) {
    await reduce.first().click({ timeout: 2000 }).catch(() => {});
    await page.waitForTimeout(150);
  }
  for (let i = 0; i < 3; i++) {
    if (!(await newestLayer(page))) break;
    await page.keyboard.press("Escape").catch(() => {});
    await page.waitForTimeout(120);
  }
  const want = screen.concreteHash;
  const now = await page.evaluate(() => location.hash);
  if (normalizeHash(now) !== normalizeHash(want)) {
    await page.evaluate((h) => {
      location.hash = h;
    }, want);
    await page.waitForTimeout(150);
  }
  let ok = await page.waitForSelector(screen.ready, { timeout: 4000, state: "attached" }).then(() => true).catch(() => false);
  if (!ok) {
    ctx.reloads += 1;
    await page.reload();
    await page.goto(ctx.base + "/app/" + want);
    ok = await page.waitForSelector(screen.ready, { timeout: 8000, state: "attached" }).then(() => true).catch(() => false);
  }
  await quiet(page, 150, 1200);
  return ok;
}

/** One screenshot, numbered and named the way §4.7 says. */
export async function shot(page, ctx, screen, index, suffix, outcome = "") {
  if (!ctx.shots) return null;
  const used = outcome ? ctx.anomalyShots || 0 : ctx.screenShots || 0;
  const cap = outcome ? ctx.maxShots : ctx.maxScreenshots;
  if (cap && used >= cap) {
    ctx.screenshotsCapped = true;
    return null;
  }
  const dir = join(ctx.dir, "screens");
  mkdirSync(dir, { recursive: true });
  const name = `${String(index).padStart(2, "0")}-${slug(screen.name, 30)}${suffix ? "-" + suffix : ""}${outcome ? "-" + outcome : ""}.png`;
  const path = join(dir, name);
  await page.screenshot({ path, fullPage: false }).catch(() => {});
  if (outcome) ctx.anomalyShots = used + 1;
  else ctx.screenShots = used + 1;
  return join("screens", name);
}

const ANOMALIES = new Set(["blank", "exception", "freeze", "console-error", "opened-layer"]);

/**
 * Click ONE element and observe (§4.1, §4.5, §8.7). Marks the screen, records
 * the row and — when a layer opened — walks it, depth 1.
 */
export async function clickAndObserve(page, el, screen, ctx, { scope = null, index = 0 } = {}) {
  const t0 = Date.now();
  const before = await fingerprint(page);
  const mark = {
    pageerrors: ctx.events.pageerrors.length,
    consoleErrors: ctx.events.consoleErrors.length,
    httpErrors: ctx.events.httpErrors.length,
    failed: ctx.events.failed.length,
    dialogs: ctx.events.dialogs.length,
    writes: ctx.events.writes.length,
    deezerOff: ctx.events.deezerOff.length,
    newPages: ctx.events.newPages.length,
    downloads: ctx.events.downloads.length,
  };
  await page.evaluate(() => {
    if (window.__bh) window.__bh.longtasks.length = 0;
  });

  let action = "click";
  const d = destructiveFor(el.name);
  if (d && d.when === "fin") {
    return makeRow(screen, el, action, "skipped-session", d.why || "session", 0, { severity: null });
  }

  let clicked = false;
  let failure = "";
  let vanished = false;
  for (let attempt = 0; attempt < 2 && !clicked; attempt++) {
    try {
      const r = await resolve(page, el, scope);
      if (!r) {
        if (attempt === 0) {
          // The screen may have changed under us (a notice dismissed, a list
          // re-windowed, a search panel left open): put it back and look once
          // more before calling it lost. A panel that survives the hash needs a
          // document reload — the plan's "state not recoverable" case, counted.
          await restore(page, screen, ctx);
          if (!(await resolve(page, el, scope))) {
            // The window of a virtual list may have moved: back to the top,
            // where the enumeration was taken, before giving up on the node.
            await page.evaluate(() => {
              const main = document.querySelector("main");
              if (main) main.scrollTop = 0;
              const host = document.querySelector(".route-host");
              if (host) host.scrollTop = 0;
            }).catch(() => {});
            await page.waitForTimeout(120);
          }
          if (!(await resolve(page, el, scope))) {
            ctx.reloads += 1;
            await page.reload();
            await page.goto(ctx.base + "/app/" + screen.concreteHash);
            await page.waitForSelector(screen.ready, { timeout: 8000 }).catch(() => {});
            await quiet(page, 200, 1500);
          }
          continue;
        }
        failure = "absent de l'arbre avant le clic";
        vanished = true;
        break;
      }
      if (el.disabled) {
        return makeRow(screen, el, "click", "unreachable", "disabled", Date.now() - t0, { disabled: true });
      }
      // §4.1/§8.7: up to three scroll attempts before calling it unreachable.
      // `scrollIntoViewIfNeeded` is minimal and can leave the target under the
      // fixed player bar (measured on the 50-world picker: a real 65×36 box at
      // y=861, covered); the next attempts centre it.
      let reach = false;
      for (let tries = 0; tries < 3 && !reach; tries++) {
        if (tries === 0) await r.loc.scrollIntoViewIfNeeded({ timeout: 2500 }).catch(() => {});
        else {
          await r.loc.evaluate((node) => node.scrollIntoView({ block: "center", inline: "center" })).catch(() => {});
          await page.waitForTimeout(120);
        }
        reach = await reachable(r.loc);
      }
      // A hidden quick action (a desktop hover affordance) is unreachable by
      // design: decided in milliseconds, not by a 4 s click timeout.
      if (!reach) {
        return makeRow(screen, el, "click", "unreachable", "invisible ou couvert, aucun ancêtre défilable", Date.now() - t0, { vanished: true });
      }
      if (el.role === "textbox" || el.role === "searchbox") {
        // §8.5's gesture, applied wherever an input lives: a field answers a
        // typed value, not a click (a focus is not a state a snapshot sees).
        // A colour field rejects anything that is not a colour: `#123456` is
        // the same hostile value on that input (`zzz` made it a false bug).
        const inputType = await r.loc.evaluate((n) => n.getAttribute("type") || "").catch(() => "");
        await r.loc.fill(inputType === "color" ? "#123456" : "zzz", { timeout: 4000 });
        await page.keyboard.press("Escape");
        action = "type";
      } else if (el.role === "combobox") {
        // The 2nd option: the 1st is usually the current state.
        const isNative = await r.loc.evaluate((n) => n.tagName === "SELECT").catch(() => false);
        if (isNative) {
          await r.loc.selectOption({ index: 1 }, { timeout: 4000 });
          action = "select";
        } else {
          await r.loc.click({ timeout: 4000 });
        }
      } else {
        await r.loc.click({ timeout: 4000 });
      }
      clicked = true;
    } catch (e) {
      failure = String(e.message || e).slice(0, 160);
      if (!/detached|not visible|not stable|Timeout/i.test(failure) || attempt === 1) break;
      // §8.7: one retry, only when the node was replaced by a navigation.
      await page.waitForTimeout(120);
    }
  }
  if (!clicked) {
    return makeRow(screen, el, "click", "unreachable", failure || "click impossible", Date.now() - t0, { vanished });
  }

  const clickAt = Date.now();
  if (d) {
    // Reached the confirmation? Wait up to 5 s for a dialog or a layer.
    const confirmDeadline = Date.now() + 5000;
    let confirmed = ctx.events.dialogs.length > mark.dialogs;
    while (!confirmed && Date.now() < confirmDeadline) {
      if (await newestLayer(page)) {
        confirmed = true;
        break;
      }
      confirmed = ctx.events.dialogs.length > mark.dialogs;
      if (!confirmed) await page.waitForTimeout(100);
    }
    if (!confirmed) {
      const row = makeRow(screen, el, "click", "blocked-destructive", "no-confirm: l'action part directement", Date.now() - t0, { confirm: "no-confirm" });
      row.evidence = await shot(page, ctx, screen, index, slug(el.name, 30), "blocked-destructive");
      return row;
    }
    // Cancel: Escape closes a sheet, the dialog was already dismissed.
    for (let i = 0; i < 3; i++) {
      const layer = await newestLayer(page);
      if (!layer) break;
      const cancel = layer.getByRole("button", { name: /^(Annuler|Fermer|Réduire|Non)/i }).first();
      if ((await cancel.count()) > 0) await cancel.click({ timeout: 2000 }).catch(() => {});
      else await page.keyboard.press("Escape").catch(() => {});
      await page.waitForTimeout(150);
    }
    await page.waitForTimeout(200);
    const after = await fingerprint(page);
    const returned = after.hash === before.hash && (after.aria === before.aria || after.layers === before.layers);
    const row = makeRow(
      screen,
      el,
      "click",
      "blocked-destructive",
      returned ? "confirm-reached" : "confirm-reached, annulation laisse l'écran changé",
      Date.now() - t0,
      { confirm: "confirm-reached", cancelBroken: !returned, reversible: !!d.reversible }
    );
    return row;
  }

  // §8.3: "attendre 1 200 ms ou la fin d'une navigation". A navigation or a
  // layer is a definite RESULT, so the window ends there; `nothing` keeps the
  // whole 1 200 ms of silence, which is the only thing that proves it.
  const changedEarly = await page
    .waitForFunction(
      ({ hash, layers }) => {
        const count = new Set(
          [...document.querySelectorAll("[role=dialog],[role=menu],.sheet,[aria-modal]")].filter((el) => el.getClientRects().length)
        ).size;
        return location.hash !== hash || count > layers;
      },
      { hash: before.rawHash, layers: before.layers },
      { timeout: OBSERVATION_MS, polling: 50 }
    )
    .then(() => true)
    .catch(() => false);
  if (changedEarly) await page.waitForTimeout(250);
  const t1 = Date.now();
  const after = await fingerprint(page);
  const longtasks = await page.evaluate(() => (window.__bh ? window.__bh.longtasks.slice() : []));
  const observed = {
    pageerrors: since(ctx.events.pageerrors, mark.pageerrors, t0, t1),
    consoleErrors: since(ctx.events.consoleErrors, mark.consoleErrors, t0, t1),
    httpErrors: since(ctx.events.httpErrors, mark.httpErrors, t0, t1),
    failed: since(ctx.events.failed, mark.failed, t0, t1),
    dialogs: since(ctx.events.dialogs, mark.dialogs, t0, t1),
    deezerOff: since(ctx.events.deezerOff, mark.deezerOff, t0, t1),
    newPages: since(ctx.events.newPages, mark.newPages, t0, t1),
    downloads: since(ctx.events.downloads, mark.downloads, t0, t1),
    longtasks,
  };
  let { outcome, detail } = classify(before, after, observed);
  if (outcome === "nothing") {
    await page.waitForTimeout(NOTHING_CONFIRM_MS);
    const t1b = Date.now();
    const after2 = await fingerprint(page);
    const long2 = await page.evaluate(() => (window.__bh ? window.__bh.longtasks.slice() : []));
    const observed2 = {
      pageerrors: since(ctx.events.pageerrors, mark.pageerrors, t0, t1b),
      consoleErrors: since(ctx.events.consoleErrors, mark.consoleErrors, t0, t1b),
      httpErrors: since(ctx.events.httpErrors, mark.httpErrors, t0, t1b),
      failed: since(ctx.events.failed, mark.failed, t0, t1b),
      dialogs: since(ctx.events.dialogs, mark.dialogs, t0, t1b),
      deezerOff: since(ctx.events.deezerOff, mark.deezerOff, t0, t1b),
      newPages: since(ctx.events.newPages, mark.newPages, t0, t1b),
      downloads: since(ctx.events.downloads, mark.downloads, t0, t1b),
      longtasks: long2,
    };
    const again = classify(before, after2, observed2);
    if (again.outcome !== "nothing") {
      outcome = again.outcome;
      detail = `${again.detail} (après ${OBSERVATION_MS} ms)`;
    } else {
      detail = `aucun changement en ${OBSERVATION_MS + NOTHING_CONFIRM_MS} ms`;
    }
  }
  const ms = Date.now() - t0;
  const row = makeRow(screen, el, action, outcome, detail, ms);
  if (observed.deezerOff.length && outcome === "nothing") {
    row.deezerOff = true;
    row.detail = "l'action demande Deezer, éteint sur le banc";
  }
  if (ANOMALIES.has(outcome) || (outcome === "nothing" && namedAction(el.name))) {
    row.evidence = await shot(page, ctx, screen, index, slug(el.name, 30), outcome);
  }
  row._layerOpened = outcome === "opened-layer";
  return row;
}

/** Walk the controls of the layer a click just opened, depth 1 (§8.4). */
export async function walkLayer(page, screen, ctx, index) {
  const rows = [];
  const layer = await newestLayer(page);
  if (!layer) return rows;
  const key = await layerKey(layer);
  if (ctx.layersSeen.has(screen.name + "|" + key)) return rows;
  ctx.layersSeen.add(screen.name + "|" + key);
  const snapshot = await layer.ariaSnapshot().catch(() => "");
  if (!snapshot) return rows;
  const nodes = parseSnapshot(snapshot).filter((n) => INTERACTIVE_ROLES.has(n.role));
  const els = withOccurrences(nodes).map((n) => ({
    role: n.role,
    name: n.name,
    state: n.state,
    via: "aria",
    selector: `role=${n.role}[name=${JSON.stringify(n.name)}]`,
    signature: signature(n.role, n.name),
    occurrence: n.occurrence,
    duplicateOf: n.duplicateOf ? signature(n.duplicateOf.role, n.duplicateOf.name) : null,
    disabled: /\[disabled\]/.test(n.state),
    inLayer: true,
  }));
  await markDomDuplicates(page, els, layer);
  for (const el of els) {
    if (ctx.budgetReached()) {
      ctx.stopped = "budget";
      break;
    }
    if (el.duplicateOf) {
      const dup = makeRow(screen, el, "click", "duplicate", "frère dédupliqué", 0, { inLayer: true });
      rows.push(dup);
      ctx.counts.duplicates += 1;
      continue;
    }
    const started = Date.now();
    const row = await clickAndObserve(page, el, screen, ctx, { scope: layer, index });
    row.inLayer = true;
    if (process.env.BUGHUNT_DEBUG) console.log(`  L ${row.outcome.padEnd(18)} ${String(Date.now() - started).padStart(5)} ms  ${el.role} ${JSON.stringify(el.name).slice(0, 60)}`);
    if (row.outcome === "navigated") {
      rows.push(row);
      ctx.clicks += 1;
      return rows; // the layer is gone; restore() puts the screen back
    }
    if (row.outcome === "opened-layer") {
      ctx.layersOutOfDepth.push(`${screen.name} : « ${el.name || key} »`);
      // depth 1: close the nested layer instead of walking it
      for (let i = 0; i < 2; i++) {
        if (!(await newestLayer(page))) break;
        await page.keyboard.press("Escape").catch(() => {});
        await page.waitForTimeout(120);
      }
    }
    rows.push(row);
    ctx.clicks += 1;
    if (!(await newestLayer(page))) break;
  }
  // Close the layer by its own control when it has one, then Escape.
  const again = await newestLayer(page);
  if (again) {
    const close = again.getByRole("button", { name: /^(Fermer|Annuler|Réduire|Non)/i }).first();
    if ((await close.count()) > 0) await close.click({ timeout: 2000 }).catch(() => {});
    for (let i = 0; i < 2 && (await newestLayer(page)); i++) {
      await page.keyboard.press("Escape").catch(() => {});
      await page.waitForTimeout(120);
    }
  }
  return rows;
}

/**
 * §4.4/§8.7 — walk one screen: enumerate, click what survives dedup, walk the
 * layers opened, restore between clicks, screenshot the screen once. Returns
 * { rows, elementsFound, screenshot }; `rows` already carries the layer rows.
 */
export async function walkScreen(page, screen, ctx, index) {
  const rows = [];
  const opened = await openScreen(page, ctx.base, screen, ctx.ids);
  if (!opened) {
    const diag = await page.evaluate(() => ({ href: location.href, text: (document.querySelector("main")?.innerText || "").length, ready: !!document.querySelector("main .wp .chip") })).catch(() => ({}));
    console.error(`bughunt: open failed ${screen.hash} ${JSON.stringify(diag)}`);
    ctx.counts.screensFailed += 1;
    ctx.failedScreens.push(screen);
    console.error(`bughunt: screen unreachable ${screen.hash} (${screen.name})`);
    return { rows, elementsFound: 0, screenshot: null };
  }
  const screenshot = await shot(page, ctx, screen, index, "", "");
  await revealAll(page);
  // §4.7: the screen's aria snapshot, next to its screenshot.
  if (ctx.shots) {
    const dir = join(ctx.dir, "screens");
    mkdirSync(dir, { recursive: true });
    const aria = await page.locator("body").ariaSnapshot().catch(() => "");
    if (aria) writeFileSync(join(dir, `${String(index).padStart(2, "0")}-${slug(screen.name, 30)}.aria.yml`), aria);
  }
  const els = await enumerate(page, screen);
  let clickable = 0;
  for (const el of els) {
    if (ctx.budgetReached()) {
      ctx.stopped = "budget";
      break;
    }
    if (el.duplicateOf) {
      rows.push(makeRow(screen, el, "click", "duplicate", "frère dédupliqué", 0));
      ctx.counts.duplicates += 1;
      continue;
    }
    // The persistent shell (everything outside .route-host) is counted ONCE
    // across the whole pass: the next screens only see it again.
    const shell = !(await inRouteHost(page, el));
    if (shell && ctx.shellSeen.has(el.signature)) {
      ctx.counts.duplicates += 1;
      continue;
    }
    if (shell) ctx.shellSeen.add(el.signature);
    clickable += 1;
    const started = Date.now();
    const row = await clickAndObserve(page, el, screen, ctx, { index });
    if (process.env.BUGHUNT_DEBUG) console.log(`  . ${row.outcome.padEnd(18)} ${String(Date.now() - started).padStart(5)} ms  ${el.role} ${JSON.stringify(el.name).slice(0, 60)}`);
    if (row._layerOpened) {
      const layerRows = await walkLayer(page, screen, ctx, index);
      rows.push(...layerRows);
      delete row._layerOpened;
    }
    rows.push(row);
    ctx.clicks += 1;
    await restore(page, screen, ctx);
  }
  return { rows, elementsFound: clickable, screenshot };
}

/**
 * Can a finger reach this element? A non-empty box, visible, and the hit test
 * at its centre lands on it (or inside it). An element under a hover overlay or
 * with `pointer-events: none` is in the tree and unreachable — the plan's
 * `unreachable`, decided in milliseconds instead of a click timeout.
 */
async function reachable(loc) {
  return loc
    .evaluate((el) => {
      const cs = getComputedStyle(el);
      if (cs.visibility === "hidden" || cs.display === "none") return false;
      if (cs.pointerEvents === "none") return false;
      const r = el.getBoundingClientRect();
      if (r.width < 2 || r.height < 2) return false;
      const x = Math.min(window.innerWidth - 1, Math.max(0, r.left + r.width / 2));
      const y = Math.min(window.innerHeight - 1, Math.max(0, r.top + r.height / 2));
      const hit = document.elementFromPoint(x, y);
      return !!hit && (hit === el || el.contains(hit) || (hit.closest && hit.closest("button, a[href], [role=button]") === el));
    })
    .catch(() => true);
}

/** Does this element live in the routed content (not in the persistent shell)? */
async function inRouteHost(page, el) {
  const r = await resolve(page, el);
  if (!r) return true;
  return r.loc.evaluate((n) => !!n.closest(".route-host")).catch(() => true);
}

/** The guard: [] when the table is complete, else one message per gap. */
export function driftProblems() {
  const problems = [];
  const routes = declaredRoutes();
  for (const route of routes) {
    const alias = ROUTE_ALIASES[route];
    const covered = alias
      ? SCREENS.some((s) => s.hash.replace(/\{.*?\}/g, "x").startsWith(alias.replace(/\/:[^/]+$/, "")))
      : SCREENS.some((s) => patternMatches(route, s.hash));
    if (!covered) problems.push(`undeclared route ${route}`);
  }
  const sections = declaredSections();
  for (const id of sections) {
    if (!SCREENS.some((s) => s.hash === "#/settings/" + id)) problems.push(`undeclared section /settings/${id}`);
  }
  return problems;
}
