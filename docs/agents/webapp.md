# Web app: screens, navigation, offline, loading

Reference notes for agents working on this area. Read [CLAUDE.md](../../CLAUDE.md) first: its process and rules apply here. Other areas: [docs/agents/](./).

**Sillon is the design system; Neezer is the product.** Its foundation is the tokens in
`src/app.css`, `lib/theme.js`, the pure rules in `lib/sillon.js` and the fonts in `src/assets/fonts/` —
build a screen from those, never from a literal colour. Rules that cost something to learn:

- **A theme is an attribute on `<html>`** (`encre` the default, `papier`, `pochette`), never a class
  or a component prop: `--bg --s1 --s2 --ln --tx --dim --ac --onac --btn --onbtn --hrt`. The old
  names (`--text`, `--text-dim`, `--bg-elev`, `--bg-card`, `--bg-hover`) are aliases of them;
  `--accent`/`--accent-2` alias `--ac-text`, not the lemon surface, so the screens not yet through
  the rebrand stay readable on Papier.
- **An accent used as TEXT or a MARK is `--ac-text` / `--ac-mark`; `--ac` is a SURFACE.** On
  Papier the lemon is a highlighter, never ink (lemon on cream is 1.16:1): `--ac-text` is the ink
  there, `--ac-mark` is too, and `--onac` stays the text colour on an `--ac` pill. The global
  `:focus-visible` ring reads `--ac-text` for the same reason. `#fff` on `--ac`/`--accent` is a bug
  in every theme (measured 1.16:1; the old purple made it 3.9:1 and hid it).
