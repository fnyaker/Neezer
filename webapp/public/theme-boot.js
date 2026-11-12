// The Sillon theme, before the first frame.
//
// It lives in its own file because the app's CSP is `script-src 'self'` with no
// 'unsafe-inline': an inline <script> in index.html is REFUSED by the browser
// (it was, and the console said so on every load). A classic script (no
// `type="module"`, no `defer`) blocks the parser, so it runs before <body> is
// painted — which is the whole point: a session in Papier, or in Pochette whose
// accent comes from the sleeve, must not flash the dark default for a frame.
//
// It is ~1 KB, comes down with the shell, and the service worker stages it
// (sw.js scrapes index.html's `src=` list). lib/theme.js reads the same keys
// afterwards and owns the rest of the session.
(function () {
  var BG = { encre: "#0C0C0B", papier: "#F3F0E8", pochette: "#0F0E10" };
  var t = "encre";
  var ac = "";
  try {
    var raw = localStorage.getItem("ui.theme");
    if (raw) {
      var v = raw;
      try {
        v = JSON.parse(raw);
      } catch (e) {
        v = String(raw).replace(/^"|"$/g, "");
      }
      if (BG[v]) t = v;
    }
    var araw = localStorage.getItem("ui.coverAc");
    if (araw) {
      try {
        ac = JSON.parse(araw);
      } catch (e) {
        ac = String(araw).replace(/^"|"$/g, "");
      }
      if (!/^#[0-9a-f]{6}$/i.test(ac)) ac = "";
    }
  } catch (e) {
    /* private mode / storage disabled: the default theme still applies */
  }
  var d = document.documentElement;
  d.setAttribute("data-theme", t);
  // Inline: the colour of the overscroll edges and of the frame behind a
  // translucent status bar, and what shows before the stylesheet lands.
  d.style.backgroundColor = BG[t];
  if (ac) d.style.setProperty("--cover-ac", ac);
  var m = document.querySelector('meta[name="theme-color"]');
  if (m) m.setAttribute("content", BG[t]);
})();
