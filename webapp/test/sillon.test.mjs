// Sillon's palette rules (lib/sillon.js): the placeholder groove, the avatar
// initials, and the WCAG lift the Pochette theme applies to a sleeve's colour.
//
// The numbers below are what makes the fallbacks trustworthy: a deterministic
// groove (not a colour that changes on reload), a spread that never parks a
// page on one disc, and an accent that is ALWAYS legible on --onac.

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

import {
  GROOVES,
  ACCENT_FALLBACK,
  ON_ACCENT,
  hashId,
  grooveFor,
  grooveBackground,
  grooveTint,
  initialsOf,
  luminance,
  contrast,
  accentFor,
  readableAccent,
  toHex,
  fromHex,
} from "../src/lib/sillon.js";

// lib/theme.js reads localStorage at import and writes <html> through the
// store subscriptions, so the tests below bring a four-line DOM. They also run
// the pre-paint script (public/theme-boot.js) in a vm with the same storage:
// the two implementations of the custom accent must agree pixel for pixel.
const bootPath = fileURLToPath(new URL("../public/theme-boot.js", import.meta.url));
const indexHtml = readFileSync(new URL("../index.html", import.meta.url), "utf8");

const fakeStorage = new Map();
const fakeProps = new Map();
const fakeMeta = { attrs: {}, setAttribute(k, v) { this.attrs[k] = String(v); } };
const noop = () => {};
const fakeDocument = {
  addEventListener: noop,
  removeEventListener: noop,
  visibilityState: "visible",
  documentElement: {
    dataset: {},
    style: {
      backgroundColor: "",
      setProperty(k, v) {
        fakeProps.set(k, String(v));
      },
      removeProperty(k) {
        fakeProps.delete(k);
      },
    },
  },
  querySelector: (sel) => (String(sel).includes("theme-color") ? fakeMeta : null),
};
globalThis.document = fakeDocument;
globalThis.location = { hash: "", pathname: "/" };
globalThis.window = {
  addEventListener: noop,
  removeEventListener: noop,
  location: globalThis.location,
  matchMedia: () => ({ matches: false, addEventListener: noop }),
  innerWidth: 1280,
  innerHeight: 900,
};
globalThis.localStorage = {
  getItem: (k) => (fakeStorage.has(k) ? fakeStorage.get(k) : null),
  setItem: (k, v) => fakeStorage.set(k, String(v)),
  removeItem: (k) => fakeStorage.delete(k),
  clear: () => fakeStorage.clear(),
};

const themeLib = await import("../src/lib/theme.js");

const themeBg = { encre: "#1E1E1B", papier: "#FBF9F4" }; // INK_BG: --s2 / --s1
const CUSTOM_SAMPLES = [
  ["#FFF176", "clair"],
  ["#7C5CFF", "moyen"],
  ["#101C3A", "sombre"],
];

// Run the real pre-paint script with a given storage snapshot. It is a
// classic script by contract (no imports), so a vm is exactly how a browser
// executes it: synchronously, before the app module.
function runBoot(storage) {
  const props = new Map();
  const attrs = new Map();
  const html = {
    style: {
      backgroundColor: "",
      setProperty(k, v) {
        props.set(k, String(v));
      },
      removeProperty(k) {
        props.delete(k);
      },
    },
    setAttribute(k, v) {
      attrs.set(k, String(v));
    },
  };
  const meta = { attrs: {}, setAttribute(k, v) { this.attrs[k] = String(v); } };
  const sandbox = {
    localStorage: {
      getItem: (k) => (Object.prototype.hasOwnProperty.call(storage, k) ? storage[k] : null),
    },
    document: {
      documentElement: html,
      querySelector: () => meta,
    },
  };
  vm.runInNewContext(readFileSync(bootPath, "utf8"), sandbox);
  return { props, attrs, meta, html };
}

function resetThemeState() {
  fakeStorage.clear();
  fakeProps.clear();
  themeLib.clearCustomAccent();
  themeLib.setTheme("encre");
}


