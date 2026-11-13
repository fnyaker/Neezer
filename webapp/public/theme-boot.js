// The Sillon theme, before the first frame.
//
// It lives in its own file because the app's CSP is `script-src 'self'` with no
// 'unsafe-inline': an inline <script> in index.html is REFUSED by the browser
// (it was, and the console said so on every load). A classic script (no
// `type="module"`, no `defer`) blocks the parser, so it runs before <body> is
// painted — which is the whole point: a session in Papier, or in Pochette whose
// accent comes from the sleeve, must not flash the dark default for a frame.
//
// It also paints the person's own accent (`ui.accent`) here, not only on the
// route they changed it from: a reload with a custom colour would otherwise
// show the theme's accent for as long as the bundle takes to parse. The maths
// is the same as lib/theme.js#accentVars — a classic script cannot import an
// ES module before paint, so the two copies exist; test/sillon.test.mjs runs
// both on the same colours and fails if they disagree.
//
// It is ~6 KB (2.5 kB gzipped), comes down with the shell, and the service
// worker stages it (sw.js scrapes index.html's `src=` list). lib/theme.js reads
// the same keys afterwards and owns the rest of the session.
(function () {
  var BG = { encre: "#0C0C0B", papier: "#F3F0E8", pochette: "#0F0E10" };
  // The ink is corrected against the most demanding surface it is drawn on
  // (Encre's --s2, Papier's --s1), exactly as lib/theme.js#INK_BG does.
  var INK_BG = { encre: "#1E1E1B", papier: "#FBF9F4", pochette: "#0F0E10" };
  var CUSTOM = ["--ac", "--btn", "--hrt", "--onac", "--onbtn", "--ac-text", "--ac-mark"];
  var MIN = 4.5;

  function clamp(v) {
    return Math.max(0, Math.min(255, Math.round(v)));
  }

  function normalize(raw) {
    if (typeof raw !== "string") return "";
    var s = raw.replace(/^\s+|\s+$/g, "").replace(/^#/, "");
    if (/^[0-9a-f]{3}$/i.test(s)) {
      s = s.charAt(0) + s.charAt(0) + s.charAt(1) + s.charAt(1) + s.charAt(2) + s.charAt(2);
    }
    return /^[0-9a-f]{6}$/i.test(s) ? "#" + s.toUpperCase() : "";
  }

  function rgbOf(hex) {
    var s = hex.slice(1);
    return [parseInt(s.slice(0, 2), 16), parseInt(s.slice(2, 4), 16), parseInt(s.slice(4, 6), 16)];
  }

  function hexOf(rgb) {
    var out = "#";
    for (var i = 0; i < 3; i++) {
      var v = clamp(rgb[i]).toString(16).toUpperCase();
      out += v.length < 2 ? "0" + v : v;
    }
    return out;
  }

  function lum(rgb) {
    var c = [];
    for (var i = 0; i < 3; i++) {
      var v = clamp(rgb[i]) / 255;
      c[i] = v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
    }
    return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
  }

  function ratio(a, b) {
    var la = lum(a);
    var lb = lum(b);
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
  }

  // Legible INK for a surface: scale up on a dark theme (keeps the hue),
  // scale down on a light one, then mix toward white/black for colours whose
  // channels cannot be scaled far enough. Mirrors lib/theme.js#inkFor.
  function inkFor(rgb, bgHex) {
    var bg = rgbOf(bgHex);
    if (ratio(rgb, bg) >= MIN) return { rgb: rgb, adjusted: false };
    var last = rgb;
    var s;
    if (lum(bg) > 0.5) {
      for (s = 0.95; s >= 0.0499; s -= 0.05) {
        last = [clamp(rgb[0] * s), clamp(rgb[1] * s), clamp(rgb[2] * s)];
        if (ratio(last, bg) >= MIN) return { rgb: last, adjusted: true };
      }
      for (var k = 0.05; k <= 1.0001; k += 0.05) {
        var mixed = [
          clamp(last[0] * (1 - k)),
          clamp(last[1] * (1 - k)),
          clamp(last[2] * (1 - k)),
        ];
        if (ratio(mixed, bg) >= MIN) return { rgb: mixed, adjusted: true };
      }
      return { rgb: [0, 0, 0], adjusted: true };
    }
    for (s = 1.05; s <= 8.0001; s += 0.05) {
      last = [clamp(rgb[0] * s), clamp(rgb[1] * s), clamp(rgb[2] * s)];
      if (ratio(last, bg) >= MIN) return { rgb: last, adjusted: true };
    }
    for (var k2 = 0.05; k2 <= 1.0001; k2 += 0.05) {
      var blend = [
        clamp(last[0] + (255 - last[0]) * k2),
        clamp(last[1] + (255 - last[1]) * k2),
        clamp(last[2] + (255 - last[2]) * k2),
      ];
      if (ratio(blend, bg) >= MIN) return { rgb: blend, adjusted: true };
    }
    return { rgb: [255, 255, 255], adjusted: true };
  }

  function varsFor(hex, theme) {
    var surface = normalize(hex);
    if (!surface || !BG[theme] || theme === "pochette") return null;
    var rgb = rgbOf(surface);
    var on = ratio(rgb, [0, 0, 0]) >= ratio(rgb, [255, 255, 255]) ? "#000000" : "#FFFFFF";
    return { surface: surface, on: on, ink: hexOf(inkFor(rgb, INK_BG[theme]).rgb) };
  }

  function stored(key) {
    var raw = localStorage.getItem(key);
    if (!raw) return "";
    try {
      return JSON.parse(raw);
    } catch (e) {
      return String(raw).replace(/^"|"$/g, "");
    }
  }

  var t = "encre";
  var cover = "";
  var accent = "";
  try {
    var savedTheme = stored("ui.theme");
    if (BG[savedTheme]) t = savedTheme;
    cover = stored("ui.coverAc");
    if (!/^#[0-9a-f]{6}$/i.test(cover)) cover = "";
    accent = normalize(stored("ui.accent"));
  } catch (e) {
    /* private mode / storage disabled: the default theme still applies */
  }

  var d = document.documentElement;
  d.setAttribute("data-theme", t);
  // Inline: the colour of the overscroll edges and of the frame behind a
  // translucent status bar, and what shows before the stylesheet lands.
  d.style.backgroundColor = BG[t];
  if (cover) d.style.setProperty("--cover-ac", cover);

  // A theme switch later clears these before painting the next theme; the boot
  // is a fresh document, but clearing makes the invariant explicit.
  for (var i = 0; i < CUSTOM.length; i++) d.style.removeProperty(CUSTOM[i]);
  var vars = accent ? varsFor(accent, t) : null;
  if (vars) {
    d.style.setProperty("--ac", vars.surface);
    d.style.setProperty("--btn", vars.surface);
    d.style.setProperty("--hrt", vars.ink);
    d.style.setProperty("--onac", vars.on);
    d.style.setProperty("--onbtn", vars.on);
    d.style.setProperty("--ac-text", vars.ink);
    d.style.setProperty("--ac-mark", vars.ink);
  }

  var m = document.querySelector('meta[name="theme-color"]');
  if (m) m.setAttribute("content", BG[t]);
})();
