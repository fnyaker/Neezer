// The glass ladder's rules, read off the SOURCE — the part a browser cannot
// prove cheaply: there is exactly ONE implementation of translucency in the
// app, and no component carries a blur value of its own.
//
// The ladder (src/app.css, "surfaces — the glass ladder"):
//   tier 1  --glass-bg   64 % of the theme fill, NO blur    cards, bars
//   tier 2  --glass-bg-2 78 % + blur(var(--glass-blur-2))   popovers
//   tier 3  --glass-bg-2 78 % + blur(var(--glass-blur-3))   sheets, modals
//   thick   --glass-bg-strong 93 %, no blur                 top bar, notices
//
// Every tier also carries the LIGHT ("the liquid finish"): two 1 px background
// gradients — a specular catch on the lit edge and a dark rim below — plus a
// two-layer shadow, and a fill mixed with 4 % of the theme's accent over 3 % of
// white (the body is lit from inside). Painted,
// never filtered: the tests below hold that, and hold the lens out.
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

// `backdrop-filter: none` or a tier's own filter token, nothing else: blur,
// saturation and the brightness of the image behind the pane live in ONE place
// (app.css), so no component can assemble a fourth expression.
const ALLOWED = /^backdrop-filter:\s*(none|var\(--glass-filter-[23]\));?$/;

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