test("the same seed always picks the same groove", () => {
  const ids = [];
  for (let i = 0; i < 2000; i++) ids.push("track-" + i, "album-" + i, "artist:" + i);
  for (const id of ids) {
    const [a, b] = grooveFor(id);
    assert.equal(grooveFor(id)[0], a);
    assert.ok(
      GROOVES.some(([c, d]) => c === a && d === b),
      `${id} picked a pair that is not one of the eight`
    );
    assert.equal(hashId(id), hashId(String(id)));
  }
});

test("eight grooves, and a screenful of ids does not land on one", () => {
  const hits = new Map();
  for (let i = 0; i < 800; i++) {
    const [a] = grooveFor("album-" + i);
    hits.set(a, (hits.get(a) || 0) + 1);
  }
  const counts = [...hits.values()].sort((x, y) => y - x);
  // A uniform 800 over 8 would be 100 a bucket; allow the ordinary spread of a
  // hash but no bucket taking a quarter of the page.
  console.log(`# grooves: ${hits.size}/8 used, largest bucket ${counts[0]}/800`);
  assert.equal(hits.size, 8, "every groove is reachable");
  assert.ok(counts[0] <= 200, `largest bucket ${counts[0]} of 800`);
});

test("consecutive ids are not all the same disc", () => {
  // The ids a page shows really are consecutive-ish (album-217, album-218…).
  let same = 0;
  for (let i = 0; i < 200; i++) {
    if (grooveFor("album-" + i)[0] === grooveFor("album-" + (i + 1))[0]) same++;
  }
  console.log(`# consecutive pairs sharing a groove: ${same}/200`);
  assert.ok(same < 80, `${same}/200 neighbours shared a groove`);
});

