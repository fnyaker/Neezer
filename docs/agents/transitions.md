# Transitions: motion tokens, the eco/reduced guard, and the element-by-element plan

Reference notes for agents working on this area. Read [CLAUDE.md](../../CLAUDE.md) first: its process and
rules apply here. Other areas: [docs/agents/](./). Read [webapp.md](webapp.md) (navigation, the glass ladder,
`nav.js`) and [playback.md](playback.md) (big-list cost) before coding a lot; [animations.md](animations.md)
owns the visualizer and the eco switch itself.

**Status (2026-11-16): T0 landed; T1–T7 to do.** The foundation is merged — `lib/motion.js` (the
tokens, the three levels, the transitions, `screenKey`/`pageEnter`/`shiftRows`), `lib/portal.js`, the
tokens and the eco/reduced guard in `app.css`, `test/motion.test.mjs` (unit + the frozen import
allowlist) and `test/motion/run.mjs` (the Chromium bench every later lot adds a scenario to). Nothing
in the app uses it yet: T1 → T7 consume it, one lot per row of the inventory below, and each updates
the *Status per lot* table at the end of this file in its own PR with its measured numbers.

## The need

The person taps something and the result appears in one frame: a page swaps, a sheet pops, a row
vanishes, a toast is just there. They asked for **a transition on every movement or change** — page change,
displacement, appearance, disappearance — **fast and discreet, never heavy** (a premium feel), and **none at
all in eco mode**. The default device is a phone on a bad mobile link with an 8 000-track library and a
4 000-track queue, so a transition must never delay the answer to a click, never add a request, and never
cost layout on a long list.

## What the sources say (research, 2026-11)

- **NN/g, "Executing UX Animations: Duration and Motion Characteristics"** (nngroup.com/articles/animation-duration):
  most UI animations 100–500 ms; simple feedback ~100 ms; a modal/screen change 200–300 ms; 400 ms is "very
  slow", for big movements on large screens only; "at 500 ms animations start to feel like a real drag".
  Entering takes **subtly longer than exiting** (popup 300 in / 200–250 out). Ease-out to enter, ease-in to
  leave. "The more frequent the animation, the more subtle and shorter".
- **Apple HIG, Motion** (developer.apple.com/design/human-interface-guidelines/motion.md): add motion
  purposefully; "aim for brevity and precision in feedback animations"; "generally avoid adding motion to UI
  interactions that occur frequently"; "let people cancel motion… don't make people wait for an animation to
  complete before they can do anything"; motion must never be the only carrier of information. App Store
  Connect's *Reduced Motion evaluation criteria* warn against removing everything: reduce, keep the meaning.
- **Material 3 motion tokens** (generated source: material-components/material-web
  `tokens/versions/v0_192/_md-sys-motion.scss`): `standard` cubic-bezier(0.2, 0, 0, 1);
  `standard-decelerate` (0, 0, 0, 1); `standard-accelerate` (0.3, 0, 1, 1); `emphasized-decelerate`
  (0.05, 0.7, 0.1, 1); `emphasized-accelerate` (0.3, 0, 0.8, 0.15); durations short1–4 = 50/100/150/200,
  medium1–4 = 250/300/350/400, long1–4 = 450–600 ms.
- **Linear** (Emil Kowalski, design engineer at Linear): UI animations "should generally stay under 300 ms";
  a tooltip opened while another one is open shows with no delay and no animation (frequency kills motion).