test("les deux filtres du verre sont écrits une fois, avec les jetons du palier", () => {
  // The filter itself (blur + the extra saturation) is defined for the two
  // overlay tiers and nowhere else; a component reads it.
  for (const n of ["2", "3"]) {
    const at = APP_CSS.indexOf(`--glass-filter-${n}:`);
    assert.ok(at > 0, `--glass-filter-${n} manque`);
    const value = APP_CSS.slice(at, APP_CSS.indexOf(";", at)).replace(/\s+/g, " ");
    assert.match(value, new RegExp(`blur\\(var\\(--glass-blur-${n}\\)\\)`), `filtre ${n} sans son flou`);
    assert.match(value, new RegExp(`saturate\\(var\\(--glass-sat-${n}\\)\\)`), `filtre ${n} sans sa saturation`);
  }
  // A CONSTANT brightness on the glass is the wrong tool (it cannot know the
  // picture): the image is normalised per sleeve instead.
  assert.doesNotMatch(APP_CSS, /--glass-bright/, "le verre porte un éclat constant");
  for (const { rel, text } of files) {
    if (!/\.(svelte|css)$/.test(rel)) continue;
    assert.doesNotMatch(text, /backdrop-filter:[^;]*brightness\(/, `${rel} éclaircit son backdrop d'un facteur fixe`);
  }
  // And no component carries a filter expression of its own. Declarations
  // only: the comment blocks above quote the technique on purpose.
  for (const { rel, text } of files) {
    for (const decl of declarations(text)) {
      const v = decl.replace(/^backdrop-filter:\s*/, "").replace(/;$/, "");
      assert.ok(/^(none|var\(--glass-filter-[23]\))$/.test(v), `${rel} écrit son propre filtre : ${v}`);
    }
  }
});

test("l'image derrière le verre est normalisée, pas multipliée par une constante", () => {
  // The cover ground and the full-screen player's backdrop both carry the
  // sleeve's normalised level; the rule itself is `levelFactor`, held by
  // test/imagelevel.test.mjs on the numbers.
  const ground = files.find((f) => f.rel === "components/GlobalBackdrop.svelte");
  assert.ok(ground, "GlobalBackdrop.svelte introuvable");
  assert.match(ground.text, /levelFactor\(palette\.average\)/, "le fond de page ne normalise pas la pochette");
  assert.match(ground.text, /--gb-bright:/, "le fond de page ne porte pas son niveau");
  assert.match(ground.text, /brightness\(var\(--gb-bright/, "le fond de page n'applique pas son niveau");
  for (const rel of ["components/DesktopNowPlaying.svelte", "components/MobileNowPlaying.svelte"]) {
    const file = files.find((f) => f.rel === rel);
    assert.ok(file, `${rel} introuvable`);
    assert.match(file.text, /--bg-bright:/, `${rel} ne porte pas le niveau de la pochette`);
    assert.match(file.text, /brightness\(var\(--bg-bright/, `${rel} n'applique pas le niveau`);
  }
  // `walk()` only collects .svelte/.css: the player's store is read directly.
  const player = readFileSync(new URL("../src/lib/backdrop.js", import.meta.url), "utf8");
  assert.match(player, /sampleOfImage\(im\)/, "la couche du lecteur n'échantillonne pas son image");
  assert.match(player, /sample\?\.factor/, "la couche du lecteur ne porte pas son niveau");
});

test("app.css définit les trois paliers, et eux seuls", () => {
  for (const token of ["--glass-bg", "--glass-blur", "--glass-sat", "--glass-border", "--glass-shadow"]) {
    assert.match(APP_CSS, new RegExp(`\\n\\s*${token}:`), `${token} manque à app.css`);
  }
  for (const n of ["1", "2", "3"]) {
    assert.match(APP_CSS, new RegExp(`--glass-blur-${n}:`), `le palier ${n} n'a pas de blur`);
    assert.match(APP_CSS, new RegExp(`--glass-sat-${n}:`), `le palier ${n} n'a pas de saturation`);
  }
  for (const n of ["2", "3"]) {
    assert.match(APP_CSS, new RegExp(`--glass-filter-${n}:`), `le palier ${n} n'a pas de filtre`);
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
  assert.match(APP_CSS, /\.menu\s*\{[\s\S]*?backdrop-filter:\s*var\(--glass-filter-2\)/);
  assert.match(APP_CSS, /\.sheet,[\s\S]*?\{[\s\S]*?backdrop-filter:\s*var\(--glass-filter-3\)/);
});

// ── the liquid finish ────────────────────────────────────────────────────────
// The LIGHT on top of the tint: a 1 px specular gradient on the lit edge, a
// 1 px dark rim below, one extra shadow step for thickness, and a fill tinted
// by the theme's accent. Measured on pixels by test/glass/run.mjs (the beam has
// to exist, be brighter at its 12 % than at its 62 %, and vanish in eco).
// The LENS (a displacement map) is deliberately absent: see the block in
// app.css, and the assertion at the end of this file which keeps it out.

const themeBlock = (name) => {
  const at = APP_CSS.indexOf(`[data-theme="${name}"] {`);
  assert.ok(at > 0, `le thème ${name} manque`);
  return APP_CSS.slice(at, APP_CSS.indexOf("}", at));
};

test("la finition liquide : un liseré spéculaire et un bord sombre par thème", () => {  for (const theme of ["encre", "papier", "pochette"]) {
    const block = themeBlock(theme);
    assert.match(block, /--glass-hi:\s*[^;]+;/, `${theme} n'a pas de lumière`);
    assert.match(block, /--glass-lo:\s*[^;]+;/, `${theme} n'a pas de bord`);
  }
  // The two gradients, and the vertical one a column needs (it has no top edge
  // on screen, only a side one).
  assert.match(APP_CSS, /--glass-beam-x:\s*linear-gradient\(/);
  assert.match(APP_CSS, /--glass-beam-y:\s*linear-gradient\(/);
  assert.match(APP_CSS, /--glass-rim-x:\s*linear-gradient\(/);
  // The tint follows the theme's accent (Pochette's sleeve included), and it
  // stays a TINT: above 8 % the fill stops being translucent glass and starts
  // being a coloured pane, which is the contrast failure this ladder forbids.
  const ac = APP_CSS.match(/--glass-tint-ac:\s*([\d.]+)%/);
  assert.ok(ac, "--glass-tint-ac manque");
  assert.ok(Number(ac[1]) > 0 && Number(ac[1]) <= 8, `teinte d'accent ${ac[1]} % hors de [0, 8]`);
  // The body is lit from inside (3 % of white) AND tinted by the theme: both
  // halves are in the fill, and the white stays small — at 4 % the card's worst
  // `--dim` pair measured 4.55:1, i.e. AA by a hair.
  const lift = APP_CSS.match(/--glass-lift:\s*([\d.]+)%/);
  assert.ok(lift, "--glass-lift manque");
  assert.ok(Number(lift[1]) > 0 && Number(lift[1]) <= 3, `lumière ${lift[1]} % hors de [0, 3]`);
  assert.match(
    APP_CSS,
    /--glass-fill:\s*color-mix\(\s*in srgb,\s*#fff var\(--glass-lift\),\s*color-mix\(in srgb, var\(--ac\) var\(--glass-tint-ac\), var\(--s1\)\)\s*\)/
  );
  for (const n of ["", "-2", "-strong"]) {
    assert.match(
      APP_CSS,
      new RegExp(`--glass-bg${n}:\\s*color-mix\\(in srgb, var\\(--glass-fill\\)`),
      `--glass-bg${n} ne lit pas la teinte du thème`
    );
  }
});

test("chaque palier peint la lumière, en une seule déclaration de fond", () => {
  // `.card`/`.glass-1` = tier 1 without blur; `.menu` = tier 2; `.sheet` = tier 3.
  const tiers = [
    [/\.card\s*\{[\s\S]*?\}/, ".card"],
    [/\.glass-1\s*\{[\s\S]*?\}/, ".glass-1"],
    [/\.menu\s*\{[\s\S]*?\}/, ".menu"],
    [/\.sheet,[\s\S]*?\{[\s\S]*?\}/, ".sheet"],
    [/\.glass-3\s*\{[\s\S]*?\}/, ".glass-3"],
  ];
  for (const [re, name] of tiers) {
    const m = APP_CSS.match(re);
    assert.ok(m, `${name} introuvable`);
    assert.match(m[0], /--glass-beam-x/, `${name} n'a pas de liseré`);
    assert.match(m[0], /--glass-rim-x/, `${name} n'a pas de bord bas`);
  }
  // The shadow steps carry a contact layer as well as an ambient one: a single
  // wide blur is a smudge, two are a thickness (measured look, Papier's 6 %).
  for (const theme of ["encre", "papier", "pochette"]) {
    const block = themeBlock(theme);
    const shadow = block.match(/--glass-shadow-2:\s*([^;]+);/);
    assert.ok(shadow, `${theme} n'a pas de --glass-shadow-2`);
    assert.ok((shadow[1].match(/,/g) || []).length >= 2, `${theme} : --glass-shadow-2 n'a pas deux couches`);
  }
});

test("une feuille posée sur une pochette prend le palier épais", () => {
  // The phone player's queue and lyrics panels are full-bleed over the sleeve,
  // not over the page's scrim: 78 % there would be well under AA, and 88 %
  // read 4.43:1, so the ONE definition in app.css gives that case the thick
  // step (93 %, which keeps its opacity and takes the fill's light).
  assert.match(APP_CSS, /\.sheet\.on-photo\s*\{[^}]*--glass-bg-strong/, "la variante « sur une pochette » manque");
  const player = files.find((f) => f.rel === "components/MobileNowPlaying.svelte");
  assert.ok(player, "MobileNowPlaying.svelte introuvable");
  assert.match(player.text, /class="sheet on-photo"/, "la file d'attente ne prend pas le palier épais");
  assert.match(player.text, /class="sheet on-photo lyrics-view"/, "les paroles ne prennent pas le palier épais");
});

test("le strip du visualiseur n'a aucun voile : il doit porter --ac-text exactement", () => {
  // The animation agent's contract: the bar strip is ONE flat colour, exactly
  // --ac-text. An `opacity` on the box that holds it composites that colour with
  // the sleeve behind (measured #a9cd33 for #d2ff3c on Encre at 0.9; 0 % of the
  // bars' pixels were the accent, 7–10 % were the blend). Nothing may veil it.
  for (const rel of ["components/DesktopNowPlaying.svelte", "components/MobileNowPlaying.svelte"]) {
    const file = files.find((f) => f.rel === rel);
    assert.ok(file, `${rel} introuvable`);
    const at = file.text.indexOf("\n  .viz {");
    assert.ok(at > 0, `${rel} : le bloc .viz manque`);
    const block = file.text.slice(at, file.text.indexOf("}", at));
    const decls = block
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => /^(opacity|filter):/.test(l));
    assert.deepEqual(decls, [], `${rel} : .viz porte un voile (${decls.join(", ")})`);
  }
});

test("éco : la lumière s'éteint avec le flou, aucun liseré ne survit", () => {
  // `transparent` rather than `none`: the four `background` declarations that
  // compose the beams must stay valid rules.
  const eco = APP_CSS.slice(APP_CSS.indexOf('html[data-eco="1"] {'));
  const block = eco.slice(0, eco.indexOf("}"));
  assert.match(block, /--glass-hi:\s*transparent/, "le liseré survit en éco");
  assert.match(block, /--glass-lo:\s*transparent/, "le bord sombre survit en éco");
  const media = APP_CSS.slice(APP_CSS.indexOf("@media (prefers-reduced-transparency: reduce)"));
  assert.match(media, /--glass-hi:\s*transparent/, "le liseré survit en transparence réduite");
  assert.match(media, /--glass-lo:\s*transparent/, "le bord survit en transparence réduite");
  // No tier redefines a beam inside those blocks: one source of light.
  for (const tail of [block, media.slice(0, media.indexOf("backdrop-filter"))]) {
    assert.doesNotMatch(tail, /--glass-beam/, "un bloc d'accessibilité redéfinit un liseré");
  }
});

test("le verre arrive au lieu d'apparaître — et pas sous `prefers-reduced-motion`", () => {
  // `@starting-style` is the arrival (a clear pane that walks to its tint in
  // 160 ms), not a keyframe animation: nothing re-runs on a re-render, no timer.
  const at = APP_CSS.indexOf("@starting-style {");
  assert.ok(at > 0, "@starting-style manque : le matériau apparaît d'un coup");
  const guard = APP_CSS.lastIndexOf("@media (prefers-reduced-motion: no-preference)", at);
  assert.ok(guard > 0, "l'arrivée du verre n'est pas derrière prefers-reduced-motion");
  assert.match(APP_CSS.slice(at, at + 400), /background-color:\s*transparent/);
  assert.match(APP_CSS.slice(guard, at), /transition:[^;]*background-color\s+160ms/);
});

test("la lentille reste dehors : aucun déplacement de fond dans l'app", () => {
  // `backdrop-filter: url(#…)` is Chromium-only (Safari drops the whole
  // declaration, Firefox ignores the filter) and re-samples the captured area
  // every frame — the one cost the ladder exists to bound. A displacement map
  // would also have to be a per-surface asset, i.e. a second definition of the
  // material. The light is painted; the lens is not imitated.
  // Declarations only: the comment blocks above quote the technique on purpose.
  const decls = (text) =>
    text
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => /^(backdrop-)?filter:\s/.test(l));
  for (const { rel, text } of files) {
    for (const decl of decls(text)) {
      assert.ok(!decl.includes("url("), `${rel} pointe un filtre SVG : ${decl}`);
    }
    assert.doesNotMatch(text, /<filter\b/, `${rel} déclare un filtre SVG`);
  }
});