- **A theme is a COLOUR PAIR.** Text on `--ac` is `--onac`; text on `--btn` is `--onbtn`. Text on the
  ink surface `--onac` is `--ac` (Papier's `--onac` and `--ac-text` are both #14130f, so
  `--ac-text` on `--onac` is 1:1 — HomeHero's Flow button and the energy notches). `#fff` on
  `var(--accent)` was right when the accent was purple and is 1.4:1 now that it is lemon (Home's
  count badge, Library's badge) — every such pair is a bug the rebrand exposes, not a taste.
- **A switch is a `--btn` track with an `--onbtn` knob when on**, and an `--s2` track ringed by
  `--dim` with a `--tx` knob when off: the off track was 1.2:1 from its card, and an `--accent`
  track put a white knob on the lemon (1.16:1). The old `--accent` surfaces belong in `--btn`;
  selections (chips, segmented buttons, checkboxes) use `--ac`/`--onac`.
- **The theme is applied BEFORE the first frame** by `public/theme-boot.js`, a classic script
  (blocking) loaded from `index.html`, because the CSP is `script-src 'self'`: an inline `<script>`
  there is REFUSED by the browser (it said so in the console on every load). It sets `data-theme`,
  the html background and `<meta name="theme-color">` from `ui.theme` / `ui.coverAc`;
  `initTheme()` in `main.js` then owns the session. Change one, change the other.
- **A custom accent is an INLINE overlay on Encre/Papier, never a fourth theme** (Réglages ›
  Apparence: native `<input type="color">`, a controlled six-digit field, "Réinitialiser",
  `ui.accent` stored as JSON). `theme-boot.js` paints it before the first frame and
  `lib/theme.js#apply` owns it afterwards; both run the same maths and `test/sillon.test.mjs`
  compares their output so the copies cannot drift. `--ac`/`--btn` are the EXACT chosen surface,
  `--onac`/`--onbtn` are pure black or white (`#0C0C0B` would miss 4.5:1 on a mid grey),
  and `--ac-text`/`--ac-mark`/`--hrt` are a legible ink variant corrected against the most
  demanding surface they are drawn on (`--s2` on Encre, `--s1` on Papier), because those tokens
  are foregrounds (focus ring, hearts, progress). Measured: `#7C5CFF` on Encre gets black text at
  4.83:1 and a lifted `#8F6AFF` at 4.51:1; `#FFF176` on Papier gets black text at 18.10:1 and a
  deepened `#736C35` at 5.08:1; a 4 096-colour cube gives a worst label of 4.58:1 and its 8 192
  theme/ink pairs a worst of 4.50:1. `apply()` clears every inline custom property before a theme
  switch, so **Pochette keeps `--ac: var(--cover-ac)`** and a custom colour can never freeze the
  sleeve (`--cover-ac` is never touched). The field refuses a non-colour (`aria-invalid`,
  `role=alert`), says when the ink was lifted/darkened (`role=status`), and its controls are
  46 px. The boot script grew to ~6.2 kB raw / 2.5 kB gzipped: the contrast maths cannot wait for
  the bundle, and it is staged with the shell like the rest of the pre-paint code. In the built
  app, the first rAF after a reload already had `--ac` with `#app` empty (measured at 390×844).
- **Typography is self-hosted** (OFL texts beside the files): Bricolage Grotesque 600–800 for titles
  (`-0.03em` on big ones), Instrument Sans 400–700 for the interface, JetBrains Mono 400–500 for
  metadata and the small capitals (`.label-mono`, `letter-spacing:.08em`). latin + latin-ext (French
  `œ` lives in ext), `font-display: swap`, and the service worker stages all six with the shell.
  Measured first screen at 1440, Encre: **51 requests / 279 kB → 57 / 369 kB**, 84 kB of it type.
  Bricolage ships WITHOUT its optical-size axis: identical text width at 14–44 px, 47 kB cheaper.
- **A missing sleeve is a groove, and a slow portrait is an initialled disc.** `Cover` paints the
  deterministic rainure from the first frame (a queued image sits ON it); `Avatar` paints the tint
  and initials UNDER the photo, so a 6 s cover request never leaves a blank `--s2` disc. A Deezer
  md5 of all zeroes is ABSENCE, not art: the API returns no URL / a 404 (a 302 to the generic
  `000…` JPEG made every fallback disappear behind the same lavender square).
- **Pochette takes the sleeve's colour**: `dominantColor()` (the canvas that already fed the gradient
  headers), lifted by `readableAccent()` until it carries 4.5:1 on `--onac` (232 colours tested,
  worst 4.50:1; the lift SCALES, it does not blend white in, or a deep blue comes out grey-lilac).
  The watcher is keyed on `currentId` — a STRING, because `current` re-emits its object on every
  progress tick — and guarded on the sleeve URL it asked for, NOT a counter: replacing a queue emits
  A, null, A, and a counter loses A's answer and leaves the fallback up forever (seen in the browser).
- **A sleeve change is a WALK, not a cut** (`app.css`, `lib/backdrop.js`,
  `DesktopNowPlaying`/`MobileNowPlaying`). `--cover-ac` is a registered `<color>` (`@property`), so
  Pochette's whole accent — `--ac`, `--btn`, `--hrt`, `--ac-mark`, which are `var(--cover-ac)` —
  transitions in CSS over 600 ms: no timer, no JavaScript, a change that arrives mid-fade is
  retargeted from where it is, and `prefers-reduced-motion` turns it off at once.
- **ONE blurred image at rest, TWO while a cover fades IN — the full-screen player's whole blur budget**
  (the app's ground below is a separate single pass, and the two are never on screen together; the
  sheet/menu glass below is the only other pass). `lib/backdrop.js` keeps at most two layers: the one
  on screen and the one fading in with the same 600 ms curve. A cover that arrives while a fade runs
  is QUEUED (the newest wins) and painted once the incoming layer is opaque (`fadeMs` 720) — a third
  layer, or dropping the opaque base, is what used to show the dark page through a skip (measured at
  90 % of the colour showing, even on one skip: the play cache emits the same art twice; the `proxyUrl`
  is now the cover's IDENTITY, so the blob does not fade the same picture again). The sleeve's halo
  is the cover's dominant colour in a `radial-gradient` MASK (`--glow-c`, read from the image already
  decoded for the backdrop — no second request), never a second `filter: blur`: 1 blur pass at rest
  and 2 in a fade, against **2 at rest and 4–6 on one/two skips** before (the glow was a blurred image
  per layer, and the cap was 8). Measured at 1280×900, one then two skips 250 ms apart: TaskDuration
  1900/1774 ms → 882/1135 ms over the same 2 s window, long tasks 57/332 ms → 50/56 ms, worst frame
  gap 72.6/133.5 ms → 55.7/67.1 ms; at 390×844, layers 1/2/3 → 1/2/2 (mobile has no halo). **Eco mode
  renders no backdrop at all** — no `.bg`, no halo, no `filter`, no image fade, one flat `--bg`
  ground, and `bg.set` is not even called (no request for a picture nobody sees); every sheet and
  menu loses its `backdrop-filter` through the shared `html[data-eco="1"]` rule (below).
  The **party guest screen** (`routes/Party.svelte`) follows the same rule outside the player: one
  layer at rest and two in its fade (the shared store), and in eco **no `.bg` and no scrim at all** —
  one flat `--bg`, the backdrop never pointed at the art (measured 390/1280: 0 `new Image()` in eco
  against 1 outside, 0 `filter: blur`).
  `prefers-reduced-motion` keeps the images and drops only the movement. Measured (`test/theme/run.mjs`,
  1280 and 390): the accent is green→red with mid colours between the two and settles in 600–660 ms,
  1 blurred layer at rest, 2 in a fade (the frame-by-frame count is `1…12…1`), 0 in eco with the flat
  ground equal to `--bg`, no frame over 51 ms, no cover request after it settles, the first frame
  after a reload already the stored accent. `DIST=<dir>` (a `vite build --outDir` elsewhere) points
  any bench at a build of its own (`test/harness.mjs`).
- **Every screen sits on the playing sleeve** (`components/GlobalBackdrop.svelte`, `lib/globalbackdrop.js`,
  `lib/coverpalette.js`). Accueil, Bibliothèque, Rechercher, Réglages and a page all share ONE fixed
  layer behind them: a 500px cover stretched to the viewport with a single `filter: blur(44px)
  saturate(1.3)`, the sleeve's tint, and a scrim. It is not the full-screen player's backdrop and is
  never displayed at the same time: `$immersiveOpen` removes it (`display:none`), so exactly ONE
  blurred image is on screen, and `data-eco` removes it entirely. No `backdrop-filter` here — the
  surfaces' glass ladder is the only other pass. The URL is the 500px art the lists and the play cache
  already fetch (`baseCover`, resolved through `offlineCovers`), so the ground adds ZERO requests to a
  screen and works offline from the stored blob; a track with NO art returns to the theme's flat
  ground (a sleeve that FAILS to load keeps the previous one — a flaky CDN must not blank the page).
  STACKING IS THE FRAGILE PART: the element must create NO stacking context (no `isolation`, no
  `position`, no `z-index`) and its layers carry `z-index:-1`, so they paint above `html`'s canvas
  (app.css) and BELOW every card, row and title of the page, because `<body>` is transparent.
  `isolation: isolate` there put the ground over the in-flow text and washed the titles out (the
  glass bench measured a washed title at 190/255; this bench's probe now reads 228/255 against an
  ink of 241). The other half is app.css's `body { background: transparent }`. The ladder used to
  force `.gb { isolation: auto !important }` from app.css while the component isolated itself; that
  override is GONE (the component declares no stacking context any more, and the glass bench stays
  green without it). Re-adding a stacking context to `.gb` puts every page's text back under the
  ground.
  A sleeve change is a 640 ms walk, not a cut: fade the group out over 220 ms, swap the art and its
  tint at the invisible point, fade back over 420 ms; the scrim never moves, so there is no flash, and
  `prefers-reduced-motion` swaps at once. The tint is `coverpalette.js`'s `dominant` colour — ONE
  64×64 decode per cover URL, cached by URL with concurrent callers sharing the in-flight promise and
  failures not stored (`colour` extractors must import it, never decode the sleeve again). Measured
  (`test/backdrop/run.mjs` on `tools/perf_api.py --serve`, 1280×900 and 390×844, 7 routes each):
  1 blurred element and 0 `backdrop-filter` on every route with no overlay open, 0 cover requests
  while navigating, 1 load of the new sleeve on a skip (the player bar's Cover and the play cache
  account for the other 3–4 requests to that same URL), 0 after it settles; fade out at 25–66 ms,
  new art at 228–245 ms, full at 578–603 ms, worst frame gap 17–33 ms, 0–56 ms of long tasks. Over a
  PURE-WHITE sleeve (the worst case) the scrim keeps `--dim` at 5.00:1 on Encre and 5.96:1 on
  Papier, `--tx` at 11.8:1 and 16.3:1. Eco mode: no image, no palette decode, no blur, no scrim —
  the theme's flat ground — and the attribute `data-eco="1"` is set on `<html>` before the shell
  renders and REMOVED (never `"0"`) when eco ends.
- **Everything above the ground wears ONE glass ladder** (`src/app.css`, *surfaces — the glass
  ladder*; `test/glass.test.mjs` reads the source, `test/glass/run.mjs` measures the pixels). Three
  tints and three blurs, and no component invents a value of its own:
  **tier 1** `--glass-bg` = 72 % of the theme fill (see `--glass-fill` below: `--s1` + 4 % of the
  accent), NO blur — cards/blocks (`.card`, so Réglages, Podcasts and `Card.svelte` all follow), the
  sidebar, the player bar, the phone's tab bar
  (`.mobilenav`), the desktop queue column (`.np`), the mobile mini-player card;
  **tier 2** `--glass-bg-2` = 86 % + `blur(var(--glass-blur-2))` (14 px) + `saturate(140 %)` —
  popovers: the contextual menu, the quality list, the sleep/speed/transcript menus, the
  remote-control chips, the public playlist's sticky bar, the projector's chips (a notice takes the
  same fill with NO blur — it is a single card, and a filter would buy a render pass for nothing);
  the notices' strip itself has no fill, so the ground shows around the cards;
  **tier 3** the same fill + `blur(var(--glass-blur-3))` (22 px) + `saturate(145 %)` — every `.sheet`
  and `.dialog` (the app's eighteen modal components), over the 60 % veil their own scrim paints.
  A fourth step, `--glass-bg-strong` (93 %, still no blur), is the sticky TOP BAR and the notices'
  strip: the only surfaces a row scrolls BEHIND (at 82 % the library's leaf tabs ghosted under the
  search field; 93 % hides them and still shows the ground). `--glass-border` (13 % of `--tx`) is the
  hairline, `--glass-shadow`/`-2` the depth (per theme: Papier's is ink at 10/16 %, the dark themes'
  is black at 42–58 %), `--glass-hover` the row hover (9 % of `--tx` over the fill, so a hover is a
  step of the same material instead of a solid colour).
  **A static surface NEVER blurs its own backdrop.** `backdrop-filter` re-reads and re-filters what
  is behind the element on every frame that backdrop changes, and the captured area grows by ~3×
  the radius on each side (Will Chan; Dimensa measured one header's blur costing 29 → 61 fps;
  Jaeyonging went 20 → 60 fps by dropping 12 of 16 of them), so a 96-card grid would be 96 passes.
  It is allowed on the two OVERLAY tiers only, and only one overlay is open at a time — measured
  `1` `backdrop-filter` + the ground's single image, never 2. The scrims under sheets used to run
  their own `blur(2px)`/`blur(6px)`, i.e. ten implementations of the same work *under* a blurred
  sheet: they are flat veils now. **No `backdrop-filter` literal is left in `src/`** — the app had 19
  declarations in 16 files with 7 distinct pixel values (2, 6, 8, 10, 14, 16, 18 px) and now has
  `--glass-blur-2` and `--glass-blur-3` only (`MobileNowPlaying`'s 16/18 px moved onto tier 3).
  **ECO and REDUCED TRANSPARENCY collapse the ladder onto the theme's own surfaces**: `--glass-bg`,
  `-2`, `-strong` → `--s1`, `--glass-hover` → `--s2`, `--glass-blur-2/3` → `0px`, the shadows →
  `none`, plus a `backdrop-filter: none !important` kill switch for anything a component adds back
  (`html[data-eco="1"]`, and `@media (prefers-reduced-transparency: reduce)` for the OS setting,
  MDN). `prefers-reduced-motion` adds nothing to turn off: the ladder has no transition of its own.
  Measured (`test/glass/run.mjs`, 3 themes × 1280×900 and 390×900, 45 surfaces, on the painted
  pixels): **69 contrasts, worst 5.09:1** — the top bar's 11 px `⌘K` hint on the search field
  (5.10–6.17 by theme), which measures the same in eco because it is the theme's own `--dim` on its
  own field; the pairs the translucency actually moves are a card's 13 px `--dim` subtitle over the
  sleeve ground at **5.44 / 5.27 / 5.79:1** (Encre / Papier / Pochette) and a sheet's 11 px count at
  6.73 / 5.34 / 7.17:1. The composite behind a line of text matches
  `alpha·surface + (1−alpha)·backdrop` to **3/255** on all 45 surfaces, so the tint really is the
  token's. Cost at 4× CPU, headless (SwiftShader: the GPU work runs on the CPU; medians of 5, each
  state paired against the same open with the blur forced off): a contextual menu **48.7 ms vs
  48.4** (+0.3 ms), a desktop sheet **111.2 vs 103.1** (+8.1 ms), a phone sheet **83.5 vs 83.1**
  (+0.4 ms) — across runs the sheet's own share moved between 8 and 30 ms with the machine's load
  and the menu's stayed under 2 ms; a scroll of the card grid is **16.7 ms median** in glass and in
  eco alike, i.e. the tint costs no frame (the old full-viewport `blur(2px)` scrim read ~1.18 M
  pixels a frame; a 440×700 sheet at 22 px reads ~0.48 M, so the pass got stronger and cheaper). States: `none` with nothing open,
  **exactly one** `backdrop-filter` (+ the ground's one image) while a menu or a sheet is open, and
  **3 values in the whole app** (`none`, `blur(14px) saturate(1.4)`, `blur(22px) saturate(1.45)`).
  Before this change: every one of those surfaces measured `alpha 1`, and `src/` carried 19
  `backdrop-filter` declarations with 7 distinct radii.
- **The LIGHT on the glass is painted; the lens is not** (`src/app.css`, *the liquid finish*;
  `test/glass.test.mjs` reads the source, `test/glass/run.mjs` measures the pixels). Apple's material
  is a stack — a tinted frosted body, a **specular** highlight on the lit edge, a darker **rim**
  opposite, a depth shadow, then the **lensing** (WWDC 2025 *Meet Liquid Glass*; Gui Rambo's Core
  Animation teardown: the specular and the dark rim are two separate SDF layers, and the lens is a
  displacement of the backdrop). The tint and the frost were already the ladder; the finish adds the
  light with PIXELS, not a pass: two 1 px gradients carried by the surface's own `background`
  (`--glass-beam-x`, brightest at 12 % of the width, gone by 62 %, a quieter catch at 86 %;
  `--glass-rim-x` under it) and a **two-layer** shadow (`--glass-shadow`/`-2` = a contact line + one
  ambient blur; a third 80 px pass was dropped: a viewport-sized blur over a full-bleed sheet is
  rasterised for no pixel). **The lens is deliberately absent**: `backdrop-filter: url(#…)` is
  Chromium-only (Safari drops the whole declaration, Firefox ignores the filter), needs a
  displacement map per surface, and re-samples the captured area every frame — the cost this ladder
  exists to bound. `test/glass.test.mjs` fails on a `<filter>`, a `filter: url(#…)` or a
  `backdrop-filter: … url(`.
  `--glass-fill` is **4 % of the theme's accent mixed into `--s1`** (`--glass-tint-ac`), so Pochette's
  glass takes the sleeve's colour and a custom accent tints the glass too — without becoming a
  coloured pane (every composite is still held to ≥ 4.5:1).
  Every tier paints it: `.card`/`.glass-1` and the desktop player's side card, `.menu`, `.sheet`,
  `.dialog`, the **top bar's lower rim** (a sticky bar's only visible edge), the **sidebar's right
  edge** (`--glass-beam-y`), the **player bar's top edge**, the phone's **thumb-row doors** and its
  swipe-up door (34 %/26 % clear glass on the frosted sleeve — no filter, and an INSET ring so the
  five 44 px targets still fit a 360 px phone), the queue's accent action `.ql .qsave` (from app.css,
  since `QueueList.svelte` is another agent's file) and the queue's hover (`--ql-hover`). `.field` is
  a WELL instead (a dark line above, a light one below).
  **The material arrives instead of appearing**: `@starting-style` makes a menu's/sheet's first style
  a clear pane and `transition: background-color 160 ms` walks the tint in — no keyframes, no timer,
  and gone under `prefers-reduced-motion`.
  **Eco and reduced transparency turn the light off with the blur**: `--glass-hi`/`--glass-lo` become
  `transparent` (not `none`: the four `background` declarations that compose the beams must stay
  valid) and the shadows `none`.
  A **sheet that sits on a photograph** takes the thick step: the phone player's full-bleed queue and
  lyrics panels are `.sheet.on-photo` → `--glass-bg-strong` (93 %), because the page's own scrim is
  not under them — tier 3's 86 % measured **4.17:1** for the À propos panel's 11 px `--dim` labels
  over a white-ish cover, 93 % gives **5.18:1** — and they cast no shadow (nothing to cast onto).
  **The visualizer strip is never veiled**: `--ac-text` is its one flat colour (the animation agent's
  contract), so `.viz` carries NO `opacity`/`filter` in either player — at 0.9 the accent was
  measured `#a9cd33` for `#d2ff3c` on Encre and `#41413b` for `#14130f` on Papier, with **0 %** of the
  bars' pixels the accent and 7–12 % the blend.
  Every card is a `role="button"` with **`aria-haspopup="menu"`** (it opens the shared context menu
  on right-click, long press and Maj+F10; `stores.js#openMenu` returns the focus to it), and the
  attribute adds no focusable and no second target — measured 36/36 cards, one target each, Escape
  from a keyboard-opened menu lands back on the card with its focus ring.
  Measured (`test/glass/run.mjs`, 3 themes × 1280×900 and 390×900, **57 surfaces**): the catch is
  **+27.6 / +21.1 / +30.6 / +34.6 / +30.7 / +23.0 over 255** above the trough (card, menu, sheet, side
  card, phone door), with **+10 to +14 within the same 1 px row** (a uniform border cannot fake that)
  and **0/255 in eco**; the surfaces this lot adds measure the phone's doors **6.28:1**, the player's
  queue sheet **5.70:1**, the À propos panel **5.18:1** and the desktop side card **6.44:1**; 1
  `backdrop-filter` + the ground's image with one overlay open, 3 values in the app, 0 in eco.
  Cost: the finish is background paint, not a pass — an interleaved A/B (four passes over the same
  menu opens at 4× CPU under SwiftShader) puts the beams, the shadow and the tint's arrival inside
  the noise of the bare material (medians **49.5–53.5 ms** against **50.0** for the whole open), and
  the bench's own paired numbers stay where they were: the blur of a sheet **under 30 ms**, of a menu
  **under 10 ms**; the card grid scrolls at **16.7 ms** a frame on the phone and 16.7–33.3 on the
  desktop, long tasks **59–170 ms** over the run. The bench's two known
  environment flaws are written down where they happen: the top bar's text sits in an opaque field
  (its tint is not comparable, so only its contrast is asserted), and headless SwiftShader drops a
  `backdrop-filter` layer for a few frames now and then (measured: three opens in a row out of
  sixteen), so a sample that cannot be the material is re-taken.
- **The sleeve-derived header gradient fades too** (`components/GradientHeader.svelte`, used by a
  show, an import job and the credits page): `--gh-c` is the second registered `<color>`, and the
  gradient is built from it with `color-mix` — 90 % of the sleeve at 0 %, 45 % of it mixed with black
  at 55 %, `--bg` at 100 %, the stops the old inline gradient had — so a colour ARRIVING is a 600 ms
  walk instead of the cut `transition: background` never could animate. No cover keeps the fallback
  `#3a2a55` (the old FALLBACK purple within 3 units, checked on Papier) and skips the fade entirely.
  Measured on the show page: 32 distinct gradient values while arriving, the teal reached in ~600 ms,
  no intermediate under reduced motion, no frame over 120 ms. The album, playlist and artist pages
  have no cover tint at all (a flat `--s2` band): the tint lives on those three pages only.
- **The bar's level speaks the faders' language** (`components/PlayerBar.svelte`): an 8 px track
  inset by the thumb's radius, a 22 px thumb whose centre rides the track's ends, a 44 px hit box,
  the fill inside the track, and the value (`14 %`, mono 10 px) under it. The native range is
  invisible on top and does the mapping — its thumb is 22 px, so a click at 14 % of the TRACK is
  14 % (the surviving 10 px thumb rule made it 21 %) — which keeps the keyboard and the screen
  reader for free. Measured at 1920/1440/1280/1181: thumb centre on the track centre to 0.00 px both
  ways at 0/50/100/14 %, hit area 84×44 or 72×44, no overflow, click 14 % → 0.14. The level is not
  mounted at all below 1181 px (no room in the third column; the mute icon and the full-screen
  player keep the sound reachable), so a 641 px bar has no slider — `test/volume/run.mjs` holds it.
- **Missing art is a record groove, never an empty square** (`lib/sillon.js`): DESIGN §4's repeating
  radial grain, from eight dark pairs chosen by FNV-1a over `src|alt|fallbackId|kind` (Cover) or the
  name (Avatar's initials). Deterministic: a placeholder that changes colour on reload reads as a
  loading state.
- **The page margin is a pair of tokens**, `--page-y` / `--page-x` (32/32, 20/20 ≤ 640 px). Anything
  that bleeds to the screen's edge reads THEM (`components/GradientHeader.svelte` did not, and the
  day the top padding moved it left a 4 px band of background above itself).
- **Controls and surfaces**: `.btn` (+ `-primary`, `-ghost`, `-sm`, `-icon`), `.chip` (+ `-active`),
  `.card`, `.field`, `.section-title`, `.page-title`, `.mono`, `.label-mono`, `.kbd`; `.pill` is now
  a solid `--btn` pill (the purple gradient and its glow are gone). 40 px high on a mouse, 44 under a
  finger (`@media (pointer: coarse)`) — §10's touch rule without changing the desktop rhythm.
- **A component's own class must not borrow a design-system name.** `routes/Party.svelte` put
  `class="field"` on its join form, and the global `.field` pill (40 px, `align-items:center`)
  clipped the label-over-input column so the name input ran under « Rejoindre l'écoute ». It is
  `.party-field` now — measured 390×844: the field is 76 px (19 px label + 6 px gap + 51 px input),
  the input ends 14 px above the button, no overflow; 360×640 the same.

**The Sillon handoff (the old `DESIGN.md`, §1–§10), in one place.** Code comments that cite
`DESIGN §n` mean these rules; the handoff file itself was deleted once the design landed, because
this is the lasting part and the rest was a one-shot maquette. §1 tokens and §2 type are the bullets
above. §3 brand: **Neezer is the product, Sillon only the design direction** (the record groove);
the logo is a lemon disc with two concentric circles and a centre dot; the tagline is « Ton archive
musicale, gravée à ta façon. » §4 fallbacks are above. §5 layout: **232 px sidebar | content | 340 px
queue panel** (optional) over a **88 px player**. §6 pages:
- **Accueil** — full-accent hero « Quelle énergie ce matin ? » with the six energy chips and *Lancer
  mon Flow*; a *Reprendre* card beside it; then *Par style* chips, a *Nouveautés* cover rail, *Tes
  artistes* avatars and a dense *Pour toi* list.
- **Explorer** (ex-Rechercher) — source chips (Ma bibliothèque, Deezer, YouTube, Podcasts), recent
  searches and style tiles. Never an empty page.
- **Bibliothèque** — global counters in the header, pill tabs (Titres, Artistes, Albums, Playlists,
  Podcasts, Hors-ligne, Mes fichiers), *Indisponibles* as a filter; tracks keep the shared list
  below; artists are a 7-column avatar grid, playlists a card grid; every empty state has a drawing,
  a sentence and an action.
- **Liste de titres** (shared) — `36 px | titre + artiste (cover 44 px) | album | qualité (badge
  mono FLAC/OPUS) | + et cœur (76 px) | durée`. The current track is accent with ▶ in place of its
  number; **+** opens the playlist modal (§8).
- **Artiste** — `--s2` banner, 168 px avatar, 88 px name, *Lire* / *Radio* / heart; tabs Aperçu,
  Titres, Crédits; latest-release rail, top tracks, albums.
- **Playlist** — 200 px cover, 72 px title; *Lire*, *Mélanger*, *Télécharger*, *Ajouter des titres*,
  *Modifier*, and ⋯ for the destructive actions (deleting is never a bare icon again).
- **Nouveautés** — cards grouped by release (132 px cover, Album/Single, track count, *Lire*,
  *Ajouter aux favoris*).
- **Réglages** — `200 px | contenu`; fixed groups Écoute (Lecture & son, Apparence, Animations),
  Bibliothèque (Téléchargements, Découverte), Comptes & serveur (Sources, Compte, Serveur,
  Assistance), a *Chercher un réglage* field, cards on `--s1`; the Deezer state is a pill with
  *Tester*. The sections keep their ids (`playback`, `viz`, `downloads`, `discovery`, `sources`,
  `account`, `server`, `help`) because the benches open them by URL.

§7 player: the 88 px bar is cover 56, title/artist (click = full screen), heart and **+**, transport,
progress; right side = **clickable quality badge** (FLAC / Opus 320 / 256 / 192 / 128 / 64), devices
(remote + listen party), queue, volume, ⋯ (sleep timer, share) and full screen. The full screen keeps
the sleeve ≤ 330 px and a right panel with **File d'attente / Paroles / À propos**; the queue row has
play, + playlist and ×. §8 playlist modal: 440 px, 60 % veil, `role="dialog"`, × and Échap; a track
mode with checkboxes that apply at once and a toast, a *new playlist* mode and a *save the queue*
mode; toast = a `--tx` pill 2.4 s above the player, `role="status"`. §9 mobile ≤ 640 px: a 64 px
bottom tab bar (Accueil, Explorer, Biblio, Réglages), a 64 px floating mini-player above it, and a
thumb-centred full screen (≥ 44 px controls). §10 accessibility: real `<button>`/`<a>`, `aria-label`
on every icon-only control, 44 px touch targets, a visible `--ac-text` focus ring, `aria-pressed` on
the modal's checkboxes, and no information carried by colour alone.

**Web app** (`webapp/`): Svelte 5 + Vite 7 SPA, hash routing (svelte-spa-router), consuming `/api`.
Existing components still use the legacy `$:` / `export let` syntax; follow the file you edit.
Builds into `neezer/webui/dist`. No emoji in the UI — all glyphs go through
`src/components/Icon.svelte` (Lucide-style SVG). Home is **card-based** (recommended
playlists / albums / artists), not track shelves. Podcasts have their own pages
(`routes/Podcasts.svelte` grid + subscribe, `routes/Show.svelte` episode list) consuming
`/api/podcasts`, `/api/podcast/<id>` and streaming episodes through `/api/stream/<episode-uuid>`
(an episode is shaped like a track whose `deezer_id` is its UUID, so the existing queue/player
plays it unchanged). Sharing goes through `components/ShareSheet.svelte` (global modal, opened via
`openShare(track)` from stores.js): whole file or an excerpt selected on a zoomable canvas waveform
(peaks from `/api/share/waveform`), cut server-side by `/api/share/clip` and handed to the Web Share
API (download fallback). Podcast markers live in `lib/markers.js`.

**A show opens by its UUID or by Deezer's id** (`/api/podcast/<x>`, lot D): a catalogue card has no row, and a UUID cannot be turned back into the provider's id, so the page takes both forms; anything else is a 404 (`tests/test_multi_d.py`).

**Export to another service** (`neezer/services/`, `neezer/webui/services.py`, the *Service* and *Liste*
tabs of `components/ExportSheet.svelte`, Réglages → Compte → *Services de streaming*; survey in
`docs/playlist-export.md`). Spotify (the operator's own app, PKCE, `POST /me/playlists` + `/items` — the
February 2026 endpoints), YouTube Music (`ytmusicapi` signed in with a tab's pasted headers; searches go
through `deezer/youtube.search` and its holds) and, for the admin, the server's Deezer account as a one-shot
NEW playlist. Apple Music, Tidal & co. have no door without a paid developer programme: the list file (CSV
with ISRC, XSPF, text) is that route. A song is exported only when the service's copy IS the recording (its
id, the ISRC read off disk, then a search judged by `availability.match_score`); nothing is created when
nothing matched or when the service stopped answering. Credentials live in `service_link`, encrypted like
`password_clear`, never lent to a remote session. The Spotify PKCE state is kept server-side, not in the
session: Android opens the consent page in the system browser.

**Navigation, the home and the library** (design and sources: the Sillon handoff above).
Four destinations — Accueil, Rechercher, Bibliothèque, Réglages (Material 3 and Apple's tab-bar rules: five or fewer,
one-word labels); podcasts live IN the library. **A title on its own plays and then its radio**
(`actions.playThenRadio`, `TrackList radio`): the queue is that title alone with context `{kind: "radio", id,
artist}` and `Player.svelte#ensureUpcoming` tops it up from `/api/radio/next` at once — no request before the sound;
`artist` lets the server radio on the artist while a catalogue title's row does not exist yet. Inside a collection
(album, playlist, favourites, the Nouveautés page) a tap plays the collection from there. The home is: the Flow (the
discovery level behind one chip, the energy notches under it — `EnergyPicker.svelte`), **Nouveautés as TITLES** (one per release, `/api/releases` `tracks`, read from
`artist_release.tracks`, which the releases watcher fills when it finds a release — `releases._fill_tracklists`,
`TRACKLISTS_PER_RUN` a run, an outage leaves it for the next run, Deezer answering "gone" keeps `[]`), the favourite
artists (`/me/artists`: stars + library count + the person's listens, three statements), Pour toi (titles), recently
played shortcuts. **Mixes d'ambiance** closes the home (`MoodCard.svelte`, `GET /api/moods`):
one catalog request on mount, no section when nothing is ready, one `/api/moods/<id>` per card tap;
the card is a single 44 px+ button (sleeve, name, subtitle, title count), and the queue it starts is
FINITE (`autoplay: false`), so the endless radio continuation never turns a 40-title mix into a
station. The catalog is built from measured bands only — no provider — and paints from the offline
`/api` cache when the link is bad. **The library has two levels** (`components/LibraryNav.svelte`): a Musique / Podcasts
segmented control, then a leaf strip (`/library/<leaf>`: Titres, Historique, Artistes, Albums, Playlists, Téléchargés, Mes
fichiers; Podcasts → Abonnements), the rare "Indisponibles" list kept on the right as a filter. The leaf is still what
the URL names, so every deep link, `#/library/artists` from the home and Back keep working; the world is derived from
it (no second route), and Musique remembers the leaf it was left on (`lastMusic` in the nav screen memory). A hash-only
move between two library URLs updates the leaf instead of being ignored; a leaf switch uses `replace`, so it raises no
history entry and does not scroll the page. The leaf strip scrolls sideways under a mask fade on a phone and never
wraps: the old chip row was 148 px over three rows at 390 px, the two-level nav is 110 px and keeps four leaves + the
filter on screen; desktop caps the segmented control at 480 px. On touch the
segment is 44 px, leaves 45 px and the filter 44 px. The podcast grid mounts once and is kept hidden across a world
switch: its search survives and coming back costs no request (the first open asks `/api/podcasts` once). Adding is ONE
"+" menu. A track row is played by its
title too (the whole row is the target), and without a number the play glyph sits on the cover. Starring an artist
FOLLOWS it; archiving its discography (`on_fav_artist`) is off by default.

**« Historique » is the library's own leaf** (`routes/History.svelte`, `#/library/history`,
`GET /api/history`): one page is one request, with a keyset cursor `(at, id)` so a play recorded while
the person pages cannot drop or repeat an event. « Charger plus » appends the next page and keeps the
rows already read (one request per click); the filter chips Tout / Écoutes / Sauts refetch one page and
keep the 30-day summary. Day headers are 44 px and event rows 72 px, and the list is windowed: 6 rows
mounted at 1440×900, 24 for 100 events after scrolling, 10 on a 390 px phone. The row carries a glyph
AND a word (Écouté / Passé / Partiel), never colour alone; a failure is `LoadFailed` + « Réessayer »
(3.3 s with the API's retry budget), never a spinner. Personal history is never written to
`localStorage` nor to the offline `/api` cache. Measured 390/641/768/900/1024/1200+panel/1440+panel:
no sideways overflow; the tops stack under 720 px of content.

**The onboarding** (`routes/Onboarding.svelte`, `components/OnboardingArtistCard.svelte`,
`OnboardingAccount.svelte`, `lib/onboarding.js`) is a branch **before the app shell**: the server's
`user.onboarding` (and `GET /api/onboarding`) decides `artists|account|done`, the URL only names
the visible step (`#/onboarding/artists`, `#/onboarding/accounts`), a `done` account is sent to
`/`, and an old session without the field never enters it. The shell's connectivity/nav/version
side effects and the network badge are suspended; finishing reloads `/` so the shell boots. The
artist grid mounts **36 tiles first** (2/3/4/5/6 columns at 360/641/1440), then one GET per 36 by
`after`; a like paints in the frame (11–21 ms at 4×), inserts **2** similar tiles (never a
duplicate, cap 12), sends one POST, and polls read-only at most five times when the server answers
`pending`. The account step costs **1 GET** on entry (`GET /api/onboarding/account/options`, the rich
`deezer|youtube|imports` shape the cards need; 2 SELECT / ~931 B server-side), then 1 GET/1.5 s
while a linked account imports. An import is 1 POST + 1 GET/1.5 s while visible, and every card can
be skipped.
Offline likes go to a per-account `sessionStorage` queue and are flushed sequentially on return.
« Terminer » waits for the queue, POSTs `artists/done`, then `accounts`; the two-attempt
“Passer pour l'instant” only appears when the server says the feed is unavailable. 44 px, roving
tab index, `aria-pressed`, `role=status`, no emoji, no comment.

**A shared playlist is role-driven** (`routes/Playlist.svelte`, `PlaylistMembersSheet.svelte`,
`lib/actions.js`): `editable`/`can_manage`/`role` come from the payload, never `$isAdmin`. The owner
sees rename/delete/collaborators/public link; an editor adds/removes/reorders and may leave; a
viewer reads and is told to ask the owner; an admin keeps the override without appearing in the
member list. The header shows the owner, an avatar stack (3 × 22 px + `+N`) opening the sheet,
« Modifié par X · date » and a stale pill. Opening the sheet costs **0 requests**; 100 members are
windowed to **20 rows**; invite/role/remove/leave are one request each. Every mutation carries
`version` or `members_version`; a **409** discards the queued operation, does **one GET per scope**
(never a replay), keeps the audio queue untouched and says who changed it. The `/version` probe costs
**0 in a solo playlist** and runs 30 s / 10 s on focus only when the playlist has more than one
member; if the page is not editing it reloads, otherwise it offers « actualiser ». The sidebar and
Library gain a « Partagées avec moi » group (owner subtitle), the picker offers only `editable`
lists while creation stays open to every account, and `ExportSheet` shows the public link only with
`can_manage`. `/me/playlists` is refetched at most once a minute on focus. Measured: 2 requests to
open an editable page, 1 for a reader, 1 GET per 30 s probe, 0 per solo, 1 per mutation; no sideways
overflow at 360/390/768/1280/1920, 44 px touch targets (40 px pointer) and 60 px rows.

**The invitation page** (`routes/Invite.svelte`, `lib/invite.js`) is public and renders before
the login and before the app shell, exactly like a public playlist: `#/invite/<token>` keeps the
token in the fragment and sends it only in the JSON body of `POST /api/invites/inspect|redeem`.
A valid token costs **1 inspect**; an expired, revoked, used or unknown one gets the same generic
message and 0 POST; a success costs **1 inspect + 1 redeem**, opens the server session and
`replace`s to `#/onboarding`, removing the token from the URL. The form folds the name as it is
typed, enforces the 3–32 `[a-z0-9_.-]` shape and a 12–128 password with a four-step meter, keeps a
10-minute `sessionStorage` draft (token, name, stable `request_id`, **never** the password), and
replays the same request after a refresh. A signed-in person is offered « Retour à l'application »
rather than a second account. 48 px fields, 44 px secondary targets, no sideways overflow at
360/390/641/1440.

**« Récap » is a stories screen** (`routes/Recap.svelte`, `components/RecapStory.svelte`,
`RecapSlide.svelte`, `RecapShare.svelte`, `lib/recap.js`, `lib/recapyears.js`): `#/recap` resolves the
latest year and `replace`s to `#/recap/<year>`, which is a reloadable deep link. Opening costs 2
requests cold (`/years` + the year) and 1 warm, a year change 1, a card advance 0; the bridge mounts
only the active slide and the next one, 9 cards maximum. Auto-advance is 7 s only at ≤ 900 px and
not under `prefers-reduced-motion`, and pauses on touch-hold, page hide, focus loss and share. The
cards sit on a 488 px minimum bridge; at 390×844 the bottom is 700 px and the mini-player starts at
708 px, while 360×640 scrolls the page ~176 px rather than clipping a card. The two-column desktop
layout triggers on a **760 px content container**, not on a 900 px window (the 232 px sidebar would
not fit otherwise). The share poster is 1080×1350, drawn locally from `offlineCovers` only, 0
requests, `toBlob` 17.6–181 ms depending on the card and CPU (54.8 ms under 4× CPU on the first
card), PNG **887 705 B** in the bench and ≤ 2 MB. The first card mounts 4 covers, the nine cards 14
at most, 5 in flight; those are the payload's own `/api/cover` URLs, not the shell's playlist
thumbnails. Like History, a recap never enters `localStorage` or the offline `/api` cache. The Home
card appears only for `nowYear` or `nowYear - 1` from `/years` (never the device clock), costs one
`/years` per session, and draws four deterministic Sillon grooves because `/years` carries no
covers.

**Nouveautés is two shelves, then the per-release list** (`routes/Releases.svelte`,
`components/ReleaseCard.svelte`, `routes/ReleaseShelf.svelte`). `close_to` splits the payload:
« Tes artistes » is the person's own (favourites and the artists they listen to), « Découverte »
the releases Deezer says are close to a favourite. Each card previews SIX titles (40 px sleeve,
artist, title, duration — never TrackList's album/quality/action columns) and the whole card is ONE
link to a playlist-like but READ-ONLY page (banner with a cover mosaic, Lire / Mélanger, search and
sort, headings — no rename, reorder, add or remove). An empty card is not rendered; a lone card
spans the row and its six titles read in two columns at ≥ 900 px. The `{releases, tracks}` payload
lives in `Releases.svelte`'s module scope, keyed on the account: a card click paints the shelf from
memory with **0 API request**, and the shelf's chunk (6.7 kB JS + 4.0 kB CSS) is prefetched
during idle — measured 157–218 ms from the click to the shelf title under 4x CPU throttle, for a
payload of 117 kB / 8.2 kB gzip on 8 000 tracks. `/releases/seen` is posted once per fetched feed. "Toutes les sorties"
is collapsed by default, keeps its state across Back (`rememberScreen`), and opens itself when no
shelf has a title yet.

**≤ 640 px is a different SHELL, not a narrower desktop** (`src/mobile.css`, `lib/viewport.js`,
`components/MobileNav.svelte`, `PlayerBar.svelte`'s mobile block, `App.svelte`). One column sized to
`100dvh` (a phone's URL bar makes `vh` lie), its bottom padded by `--mini-h + --tabbar-h + --safe-bottom`;
the tab bar is EXACTLY 64 px of content (box-sizing content-box, the home indicator added as padding
UNDER it, never folded into the height) and the mini player is a 64 px card floating 8 px above it, 12 px
from each side. Tokens are structure only (`--mini-h --tabbar-h --player-h --safe-top --safe-bottom`,
`--page-x` already 20) — the colours stay the theme's. ONE breakpoint: `MOBILE_MQ` in `lib/viewport.js`
is what JavaScript reads, the `@media (max-width: 640px)` blocks are the same number in CSS; 641 px is
a narrow DESKTOP (sidebar + 88 px bar). The mini card keeps the bench contract (`footer.player`,
`.now`, `.info`, `.info .t`, `.pp`, `.next`) and carries only sleeve, names, play 44, next 36 and a 2 px
progress line; everything else is hidden, the card itself opens the full-screen player. **A press on a
control must not bubble into the card's tap target**: `player.toggle()` re-renders the icon, so by the time
the event reaches `<footer>` its `target` is a DETACHED glyph and `closest("button")` finds nothing —
read `composedPath()` (captured at dispatch) instead, or every press of play opened the player (seen in
the bench). The Now-Playing column is a third column, so on a phone it is not mounted at all and a
`ui.nowPlaying` left true on the desktop is closed on boot (it used to paint off-screen). Measured
(`test/mobile/run.mjs`, 360/390/639/640/641): no sideways overflow at any width, bar 65 px with its
border, card bottom exactly 8 px above the bar, play 44×44, `data-theme` kept across the player; 641 px
still 88 px bar and the desktop panel mounted.

**Search leads with the titles** (`routes/Search.svelte`, `lib/searchrank.js`, `components/TopResult.svelte`).
Tabs Tout · Titres · Artistes · Albums · Playlists; there is no Podcasts tab (the user's ask): the shows are mixed
into the albums after the first two, each card saying "Podcast". "Tout" opens on a **best result** only when the query
plainly names it — an artist (or, for the admin, a show) named exactly among the first three, or the first artist
whose name the query starts (three letters at least) — beside the ten titles on a wide screen, above them on a
phone; a title search gets no artist on top. Then the artists, the albums with the shows, the playlists. A title
tapped here plays and then its radio (`TrackList radio`). With Deezer out (or nobody's account serving the search)
the library answers alone: titles, and the artists and albums by NAME that hold a title on disk
(`webui._local_search_entities`, two statements). Each section carries a "Tout afficher" that switches to its tab.

**A broad search can become a radio** (`lib/queryradio.js`, `routes/Search.svelte`). At **30 distinct results**
(titles, artists, albums, playlists, shows — each counted once, the fallback's hits only once that source has
answered, shows only for the admin who sees them) a primary "Lancer la radio" button rides the search field: 44 px,
full-width under the field on a phone. It opens with the titles **above the search's own bar** (`CATALOGUE_MIN` 0.5,
`YOUTUBE_MIN` 0.75), strongest first (a library row wins a tie) — the first one plays at once, before any request —
then one `POST /api/radio/next` seeds from the best title (with its artist, for the catalogue row not in the library
yet) or from the best artist, and appends what is new. A queue of three titles or fewer is left to
`Player.svelte#ensureUpcoming`, which tops it up at once anyway: that is what keeps one press to ONE radio request.
The context is `radio` / `artist-radio` with `title: "Radio de <query>"`, so another playback replaces it like any
queue. Two clicks in one burst share one launch, a newer search's radio (or any playback started while the top-up
is in flight) supersedes a late answer instead of having it appended, and no seed is an honest error toast, never a
silent no-op.

**The YouTube fallback is ON by default and inside the same list** (`lib/searchrank.js#matchScore`,
`rankSearchResults`). It is asked once the query settles — 800 ms after the last keystroke, at once on Enter, a
recent term or a deep link, and never when the account lacks `youtube:search` or the source is off: one request
per search, not one per letter. Its hits are woven into the Titres list (no second section): the catalogue's order
is kept and library rows stay first; an identical recording (same id, or same title+artist once cosmetic
decorations such as "Clip officiel" are stripped) is dropped; a video only LEADS when its match is strictly closer
than the best catalogue answer (`YOUTUBE_MIN` 0.75 > `CATALOGUE_MIN` 0.5 — a different artist, live, remix or cover
falls under the bar and is not shown), otherwise it follows the catalogue. Every YouTube row says `YT` and its Opus
rate (`OPUS 133` before archiving, the file's real bitrate after) — in the quality cell, or on the artist line
(`.qinline`) where a narrow row hides that cell. A failed search says why and offers "Réessayer"; an empty one is a
single quiet line, never an empty section.

**Réglages is two levels** (`routes/Settings.svelte`): a list of categories (Lecture, Animations, Téléchargements,
Découverte, Sources, Compte, Serveur for an admin, Assistance), each a page — the iOS/Android settings pattern. ONE route
`/settings/:section?`, so switching category updates `params` instead of remounting the page and refetching every card. A
phone shows the list OR a category (back returns to the list, `replace` on a deep link); ≥ 900 px shows a column beside the
open category. A link into a setting names its category (`/settings/sources`, `/settings/account`); old `fx`/`quality`
names are mapped. Benches open the category they test (`#/settings/playback`, `#/settings/downloads`), never bare `#/settings`.

**The admin user dashboard** (`components/AdminUsers.svelte`, `InviteSheet.svelte`,
`lib/adminusers.js`) is a category **Utilisateurs**, mounted only for a local admin — never for a
remote controller and never for a member. The pane widens to 1100 px: a real table at ≥ 1280 px,
cards below. Opening the category no longer fires the eight server-card loads (they wait for their
own tab). It loads **2 GET** (one account page + pending invitations), then **1 GET** per
search or « Charger plus », **0** for a local state/role filter, **2** for Rafraîchir, and **1**
per mutation; it never polls, keeps a 60 s memory copy, and caps the loaded set at 250 accounts /
200 invitations. Because the server page has no `state`/`role` parameter yet, those filters apply
to the loaded rows and the screen says so; the client is ready for a server filter when it lands.
Pending invitations are rows of the same list (`kind:"invite"`), with their state and expiry, and
the header stays under the top bar (`top: var(--topbar-h)`), a failed search says
« Recherche indisponible » instead of counting stale rows, and the invite row's date column reads
its expiry (« expire dans 3 j »). The token is shown **once** in `InviteSheet` (copy with a manual fallback, revoke, renew). Deleting
an account asks for its exact name and shows the upload/playlist counts; the server's 403
`self_disable`/`self_delete` and 409 `last_admin`/`server_account_holder` are translated to French
administrative sentences. The account's own row pre-disables destructive actions, but the server
remains the only authority. Layout: 390/641 cards (no sideways overflow), 1280/1440 table, 44 px
controls.

**The Égaliseur card is a picker, a meter and ten faders** (`components/AudioEffects.svelte`,
`lib/eqpresets.js`). It is its own card under the volume/bass one. The presets are two groups — Intégrés (5)
and Mes préréglages — as a horizontal snap rail at ≤640 px (44 px chips, the next one clipped as the scroll
affordance, no scrollbar) and as a vertical scrolling list in two columns on desktop (the 760 px settings
pane). Every preset is reachable; the active one is highlighted and **revealed when the card mounts** (on a
ten-preset rail it would otherwise mount out of view). Clicking applies the whole stored chain (EQ, curve,
bass, normalization). A selected user preset that has been edited says « modifié » and enables « Mettre à
jour », which writes the ten bands and the bass lift only; a built-in is code, so it offers « Enregistrer
comme… » instead. Update and delete are undoable toasts, never a confirmation dialog. The live per-band
meter and its saturation warning sit above the faders; the meter's subscription is scoped to the card being
on screen and the page visible, and the warning's « Réduire le volume de X dB » lowers the player volume
optimistically with an undo toast (cost, colours and the peak-hold reset: playback.md). Measured 390x844:
card 350 px wide, no sideways overflow, chips 44 px, all 15 presets in the DOM and the rail scrollable; at
1440: 760 px card, bars centred on their frequency labels to 0.01 px.

**Long grids grow as they scroll** (`lib/grow.js`: `STEP`, `use:nearEnd` on a sentinel after the last tile;
Library's Artistes, Albums, Playlists). 300 followed artists mounted at once cost 1.2 s of main thread on the
4x-throttled desktop (0.9 s phone) before the chip answered; mounting 36 then the next 36 as the end comes
within 800 px: 0.18 s / 0.11 s (`test/clicks/run.mjs`, *biblio: Artistes*; Albums 0.63 s → 0.06 s). The
observer's ROOT must be the scrolling `<main>`: against the viewport the margin is clipped by `<main>` and the
grid never grows. An observer only reports changes, so after each step it is re-observed (a tall screen
that still shows the sentinel asks for the next step).

**Swipe a track row right to queue it next** (`components/TrackRow.svelte`, touch only). Same
gesture language as the now-playing sheet: arm on touchstart, commit to an axis after ~10px, and
leave a gesture that locks to "y" entirely to the scroller. The row slides and the strip it vacates
on the LEFT is what shows the action — drawn there rather than behind the row, so the row needs no
opaque background of its own and nothing has to match whatever page background (gradient header
included) it sits on. Deliberately wordless: the strip is ~64px wide at the point it commits, and
any honest label gets clipped to a word naming a different action ("Lire ensuite" → "Lire"); the
toast on release says it in full. A gesture starting on a control belongs to that control, one
starting at the very left edge is left to iOS's own back gesture, and a drag swallows the click the
browser synthesises from it.

**The queue is edited in ONE component** (`components/QueueList.svelte`, shared by the desktop side
panel, the full-screen player and the phone's sheet): tap to play, remove, **drag the grip to reorder**,
*Vider la file*. Only what is AFTER the playing track can be moved or removed. The list is windowed, so a
drag never depends on a row being mounted: the target comes from geometry (`VirtualList.indexAt`), the
picture under the finger is a separate floating copy, the pointer is followed on the WINDOW (the row may
leave the DOM while the list scrolls under the finger toward the end of a 4 000-track queue — `autoScroll`,
whose edge zone is capped at a quarter of the list), and `touch-action: none` on the grip alone is what
leaves the rest of the row to the scroller. `player.move(from, to)` carries the playing track's index along
and does NOT bump `seq` (a bump is a deliberate navigation: the audio owner would restart the track); under
shuffle it leaves `_orig` alone. `player.clearUpcoming()` also switches `autoplay` off — a radio refilling the
queue five seconds after it was cleared is the opposite of the request — and a new queue (`playQueue`,
`shufflePlay`) switches it back on. Both are remote commands (`move`, `clear`, read level, validated server-side).

**The contextual menu is ONE spec, on every list** (`lib/trackmenu.js`). The same labels and the
same actions — lire ensuite, ajouter à la file, playlist, remplacer, genre, télécharger, partager,
radio, favori, je n'aime pas, crédits, artiste, album — are built once and opened from a track row,
the artist credit on it, a queue row, an episode, a shelf preview, a history event, a disliked
title, the editable playlist, and album/playlist/artist/show cards. It reads only stores already in
memory: opening it costs **0 `/api` request** (16 surfaces × 1280×900 and 390×844,
`test/menu/run.mjs`, 32 measured opens). A queue row drops "Lire ensuite"/"Ajouter à la file" and
gains "Retirer de la file" only while it is ahead of the playhead; the click re-resolves the title
in the queue, so a playhead that moved under an open menu cannot remove the wrong row. Shift+F10
and the Menu key open it on the focused row, Escape closes it and gives the focus back to the
trigger (and `ImmersivePlayer` no longer closes the full-screen player on that same Escape),
`aria-haspopup="menu"` sits on the trigger where ARIA allows it, a drag in progress never opens one,
and a right-click inside a selected text keeps the browser's own Copy menu. The bench holds 242
checks.

**Sleep timer** (`lib/sleep.js`, `components/SleepButton.svelte`): minutes, or the end of the track. The deadline
is a wall-clock comparison, never ticks counted (a phone on a bedside table is throttled or frozen and must still
end where the clock says). The last 15 s fade through `sleepFade`, a multiplier the audio owner applies ON TOP of
the volume, so the user's own level is never touched and never has to be put back. "End of the track" holds the
crossfade and the silence trim back (`sleepStopsAtTrackEnd`, also in `partyPlan` and `beginCrossfade`) so the file
finishes, then `player.advancePaused()` lines the next track up paused. It fires a PAUSE, not a stop. Not offered
on a remote control: the timer runs where the audio is.

**Podcast speed** (`lib/speed.js`, `components/SpeedButton.svelte`, store `podcast.speeds`, mirrored to remote
controls): per show, applied to BOTH audio elements and as `defaultPlaybackRate` too (`load()` resets the rate to
it on every source), `preservesPitch` on. Music is never touched. Forced to 1 while a listen party is hosted: its
guests schedule the audio in real time, a faster host would leave them further behind every second.

**Explore** (`/api/explore`, `/explore/genre/<id>`, `/explore/countries`, `routes/Explore.svelte`): Deezer's public
API only — the world and per-genre charts, editorial releases, the per-country chart playlists. A screen is several
calls, run side by side; the answer is kept ten minutes but ONLY WHEN COMPLETE (`_explore_cached`): a call that
failed is a hole in the answer, not something to remember. The account owner's (admin) screen, like the home.

**Under the full-screen player** (`components/NowPlayingMore.svelte`), top to bottom: the track's **credits**, the
artist (photo, fans, top titles, similar artists; the public API has no biography, so the card does not pretend to)
and — LAST, because it is the long one — tracks that sound like the playing one (the radio's mix: one call to swap
for our own embeddings later). On the phone a swipe UP is the mirror of the swipe down that dismisses
(`MobileNowPlaying`: `revealMove` / `revealEnd` beside `dismissMove` / `dismissEnd`, from the sheet or forwarded
from the cover; the panel follows the finger and settles open or shut; a swipe down drags it back down from anywhere
on it — the header always, the body once it is scrolled to the top and the finger's first move is down, so a
scroll or a sideways shelf keeps its gesture; a cancelled gesture decides nothing). On the desktop it is the *À propos* tab. Following any link from in there leaves the
player (a `hashchange` listener). **For a podcast the same door becomes the episode's dossier**:
Chapitres → Description → Informations, with no separate “Épisode” tab; the empty “Paroles” door/column
disappears for a podcast on both mobile and desktop, while the swipe-up hint stays. `lib/chapters.js`
is the only store (keyed by episode **id string**, deduped, 40 entries); openings cost **0 requests
before the door, 1 on the first open, 0 per tick and 0 on reopen**. Rows are 60 px touch / 56 px
desktop, the current one carries `aria-current="step"`, and above 60 chapters the shared `VirtualList`
mounts 18 rows at 100 (13 after scrolling, ≤ 33). Ticks appear on the full-screen and desktop rail
only, `aria-hidden`, and only when `(width − thumb) / N ≥ 7 px` (2 ticks for 3 chapters, 0 for 100 on
239 px). The Show page chip costs one request per expanded episode, unfolds 4 rows ≤ 640 px / 8 above,
then “Voir les N chapitres” opens a body-portalled sheet that returns focus to the chip. A
non-archived episode, an empty scan and a network failure each say so; no chapter is invented.
Chapters are deliberately not in the offline `/api` cache.

**Transcription lives in the same About panel** (`components/TranscriptPanel.svelte`,
`lib/transcript.js`, `transcriptcache.js`): the podcast panel becomes `Chapitres | Transcription`,
one page of **200** timed lines, the current segment highlighted and followed, tap-to-seek without
restarting playback, a “back to the live position” pill and an internal debounced search. A ready
transcript costs **0 requests while listening**, 1 GET for the next page or an internal search, and
is cached in IndexedDB `neezer-transcripts` (50 pages / 5 MB, 4 per episode, key
`episode:limit:offset`, `ready` only, local search; never in `REMOTE`). A `queued|running` job polls
once per 10 s with `limit=1`, never in `ready`; the status/error copy is French and the admin can
start/stop/delete, while every account reads. A search hit opens the full-screen player **and** the
About panel on the Transcription segment at the hit's timecode (`panelRequest` is consumed by the
desktop/mobile shell with `segToken`, expiring after 3 s); the episode page has the same jump. The
global Space shortcut ignores interactive targets (`button/a/input/textarea/select/[role=button]/
[contenteditable]`). Settings carries the admin transcription card (enabled, language, limits,
search scope, engine/model probe) and the two chapter switches. Measured: 0/1/0 requests on a ready
transcript, 1 GET per page/search, 1 poll per 10.5 s while running, 200 lines at 390/641/1440 with no
sideways overflow, 44 px controls.

Its requests are keyed on the track's id as a STRING: `track` is the store's
object, handed over again on every progress tick, and a reactive statement reading it asked for the credits four
times a second (the browser test counts requests).

**The desktop player adds reading columns before the queue gives way** (`DesktopNowPlaying.svelte`,
`Lyrics.svelte`). The queue card is the one thing that never disappears: at **1200 px** (Material's *large*)
the lyrics become a persistent left column — synchronised, scrolled to the active line even when the column
mounts mid-track (`onMount` + `tick`; the old tab opened at the top and stayed there) — and at **1600 px**
(*extra-large*) `À propos` gets a fourth card to the right of the queue. Space coming back is taken in that
order: the facts return to the queue card's tab first, then the lyrics. Measured with the animation strip on:
1920×1080 four columns 340 · 520 · 360 · 360 px; 1600×900 288 · 512 · 360 · 304; 1280×900 three
columns 269 · 520 · 360; 1200×900 252 · 476 · 360 (the player's floor); 1199×900 two columns 520 + 360
with all three tabs back. The strip itself is **44 px, and 56 px where the column can pay for it**
(`--viz-strip-h` on `.main`): from 880 px of window height without a Flow, and only from 1060 with
one, whose 92 px steering row takes the band first. It is centred by auto margins in the empty band
under the transport, never against the controls or the footer: measured at 56 px, 1280×900 y 757–813
with the transport ending at 728 and the footer at 842–876 (the footer keeps its 24 px of stage
padding — the 12 px come out of the free band, not off the bottom); 1440×900 and 1920×1080 the same;
1280×870 falls back to 44; 641×900 (the narrow side-panel layout) does not draw it at all. The mobile
strip stays 40 px between the transport and the thumb row (390×844: 662–702, thumbs at 750; a 34 px
stripe at 360×640 became 40 px again with `flex:none`, the sleeve absorbing the 6 px).
`test/vizstrip/run.mjs` holds the geometry and the canvas — `520 × 56`, the visualizer sizes itself
from its host. The queue card's tab row is a single dead pill once four columns are up, so it is
replaced by the same label header the other columns use. **The Flow's steering row is 92 px high**
(its Genres door, two feedback chips and the energy wrap on two lines in the 520 px player), so under
**950 px high** the strip yields to it rather than pushing the footer off screen: 1440×900 Flow now
ends its footer at 848–882 with the strip hidden, 1920×1080 keeps the strip at 56 px, and 1440×900
without Flow keeps it too.

**The phone's lyrics are a view, not a hidden gesture** (`MobileNowPlaying.svelte`): the labelled « Paroles »
door in the thumb row opens a full-height sheet whose own header names the track and the artist, with a 44 px
close. One scroller only — the sheet is `overflow:hidden`, `Lyrics.svelte` keeps `overflow-y:auto` and follows
the active line; opening it scrolls straight to that line even mid-track (the same `onMount` + `tick` as the
desktop column). It closes the queue sheet when it opens and the queue closes it back; `À propos` closes both.
Measured 390×844: 30 lines, close 44×44, the list scrolls 169 → 281 as the active line goes 15 → 30; at
360×640 the five thumb targets span 338 px of 360, no overflow, and the sheet still fills the screen.

**La carte de paroles** (`Lyrics.svelte`, `LyricCard.svelte`, `lib/lyriccard.js`): « Créer une carte »
opens a 1080×1350 (4:5) canvas poster from one to three tapped lines (the active line by default,
always in lyric order; a fourth tap is refused, the last line cannot be removed). The sleeve comes
from `offlineCovers` only — never a request — and a missing one is Sillon's deterministic groove;
`dominantColor` tints the halo under a scrim, so cream text stays ≥ 6.08:1 even on a white sleeve.
Share = Web Share with a PNG File, else a download; the Android bridge's `shareFile` only takes a
server URL, so the app says so rather than pretending. Measured at 4× CPU: sheet 11.9 ms, preview
paint 189.5 ms, line tap 2.1 ms, `toBlob` 375 ms; PNG 1.2–1.6 MB. Zero `/api` and zero sleeve
requests at open, selection and share.

**Full credits** (`neezer/trackcredits.py`, `neezer/webui/credits.py`, `GET /api/track/<id>/credits`): the
performers are `TrackArtist` rows; everybody else on the sleeve (author, composer, producer, publisher…) is a
`track_credit` row (schema `20261030`): one per person, role and SOURCE — `deezer` (the gateway's
`SNG_CONTRIBUTORS`, stored when a track is archived), `tags` (an upload's composer/lyricist/producer, stored at
import), `musicbrainz` (stored by the enrich queue). Each source is replaced on its own; `merge` keeps Deezer's and the
tags' roles whole and lets MusicBrainz only FILL a role neither named (its lyricist is skipped when Deezer named an
author). The panel reads the database, completes it from the files (sidecar `gw`/`facts`, the upload's tags) and
stores what only the files said; only a track NOT on disk is worth a question to Deezer (kept a day in memory, and
stored when the track is a library row, so it answers with Deezer away). Before this, an archive made before the
sidecar kept `SNG_CONTRIBUTORS` named its main artist and nobody else, forever — what the person reported. The
library already on a server is completed ONCE by `trackcredits.BackfillJob` (`libjob` with a `page_done` hook, run 45 s
after a start until it finishes with Deezer answering; Réglages → Archive → "Crédits des titres" shows it): the disk
first, then Deezer **50 tracks per `song.getListData` call** (falls back to one `song.getData` a track if the list
call ever comes back without credits), each call behind `quiet_wait`, the answer written into the sidecar too; an
outage fails those tracks with "Deezer injoignable (…)" and writes nothing. A name links to the library's artist of
that name (catalogue artists only) or, once credited on 2+ tracks, to `#/credits/<name>` (`routes/Credits.svelte`,
`GET /api/credits/person?name=`: indexed on the folded `name_key`, performer roles left out, `_visible_tracks`); an
artist page grows a "Crédits" tab when `credited` > 0. Every track menu has "Voir les crédits"
(`CreditsSheet.svelte`, same request, kept offline like the panel's). Measured on 8 000 tracks: the panel 8 ms / 6
statements, a 400-title credits page 31 ms / 4 statements. Known roles come in a sleeve's order and French labels
(`ROLES`), an invented role under its own name after them, everything bounded (40 names a role, 160 characters).

**Back goes to the SCREEN, not the route** (`lib/nav.js`): hash routing gives real history, but the
router destroys a page's component state on the way out and rebuilds it empty on the way in — so
coming back from an album landed you at the top of a blank search page. Every history entry gets an
id stamped into `history.state`, and against it nav.js keeps the scroll offset plus a scratch object
the screen fills in (`rememberScreen` / `recallScreen`: the search query + results + tab, the
Library tab and playlist filter, the Artist tab). Only a *traversal* gets that state back — a fresh
push onto the same route starts clean, so tapping "Rechercher" in the nav still gives an empty box.
Two traps are already paid for: the router's own hashchange listener is registered at import time
and builds the new screen synchronously inside it, so nav.js resolves the current entry **lazily**
(`syncEntry` on every entry point) rather than racing listeners; and a scroll restore must **settle**
(the offset holding on a page whose height stopped moving) instead of stopping at the first frame it
fits, because popstate lands one task before hashchange and those first frames can still be
measuring the page being left. A reload restarts nav.js's id counter while the entries already in
the stack keep theirs: `syncEntry` adopts the id it finds (`seq = max(seq, id)`) or the first push
reused id 1 and two entries shared one offset — measured, Back landed at 0 instead of the 120 px
the person had left.

**Offline & versioning** (the SPA is an *install*, not a page — treat it as one):
- `public/sw.js` serves the shell **cache-first** (an instant launch on any network) and stages a new
  build on demand: it fetches the new `index.html` + every asset **the build manifest lists** and
  only then publishes the shell, so an interrupted update leaves the previous *complete* build in
  place. It never touches `/api` or audio. The manifest (`assets` in `dist/version.json`, written by
  `vite.config.js#buildStamp` from the emitted bundle) is what makes code-splitting safe: scraping
  `href=`/`src=` out of `index.html` finds only what Vite statically preloads and says nothing about
  a chunk reached through a dynamic `import()`, so a lazily-loaded route would install, go offline,
  and then fail the first time somebody opened it. The HTML scrape is kept as the fallback, for an
  older server whose `version.json` has no `assets`.

- **The cache belongs to ONE account** (`lib/account.js`, `apicache.js`, `offline.js`): the owner
  is `origin + "/id:" + user.id` (`/api/me`, `/api/login` and the invitation redeem all carry the
  stable id; a saved account from an older build falls back to its name until the next boot), every personal key is
  `neezer:<origin>/<name>:<key>`, and the IndexedDB store is keyed by `(owner, path)`. Without a
  known owner nothing is read or written; a 401/403/404 deletes the entry; a logout/account switch
  purges that owner's rows and keys (then reloads, which clears module caches). Queue, position,
  recents, last playlist, podcast progress/speeds are scoped; `search.recent`, `flow.*` and the
  discovery levels are purged; device settings (theme, volume, quality, output, viz, EQ) stay
  global. Downloads carry `meta.owner` and are listed/played/deleted only for their account —
  another account's bytes are never removed, only hidden. A v1 cache without an owner is dropped
  once, and old personal keys are adopted by the remembered account; a name-keyed owner from an
  older build migrates to the id form on the first boot (one reload, then stable).
- **A 5xx is not a logout.** At boot only a 401 clears `auth.user`; a 5xx/network error keeps the
  remembered session, paints the shell with « Le serveur ne répond pas » + « Réessayer » (or a
  retry screen without a remembered session) and never shows the login form. `revalidate()` follows
  the same rule.
- **A failed request is never an empty state.** `LoadFailed.svelte` covers 404/401/403/offline/5xx
  and Deezer, prints the cause and offers one « Réessayer » (one click = one request); screens that
  already carry their own sentence keep it. `favTracks`/`playlists` stay `null` with a separate
  error (`favTracksError`/`playlistsError`) instead of `[]`, so the Library/Podcasts/Show/Sources
  never say « 0 titre » or « Rien ici » for a 500. Rows offer **44 px** on a coarse pointer and
  40 px with a mouse (the play/actions columns grow via `--play-col`/`--acts-col`), and a ⋯ menu
  opens focused on its first item, walks with ↑/↓/Home/End, and closes on Échap with focus returned;
  the sheets share `lib/modal.js` for `aria-modal` and the focus trap.

**The heavy screens load when they are opened.** Everything on the first screen is in the main
bundle and worth being there; three things are not, and together they were a third of what every
visitor downloaded, parsed and compiled before the first note played — which is the "the whole app
got slower" this answers. **Réglages** carries the animation catalogue, **the genre studio** carries
a WebAssembly trainer, and **the projector** is a screen most people never open, so all three are
`wrap({ asyncComponent })` routes. The scene modules are split the same way: `lib/viz/index.js`
imports a scene on demand and `createScene` returns a PROMISE (a caller must cope with the scene
arriving late and with the mode having changed again — `Visualizer.svelte` holds a token per build
for exactly that). Two files exist only to keep the catalogue OFF the critical path:
`lib/viz/modes.js` is the registry with no scene imports (the now-playing screens were dragging the
whole scene tree in for two constants) and `lib/viz/worlds/catalogue.js` is each world's name and
blurb without the shader that draws it, because `skins.js` and the settings gallery only need to
know an id is real. Every world is its own chunk (one literal `import()` each in
`worlds/index.js`), fetched the first time it is on screen. Rollup hoists a module two chunks share
into their common parent, so ONE static importer left in the main graph puts the whole tree back:
`palette.js` and the settings panel therefore load `skins.js` and the catalogue dynamically. Measured,
main bundle **656 kB → 457 kB** (223 → 158 kB gzipped), CSS **128 kB → 85 kB**; the GL engine is a
55 kB chunk and the largest world 25 kB (the microwave, 9 kB gzipped — fetched only when it is on screen).
- `lib/appversion.js` is the other half: the bundle's own id (`__APP_BUILD__`, injected by
  `vite.config.js`, also written to `dist/version.json`) is compared with the server's
  (`/app/version.json`, never cached). Different → stage in the background → reload (automatically
  only within the first 90 s of a session, otherwise via a notice; guarded against reload loops).
  It also does the **startup-only** Android update check (`window.NSNative.appVersion()` vs
  `/api/version`'s `android.version`).
- **A cached answer is a success, never a failure.** A failed list fetch must never be stored as the
  offline copy — a playlists error cached forever turned the offline screen into a permanent empty
  state — and the service worker never caches an error response. Offline playback is for bytes
  actually downloaded: a track that only exists as a server-side “local” file is not playable while
  the device is offline, and the UI says so instead of handing the player a URL that cannot answer.
  Durations of an hour or more are `h:mm:ss` (`lib/format.js`), never `92:15`.
- `lib/reconcile.js` merges a refetched list into the one on screen (identity preserved for
  unchanged rows) — playlists and favourites paint from the offline cache first and the network copy
  is reconciled in, never swapped wholesale. `lib/actions.js#warmPlaylists` pulls every playlist's
  tracklist into the offline cache in the background, one at a time.
- Persistent, actionable messages go through the `notices` store + `components/Notices.svelte`
  (toasts are for transient confirmations only).
- The account's playlists live in ONE store (`stores.js#playlists`, owned by
  `actions.js#userPlaylists`/`invalidatePlaylists`): the sidebar, the library tab and the "add to
  playlist" sheet all paint from it, so a playlist created from a sheet that opens OVER a screen —
  which never remounts it — is listed everywhere at once. `upsertPlaylistLocal` /
  `removePlaylistLocal` show a create/rename/delete instantly; `invalidatePlaylists` then refetches
  (debounced — `/me/playlists` counts every playlist's tracks, and the edit paths call it per
  operation).
- **Playlist folders** (`lib/playlistfolders.js`, `FolderSheet.svelte`,
  `MoveToFolderSheet.svelte`) are a per-person view of that same store: the sidebar shows the
  folders collapsed by default (state kept per device in `localStorage`), then the unfiled lists,
  then the Deezer import fold; the Library's Playlists tab renders one section per folder plus
  « Sans dossier » and « Depuis Deezer ». Every signed-in account loads its own + shared lists
  (no `$isAdmin` gate); a shared card carries its owner and only `editable`/`can_manage` open the
  edit paths — moving it to one's own folder stays allowed, it is the viewer's organisation.
  First paint is bounded to **8 sections × 12 cards = 96** (500 playlists / 20 folders at 390×844:
  96 cards, no sideways overflow); each scroll step adds 12 per section and 8 sections. Creation,
  rename, move, reorder and delete are **one request each** (undo is 2: recreate + order); opening
  a folder is 0. The Organiser mounts all rows only up to 60 — above that the shared `VirtualList`
  takes over: a 500-playlist folder opened in **287 ms at 4× CPU with 11 rows mounted**, against
  956 ms cold / 1 225 ms at 4× without it. The two sheets are mounted once at the end of
  `Sidebar.svelte` and portalled to `<body>`; do not mount a second copy in `App.svelte`. Offline,
  a folder mutation fails fast with « Hors ligne : impossible de modifier les dossiers » and never
  paints a false success.
- **ReplayGain is preloaded, never fetched from under a playing track** (`lib/gaincache.js`).
  Normalization is STATIC — `loadTrack` picks the gain at the source handover and holds it for the
  whole track — so a gain that lands one request later is not a refinement, it is the volume moving
  in the middle of a song. The current track and the next `GAIN_WINDOW` are therefore primed
  together in one call (`POST /api/gains`, DB rows first then a single `song.getListData` for the
  rest) as soon as the queue moves, and the answers are kept on the device (IndexedDB, negatives
  too, with a TTL), so a track played once never waits for the network again. The prefetcher and
  the downloader prime it with the audio — what has its bytes on the device has its gain there. A
  gain that still arrives late is applied only within `GAIN_LATE_GRACE` of the track's start;
  past that it is cached for next time rather than jumped in. The one deliberate exception is the
  user changing the normalization level, which may move it now.
- Cover art: a cached blob (`offlineCovers`, keyed resolution-independently by `coverKey`) is the
  **preferred** source in `Cover.svelte`, not a fallback — it's the server's archived 1000px art, so
  waiting for a CDN request to fail first is pure delay. Anything you play caches its cover
  (`playcache.cacheCoverFor`), so hi-res art works offline for everything you've listened to.
  The IndexedDB database names (`nsupy-apicache`, `nsupy-gains`, `nsupy-offline`,
  `nsupy-playcache`) and `NSUPY_BUILD_ID` predate the Neezer rename and stay: they are the storage
  identity of an installed app. Renaming them empties the device's caches and its downloaded
  tracks, so this is a persisted name, not a brand token.
  That source is an OBJECT URL, so the CSP's `img-src` must carry `blob:` (`web.py`,
  `tests/test_security.py`): without it the browser refuses every cached cover, the refusal reads
  exactly like a corrupt file, and the cache "repaired" it by deleting the bytes — which is how a
  phone lost the art of every download. So a CSP refusal is logged as one (`log.js`,
  `securitypolicyviolation`), and a cached cover is only ever dropped once `createImageBitmap` —
  which no CSP governs — confirms the bytes do not decode (`imagebytes.js#blobDecodes`); covers
  the old behaviour deleted are re-fetched in the background (`offline.js#repairMissingCovers`).

**The Flow holds the playing title and AT MOST the next** (`lib/flow.js`): the next is chosen `PICK_AHEAD` (20) s
before the transition, crossfade counted (`Player.svelte` calls `tick` on every position update), with what the
session said (heard seconds, the `FlowControls.svelte` buttons, the notch); one request per title, a few hundred
bytes. A reserve title asked 6 s into each track makes "suivant" instant and covers a failed pick; a hidden page
picks at once (its timers are throttled, and the app's own player must know the next). A feedback press after the
pick picks again (`replaceUpcoming`). `ensureUpcoming` leaves the Flow alone. A Flow queue saved by an older SPA (no
`context.id`) is adopted into a new session. The steering row costs the full-screen cover its height
(`.main.steer .cover`; `test/fullscreen/run.mjs` sweeps 9 sizes WITH it).

**The phone's full-screen player is a thumb column** (`MobileNowPlaying`, DESIGN §9). Header: reduce (44), then
« En lecture »/Flow with the served genre and BPM in mono, then — right — the podcast speed chip, the sleep moon, the
**quality badge** (`QualityMenu`, its small mono outline on a 44 px hit area) and ⋯ (the track menu: share, credits,
artist, album, downloads). The sleeve is a square `min(78vw, 42vh)`, the title 30 px (one line, ellipsis), the artist
15 px `--dim`, the seek a 6 px rail on a 44 px grip, the transport 44 px targets around a 68 px play. Under it, the
thumb row: add to a playlist, listen party, remote, lyrics, « File · N » — 44 each, the queue button named exactly
*File d'attente* because the benches click that name. **Whatever sits under the header shaves the sleeve** (the lyric
line only exists when there is one, the strip and the Flow row give their height back) rather than pushing the
transport off the screen: 360x640 measures 248 px square (269 without the strip, 212 with the Flow row too),
390x844 304 px (width-capped), body and mid overflow 0. Safe areas read `--safe-top` / `--safe-bottom` with an
`env(safe-area-inset-*)` fallback until G1's tokens land. **The three sheets (queue, lyrics, the
slid-up About panel) wear the glass ladder's tier 3** — `blur(var(--glass-blur-3))
saturate(var(--glass-sat-3))`, measured `blur(22px) saturate(1.45)` at 390×844 — and lose the filter
in eco through the shared `html[data-eco="1"]` collapse (measured `backdrop-filter: none`, `--s1`
ground). No component px value of their own: the old ad-hoc 16/18 px are gone. At 1280 the mobile
player is not mounted at all, so those sheets are not either. The queue and lyrics panels are
`.sheet.on-photo` — the THICK fill (93 %), because they sit on the sleeve and not on the page's
scrim (measured 4.17:1 at 86 %, 5.18:1 at 93 % for the 11 px `--dim` labels) — plus the material's
hairline under their header, and they cast no outer shadow (nothing to cast onto). The thumb row's
doors are clear glass: 34 % of `--s1`, the specular hairline, and an INSET ring rather than a border,
so the five 44 px targets still fit a 360 px phone (measured: the doors' `--dim` label at 6.28:1).

**Explorer is ten parent styles, not a genre search** (`components/ExplorerStyles.svelte`,
`GET /api/explorer/styles`, `POST|GET /api/explorer/discover`). The landing asks the server for the ten
PARENT styles of the caller's own taste (`discover.parent_styles`: Boogie is served as Disco/funk); it never
puts a style name in the search field. A style opens a discovery sheet with a 1–5 slider. The POST starts
one worker; the GET only reads (`pending`/`ready`/`idle`) and never starts work, so a page cannot trigger a
Deezer sweep by opening a URL. Poll every 2 s, stop on `idle`, `error`, `429` or `503`; those two carry
`Retry-After` and the sheet counts it down instead of spinning. One POST is one discovery: measured, a
budget refusal is **1 POST / 0 GET**, a poll refusal is **1 GET** (the retry in `api.get` would have
replayed it), and an `idle` answer is **1 GET then 0**. The Flow picker (`FlowPicker.svelte` inside
`FlowControls.svelte`) shows the selected genres and the diversity level in one sheet on phone and desktop;
it consumes the existing `flow.js` actions and changes no queue rule.

**A style tile starts a genre station with its disc** (`ExplorerStyles.svelte`, `lib/genrestation.js`,
`GET /api/radio/genre/<path:style>`, `POST /api/radio/genre/next`). The tile is a card with TWO
sibling buttons, never a button in a button: the `.open` overlay keeps the Deezer discovery sheet,
and a 44 px play disc (always visible — Spotify's card play button, not a hover secret) starts the
station through `player.playQueue(..., {kind: "genre-radio", id, title: "Station <style>"})`. One
request, then the player tops up with `api.genreRadioNext`; an empty branch is one toast, a failure
is one toast and the sheet stays usable, and the pressed disc shows a spinner + `aria-busy`.
« Toutes les stations » lists every analysed genre from `/api/flow/styles` (name, parent, branch
count) with an accent-insensitive local filter; it paints the session/localStorage copy first and
asks ONCE per open, never per tile. Measured 1440/390: 132 rows, 2 opens = 2 requests, disc 44×44,
no sideways overflow at 390.

**A sidebar playlist's thumb is its real cover** (`components/Sidebar.svelte`, `Cover.svelte`,
`lib/coverqueue.js`). The 26 px tile renders `Cover` with the playlist's `cover` and keeps the deterministic
groove only while loading or when no cover exists. The shared queue bounds it: **6 requests in flight**, an
horizon of two viewports, and no request for a playlist without art. On a 61-playlist desktop, the first
cover painted in **186 ms**, 30 requests for the rows near the viewport, and scrolling to the bottom asked
15 more (37/60 total, never more than 6 at once); the phone (≤640 px) hides the sidebar and makes **0**
cover requests.

**« Titres recommandés » is the bottom of a playlist** (`routes/Playlist.svelte`,
`GET /api/playlist/<id>/recommendations`): only for an editable playlist, never in edit/search mode or
on a remote/read-only session; asked AFTER the playlist's own payload, once per open, cached in module
memory per playlist and invalidated by any add/remove. **+** adds through the page's optimistic
`onAdded` (the row leaves at once, returns if the save failed); *Tout ajouter* sends exactly the
visible rows in ONE `POST /api/playlist/<id>/tracks`; a failure is a compact line with « Réessayer »,
never a skeleton. 12 rows, ~7 kB / 1.7 kB gzip, 12 statements whatever the playlist size.

**A playlist can be published as a read-only link** (`ExportSheet.svelte` → « Lien public »,
`routes/SharedPlaylist.svelte`, `/pl/<token>`): the sheet creates/reads/revokes the capability, copies
the full URL, and says that only archived tracks play. The public page is loaded BEFORE login, with
no app shell and no global player: its own `<audio>` queue, Lire/Mélanger, next on `ended`, previous,
seek and volume, the capability cover URLs, a disabled « Non archivé » row, and a calm « Ce lien
n'est plus valide. » for a revoked/expired token. No edit, favourite, add, download or library
navigation. The short link is a real HTML page (OG title/description, meta refresh into
`#/shared/playlist/<token>`, noindex, no-referrer). Measured 390/1440: no sideways overflow, 44 px
targets; a 4 000-title page is 50 ms / 5 statements. The list mounts **100 rows at first** and
« Afficher les N titres suivants » extends it in slices, so the DOM and the cover queue never mount
4 000 rows on a phone; the queue itself still holds every streamable title.
