// The glass ladder's rules, read off the SOURCE — the part a browser cannot
// prove cheaply: there is exactly ONE implementation of translucency in the
// app, and no component carries a blur value of its own.
//
// The ladder (src/app.css, "surfaces — the glass ladder"):
//   tier 1  --glass-bg   72 % of --s1, NO blur          cards, bars, notices
//   tier 2  --glass-bg-2 82 % + blur(var(--glass-blur-2)) popovers
//   tier 3  --glass-bg-2 82 % + blur(var(--glass-blur-3)) sheets, modals
//
// A `backdrop-filter` is therefore one of: `none`, or the tier-2/tier-3
// expression written with the tokens. A literal (`blur(2px)`, `blur(6px)`,
// `blur(18px)`…) in a component is the drift this test forbids — the app had
// six different ones, each a separate render pass doing the same work.
//
// There is no exception left: MobileNowPlaying.svelte (the theme agent's file)
// moved its two overlays onto --glass-blur-3 / --glass-sat-3 once the tokens
// existed. The list stays, so a future exception has to be written down here.

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const SRC = fileURLToPath(new URL("../src/", import.meta.url));
const APP_CSS = readFileSync(new URL("../src/app.css", import.meta.url), "utf8");

// Documented exceptions: a file another agent is converting.
const EXCEPTIONS = new Set();

function walk(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...walk(path));
    else if (/\.(svelte|css)$/.test(name)) out.push(path);
  }
  return out;
}

const files = walk(SRC).map((path) => ({
  rel: relative(SRC, path).replaceAll("\\", "/"),
  text: readFileSync(path, "utf8"),
}));

// `backdrop-filter: none` or the ladder's own expression, nothing else.
const ALLOWED =
  /^backdrop-filter:\s*(none|blur\(var\(--glass-blur-[23]\)\)\s*saturate\(var\(--glass-sat-[23]\)\));?$/;

function declarations(text) {
  return text
    .split("\n")
    .map((line) => line.trim().replace(/\s*!important;?$/, ";"))
    .filter((line) => line.startsWith("backdrop-filter:"))
    .map((line) => (line.endsWith(";") ? line : line + ";"));
}

test("aucun backdrop-filter littéral : chaque déclaration est un palier ou `none`", () => {
  const offenders = [];
  for (const { rel, text } of files) {
    if (EXCEPTIONS.has(rel)) continue;
    for (const decl of declarations(text)) {
      if (!ALLOWED.test(decl)) offenders.push(`${rel}: ${decl}`);
    }
  }
  assert.deepEqual(offenders, [], `déclarations hors paliers :\n  ${offenders.join("\n  ")}`);
});

test("les seules valeurs de blur du projet sont les trois paliers", () => {
  // Every blur() that reaches the compositor comes from a tier token; the two
  // literals left are MobileNowPlaying's, listed by name rather than invented.
  const literals = new Set();
  const tokens = new Set();
  for (const { text } of files) {
    for (const m of text.matchAll(/backdrop-filter:\s*blur\((var\(--glass-blur-[123]\)|\d+(?:\.\d+)?px)\)/g)) {
      if (m[1].startsWith("var(")) tokens.add(m[1]);
      else literals.add(m[1]);
    }
  }
  assert.deepEqual([...literals].sort(), [], "un composant a réintroduit un flou littéral");
  assert.deepEqual([...tokens].sort(), ["var(--glass-blur-2)", "var(--glass-blur-3)"], "un palier inconnu est utilisé");
});

test("app.css définit les trois paliers, et eux seuls", () => {
  for (const token of ["--glass-bg", "--glass-blur", "--glass-sat", "--glass-border", "--glass-shadow"]) {
    assert.match(APP_CSS, new RegExp(`\\n\\s*${token}:`), `${token} manque à app.css`);
  }
  for (const n of ["1", "2", "3"]) {
    assert.match(APP_CSS, new RegExp(`--glass-blur-${n}:`), `le palier ${n} n'a pas de blur`);
    assert.match(APP_CSS, new RegExp(`--glass-sat-${n}:`), `le palier ${n} n'a pas de saturation`);
  }
  // Three tiers, not a gradient of twelve values: one blur per tier, and tier 1
  // is 0 (a static card never blurs its own backdrop).
  // The FIRST declaration per tier is the `:root` one; the eco blocks later in
  // the sheet set them to 0 px, which the next test checks.
  const tiers = new Map();
  for (const m of APP_CSS.matchAll(/--glass-blur(?:-(\d))?:\s*([^;]+);/g)) {
    const tier = m[1] || "1";
    if (!tiers.has(tier)) tiers.set(tier, m[2].trim());
  }
  assert.deepEqual([...tiers].sort(), [["1", "0px"], ["2", "14px"], ["3", "22px"]]);
});

test("éco et transparence réduite : aucun flou ne survit, les surfaces sont opaques", () => {
  const eco = APP_CSS.slice(APP_CSS.indexOf('html[data-eco="1"] {'));
  assert.ok(eco.length > 0, "la règle html[data-eco=\"1\"] manque");
  const block = eco.slice(0, eco.indexOf("}"));
  for (const token of ["--glass-bg", "--glass-bg-2", "--glass-border", "--glass-shadow"]) {
    assert.match(block, new RegExp(`${token}:`), `${token} n'est pas remis à plat en éco`);
  }
  assert.match(block, /--glass-blur-2:\s*0px/, "le palier 2 garde un flou en éco");
  assert.match(block, /--glass-blur-3:\s*0px/, "le palier 3 garde un flou en éco");
  // A kill switch, so a component that reintroduces a filter cannot survive.
  assert.match(eco, /html\[data-eco="1"\] \*[^{]*\{[^}]*backdrop-filter:\s*none !important/);
  // The OS's own switch does the same (MDN: prefers-reduced-transparency).
  const media = APP_CSS.slice(APP_CSS.indexOf("@media (prefers-reduced-transparency: reduce)"));
  assert.ok(media.length > 0, "la requête prefers-reduced-transparency manque");
  assert.match(media, /--glass-blur-2:\s*0px/);
  assert.match(media, /backdrop-filter:\s*none !important/);
});

test("les surfaces consomment les paliers, jamais un --s1 littéral pour un fond translucide", () => {
  // The four surfaces this change owns must read the ladder, not a theme
  // surface: a `background: var(--s1)` there would be opaque again.
  const owned = {
    "components/Sidebar.svelte": /--glass-bg\b/,
    "components/TopBar.svelte": /--glass-bg-strong\b/,
    "components/PlayerBar.svelte": /--glass-bg-2?\b/,
    "components/Card.svelte": /--glass-bg\b/,
    "components/Notices.svelte": /--glass-bg-2\b/,
    "components/ContextMenu.svelte": /glass|--glass/,
    "components/QualityMenu.svelte": /glass|--glass/,
  };
  for (const [rel, re] of Object.entries(owned)) {
    const file = files.find((f) => f.rel === rel);
    assert.ok(file, `${rel} introuvable`);
    assert.match(file.text, re, `${rel} ne lit pas le palier partagé`);
  }
  // Menus and sheets take their material from app.css: one rule for the
  // eighteen sheets, not eighteen copies.
  assert.match(APP_CSS, /\.menu\s*\{[\s\S]*?backdrop-filter:\s*blur\(var\(--glass-blur-2\)\)/);
  assert.match(APP_CSS, /\.sheet,[\s\S]*?\{[\s\S]*?backdrop-filter:\s*blur\(var\(--glass-blur-3\)\)/);
});