- **Apple Music / Spotify** (kodeco.com "Recreating the Apple Music Now Playing Transition"; aplus.rs "card-like
  popup apple music"; Spotify community threads on the iOS mini player): the now-playing card grows out of the
  mini player with a spring that barely overshoots; Spotify's users complain precisely when the mini player
  "appears all of a sudden" with no fade on close — the asymmetry this plan removes.
- **View Transitions API** (caniuse.com/view-transitions, Oct 2026: 93.17 % global; Chrome 111+, Safari 18+,
  Firefox 144+; Android WebView follows the installed Chromium; our `minSdk` is 26, so an Android 8 phone may
  run an old WebView): during a transition `::view-transition` covers the viewport and **clicks are
  swallowed** (blog.master.dev "View Transitions: careful not to make stuff unclickable", Chris Coyier, 2026;
  CSSWG #7797 / #11596). Measured below: the click target during a transition is `HTML`, and the DOM update
  waits one frame (12.7 ms at 1x). **Not used** (rule M3).
- **Svelte 5.56 transitions** (read in `node_modules/svelte/src/internal/client/dom/elements/transitions.js`
  and `src/animate/index.js`): transitions run on the **Web Animations API** (`element.animate`, keyframes
  sampled from `css(t)` at 60 Hz), so a CSS `!important` rule never stops them — the eco guard needs a JS half.
  They are **local by default**: an intro plays only when its own `{#if}`/`{#each}` block changes after it
  first ran (a component's root element counts as part of the parent's block). A bidirectional `transition:`
  **reverses from where it is** when toggled mid-way; separate `in:`/`out:` restart from 0 (a flash on a fast
  re-open). A transition may return `({direction}) => config`, which is how enter and exit get different
  durations while staying reversible. `duration: 0` takes an instant path (no Animation object). During an
  outro Svelte sets `inert` on the element. `animate:flip` runs `getComputedStyle` + two rects per item and
  only on **reorders** of a keyed `{#each}`.
- **An intro never plays inside a `{#key}` block, nor one `{#if}` deeper than the block that changed**
  (T6a measured both in Chromium). `transition()` registers the entrance while its block effect is still
  initialising and only runs a local intro once that block has already run (`run = !block || (block.f &
  REACTION_RAN) !== 0` in `transitions.js`), and it walks up over TRANSPARENT effects only — a `{#key}`
  branch and a plain `{#if}` branch are not transparent, an `{:else if}` branch is. So: a panel a tab switch
  must re-enter is its own `{#if}` at the level that already ran (Search's shelves), and a change that keeps
  its element mounted plays through the module's `animate()` (the best result of another entity, T6a). A
  `{#key}` around a transition is a transition that does nothing.
- **Animating windowed lists** (web.dev "Virtualize large lists with react-window"; Stack Overflow "animate
  entry of only new elements in a virtualized list"): rows are recycled as you scroll, so an enter animation
  bound to a row's mount **replays on every scroll**, and a row scrolled out cannot play an exit.
- **Backdrop root** (Filter Effects 2): an ancestor with `opacity < 1`, `filter`, `mask`… bounds what a
  `backdrop-filter` sees. Measured below: it is real in our Chromium.

## What a throwaway prototype measured (never committed)

A standalone page (fixed 56 px rows, the same windowing as `VirtualList.svelte`, 8 000 items; a 300-card
grid with the `.card` shadows; a 14 px `backdrop-filter` pane over stripes) driven by Playwright in headless
Chromium with SwiftShader, CPU throttle via CDP, metrics from `Performance.getMetrics` and rAF gaps over the
400 ms after the action. **Headless Chrome produces no frame without damage**, so rAF gaps are only meaningful
while something animates (idle rows read 0 frames at 4x); counts are the reliable signal.

| Scenario | 1x: layouts / worst frame | 4x: layouts / sync ms | Reading |
|---|---|---|---|
| Page enter (opacity + 8 px rise, 180 ms, WAAPI) over a **windowed** 8 000-row list | 2 / 16.7 ms (0 frames > 17.5) | 2 / 6.5 | compositor only, size-independent |
| Same over a 300-card grid | 2 / 16.8 ms | 1 / 1.7 | fine |
| Same over 8 000 **plain** (non-windowed) rows | 2 / **66.7 ms** (layout 6.4 ms) | 1 / layout 37.7 ms | windowing is what makes it free |
| Queue row shift: 40 mounted rows `translateY(56px → 0)`, 200 ms, no reads | 2 / 16.7 ms | 2 / 20.1 | the plan's queue remove/insert |
| Naive FLIP on 8 000 plain rows (rect ×2 per row) | 82 ms sync, style 735 ms, **1 016 ms frame** | 1 166 ms sync, style 11.9 s | forbidden |
| Accordion: `height 0 → 300px` above the windowed list | **12** layouts / 16.8 ms | 12 | one layout per frame |
| Same block entering by `opacity + translateY(-8px)` | 2 / 16.8 ms | 2 | prefer it |
| Eco rule switched on, 8 000 plain rows mounted | style 60.4 ms once | 248 ms once | one-off, on the toggle only |
| Eco rule switched on, windowed list | style 0.4 ms | 1.9 ms | negligible |
| `document.startViewTransition` | update delayed 12.7 ms; click target during it = `HTML` | 14.7 ms | swallows clicks |
| Glass pane, luminance std-dev of the pane (bare stripes = 127.4) | at rest 5.0; **ancestor `opacity: .98` → 127.4**; ancestor `transform` → 5.0; the pane's OWN `opacity: .98` → 5.8 | — | fade the pane, never its wrapper |

Consequence of the last row: today's `<div class="overlay" transition:fade><div class="sheet" transition:scale>`
(CreditsSheet, ExportSheet, FlashWarning, GenreBulkSheet, GenreTagSheet, PlaylistMembersSheet, ReplaceSheet)
**drops the sheet's blur for the whole fade** and snaps it back at the end. The plan's `veil` transition
animates the overlay's `background-color` alpha instead of its opacity (rule M8).

## The rules (each one bought by a source or a number above)

- **M1 — Transform and opacity only.** Every transition animates `transform` (translate/scale) and `opacity`
  on the moving element. Exceptions, each justified: `background-color` of a veil (M8) and of a selected
  row/hover (paint of one element); `height` in `collapse` (M10) for small blocks only. Never `width`,
  `height`, `top/left`, `margin`, `padding`, `grid-template-*`, `filter`, `backdrop-filter`, `box-shadow`.
- **M2 — The click answers first.** A transition never delays the state change: the store/DOM updates in the
  same handler, the animation plays on top. Nothing waits for an animation to end (no `await` on
  `.finished`, no logic on `animationend`/`transitionend`/`introend` — there is none today, keep it so; the
  one exception is a VISUAL hand-off, rule M12). Everything is interruptible: a second click reverses or
  cancels from the current position (bidirectional `transition:`, `Animation.cancel()`).
- **M3 — No View Transitions API, no route outro.** Pages enter; the old page leaves in the same frame
  (svelte-spa-router destroys it). Keeping two pages mounted would double the DOM, break `nav.js`'s scroll
  restore (which settles on the NEW page's height) and swallow clicks.
- **M4 — Nothing animates on mount of a windowed or growing list row.** No `in:`/`out:`/CSS entrance on rows
  of `VirtualList`, `TrackList`, `QueueList`, the `use:nearEnd` grids (`lib/grow.js`), the sidebar playlist
  lists or any `{#each}` that can exceed 50 items. A row moves only on an **explicit action** (remove, insert,
  drag, reorder) through `shiftRows` (fixed row height, zero layout reads). `animate:flip` only on keyed lists
  that can never exceed 50 items (toasts, notices, chips) — measured: FLIP on 8 000 rows is a 1 s freeze.
- **M5 — Exits are shorter than entrances.** Enter uses `EASE.enter` (M3 emphasized-decelerate), exit uses
  `EASE.exit` (M3 emphasized-accelerate) and ~70 % of the enter duration; an in-place move (indicator,
  reorder, collapse) uses `EASE.standard`.
- **M6 — Frequent things barely move.** Hover, press, selection, play/pause icon, a counter: ≤ 140 ms, ≤ 4 px
  or opacity only. Search results updating per keystroke, a seek, a volume drag, the playing position, a
  progress bar fed live: **no transition** (they ARE the motion; animating them adds lag).
- **M7 — Replace in place = intro only.** When B replaces A in the same flow position (skeleton → content,
  tab panel → tab panel, LoadFailed → content, play → pause glyph, one title → the next), A disappears at
  once and B enters (fade, ≤ 4 px). An outro on an in-flow element would keep A's height while B enters:
  a layout shift.
- **M8 — Never fade an ancestor of a glass pane.** `opacity < 1` on an ancestor turns the pane's blur off
  (measured 5.0 → 127.4). Veils fade their `background-color` (`veil`), panes fade their own opacity, page
  and player containers move by `transform` or fade only when they hold no open pane.
- **M9 — `position: fixed` lives in `<body>`.** A `transform` on an ancestor becomes the containing block of
  a fixed descendant. Every fixed overlay rendered from inside a page is portaled with `use:toBody`
  (`lib/portal.js`, T0). The route host's page-enter transform (T1) relies on it.
- **M10 — `collapse` (height) only for small blocks.** It costs one layout per frame (12 layouts measured over
  200 ms). Allowed for an accordion/details body under ~400 px tall that is not above a non-windowed list of
  more than 200 nodes; otherwise expand instantly and fade the content in (`fadeIn`).
- **M11 — No stagger, no spring overshoot, no parallax.** One element, one curve. Stagger multiplies cost on
  300-tile grids and reads as "heavy"; overshoot is Apple's "very little" at best and looks cheap on the web.
- **M12 — Hand-offs by event, not by clock.** When a covering layer opens (full-screen player), what it hides
  is hidden on its `introend`, and shown again on its `outrostart` — never on a timer.
- **M13 — Eco = no animation at all** (`stores.js#ecoMode`, the `data-eco="1"` contract on `<html>`), except
  the **status indicators** marked `data-motion="status"` (spinners, indeterminate bars, the Récap's 7 s
  countdown bar): CLAUDE.md rule 2 requires a running job to SAY so, and a frozen spinner reads as a hang.
  This is the one deliberate deviation from "none"; to revert it delete the `:not([data-motion="status"])`
  in the guard.
- **M14 — Reduced motion = no displacement, short fades.** `prefers-reduced-motion: reduce` keeps opacity
  transitions (capped at 140 ms) and removes every translate/scale (HIG + App Store criteria: reduce, don't
  strip meaning). The tokens cannot reach a literal a component hard-codes: a switch knob's
  `transform: translateX(16px)` has to be killed by the component's own `@media (prefers-reduced-motion:
  reduce) { .knob { transition: none } }`, or the bench's reduced check fails on a running Animation whose
  keyframes carry a transform (T6d measured exactly that; the knob then jumps, the track still fades).
- **M15 — One source of truth.** Durations, easings and distances come from `lib/motion.js` (JS) and the
  `--dur-*`/`--ease-*`/`--move-*` tokens of `app.css` (CSS). No component imports `svelte/transition` or
  `svelte/animate` directly, no literal duration in a `transition:`/`animation:` declaration
  (`test/motion.test.mjs` enforces both, T0 with an allowlist, T7 empties it).

## The tokens

CSS, in `app.css`'s `:root` block (T0). Names are final; values are the plan's.

| Token | Value | Use |
|---|---|---|
| `--dur-instant` | 90ms | hover, press, selection colour, switch thumb, checkbox |
| `--dur-fast` | 140ms | small exits, tab panel fade, glyph swap, badge, traversal page fade; the reduced-motion cap |
| `--dur-base` | 200ms | menus/popovers/toasts/notices enter, page enter (push), collapse, tab indicator, row shift, desktop dialog and full-screen enter |
| `--dur-sheet` | 280ms | phone bottom sheet enter |
| `--dur-sheet-out` | 200ms | phone bottom sheet exit |
| `--dur-screen` | 320ms | phone full-screen player enter |
| `--dur-screen-out` | 240ms | phone full-screen player exit |
| `--ease-enter` | cubic-bezier(0.05, 0.7, 0.1, 1) | M3 emphasized-decelerate |
| `--ease-exit` | cubic-bezier(0.3, 0, 0.8, 0.15) | M3 emphasized-accelerate |
| `--ease-standard` | cubic-bezier(0.2, 0, 0, 1) | in-place moves |
| `--move-s` | 4px | glyph/title swap |
| `--move-m` | 8px | page enter, toast, notice, popover offset |
| `--scale-pop` | 0.96 | menus, popovers |
| `--scale-dialog` | 0.97 | centered dialogs |
| `--scale-screen` | 0.98 | the desktop full-screen player (T0 added it: the `screen` row names 0.98, and a literal in `motion.js` would be the drift M15 forbids) |

Under `html[data-eco="1"]` every `--dur-*` is `0ms` and every `--move-*` is `0px`, `--scale-*` `1`; under
`@media (prefers-reduced-motion: reduce)` every `--move-*` is `0px`, `--scale-*` `1`, and every `--dur-*`
above 140 ms becomes `140ms`.

JS, in `webapp/src/lib/motion.js` (T0): `DUR = { instant: 90, fast: 140, base: 200, sheet: 280, sheetOut: 200,
screen: 320, screenOut: 240 }`, `MOVE = { s: 4, m: 8 }`, `SCALE = { pop: 0.96, dialog: 0.97 }`,
`EASE_CSS = { enter, exit, standard }` (the strings above) and `EASE = { enter, exit, standard }` as `t → t`
functions built by `bezier(x1, y1, x2, y2)`: the classic UnitBezier solver (WebKit `UnitBezier.h`, the same
algorithm as the `bezier-easing` package: 8 Newton–Raphson iterations on x with an epsilon of 1e-6, falling
back to 20 bisection steps). The two tables must agree; `test/motion.test.mjs` parses `app.css` and checks it.

## The central guard

**CSS half (T0, `app.css`, next to the existing `html[data-eco="1"]` glass block).** It stops every CSS
transition and keyframe animation, including the ones written with literal durations the token sweep has not
reached yet:

```css
html[data-eco="1"] :not([data-motion="status"]),
html[data-eco="1"] :not([data-motion="status"])::before,
html[data-eco="1"] :not([data-motion="status"])::after {
  transition-duration: 0.01ms !important;
  transition-delay: 0s !important;
  animation-duration: 0.01ms !important;
  animation-delay: 0s !important;
  animation-iteration-count: 1 !important;
  scroll-behavior: auto !important;
}
```

`0.01ms`, not `0s`: a `0s` transition does not run at all, so it fires no `transitionend` — a future listener
would hang. Measured cost: 0.4 ms of style the moment eco turns on with a windowed list, 60 ms with 8 000
plain nodes (once, on the toggle). The same file's `@media (prefers-reduced-motion: reduce)` block already
neutralises `.fade-in`, `.btn`, `.chip`, `.card`, `.pill`; it gains the token overrides above, and `main {
scroll-behavior: auto }` already lives there.

**JS half (T0, `lib/motion.js`).** Svelte transitions are WAAPI and ignore CSS:

- `export const motionLevel` — a readable store: `"off"` when `ecoMode` is true, else `"reduced"` when
  `matchMedia("(prefers-reduced-motion: reduce)")` matches (listened to live, `change` event), else `"full"`.
  `export function level()` reads it synchronously (`get`).
- Built by `export function createMotion({ eco, reduceMq })` so `node --test` can inject a writable and a fake
  MediaQueryList; the module's default export wires `stores.js#ecoMode` and `window.matchMedia`, and is
  SSR/Node-safe (`typeof window === "undefined"` → `"full"`).
- Every transition below reads `level()` **when it starts** (Svelte calls the function once per run): `"off"`
  → `{ duration: 0 }` (Svelte's instant path); `"reduced"` → the same function with every translate/scale
  removed and `duration = Math.min(duration, DUR.fast)`; `"full"` → as specified.
- `export function scrollBehavior()` → `"smooth"` only when `level() === "full"`, else `"auto"`; every
  `scrollTo({ behavior })` / `scrollIntoView` in the app uses it (`VirtualList.scrollToIndex`,
  `lib/scroll.js#followScroll`, `TranscriptPanel`).
- `export function animate(el, keyframes, opts)` — the one wrapper for imperative WAAPI (page enter, row
  shift): returns `null` and does nothing when `level() === "off"`; in `"reduced"` drops the keyframes'
  `transform` and caps the duration; otherwise `el.animate(keyframes, { fill: "none", ...opts })`. It cancels
  any previous animation it started on the same element (a `WeakMap`), which is the interruption rule.

## `lib/motion.js` — the transitions (T0)

All have the Svelte signature `(node, params) => config | (({direction}) => config)`, use `css` only (never
`tick`: a main-thread loop), and default to the values in the table. Bidirectional use (`transition:`) is the
default for anything that can be toggled; `in:` alone where M7 applies.

| Name | What it animates | Enter | Exit | Params |
|---|---|---|---|---|
| `fadeIn` | opacity 0→1 | `DUR.fast`, `EASE.enter` | (use `in:` only) | `duration` |
| `fade` | opacity | `DUR.base`, enter | `DUR.fast`, exit | `duration`, `out` |
| `rise` | opacity + `translateY(y → 0)` | `DUR.base`, enter | `DUR.fast`, exit | `y` (default `MOVE.m`; negative = from above) |
| `swap` | opacity + `translateY(MOVE.s → 0)` | `DUR.fast`, enter | (in only) | `y`, `scale` (P13's glyph wants `scale: 0.85` instead of the translate) |
| `pop` | opacity + `scale(SCALE.pop → 1)` + `translateY(±MOVE.s)` toward the anchor | `DUR.base`, enter | `DUR.fast`, exit | `origin` (CSS transform-origin, e.g. `"top right"`), `placement` (`"down"`/`"up"`) |
| `dialog` | opacity + `scale(SCALE.dialog → 1)` | `DUR.base`, enter | `DUR.fast`, exit | — |
| `sheet` | phone (`viewport.js#isMobile()`): `translateY(100% → 0)`, NO opacity; desktop: same as `dialog` | phone `DUR.sheet` / desktop `DUR.base`, enter | phone `DUR.sheetOut` / desktop `DUR.fast`, exit | `fromY` (px, a dragged start position) |
| `veil` | `background-color` alpha 0 → the element's computed alpha (read once via `getComputedStyle(node).backgroundColor`) | same duration as the sheet it serves (`for: "sheet"\|"dialog"\|"menu"`) | idem | `for` |
| `panel` | opacity + `translateX(16px → 0)` (desktop side column) | `DUR.base`, enter | `DUR.fast`, exit | `x` |
| `screen` | phone: `translateY(100% → 0)`; desktop: opacity + `scale(0.98 → 1)` | `DUR.screen` / `DUR.base` | `DUR.screenOut` / `DUR.fast` | `fromY` |
| `collapse` | `height` + opacity (Svelte `slide` semantics, M10) | `DUR.base`, standard | `DUR.fast`, standard | `axis` (y only) |
| `flipSmall` | Svelte `flip` with `duration: DUR.base`, `EASE.standard` | — | — | for keyed lists ≤ 50 items only (M4) |

**What T0 decided where the table left a choice** (all in `lib/motion.js`):

- `sheet`/`screen` on a PHONE are instant under reduced motion (`duration: 0`) rather than faded: their
  own rule is "no opacity" (M8, the pane is glass), so removing the slide leaves nothing to play — the
  sheet appears. The desktop shapes keep their fade, capped at 140 ms.
- `collapse` under reduced motion drops the `height` walk and keeps a 140 ms fade: a block growing
  pushes everything below it, which is displacement.
- `shiftRows` and `flipSmall` do nothing at all when the level is not `"full"`: they are pure
  displacement, and `animate()` would otherwise be asked to play empty keyframes.
- `veil` builds its fade with `color-mix(in srgb, <the computed colour> t%, transparent)` rather than
  parsing the colour: it lands exactly on the tint the element's own sheet asked for, whatever
  serialisation the browser produces (`rgba(…)`, `color(srgb …)`, `oklab(…)`). Its own alpha is
  preserved at `t = 1`. `params.color` overrides the computed read (that is how `node --test` drives it).
- `swap` takes a `scale` as well as the `y` offset, so P13's play/pause glyph can be "opacity + scale
  0.85 → 1" without editing this file (a lot does not own it).
- The `screen` transforms are written as `translateY(calc(u * 100%))`, not `translateY(100%)` scaled by
  the transition's own `t`: Svelte samples `css(t, u)` and the browser resolves the percentage against
  the element's height, which is what makes a bottom sheet slide by its own size on any phone.
- `pageEnter` and `shiftRows` go through `animate()`, so a fast second navigation cancels the first and
  reduced motion drops the movement there too (one implementation of the level, not two).


Helpers:

- `export function screenKey(path)` — the page-transition key. `"/library…"`, `"/settings…"`, `"/search…"`,
  `"/import…"` collapse to their first segment (their sub-paths are in-screen state written with
  `replace()`, e.g. `Library.svelte:111`); everything else is the full path (`/album/1` → `/album/2` is a new
  page). Unit-tested on every route in `App.svelte`.
- `export function pageEnter(node, { traversal })` — called by `App.svelte` on each `onRouteLoaded` whose
  `screenKey` differs from the last one: `traversal` (a back/forward, from `nav.js`) → opacity 0→1 over
  `DUR.fast` (nothing moves, so the restored scroll offset is not seen to slide); a push → opacity 0→1 +
  `translateY(MOVE.m → 0)` over `DUR.base`, `EASE.enter`. Through `animate()`, so a fast second navigation
  cancels the first.
- `export function shiftRows(container, fromIndex, deltaRows, rowH, startIndex)` — for fixed-height windowed
  lists: every currently mounted row (`container.children`) whose absolute index (`startIndex + i`) is ≥
  `fromIndex` gets `animate(row, [{ transform: translateY(-deltaRows*rowH px) }, { transform: none }],
  { duration: DUR.base, easing: EASE_CSS.standard })`. No `getBoundingClientRect`, no style read. Call it
  right AFTER the store mutation, in the same task, after `await tick()` (so the rows are the new ones).
- `lib/portal.js#toBody(node)` — the existing nine local copies' behaviour (append to `document.body`,
  remove on destroy), exported once. Only the files a lot already edits switch to it (no drive-by refactor).
- `stores.js#immersiveShown` (T3) — `writable(false)`: true from the full-screen player's `introend` to its
  `outrostart`; `GlobalBackdrop` hides on it instead of on `immersiveOpen` (M12).

## Inventory — every element, its transition, and its trap

**Defaults that apply to every row unless the row overrides them:** eco = instant (duration 0, CSS guard);
reduced = opacity only, ≤ 140 ms; interruption = the bidirectional transition reverses from where it is, a
WAAPI one is cancelled by the next; focus = unchanged by the motion (a dialog focuses its first control on
mount, not at `introend`; the trigger gets the focus back on close at once, the outro is `inert`). "—" in a
duration cell = no animation, by rule.

### S — The shell and navigation (lot T1)

| # | Element (file) | Trigger | Transition | Enter | Exit | Trap / note |
|---|---|---|---|---|---|---|
| S1 | Page (`App.svelte`: wrap `<Router>` in `<div class="route-host">`, `onRouteLoaded`) | push / back / forward to another `screenKey` | `pageEnter` | push 200 rise 8 px; traversal 140 fade | — (M3) | Async routes (Réglages, Genres, Import, Nouveautés, Crédits, Récap) fire `onRouteLoaded` after the chunk arrives — animate then, not on `$location`. Same `screenKey` (Library tabs, Settings sections) → no page animation, the screen animates its own content (T6). `.route-host` is a plain block (no padding, no `display: contents`: contents has no box to animate). |
| S2 | Boot screens (`App.svelte` `.loading`, `.boot-retry`, `.rq-wait`, `.boot-banner`) | auth resolved, server back | `.loading` → content: `fadeIn` on the shell root (`.layout`); `.boot-banner` `rise` y −8 | 140 / 200 | banner 140 | The very first paint of the app must not animate (Svelte local intro: true by construction — the block has not run yet). |
| S3 | Now-Playing column (`NowPlaying.svelte` `aside.np`, `App.svelte` `.layout.np-open`) | queue button in the player bar | `panel` (replaces `transition:slide` axis x, which animated the WIDTH of a grid column every frame) | 200 | 140 | Keep `.np-open` on `.layout` until the aside's `outroend` (a local `npMounted` flag), or the outroing aside wraps to a new grid row. The main column reflows once, instantly (Spotify's right panel does the same). |
| S4 | Sidebar (`Sidebar.svelte`): active item | route change | background-color + colour, `var(--dur-instant)` | 90 | 90 | Colour only (M1 exception). |
| S5 | Sidebar folder (`openSet`, line ~234) | folder chevron | body: `collapse` if the folder holds ≤ 30 playlists, else instant + `fadeIn`; chevron rotate 180°, `--dur-base` standard | 200 | 140 | M10. |
| S6 | Sidebar « Deezer » auto playlists (`deezerOpen`) | toggle | as S5 | 200 | 140 | idem |
| S7 | Sidebar playlist rows (`{#each own/shared/auto}`) | a playlist created / deleted | none on mount (M4); a CREATED playlist (id not in the previous list) gets `fadeIn` once | 140 | — | Track "previous ids" in a Set; never animate the initial render. |
| S8 | Sidebar thumbs (`Cover`) | image load | Cover's own fade (T4) | — | — | — |
| S9 | Mobile tab bar (`MobileNav.svelte`) | route change | active colour `--dur-instant`; the active glyph `swap` | 90 / 140 | — | Its height is a contract (64 px): nothing may change size. |
| S10 | Top bar (`TopBar.svelte`): search field (hidden on `/search`) | entering/leaving `/search` | `fade` | 140 | 140 | It is sticky in `<main>`: no transform on it (sticky + transform = jitter on scroll). |
| S11 | Back button (`BackButton.svelte`) | history depth 0 ↔ > 0 | `fade` | 140 | 140 | Reserve its box (visibility) so the title does not jump — M7. |
| S12 | `main` scroll on navigation (`nav.js`) | push → top; back → restored offset | none (instant) | — | — | `main { scroll-behavior: smooth }` makes `nav.js`'s programmatic restore animate; T1 sets `scroll-behavior: auto` on `main` during a restore (or uses `scrollTo({behavior:"auto"})`), so Back lands where you were in one frame. |

### O — Overlays: menus, popovers, sheets, dialogs (lots T2a, T2b, T2c)

Pattern for every sheet/dialog (M8): the overlay keeps wrapping the pane, the **overlay** gets
`transition:veil={{ for: "sheet" }}` (no opacity), the **pane** (`.sheet`/`.dialog`) gets `transition:sheet`
(or `dialog` for a centered confirm), both inside the same `{#if}` so both run. Escape/backdrop click close at
once; the outro is `inert` (Svelte). `@starting-style`'s 160 ms tint walk in `app.css` stays (it is the
material, not the motion) but its literal `160ms ease-out` becomes `var(--dur-base) var(--ease-enter)`.

| # | Element (file) | Trigger | Transition | Enter | Exit | Trap / note |
|---|---|---|---|---|---|---|
| O1 | Context menu, desktop (`ContextMenu.svelte` `.menu`) | right-click, ⋯, Shift+F10 | `pop`, `origin` = the corner nearest the pointer (from the flip decision the component already makes) | 200 | 140 | 0 `/api` request stays 0 (`test/menu/run.mjs`, 242 checks); Escape returns focus at once. `.backdrop` (invisible) gets no transition. |
| O2 | Context menu, phone sheet (`ContextMenu.svelte` ≤ 640 px) | same | `sheet` (replaces the CSS `sheet-up` keyframes, which had no exit) + `veil` on `.backdrop` | 280 | 90 | Remove `@keyframes sheet-up`. **Landed at 280 in / 90 out** (T2a): `test/menu/run.mjs` asserts the menu is out of the DOM 180 ms after Escape, and the `{#if}` branch waits for its LONGEST transition, so the 200 ms sheet-out token cleared it late 16/16 times and 140 ms still missed once; the scrim follows the sheet's exit. |
| O3 | Context sub-menu (`openSub`) | hover/tap an item with `sub` | `collapse` (≤ 12 items) | 200 | 140 | Inside a scrolling `max-height` menu: keep `overflow` as is. |
| O4 | Quality menu (`QualityMenu.svelte` `ul.menu {placement} {align}`) | tap the badge | `pop`, origin from `placement`/`align` (`down left` → `top left`…) | 200 | 140 | Used in the player bar AND the full-screen footers (inside the phone player: a transform ancestor is fine, M8). |
| O5 | Sleep menu (`SleepButton.svelte`) | tap the moon | as O4 | 200 | 140 | idem |
| O6 | Speed menu (`SpeedButton.svelte`) | tap the chip | as O4 | 200 | 140 | idem |
| O7 | Credits sheet (`CreditsSheet.svelte`) | menu « Crédits » | `veil` + `sheet` (replaces fade 150 + scale 160) | 280 / 200 | 200 / 140 | Fixes the blur loss (M8). |
| O8 | Export sheet (`ExportSheet.svelte`), incl. its tabs | menu « Exporter » | `veil` + `sheet`; tabs as T-rows (I3) | idem | idem | idem |
| O9 | Flash warning (`FlashWarning.svelte`) | enabling a flashing world | `veil` + `dialog` | 200 | 140 | idem |
| O10 | Genre bulk sheet (`GenreBulkSheet.svelte`) | admin, album « Genres » | `veil` + `sheet` | 280 / 200 | 200 / 140 | Lazy-loaded: the first open fetches the chunk. A block that is TRUE on its first run plays **no** intro (measured: the pane mounted with `transform: none`, no Animation at all), so the component flips a local `ready` flag in `onMount` and the intro plays on that change. |
| O11 | Genre tag sheet (`GenreTagSheet.svelte`) | menu « Genre » | `veil` + `sheet` | idem | idem | — |
| O12 | Replace sheet (`ReplaceSheet.svelte`), its suggestion list | menu « Remplacer » | `veil` + `sheet`; suggestions arriving: `fadeIn` on the list block (M7) | idem | idem | No per-row intro. |
| O13 | Playlist members (`PlaylistMembersSheet.svelte`) | « Membres » | `veil` + `sheet` | idem | idem | Its list is windowed: M4. |
| O14 | Playlist picker (`PlaylistPicker.svelte`) | « Ajouter à une playlist » | `veil` + `sheet` (it had none) | idem | idem | A toast born over it anchors at the top (Toasts.svelte) — unchanged. |
| O15 | Share sheet (`ShareSheet.svelte`), incl. its tabs and preview | menu « Partager » | `veil` + `sheet`; tab panels `fadeIn` | idem | idem | The preview canvas renders once at mount — do not re-render per animation frame. |
| O16 | Folder sheet (`FolderSheet.svelte`) | « Nouveau dossier » | `veil` + `sheet` | idem | idem | — |
| O17 | Move-to-folder (`MoveToFolderSheet.svelte`) | « Déplacer » | `veil` + `sheet` | idem | idem | — |
| O18 | Invite sheet (`InviteSheet.svelte`) | admin « Inviter » | `veil` + `sheet` (mounted by `AdminUsers`' `{#if}`: the root transition plays, Svelte local rule) | idem | idem | — |
| O19 | Account picker (`AccountPicker.svelte`) | import « Choisir » | `veil` + `sheet`; add `use:toBody` (M9) | idem | idem | It is `position: fixed` inside a page today. |
| O20 | Add tracks (`AddTracksSheet.svelte`) | playlist « Ajouter des titres » | `veil` + `sheet`; add `use:toBody` (M9) | idem | idem | Its results list: M4. |
| O21 | Party sheet (`PartySheet.svelte`) | « Écoute partagée » | `veil` + `sheet`; the `pulse` keyframes = status → `data-motion="status"` only if it means "waiting for guests", else let eco stop it | idem | idem | — |
| O22 | Remote sheet (`RemoteSheet.svelte`) | « Télécommande » | `veil` + `sheet`; `spin` = status | idem | idem | — |
| O23 | Chapter sheet (`ChapterSheet.svelte`) | podcast « Chapitres » | `veil` + `sheet` (replaces `in:fade` with its own reduced check) | idem | idem | Drop its local `reduceMotion` (motion.js decides). |
| O24 | Lyric card (`LyricCard.svelte` `.lc-backdrop`, `lc-in` keyframes) | « Créer une carte » | `veil` + `sheet`; replace `lc-in` | idem | idem | `lc-spin` = status. |
| O25 | Party / Remote buttons (`PartyButton.svelte`, `RemoteButton.svelte`) | state change (active) | colour `--dur-instant`; glyph `swap` | 90 / 140 | — | — |

### P — The player (lot T3)

| # | Element (file) | Trigger | Transition | Enter | Exit | Trap / note |
|---|---|---|---|---|---|---|
| P1 | Phone full-screen player (`MobileNowPlaying.svelte` `.m`) | tap the mini card / swipe up; close button, swipe down, Escape | `screen` with `fromY: dragY` on exit (replaces `transition:fade 140`) | 320 | 240 from the dragged position | `.m` carries `style="transform:translateY(dragY)"` and a CSS `transition: transform 0.28s` for the snap-back: the Svelte animation overrides the inline transform while it runs, so the exit MUST start at `dragY` (param read at outro start) or it jumps to 0 first. Defer the visualizer canvas/GL start to `introend` (no shader compile inside the slide). No opacity on `.m` (its inner `.sheet.on-photo` panes are glass, M8). |
| P2 | Desktop full-screen player (`DesktopNowPlaying.svelte` `.d`) | the player bar's cover/expand; Escape | `screen` (desktop: opacity + scale 0.98) | 200 | 140 | Its children hold no open glass at open time; a QualityMenu open while closing is closed first (existing Escape order). |
| P3 | Ground hand-off (`GlobalBackdrop.svelte` `.gb.hidden`, `stores.js#immersiveShown`) | P1/P2 intro end / outro start | none; `class:hidden={$immersiveShown}` instead of `$immersiveOpen` (M12) | — | — | Hiding on `immersiveOpen` blanks the page's ground under the still-sliding player. Keeps "exactly ONE blurred image on screen" (webapp.md) at rest. |
| P4 | Phone player queue / lyrics views (`showQueue`, `showLyrics`, `.sheet.on-photo`) | « File · N », « Paroles » | `sheet` (phone path) replacing `fade 120` | 280 | 200 | The queue inside is windowed (M4); `scrollToIndex` on open uses `smooth: false` (already). |
| P5 | Phone player « À propos » panel reveal (`panelShown`, `reveal`) | swipe up the door | follows the finger (no transition while held), glide `var(--dur-base) var(--ease-standard)` on release | 200 | 200 | Existing gesture; only tokenise the literal. |
| P6 | Phone player cover carousel (`slots`) | swipe / skip | existing scroll-snap (native momentum) | — | — | Do not add a transform animation: the browser owns it. |
| P7 | Track change in both players: title/artist block | `trackKey($current)` changes | `{#key trackKey($current)}` + `swap` on the text block | 140 | — | Key on the STRING (`currentId`), never `$current` (it re-emits on every progress tick — webapp.md). |
| P8 | Desktop player cover crossfade (`.cover-fade`, `{#key trackKey}`) | track change | keep in/out fade, durations → `DUR.base`/`DUR.fast` through `fade` | 200 | 140 | Absolutely positioned layers: an outro is fine here (no flow). Already eco-gated by `{#if !$ecoMode}` — keep. |
| P9 | Backdrop layers (`$bg` each, `bg-in` keyframes) | cover change | unchanged (lib/backdrop.js owns the 600 ms walk; webapp.md) | — | — | Not this plan's to retime; eco already removes it. |
| P10 | Live lyric line (`{#key $currentLyricLine}` `in:fade 220`) | next line | `swap` (140) | 140 | — | Already `!$ecoMode`. |
| P11 | Desktop player tabs (`tab === "queue" / "more" / lyrics`) | tab click | panel `fadeIn`; indicator as I3 | 140 | — | The queue tab's list is windowed. |
| P12 | Player bar (`PlayerBar.svelte`) title/artist | track change | as P7 | 140 | — | `footer.player` contract (bench selectors) unchanged. |
| P13 | Player bar play/pause glyph (`.pp`) | toggle | glyph `swap` (opacity + scale 0.85 → 1 instead of translate, `--dur-fast`) | 140 | — | The press must repaint in the same frame (the state flips first, M2). `.busy` spinner = status. |
| P14 | Player bar shuffle / repeat / favourite `.on` | toggle | colour `--dur-instant`; favourite heart `swap` | 90 / 140 | — | — |
| P15 | Player bar empty ↔ track (`{#if $current}`) | first play | `fadeIn` on `.now` | 140 | — | — |
| P16 | Player bar volume, seek, time, podcast progress | drag / playback | **none** (M6) | — | — | — |
| P17 | `NowPlayingMore.svelte` sections (credits, artist, top, related, similar) arriving | fetch resolves | `fadeIn` per section block (M7) | 140 | — | No per-card intro; `similar === null` → content swap is M7. |
| P18 | `Lyrics.svelte` follow-scroll | line change | `followScroll` with `scrollBehavior()` | — | — | Reduced/eco: instant jump. |
| P19 | `TranscriptPanel.svelte`: its `.menu` | ⋯ | `pop` | 200 | 140 | — |
| P20 | `TranscriptPanel.svelte`: hits/segments arriving, `slide`/`pulse`/`flash` keyframes | job progress | blocks `fadeIn`; `slide` (indeterminate bar) and the job `pulse` = status; `flash` (a found segment highlight) stays, eco stops it | 140 | — | `scrollTo` uses `scrollBehavior()` (drop its own `reduce` check). |
| P21 | `TrackCredits.svelte`, `ChapterTicks.svelte` | data arrives | `fadeIn` on the block | 140 | — | Ticks are positioned on the seek bar: no transform on them. |
| P22 | `Visualizer.svelte` | — | unchanged (animations.md); `grain-drift` keyframes: eco has no canvas at all already | — | — | — |
| P23 | Remote banner/chip `class:top={$immersiveOpen}` (`RemoteBanner`, `RemoteHostChip` — files in T5) | player opens | position change top ↔ bottom: `fade` out/in rather than a jump (T5 owns the files) | 140 | 140 | — |

### L — Lists, rows, cards, images (lot T4)

| # | Element (file) | Trigger | Transition | Enter | Exit | Trap / note |
|---|---|---|---|---|---|---|
| L1 | Windowed rows (`VirtualList.svelte`, `TrackList.svelte`) | scroll | **none** (M4) | — | — | Rows remount on scroll; an intro would replay. |
| L2 | `VirtualList` API: `export function shift(fromIndex, deltaRows)` → `shiftRows(winEl, fromIndex, deltaRows, rowH, start)` | caller after a mutation | row shift | 200 standard | — | The window is keyed `trackKey:index`, so after a removal every following row REMOUNTS with a new key: animating "the same element" is impossible; the shift animates the NEW rows from their old offset, which is visually identical (fixed height). |
| L3 | Queue: remove a row (`QueueList.svelte`) | ✕ on an upcoming row, menu « Retirer » | removed row: none (it is gone at once, M2); rows after it: `vl.shift(index, -1)`… i.e. they start one row LOWER and glide up | 200 | — | 4 000-track queue: only the ~40 mounted rows animate (measured 2 layouts, worst frame 16.7 ms). |
| L4 | Queue: insert (« Lire ensuite », « Ajouter à la file » while the queue is visible) | menu / swipe | rows after the insertion: `shift(at, +1)` (start one row higher, glide down); the new row `fadeIn` once (only that key, tracked) | 200 / 140 | — | Only when the insertion point is mounted. |
| L5 | Queue: drag to reorder | grip drag | rows between `from` and `to` translate ±rowH with `transition: transform var(--dur-base) var(--ease-standard)` (CSS, classes on mounted rows) so the list MAKES ROOM instead of today's before/after line; the floating copy follows the finger with no transition; on drop the transforms clear in the same frame as `player.move` | 200 | — | `indexAt` geometry unchanged; `autoScroll` unchanged; `test/queue/run.mjs` (4 000 rows, touch) must stay green. The dragged row's placeholder keeps its slot (opacity .35). |
| L6 | Queue « Vider la file » | button | the store clears at once (M2), the rows go in the same frame (no row outro, M4/M7), and the empty state / the remaining "now playing" row block gets `fadeIn` | 140 | — | Never delay `player.clearUpcoming()` behind a fade. |
| L7 | Track row (`TrackRow.svelte`): hover, `.active` (playing), selection | pointer / playback | colour `--dur-instant`; the playing `eq` bars = decorative (eco stops); `rowspin` (loading this row) = status | 90 | 90 | No transform on the row root (the swipe owns transform). |
| L8 | Track row swipe (`.swipe .track/.action`) | touch drag | finger 1:1, release glide `var(--dur-base) var(--ease-enter)` (replaces the literal 0.22 s) | 200 | — | Already reduced-gated; keep. |
| L9 | Track row download / source state badges | download progress / done | badge `swap` on state change; progress ring = status | 140 | — | — |
| L10 | Editable playlist (`PlaylistTracks.svelte`, `animate:flip`) | drag reorder | `flipSmall` only when the list is NOT windowed and ≤ 50 rows; otherwise L5's make-room pattern | 200 | — | Today `flipMs` on a possibly long list: check `tracks.length` and switch. |
| L11 | `TrackBrowser.svelte` | filter / sort | content swap = M7 `fadeIn` on the list container once per committed change (not per keystroke) | 140 | — | — |
| L12 | Cards (`Card`, `ArtistTile`, `ReleaseCard`, `PodcastCard`): hover lift, press | pointer | press `scale(0.98)` on `:active`, `--dur-instant`; hover existing, tokenised | 90 | 90 | No intro on grid tiles (grow.js grids, M4). `PodcastCard` `spin` = status. |
| L13 | Cover image (`Cover.svelte` `img.loaded`) | full-size art decoded | opacity, `var(--dur-base) var(--ease-enter)` (was 0.35 s ease) | 200 | — | Already-cached art is marked loaded before paint (no fade) — keep that path. Eco: instant (guard). |
| L14 | `ArtistLine.svelte` | — | none | — | — | Text only. |
| L15 | `lib/scroll.js#followScroll`, `VirtualList.scrollToIndex` | follow the playing row | `scrollBehavior()` | — | — | — |

### F — Feedback: toasts, notices, network, loading (lot T5)

| # | Element (file) | Trigger | Transition | Enter | Exit | Trap / note |
|---|---|---|---|---|---|---|
| F1 | Toasts (`Toasts.svelte`) | `toasts.push` / dismiss / timeout | `rise` (y +8 from below, −8 when `anchorTop`) + `flipSmall` on the stack | 200 | 140 | Drop the local `reduce` matchMedia. ≤ 5 toasts: flip allowed. |
| F2 | Notices (`Notices.svelte`) | persistent notice added / dismissed | `rise` y −8 + `flipSmall` | 200 | 140 | Sticky under the top bar: transform on the notice, not on `.notices`. |
| F3 | Network pill (`NetworkIndicator.svelte`) | offline / reconnected | `rise` y −8 | 200 | 140 | Drop its read-once `reduced` (motion.js is live). |
| F4 | Remote banner (`RemoteBanner.svelte`) `.pill`, `.ended` | remote session | `.pill` `rise`, `.ended` `veil`+`dialog`; `pulse`/`blink` = status (it says "connected/live") | 200 | 140 | Position switch on `class:top` (P23): `fade` swap. |
| F5 | Remote host chip (`RemoteHostChip.svelte`) | lent player | `rise`; `pulse` = status | 200 | 140 | — |
| F6 | `LoadFailed.svelte` | a screen's load fails | `fadeIn` + `swap`-like 4 px rise on the block | 140 | — | « Réessayer » → the spinner appears on the button at once (M2). |
| F7 | `Skeleton.svelte` shimmer | loading | shimmer is decorative: eco and reduced → static `--s2` block (guard + `@media` in the file); content replacing it = M7 (the screen's `{:else}` block gets `fadeIn`) | 140 | — | No skeleton outro. |
| F8 | `SourceBadge.svelte` | source known | `swap` on change | 140 | — | — |

### I — In-screen state (lots T6a–T6f)

Generic patterns every T6 lot applies to its files:

- **I1 — Skeleton/LoadFailed → content**: the content branch of `{#if loading}…{:else if loadError}…{:else}`
  gets `in:fadeIn` on its root element. Svelte's local rule makes it play only when the data arrives AFTER
  mount (a cache hit at mount does not animate — right).
- **I2 — Tab panels** (`role="tablist"`): the panel root gets `in:fadeIn` (140, opacity only, no translate:
  panels differ in height, M7). A panel the previous tab ALREADY showed (a shelf that « Tout » shows and that
  its own tab shows too) does not re-enter: nothing was replaced (M7). The panel must be its own `{#if}` at
  the level that already ran — see the `{#key}` note above.
- **I3 — Tab indicator**: one absolutely positioned underline/pill per tablist moved by `transform:
  translateX() scaleX()`, `transition: transform var(--dur-base) var(--ease-standard)`; if the component
  styles the active tab by a border instead, colour transition `--dur-instant` is acceptable.
- **I4 — Accordions** (`<details>`, `aria-expanded`): `collapse` if M10 allows, else instant + `fadeIn`;
  chevron rotation `--dur-base`.
- **I5 — Switches/checkboxes**: thumb `transform` `--dur-instant`, track colour `--dur-instant`.
- **I6 — Sections appearing after a fetch** (shelves, cards blocks, "Titres recommandés"): `fadeIn` on the
  section, never per card (M11).
- **I7 — Inline confirmations / error lines / « Enregistré » ticks**: `fadeIn`; removal instant.
- **I8 — Progress bars of jobs**: width → `transform: scaleX()` with `transition: transform var(--dur-base)
  linear`; indeterminate bars = status.
- **I9 — Spinners** (`spin`, `ytspin`, `radio-spin`, `lc-spin`, `rs-spin`…): add `data-motion="status"`.

| # | Lot | Files | Specific items (beyond I1–I9) | Trap / note |
|---|---|---|---|---|
| I10 | T6a | `routes/Home.svelte`, `HomeHero`, `ResumeCard`, `RecapHomeCard`, `MoodCard`, `routes/Search.svelte`, `TopResult`, `TranscriptHits` | Home shelves I6; HomeHero carousel/suggestion swap `fadeIn`; Search: results area appears once (empty → results) `fadeIn`, **not per keystroke** (M6); search tabs I2/I3; the query-radio offer `rise`; `radio-spin`/`ytspin` I9; TopResult card `fadeIn` when it changes identity (keyed by entity ref) | Search keeps `recallScreen` state on Back: restored results must NOT animate (they render at mount → local rule). |
| I11 | T6b | `routes/Library.svelte`, `LibraryNav`, `History`, `Playlist`, `Album`, `Artist`, `PersonCredits`, `routes/Credits.svelte` | LibraryNav segmented control + leaf strip indicator I3 (it already has `transform 0.18s`: tokenise); Library tab content I2 (`replace()` route → same screenKey → no page fade, this I2 is the only motion); playlist edit mode / selection bar `rise` from the bottom; folder groups I4; Playlist « Titres recommandés » I6; Artist tabs I2/I3; History day groups none (long list, M4) | Library tabs are written with `replace()`: verify no double animation (S1 skips, I2 plays). |
| I12 | T6c | `routes/Podcasts.svelte`, `Show`, `PodcastEpisodePanel`, `ChapterList`, `Releases`, `ReleaseShelf`, `Import`, `SharedPlaylist`, `GradientHeader` | Episode panel tabs I2/I3 and expand I4; ChapterList `ch-shimmer` = decorative skeleton (F7 rule); Releases two shelves I6; Import job progress I8, its `slide` keyframes = status; SharedPlaylist (outside the router: its own screen) root `fadeIn`; GradientHeader's 600 ms colour walk unchanged (webapp.md) | Show's 1 921 lines: touch only the listed blocks. |
| I13 | T6d | `routes/Settings.svelte` + `AccessSettings`, `AppearanceSettings`, `AudioEffects`, `MyAccounts`, `ServicesSettings`, `SourceOrder`, `TranscribeSettings`, `VisualizerSettings`, `WorldPicker`, `AdminUsers`, `CreditsBackfill`, `EnrichBackfill`, `SimilarRebuild`, `YoutubeConnect`, `HardReload`, `EcoToggle`, `TrimControl`, `DislikedTracks`, `UnavailableList`, `AutomatchPanel`, `JobFailures` | Settings category list → section: same screenKey; the section content `fadeIn` keyed on the section param; phone two-level (list ↔ section) `rise`; `bar-slide` keyframes = status; AdminUsers `.overlay`+`.dialog` (detail, confirm) → `veil` + `dialog`; switches I5; job cards I8; AutomatchPanel `transition:slide` → `collapse`; SourceOrder reorder ≤ 10 items → `flipSmall`; EcoToggle: the switch itself animates its thumb only when turning eco OFF (turning it on is already eco) | Settings carries the animation catalogue chunk — no new import. |
| I14 | T6e | `routes/Genres.svelte`, `GenreTree`, `GenreLibrary`, `GenreExtractor`, `GenreTiles`, `ExplorerStyles`, `StylePicker`, `DiscoverySlider`, `EnergyPicker`, `FlowPicker`, `FlowControls`, `FlowUpNext`, `WhoPicker` | Genres tabs I2/I3; GenreTree node expand I4 (tree may be big: `collapse` only for ≤ 30 children) and its dnd (svelte-dnd-action has its own flip: set `flipDurationMs` from `DUR.base`, 0 when `level() !== "full"`); ExplorerStyles `.veil` discovery/station sheets → `veil` + `sheet` (already `toBody`); FlowControls `toBody` popover → `pop`; FlowUpNext swap `swap`; pickers' selected chip colour I5 | `ext-slide`/`slide` keyframes = status. |
| I15 | T6f | `routes/Onboarding.svelte`, `OnboardingAccount`, `OnboardingArtistCard`, `Login`, `Invite`, `Recap`, `RecapStory`, `RecapSlide`, `RecapShare`, `Party`, `RemoteClaim`, `Viz` | Onboarding step change: `rise` keyed on the step (push direction) / fade on back; artist card select `swap` on the check; Login error line I7; Recap story slide change: opacity crossfade `DUR.base` (two absolutely positioned slides, so an outro is allowed), `recap-fill` timer bar = status (it counts down a JS timer), `recap-shimmer` = skeleton; RecapShare `.rs-backdrop` → `veil` + `sheet`; Party guest: background layer stays (shared store), latency chip / resume button `fade` (tokenised); RemoteClaim `spin` = status; Viz (projector) has no UI motion beyond its canvas | Party/Invite/Login render OUTSIDE the router: no S1 there; their roots `fadeIn` once auth resolves. |

**What T6f decided where the row left a choice** (all in the files above):

- **The story's card change is an intro, not a crossfade — and that is M7, not a shortcut.** The bridge
  mounts the active card and the next one (webapp.md: not a third, its sleeves are a measured budget of
  14), so the card being LEFT is already out of the `{#each}` when the class changes: an `out:fade` on
  `.slide-wrap` keeps it painted, but the element then carries `.active` for the length of its outro and
  `test/recap/run.mjs` requires exactly ONE `.slide-wrap.active` (it read two and stopped). A third
  mounted slide would break the sleeve budget instead. So the entering card fades in over
  `DUR.base` (200 ms, opacity only, absolutely positioned: no layout) and the leaving one goes in the same
  frame — M7's "replace in place = intro only", which is also what a 7 s auto-advance wants.
- **`RecapShare` is portalled** (`use:toBody`): it is `position: fixed` and rendered from a page, and T1's
  page-enter transform would otherwise become its containing block (M9) for the 200 ms it runs.
- **The Récap's 7 s bar is a status indicator with the JS clock's own duration**: `--fill-ms` is set from
  `AUTO_MS` (7000) by the component, the animation is a `scaleX()` (M1), and the mark is
  `data-motion="status"` only while it runs (M13). Same for `spin`/`mini-spin`/`pulse` on the party screen.
- **Onboarding's step transition reads its direction from the action** (`stepBack` set by the back button,
  cleared by the two places that push the account step): `rise` with `y: 0` IS the plan's "fade on back" —
  an opacity walk with an identity transform, which the level's own reduced variant drops entirely.
- **The rail and the import bar became transforms**: both were `width` transitions (M1 forbids width), and
  both now ride `translate/scaleX` on the token durations.
- **The motion bench's per-level request comparison and the Récap do not agree yet.** `scenarios/misc.mjs`
  walks the deck to its end once, pauses the phone's 7 s auto-advance through the app's own focus path, and
  stands on the second-to-last card so the act steps onto the text-only share card. Even so, 1–5 cover
  requests (`/api/cover/<id>` and the `d41d8cd…` placeholder) land inside the act window at 1440 in
  REDUCED and ECO and not at full motion: `Cover` (T4, still a literal 350 ms fade) re-requests art when a
  card activates or the screen re-renders, and the queue admits it a frame or two later — the same traffic
  is what the runner's eco "nothing animates" check sees as a 0.01 ms remnant. The two scenarios' own
  assertions (one `.active`, a 200 ms opacity transition mid-fade, no transform; a veil on
  `background-color` and a sheet, both 0 ms in eco and ≤ 140 ms reduced) pass in all six level-runs, 127
  checks of 132. **T4 or T7 closes it** by tokenising `Cover`'s fade and settling its requests.
- **`test/motion.test.mjs`'s import allowlist is a SUBSET check**, not the exact set of 19: every lot of
  group A converts some of those files in its own PR, on files that are disjoint between lots, so an
  equality turned each of the thirteen PRs red for a line outside its own column (`routes/Party.svelte` was
  this lot's). What still holds at every point is that no file is ADDED to the list; T7 deletes it.

### N — Never animated (by rule)

Seek, volume and EQ fader drags; the playback clock and every live number (BPM, levels, meter); progress fed
in real time; the visualizer canvases (they are their own motion, animations.md); search results per
keystroke; scroll position (except `followScroll` with `scrollBehavior()`); rows mounting from scrolling; the
first paint of the app; the theme switch (`--cover-ac` 600 ms walk is animations/webapp territory, already
reduced-gated, and the guard stops it in eco); hover tooltips opened while another is open (Linear rule).
`Icon.svelte`, `Avatar.svelte`, `Player.svelte` (the audio owner) get no change.

## Lots

Every lot: branch from `master` as a sibling of the session branch; read this file + the area notes of the
files it touches; one writer per file (the *Files* column is exhaustive — a file not listed is not yours);
`cd webapp && npm test && npm run build`; the benches named in its row; screenshots (desktop 1440×900, phone
390×844, the full-screen player's narrow side panel) mid-animation and at rest; the numbers go in the PR and
in the *Status per lot* table below.

| Lot | Depends on | Parallel group | Files (exclusive) | Done when |
|---|---|---|---|---|
| **T0** foundation | — | alone, first | NEW `webapp/src/lib/motion.js`, NEW `webapp/src/lib/portal.js`, `webapp/src/app.css` (tokens, guard, reduced overrides, `@starting-style` and `.fade-in` tokenised), NEW `webapp/test/motion.test.mjs`, NEW `webapp/test/motion/run.mjs` + `webapp/test/motion/scenarios/` (empty dir with a README line), `docs/agents/commands.md` (bench line), this file | unit tests: bezier matches CSS (`EASE.enter(0.5)` within 1e-3 of the solved value), tokens CSS = JS, `screenKey` on every route, `createMotion` levels (eco wins over reduced; live `change`), every transition returns duration 0 when off and no `transform` when reduced; lint test with an allowlist of today's 19 files importing `svelte/transition`/`svelte/animate`; the eco guard verified in Chromium (`getAnimations()` empty after toggling a `.btn` hover in eco) |
| **T1** shell | T0 | A | `App.svelte`, `Sidebar.svelte`, `MobileNav.svelte`, `TopBar.svelte`, `BackButton.svelte`, `NowPlaying.svelte`, `lib/nav.js` (export `lastNavWasTraversal()`), `test/motion/scenarios/shell.mjs` | S1–S12; `test/clicks/run.mjs` nav INP unchanged ±10 %; `test/mobile/run.mjs`, `test/backdrop/run.mjs` green |
| **T2a** menus | T0 | A | `ContextMenu`, `QualityMenu`, `SleepButton`, `SpeedButton`, `scenarios/menus.mjs` | O1–O6; `test/menu/run.mjs` 242 checks green, 0 `/api` per open |
| **T2b** sheets 1 | T0 | A | `CreditsSheet`, `ExportSheet`, `FlashWarning`, `GenreBulkSheet`, `GenreTagSheet`, `ReplaceSheet`, `PlaylistMembersSheet`, `PlaylistPicker`, `ShareSheet`, `scenarios/sheets1.mjs` | O7–O15; glass check: pane std-dev at 50 % of the enter ≤ 10 (blur kept) |
| **T2c** sheets 2 | T0 | A | `FolderSheet`, `MoveToFolderSheet`, `InviteSheet`, `AccountPicker`, `AddTracksSheet`, `PartySheet`, `RemoteSheet`, `ChapterSheet`, `LyricCard`, `PartyButton`, `RemoteButton`, `scenarios/sheets2.mjs` | O16–O25; `test/party/run.mjs` and `test/remote.test.mjs` (in `npm test`) green |
| **T3** player | T0 | A | `ImmersivePlayer`, `MobileNowPlaying`, `DesktopNowPlaying`, `PlayerBar`, `NowPlayingMore`, `Lyrics`, `TranscriptPanel`, `GlobalBackdrop`, `Visualizer`, `TrackCredits`, `ChapterTicks`, `lib/stores.js` (`immersiveShown` only), `scenarios/player.mjs` | P1–P22; `test/clicks/run.mjs` open/close player INP ±10 %; `test/fullscreen/run.mjs`, `test/backdrop/run.mjs` (1 blurred layer at rest), `test/mobile/run.mjs`, `test/panel/run.mjs` green |
| **T4** lists | T0 | A | `VirtualList`, `TrackList`, `QueueList`, `TrackRow`, `PlaylistTracks`, `TrackBrowser`, `Card`, `ArtistTile`, `ReleaseCard`, `PodcastCard`, `Cover`, `ArtistLine`, `lib/scroll.js`, `scenarios/lists.mjs` | L1–L15; `test/queue/run.mjs` (+ `--phone`) and `test/ui/run.mjs` (8 000 tracks) unchanged; remove on a 4 000 queue: ≤ 3 layouts during the shift |
| **T5** feedback | T0 | A | `Toasts`, `Notices`, `NetworkIndicator`, `RemoteBanner`, `RemoteHostChip`, `LoadFailed`, `Skeleton`, `SourceBadge`, `scenarios/feedback.mjs` | F1–F8, P23; `test/failure/run.mjs` green |
| **T6a** home & search | T0 | A | see I10, `scenarios/home-search.mjs` | I10; `test/clicks/run.mjs` search typing INP unchanged |
| **T6b** library & pages | T0 | A | see I11, `scenarios/library.mjs` | I11; `test/ui/run.mjs` unchanged |
| **T6c** podcasts & releases | T0 | A | see I12, `scenarios/podcasts.mjs` | I12; `test/podcast/run.mjs`, `test/import/run.mjs` green |
| **T6d** settings | T0 | A | see I13, `scenarios/settings.mjs` | I13; `test/admin/run.mjs` green |
| **T6e** genres & flow | T0 | A | see I14, `scenarios/genres.mjs` | I14; `test/genretree/run.mjs` green |
| **T6f** onboarding, recap, party | T0 | A | see I15, `scenarios/misc.mjs` | I15; `test/onboarding/run.mjs`, `test/recap/run.mjs`, `test/party/run.mjs` green |
| **T7** closing | all of A | alone, last | `test/motion.test.mjs` (empty the allowlist, add the literal-duration lint), this file (numbers, status), `docs/agents/webapp.md` + `animations.md` (one line each pointing here), any file still failing the lint | full `test/motion/run.mjs` green on both viewports × three levels; every bench named above green on the merged result |

Group A = 14 lots on disjoint files: all can run at the same time once T0 is merged. Recommended order when
fewer workers are available (most visible first): T1, T3, T2a, T2b, T5, T4, T2c, T6b, T6a, T6d, T6c, T6e,
T6f. The only shared write is `test/motion/scenarios/<lot>.mjs`, one file per lot.

## Acceptance criteria (measurable, enforced by `test/motion/run.mjs`)

The bench (T0 writes the runner, each lot its scenario file) starts `tools/perf_api.py --serve` through
`test/harness.mjs`, and runs every scenario at **1440×900** and **390×844** (`isMobile`, `hasTouch`), in three
motion levels: full; reduced (`page.emulateMedia({ reducedMotion: "reduce" })`); eco
(`localStorage.setItem("viz.eco", "true")` before boot). A scenario is `{ name, setup(page), act(page),
settle(page) }`. Checks per scenario:

1. **The click answers in < 100 ms**: Event Timing duration of the act's input (as `test/clicks/run.mjs`
   measures it) ≤ 100 ms at THROTTLE=1, and at THROTTLE=4 within +10 % of the same scenario on a build of
   the parent commit (`DIST=` points the harness at it).
2. **No frame over one refresh while it moves**: at THROTTLE=1, full level, the rAF gaps from the second frame
   after the input to the end of the enter duration: worst ≤ 17.5 ms (the prototype's transform/opacity
   cases read 16.7–16.8). The first frame carries the DOM build and is judged by (1).
3. **Compositor only**: `Performance.getMetrics` LayoutCount delta over the animation window ≤ 3 (prototype:
   2), except scenarios declared `collapse` (≤ 1 per frame).
4. **No long task** (> 50 ms) during the animation window at THROTTLE=1.
5. **Requests unchanged**: the `/api` and image requests during act+settle are the same multiset in the
   three levels and equal to the parent commit's (`page.on("request")`).
6. **Eco = nothing moves**: two rAF after the act, `document.getAnimations()` filtered to targets outside
   `[data-motion="status"]` is empty, and no element's computed `transition-duration` exceeds `0.00001s`
   (sampled on the scenario's selectors).
7. **Reduced = no displacement**: every running Animation's `effect.getKeyframes()` has no `transform`,
   `translate` or `scale` other than `none`/identity, and its duration is ≤ 140 ms.
8. **Interruption**: each overlay scenario also clicks open → close → open within 60 ms; the pane ends open,
   one instance in the DOM, and focus inside it.
9. **Pictures**: `SHOTS=<dir>` saves the frame at 50 % of the enter duration and at rest, per viewport, plus
   the full-screen player's narrow side panel; `VIDEO=1` records a Playwright video. Looked at by the lot's
   author and attached to the PR.

**What the bench costs the app, and what the app costs the bench** (learned in T1, `test/motion/scenarios/shell.mjs`):

- **Wait the offline warmer out before measuring.** `lib/actions.js#warmPlaylists` starts 10 s after
  `/me/playlists` answers and then pulls one playlist every `WARM_GAP` = 1 500 ms, up to `WARM_MAX` = 25 — about
  48 s of deliberate background traffic. A window that catches one of those requests asks for a playlist the
  other two levels did not, and check 5 fails on a request the transition never made. T1's first scenario drains
  it (the warm is over once ≥ 1 playlist has been pulled and the link has been silent for 2.5 s, capped at its
  whole budget) — 50 s once per context, then every window is clean.
- **A scenario's destination should carry no cover art of its own** while `Cover.svelte` still fades an image
  in with the literal `0.35s ease` of row L13: that fade is 350 ms in reduced motion and is measured as if the
  shell had made it. Pick screens without sleeves (Réglages, Explorer) and let the source page's own entrances
  finish in `setup`.
- **The bench waits for the shell's first-paint entrances before probing.** `run.mjs`'s `open()` used to
  read the page three frames after it answered; S2's `.layout` fade is still running then, and the
  foundation's "reduced motion starts no transition" hover probe measured it instead of the `.btn`. It now
  waits (bounded) for `__motion.running()` to empty, which is what "a quiet page" always meant.
- **The frozen import allowlist is a CEILING, not an equality.** `test/motion.test.mjs` fails when a file
  OUTSIDE its 19-name list imports `svelte/transition`/`svelte/animate`; a lot that moves one of those files
  onto `lib/motion.js` simply stops matching, and the list is emptied in T7. (An equality turns red on every
  lot of group A at once — it did, for T6b and T1.)
- **A detail page for the Back scenario is a SHORT playlist.** A 4 000-track page keeps mounting rows and
  their sleeves after any reasonable baseline, and a cover that arrives on the wrong side of the window makes
  the request check compare two different pages.

## Prompt for a coding worker (one per lot)

> Tu codes le lot **{LOT}** du plan `docs/agents/transitions.md` (Neezer, webapp Svelte 5). Lis d'abord
> `CLAUDE.md`, puis `docs/agents/transitions.md` en entier, puis `docs/agents/webapp.md` et
> `docs/agents/playback.md` pour les fichiers que tu touches. Tes fichiers — et SEULEMENT eux — sont dans la
> ligne {LOT} du tableau *Lots*. Applique chaque ligne {IDS} de l'inventaire exactement : transition de
> `lib/motion.js` (jamais `svelte/transition` direct), durées et easings par les tokens, règles M1–M15. Ne
> change aucun comportement fonctionnel ; le clic met à jour l'état d'abord, l'animation suit. Écris
> `webapp/test/motion/scenarios/{FILE}.mjs` avec un scénario par ligne de l'inventaire qui a un déclencheur
> cliquable. Vérifie : `cd webapp && npm test && npm run build`, `node test/motion/run.mjs`, les benchs de la
> colonne *Done when*, captures desktop/téléphone/panneau étroit du lecteur plein écran. Mets les nombres
> dans la PR et dans *Status per lot*. Branche sœur de ta branche de session, PR vers `master`, CI entièrement
> verte (lis les logs de `security.yaml`), fusionne.

## Status per lot

| Lot | State | PR | Measured |
|---|---|---|---|
| T0 | done | #213 | `npm test` 779/779 (12 new); `EASE.enter(0.5)` = 0.950248 against the CSS curve's 0.950247 (worst \|JS−CSS\| 1.4e-6 over 57 samples, cap 1e-3); the bench 116 checks / 0 failure in 15 s, at 1440×900 and 390×844 × full/reduced/eco: computed tokens 90/140/200/280/320 ms, 8 px/4 px, 0.96/0.97/0.98 — 0 ms, 0 px, 1 in eco, and 140 ms, 0 px, 1 under reduced; `document.getAnimations()` empty after a real `.btn` hover (and three frames later) in eco with the `.btn`'s own computed `transition-duration` 1e-05 s, against 1 × 120 ms `background-color` at full motion; an unmarked 1 s CSS animation stopped in eco (1e-05 s) while `[data-motion="status"]` keeps its 1 s and stays on the timeline; the emitted `translateY(calc(u * 100%))`, `calc(u * 120px)` and `scale(0.96)` resolve to the right matrices and a 50 % veil to alpha 0.3; 0 page error. |
| T1 | done | #222 | `npm test` 779/779 ; `node test/motion/run.mjs` **164 vérifications, 0 échec** pour les 8 scénarios de ce lot (× 1440/390 × plein/réduit/éco), et **350 vérifications / 0 échec** sur le banc complet rebasé (26 scénarios, 4 fichiers) ; `test/clicks/run.mjs` (4× CPU, médiane de 9) clic→peinture des nav : Accueil **16 → 16 ms**, Rechercher **56 → 48**, Bibliothèque **64 → 48**, Réglages **64 → 64**, Accueil(2) **48 → 48** (les onglets de bibliothèque et le plein écran varient de ±40 % d'un passage à l'autre : bruit de la machine, T1 ne touche à aucun de ces fichiers) ; `test/mobile/run.mjs` 360/390/639/640/641 tout vert ; `test/backdrop/run.mjs` tout vert. S1 : la page monte de 8 px en 200 ms sur un push, fondu seul de 140 ms en traversal, rien sur un changement d'onglet (même `screenKey`) ni au premier écran ; S3 : la colonne tient `.np-open` jusqu'à `outroend` de l'aside (sinon la colonne qui part se replie sur une nouvelle rangée de grille) ; S5/S6 : corps de dossier par `collapse` jusqu'à 30 playlists, fondu au-delà, chevron qui tourne de 90° ; S7 : seul un id absent du rendu précédent reçoit le fondu de 140 ms (les 60 lignes de `/me/playlists` ne bougent pas) ; S10/S11 : fondu de 140 ms, sans `transform` (barre collante) ; S12 : déjà instantané (`scrollTo({behavior:"instant"})` dans `lib/nav.js`), vérifié par le scénario Retour. |
| T2a | done | #225 | `npm test` 779/779; `npm run build` 586 modules / 30 s. `test/motion/run.mjs` **152 checks / 0 failure** (O1–O6 × 1440×900 and 390×844 × full/reduced/eco, 0 page error); `test/menu/run.mjs` **242/242** with every open still costing 0 `/api` request. Measured (probe in the PR): the desktop `pop` runs **200 ms** in and **140 ms** out, `transform-origin` on the pointer's own corner (`0px 591.25px` = bottom-left for a menu pushed back up; the quality list above its badge reads `208px 337.34px` = bottom-right); the phone `sheet` **280 ms** in (O2's exit is 90, see that row) and the scrim walks its own `background-color` alpha (the sheet keeps its blur); the sub-menu `collapse` 200/140 ms with the chevron on `--dur-base var(--ease-standard)`; O4–O6 `pop` 200/140 ms from `placement`/`align`. Escape → menu out of the DOM **154 ms** (desktop pop, 140 ms animation + Svelte's one-frame dummy), **105 ms** (phone sheet + scrim), **160 ms** (sub-menu). Eco: 0 animation outside `[data-motion="status"]`; reduced: no `transform`, ≤ 140 ms; requests identical in the three levels (0 added by any open, 0 by any close). |
| T2b | done | #229 | `npm test` 779/779; `npm run build` green (587 modules, 30 s); `node test/motion/run.mjs` avec ce fichier seul : **174 vérifications, 0 échec des règles de mouvement** — 8 scénarios × 1440×900 et 390×844 × plein/réduit/éco ; **2 rouges, tous deux la comparaison de requêtes** (des pochettes paresseuses de la page partagée, l'item de banc que T6a a listé pour T7), aucune requête ajoutée par une transition ; chaque feuille O7–O15 entre par `veil` + `sheet` : panneau et voile **280 ms** au téléphone / **200 ms** au bureau, sortie 200/140, en réduit le panneau ne garde qu'une opacité ≤ 140 ms (instantané au téléphone) et l'éco ne crée **aucune animation** ; **verre** : écart-type de luminance du panneau à 50 % de l'entrée **2,52** (1440), **3,59** (390), **8,13** (641, panneau latéral du lecteur) contre **20,2 / 31,4 / 44,8** pour le contrôle à opacité d'ancêtre — le flou est conservé (cap 10) ; `test/menu/run.mjs` 242/242 ; 0 erreur de page. |
| T2c | to do | — | — |
| T3 | to do | — | — |
| T4 | to do | — | — |
| T5 | to do | — | — |
| T6a | done | #226 | `test/motion/scenarios/home-search.mjs` — 7 scenarios × 2 viewports, every `Element.prototype.animate` call logged (a count, not a guess). **Alone on the bench: full motion 225 checks / 0 failure, eco 0 / 0 failure**, and the counts are the rules: an answer's `.leadwrap` + its three shelves enter (140 ms each), a SECOND term of the same visit logs **0** entrance (M6), a tab switch logs **0** for « Artistes » (already on screen under « Tout ») and **1** for « Albums » (140 ms, opacity only), another best result **1 × 140 ms**, the radio offer **200 ms with a transform** (opacity only under reduced), « Reprendre » 140 ms when the track block replaces the idle one, **0** `.rel` card entrance (M11); eco: nothing at all, `[data-motion="status"]` spinners aside. Reduced: 5 failures, **none of this lot's rows** — `Cover.svelte`'s literal `opacity .35s` `.loaded` fade (350 ms > the 140 ms cap, T4's L13) ×3 and T0's foundation check "a `.btn` hover starts no transition under reduced motion", which now sees the home's shelves arriving (it must read its own probe's animations, or wait for the app to settle) ×2. Requests: 1 `/api/search` per settled search, **0 per keystroke**; typing 5 letters at 4× CPU worst event **24 ms / 1 request** (parent build 24 ms / 1); `test/clicks/run.mjs` unchanged (Rechercher 32 ms, Accueil 16–24 ms, plein écran 200/96). `npm test` 779/779, build OK. Merged with the three other scenario files (25 scenarios × 2 viewports × 3 levels, 12 failures): every rule check of this lot still passes; 5 are the request multiset of `sections après le chargement` (0 failure when the file runs alone — the shared page's warm cover cache and the preceding file's timing, the same effect on 5 other lots' scenarios: a bench-level item for T7: one page per file, or compare requests on a cold cache). |
| T6b | done | #223 | `node test/motion/run.mjs`: 152 checks / 0 failure (6 scenarios × 2 viewports × 3 levels); full: pill 200 ms [transform], panel/podcast/group-body/tab/edit-glyph 140 ms [opacity] (glyph + 4 px), leaf underline 200 ms [transform] + 90 ms [colour], chevron 200 ms [transform]; reduced: opacity only at 140 ms, **zero** transform (the pill is PLACED, the chevron POSED, M14); eco: only the guard's 0 ms remnant, nothing moves (M13); same /api + image request multiset in the three levels; LayoutCount over act+settle 1–117, all of it the content mount (12 cards = 43, 42 artist tiles = 21, the 57-row edit list = 117) — every animated property is opacity/transform; `npm test` 779/779; `test/ui/run.mjs` medians 0/0/0 + 132 ms playlist against 0/0/0 + 139 ms on a base build (5 runs each, 4× CPU). |
| T6c | to do | — | — |
| T6d | done | #215 | `npm test` 779/779; the bench 184 checks / 0 failure (4 scenarios × 2 viewports × 3 levels): the section change is ONE animation (desktop `pane-body` 140 ms opacity; phone the pane's 200 ms `rise`), the switch 90 ms on the track's colours and the knob's transform, the admin dialog `dialog` 200 ms transform+opacity plus the veil's own background-colour 200 ms; reduced = 140 ms opacity/colour only, NO transform on the knob or the dialog, veil 140 ms; eco leaves the timeline empty after 4 frames (the click's 0.01 ms remnants swept) and the `/api` + image requests are identical in the three levels; `test/admin/run.mjs` 61 checks, `test/eq/run.mjs` 35 checks green (the fader fill is a `scaleY`, not a `height`); `test/motion.test.mjs`'s importer check became a subset, so converting a file no longer turns the 13 parallel lots red. |
| T6e | done | #221 | I14, on `routes/Genres.svelte` + `GenreTree`, `GenreLibrary`, `GenreExtractor`, `GenreTiles`, `ExplorerStyles`, `StylePicker`, `DiscoverySlider`, `EnergyPicker`, `FlowPicker`, `FlowControls`, `FlowUpNext`, `WhoPicker`. `npm test` 779/779; `npm run build` 586 modules. `node test/motion/run.mjs`: **8 scénarios × 2 viewports × 3 niveaux, 0 échec** (onglets du studio, repli d'un nœud de l'arbre, « Réglages avancés », feuille des stations en `veil`+`sheet`, pastille d'énergie, pas de découverte, puce de style, popover du Flow) — en éco `document.getAnimations()` vide sur les 8, en réduit aucun `transform` et ≤ 140 ms, mêmes requêtes dans les trois niveaux. Mesuré à 1440×900, headless, 1× : feuille des stations **3 layouts / pire frame 16,7 ms / 4–6 ms de script** sur 320 ms; bascule d'onglet vers l'arbre **5 layouts / 16,7 ms / 25 ms** (dont le montage des 21 lignes fenêtrées); le même geste en éco → **0 animation** sur la timeline après 2 rAF. `test/genretree/run.mjs` 16/16 (21/21/20 lignes montées, 0 débordement). Les durées restent celles des tokens (onglets/puces 90 ms, panneaux 140 ms, `pop` 200/140, `veil`+`sheet` 280/200 au téléphone et 200/140 au bureau); les barres de progression passent en `transform: scaleX()` (I8), la barre indéterminée de l'extracteur dit `data-motion="status"`. Deux lignes de l'inventaire ne sont pas cliquables sur le banc : la feuille de découverte (aucun style dans `tools/perf_api.py`; la feuille des stations est le même couple `veil`+`sheet`) et la porte du Flow (le proxy Deezer est éteint; `flow-controles` clique « Lancer le Flow » puis la porte si elle existe) — leur `pop`/`swap` sont tenus par `test/motion.test.mjs` aux trois niveaux. |
| T6f | done | #224 | **I15, 12 files.** `npm test` 779/779; `npm run build` green; `node test/motion/run.mjs` **127 checks of 132** — 2 scenarios × 2 viewports × 3 levels, the 5 failures all cover traffic inside the request comparison (see the note above: `Cover` is T4's); the story's card change measured at **200 ms opacity (`--dur-base`), 140 ms and opacity-only under reduced motion, 0 ms in eco**, exactly one `.slide-wrap.active`, no transform in any running animation; the share sheet's veil animates **`background-color` only** (never the overlay's opacity, M8) over 200/280 ms and its pane enters on `sheet`, 0 ms in eco, portalled to `<body>` (M9); the onboarding step rises 8 px on a push and only fades on the way back; the `width` transitions of the step rail and of the import bar became `scaleX()` (M1); `test/onboarding/run.mjs` and `test/recap/run.mjs` green. `test/party/run.mjs`: its four audio-alignment segments are load-sensitive on this machine — `steady` fails on the parent commit too (p95 +10.3 ms, 59 % within 5 ms), and the same segment read 17 %/59 % on two runs of the same code; nothing in `Party.svelte`'s change touches a clock. |
| T7 | to do | — | — |