test("a missing cover draws a groove, not a flat square", () => {
  const css = grooveBackground("42|Titre|7|album");
  assert.match(css, /^repeating-radial-gradient\(circle at 68% 32%/);
  const [a, b] = grooveFor("42|Titre|7|album");
  assert.ok(css.includes(a) && css.includes(b));
  assert.notEqual(grooveBackground("a"), grooveBackground("b"));
  // the avatar tile is the same pair, softer
  assert.ok(grooveTint("a").includes(grooveFor("a")[0]));
});

test("initials are two letters, accents kept, punctuation dropped", () => {
  assert.equal(initialsOf("Angerfist"), "AN");
  assert.equal(initialsOf("Dr. Peacock"), "DP");
  assert.equal(initialsOf("DJ TÖRKE"), "DT");
  assert.equal(initialsOf("Le Wanski"), "LW");
  assert.equal(initialsOf("AC/DC"), "AC");
  assert.equal(initialsOf("Tovaritch"), "TO");
  assert.equal(initialsOf(""), "");
  assert.equal(initialsOf(null), "");
  // never more than two, whatever the name
  for (const n of ["a b c d", "Été 2024", "x", "  "]) assert.ok(initialsOf(n).length <= 2, n);
});

test("hex round-trips and luminance follows WCAG", () => {
  assert.equal(toHex([12, 12, 11]), "#0C0C0B");
  assert.deepEqual(fromHex("#D2FF3C"), [210, 255, 60]);
  assert.equal(luminance([0, 0, 0]), 0);
  assert.equal(luminance([255, 255, 255]).toFixed(3), "1.000");
  assert.equal(contrast([255, 255, 255], [0, 0, 0]).toFixed(2), "21.00");
});

test("the Pochette accent always carries 4.5:1 on --onac", () => {
  const bg = fromHex(ON_ACCENT);
  // Every groove colour, both of its pair — the exact colours the fallbacks use.
  const seeds = GROOVES.flat();
  // Plus a sweep of the whole cube, coarse enough to run and dense enough to
  // catch a dark, saturated sleeve.
  const sweep = [];
  for (let r = 0; r <= 255; r += 51)
    for (let g = 0; g <= 255; g += 51) for (let b = 0; b <= 255; b += 51) sweep.push([r, g, b]);

  let lifted = 0;
  let worst = Infinity;
  for (const rgb of [...seeds, ...sweep].map((c) => (typeof c === "string" ? fromHex(c) : c))) {
    const out = accentFor(rgb);
    const ratio = contrast(fromHex(out), bg);
    worst = Math.min(worst, ratio);
    if (contrast(rgb, bg) < 4.5) lifted++;
    assert.ok(ratio >= 4.5, `${toHex(rgb)} → ${out} only ${ratio.toFixed(2)}:1`);
  }
  console.log(
    `# ${seeds.length + sweep.length} colours, ${lifted} lifted, worst ratio ${worst.toFixed(2)}:1`
  );
});

test("a colour already legible is left exactly as the sleeve gave it", () => {
  const bg = fromHex(ON_ACCENT);
  for (const hex of ["#D2FF3C", "#F1EFE8", "#FF6B4A", "#5CF2C0", "#FFC94A"]) {
    const rgb = fromHex(hex);
    assert.ok(contrast(rgb, bg) >= 4.5, `${hex} should pass as-is`);
    assert.deepEqual(readableAccent(rgb), rgb);
  }
});

test("a dark sleeve is lifted by the least brightening that works", () => {
  const bg = fromHex(ON_ACCENT);
  // A deep, saturated blue: the case that used to come out grey-lilac when the
  // lift mixed white in instead of scaling.
  const deep = [0x14, 0x30, 0x4a];
  const out = readableAccent(deep);
  assert.ok(contrast(out, bg) >= 4.5);
  // One step less (the scale loop is ×1.05) must NOT already pass: we lighten
  // until it is readable, not until it is pale.
  const oneLess = out.map((v) => Math.round(v / 1.05));
  assert.ok(contrast(oneLess, bg) < 4.5, `one step back ${toHex(oneLess)} already passes`);
  // …and the hue is kept: blue is still the largest channel.
  assert.ok(out[2] > out[0] && out[2] > out[1], toHex(out));
});

test("a sleeve with no colour at all still gives a readable accent", () => {
  for (const black of [
    [0, 0, 0],
    [4, 4, 4],
    [0, 0, 30],
  ]) {
    const out = accentFor(black);
    assert.ok(contrast(fromHex(out), fromHex(ON_ACCENT)) >= 4.5, out);
  }
  // The designed fallback is itself legible, so an empty state can use it.
  assert.ok(contrast(fromHex(ACCENT_FALLBACK), fromHex(ON_ACCENT)) >= 4.5);
});

// -- the person's own accent (Réglages › Apparence) --------------------------

test("a custom accent keeps its exact surface and legible text at every lightness", () => {
  for (const [hex, kind] of CUSTOM_SAMPLES) {
    for (const name of ["encre", "papier"]) {
      const p = themeLib.accentVars(hex, name);
      assert.equal(p.surface, hex, `${kind} on ${name}: the surface is the chosen colour`);
      // Text on the pill is pure black or white, never a themed near-black.
      assert.ok(p.on === "#000000" || p.on === "#FFFFFF", p.on);
      const onRatio = contrast(fromHex(p.surface), fromHex(p.on));
      const inkRatio = contrast(fromHex(p.ink), fromHex(themeBg[name]));
      console.log(
        `# ${kind} ${hex} on ${name}: text ${onRatio.toFixed(2)}:1 (${p.on}), ink ${p.ink} ${inkRatio.toFixed(2)}:1${p.adjusted ? " (lifted)" : ""}`
      );
      assert.ok(onRatio >= 4.5, `${hex} on ${name}: ${onRatio.toFixed(2)}:1 on the pill`);
      assert.ok(inkRatio >= 4.5, `${hex} on ${name}: ink ${p.ink} only ${inkRatio.toFixed(2)}:1`);
    }
  }
  // The light theme must darken a pale colour, the dark one must lighten a
  // dark colour — and the flags are what Réglages uses to say so.
  assert.equal(themeLib.accentVars("#FFF176", "papier").tooLight, true);
  assert.equal(themeLib.accentVars("#101C3A", "encre").tooDark, true);
  // A colour that already passes is left alone on both.
  const mid = themeLib.accentVars("#7C5CFF", "encre");
  if (contrast(fromHex("#7C5CFF"), fromHex(themeBg.encre)) >= 4.5) {
    assert.equal(mid.adjusted, false);
    assert.equal(mid.ink, "#7C5CFF");
  }
});

test("an invalid colour is refused, a shorthand one is completed", () => {
  assert.equal(themeLib.normalizeAccent("#D2FF3C"), "#D2FF3C");
  assert.equal(themeLib.normalizeAccent("d2ff3c"), "#D2FF3C");
  assert.equal(themeLib.normalizeAccent("#d2f"), "#DD22FF");
  assert.equal(themeLib.normalizeAccent("not-a-colour"), null);
  assert.equal(themeLib.normalizeAccent("#12345"), null);
  assert.equal(themeLib.normalizeAccent("#GGGGGG"), null);
  assert.equal(themeLib.normalizeAccent(""), null);
  assert.equal(themeLib.normalizeAccent(null), null);
  assert.equal(themeLib.setCustomAccent("#nope"), null);
});

test("no colour in the cube can put an illegible label on its own button", () => {
  // The on-colour is pure black or white, so this is a hard guarantee, not a
  // sample: every one of the 4 096 coarse colours must clear 4.5:1.
  let worst = Infinity;
  let worstHex = "";
  for (let r = 0; r <= 255; r += 17) {
    for (let g = 0; g <= 255; g += 17) {
      for (let b = 0; b <= 255; b += 17) {
        const hex = toHex([r, g, b]);
        const p = themeLib.accentVars(hex, "encre");
        const ratio = contrast(fromHex(p.surface), fromHex(p.on));
        if (ratio < worst) {
          worst = ratio;
          worstHex = hex;
        }
        assert.ok(ratio >= 4.5, `${hex} → ${p.on} only ${ratio.toFixed(2)}:1`);
      }
    }
  }
  console.log(`# 4 096 colours, worst label ratio ${worst.toFixed(2)}:1 (${worstHex})`);
});

test("the accent ink clears 4.5:1 on the worst surface for every colour and theme", () => {
  let worst = Infinity;
  let worstWhat = "";
  for (const name of ["encre", "papier"]) {
    for (let r = 0; r <= 255; r += 17) {
      for (let g = 0; g <= 255; g += 17) {
        for (let b = 0; b <= 255; b += 17) {
          const hex = toHex([r, g, b]);
          const p = themeLib.accentVars(hex, name);
          const ratio = contrast(fromHex(p.ink), fromHex(p.background));
          if (ratio < worst) {
            worst = ratio;
            worstWhat = `${hex} → ${p.ink} on ${name}`;
          }
          assert.ok(ratio >= 4.5, `${hex} on ${name} → ${p.ink} only ${ratio.toFixed(2)}:1`);
        }
      }
    }
  }
  console.log(`# 8 192 pairs, worst ink ratio ${worst.toFixed(2)}:1 (${worstWhat})`);
});



test("the custom accent is persisted, applied, and reset to the theme", () => {
  resetThemeState();
  assert.equal(themeLib.setCustomAccent("#7c5cff"), "#7C5CFF");
  assert.equal(fakeStorage.get("ui.accent"), JSON.stringify("#7C5CFF"));
  assert.equal(fakeProps.get("--ac"), "#7C5CFF");
  assert.equal(fakeProps.get("--btn"), "#7C5CFF");
  assert.ok(contrast(fromHex("#7C5CFF"), fromHex(fakeProps.get("--onac"))) >= 4.5);

  themeLib.clearCustomAccent();
  assert.equal(fakeStorage.has("ui.accent"), false);
  assert.equal(fakeProps.has("--ac"), false);
  assert.equal(fakeProps.has("--btn"), false);
  // The theme's own tokens take over again (app.css owns them).
  assert.equal(themeLib.themeAccentOf("encre"), "#D2FF3C");
});

test("Pochette keeps the sleeve's accent and never lets the custom one freeze it", () => {
  resetThemeState();
  themeLib.setCustomAccent("#101C3A");
  assert.equal(fakeProps.get("--ac"), "#101C3A");
  themeLib.setTheme("pochette");
  // The custom block is gone; --ac is var(--cover-ac) again.
  assert.equal(fakeProps.has("--ac"), false);
  assert.equal(fakeProps.has("--btn"), false);
  // The sleeve fallback is painted, and is not the custom colour.
  assert.equal(fakeProps.get("--cover-ac"), ACCENT_FALLBACK);
  themeLib.setTheme("encre");
  assert.equal(fakeProps.get("--ac"), "#101C3A");
  assert.equal(fakeProps.get("--cover-ac"), ACCENT_FALLBACK);
});

test("the pre-paint script paints the same pair as theme.js, before the app", () => {
  for (const [hex, kind] of CUSTOM_SAMPLES) {
    for (const name of ["encre", "papier"]) {
      const boot = runBoot({
        "ui.theme": JSON.stringify(name),
        "ui.accent": JSON.stringify(hex),
      });
      const vars = themeLib.accentVars(hex, name);
      assert.equal(boot.attrs.get("data-theme"), name);
      assert.equal(boot.props.get("--ac"), vars.surface, `${kind}/${name}: --ac`);
      assert.equal(boot.props.get("--btn"), vars.surface, `${kind}/${name}: --btn`);
      assert.equal(boot.props.get("--onac"), vars.on, `${kind}/${name}: --onac`);
      assert.equal(boot.props.get("--onbtn"), vars.on, `${kind}/${name}: --onbtn`);
      assert.equal(boot.props.get("--ac-text"), vars.ink, `${kind}/${name}: --ac-text`);
      assert.equal(boot.props.get("--ac-mark"), vars.ink, `${kind}/${name}: --ac-mark`);
      assert.equal(boot.props.get("--hrt"), vars.ink, `${kind}/${name}: --hrt`);
      assert.ok(
        contrast(fromHex(boot.props.get("--onac")), fromHex(boot.props.get("--ac"))) >= 4.5
      );
    }
  }

  // The theme's page background is set on <html> and in the meta tag, so the
  // browser chrome and the overscroll edges match before the CSS arrives.
  const papier = runBoot({ "ui.theme": JSON.stringify("papier") });
  assert.equal(papier.html.style.backgroundColor, "#F3F0E8");
  assert.equal(papier.meta.attrs.content, "#F3F0E8");

  // Pochette ignores the custom accent and keeps --cover-ac.
  const pochette = runBoot({
    "ui.theme": JSON.stringify("pochette"),
    "ui.accent": JSON.stringify("#FFF176"),
    "ui.coverAc": JSON.stringify("#123456"),
  });
  assert.equal(pochette.attrs.get("data-theme"), "pochette");
  assert.equal(pochette.props.has("--ac"), false);
  assert.equal(pochette.props.get("--cover-ac"), "#123456");

  // A garbage value is ignored, not painted; a shorthand one is completed.
  const garbage = runBoot({ "ui.theme": JSON.stringify("encre"), "ui.accent": '"red"' });
  assert.equal(garbage.props.has("--ac"), false);
  const shorthand = runBoot({ "ui.theme": JSON.stringify("encre"), "ui.accent": '"#d2f"' });
  assert.equal(shorthand.props.get("--ac"), "#DD22FF");
});

test("index.html loads the boot script blocking, in the head, before the app", () => {
  const bootAt = indexHtml.indexOf('src="/app/theme-boot.js"');
  const moduleAt = indexHtml.indexOf('type="module"');
  assert.ok(bootAt > 0, "theme-boot.js is loaded");
  assert.ok(moduleAt > bootAt, "the app module comes after the boot script");
  const tagStart = indexHtml.lastIndexOf("<script", bootAt);
  const tagEnd = indexHtml.indexOf(">", bootAt);
  const tag = indexHtml.slice(tagStart, tagEnd + 1);
  assert.ok(!/\b(defer|async)\b/.test(tag), `the boot script must block the parser: ${tag}`);
  assert.ok(!/type="module"/.test(tag), "a module would be deferred");
  assert.ok(indexHtml.indexOf("<head>") < bootAt && bootAt < indexHtml.indexOf("</head>"));
});

