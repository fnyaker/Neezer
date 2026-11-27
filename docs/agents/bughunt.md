# Bughunt: the two walkers, and what a report means

The systematic bug hunt (`plan-bughunt`): two deterministic walkers that click **every element the
accessibility tree exposes**, on every screen, and write one report per application. It answers the
question the Monkey crawler cannot — *what did THIS click do?* — including the clicks that do
nothing. It is a map of interactions, not a pass/fail gate: never on `pull_request`.

| | Web (B1) | Native (B2) |
|---|---|---|
| Entry point | `webapp/test/bughunt/run.mjs` | `mobile/tools/bughunt/explore.sh` |
| Runs on | any machine with Chromium (`/dev/kvm` not needed) | GitHub Actions only (the `native` job of `bughunt.yaml`) |
| Report | `webapp/test/bughunt/out/bughunt-web.{json,md}` + `screens/` | `mobile/tools/bughunt/out/bughunt-native.{json,md}` + `screens/` |
| Workflow | `.github/workflows/bughunt.yaml`, `web` job | same file, `native` job (skips until B2 lands) |

Neither output is committed (`/webapp/test/bughunt/out/` and `/mobile/tools/bughunt/out/` are
gitignored): every run regenerates it, and the first run of each lot is attached to its task.

## Commands

```sh
cd webapp && npm run build && node test/bughunt/run.mjs            # the whole pass (~20 min, 32 screens)
cd webapp && node test/bughunt/run.mjs --only '#/settings/apparence' --out /tmp/bh
cd webapp && node test/bughunt/run.mjs --max-minutes 5 --max-clicks 100   # smoke
cd webapp && node test/bughunt/stress.mjs                          # the three scenarios alone
python -m unittest tests.test_agent_docs                           # the notes stay reachable
```

Exit codes: `0` no `bloquant` bug, `1` at least one `bloquant`, `2` a tool error (an undeclared
route, a missing id, a broken setup). The pass is **not** in `npm test`: it needs Chromium and a
live bench server, like the other benches of `webapp/test/*/run.mjs`.

The bench is `tools/perf_api.py --serve` with `--podcasts --recap --covers --audio`: 8 000 titles, 60
playlists, a 4 000-track playlist, Deezer off. `login` is `bench` / `Bench1` (`webapp/test/harness.mjs`).
The pass logs in, **reloads** (a hash-only navigation never re-boots the SPA — the shell's `/api/me`
would keep the pre-cookie 401), injects a 12-track queue from the bench playlist, then walks.

## The contract (plan §4) — one vocabulary for both apps

### Outcome classes, in priority order (the first that applies wins)

| # | Class | Web definition |
|---|---|---|
| 1 | `exception` | an uncaught `pageerror` during the click ± 2 s |
| 2 | `blank` | route unchanged **and** `main.innerText` < 20 chars **and** `.route-host` < 3 nodes |
| 3 | `freeze` | a `longtask` > 1 000 ms in the observation window |
| 4 | `console-error` | a console `error` or a failed request (4xx/5xx, `requestfailed`) not whitelisted |
| 5 | `navigated` | the normalised hash changes (`#/artist/1000000` → `#/artist/#`) |
| 6 | `opened-layer` | the count of `[role=dialog],[role=menu],.sheet,[aria-modal]` grows |
| 7 | `state-changed` | the normalised aria snapshot differs |
| 8 | `nothing` | none of the above in 1 200 ms (`observation_ms`, an observation window, not a wait) |
| 9 | `blocked-destructive` | in the list below: clicked **up to** the confirmation, then cancelled |
| 10 | `unreachable` | exists in the tree but is not visible/reachable (or has vanished since enumeration) |
| 11 | `skipped-session` | ends the harness session (logout, unlink): never clicked in the pass |

`blocked-destructive` carries `confirm-reached` (a dialog or a layer appeared — correct) or
`no-confirm` (the action left directly — **a serious bug**).

### What is not an error

- `401` on `/api/me` in the first seconds of boot (the shell asks before the cookie exists);
- `404` on `/api/cover/*` and `/api/stream/*` when the bench serves neither;
- `404` on `/sw.js` in a static preview;
- any request the bench aborts to Deezer's hosts (`api.deezer.com`, `*.dzcdn.net`);
- **`503` whose payload says `deezer-disabled`** — the bench runs with the proxy off, and the shell
  says so in a notice. A `503` without that code stays an error;
- `net::ERR_ABORTED`: the harness navigates after its 1 200 ms window, which cancels what was still
  in flight. A real network failure names `ERR_FAILED` / `ERR_CONNECTION_*` and is kept.

### Fingerprint normalisation

`fingerprint(page)` = `{hash, aria, layers, textLen, nodes}` where `aria` is
`locator('body').ariaSnapshot()` after, in this order: slider lines dropped (the playhead advances
alone); clocks `\d+:\d+(:\d+)?` → `#`; ISO dates and `12 mars` / `1er janv` → `#`; the bench's
library ids → `#`; 32-hex hashes → `#`; trailing spaces dropped; empty lines dropped.

Two additions the plan's list implies but does not spell, both needed for two passes to match:
**UUID keys** (a playlist id is `352b4003-…`, not 32 hex) and **seven-digit ids** (`[123]\d{6}` —
8 000 tracks go past `3000099`, which `30000\d\d` would miss). Both are benchmark identifiers; a
pass that left them would differ from the next one.

### Deduplication

A line of a list repeated 50 times is clicked **3** times (two row styles: with and without a
subtitle); the rest are counted `duplicates` with `duplicate_of` = the first clicked one's
signature. The rule is *the brother rule*: in one parent, ≥ 6 siblings with the same role and the
same child-role set → keep the first 3.

- `elements_found` = distinct elements after deduplication;
- `coverage = clicked / elements_found`, printed with both counts (`coverage=418/418 (100.0%)`);
- an `unreachable` counts as found and **not** clicked — the percentage is never inflated;
- the persistent shell (search bar, player, navigation) lives outside `.route-host` and is counted
  **once** for the whole pass, not once per screen.

### Destructive actions

`passe`: clicked up to the confirmation, then cancelled (`Escape` / « Annuler » / `window.confirm`
→ dismiss). The fingerprint must come back; a cancellation that leaves the screen changed is a
`mineur`/`sérieux` bug of its own. `fin`: never in the pass (`skipped-session`), clicked once in
`--scenario session`. `jamais`: clicked up to its sheet, never past it (the update installer).

### Severity

- **bloquant**: `exception`, `blank`, `freeze`, `nothing` on an action verb (« Lire, Ouvrir,
  Ajouter, Supprimer, Appliquer… »), `blocked-destructive` in `no-confirm`, any failed scenario;
- **sérieux**: `console-error`, `unreachable` on a verb, a layer whose every control does `nothing`,
  a cancelled destructive that does not come back;
- **mineur**: the rest (`nothing` on a decoration, an unnamed node, a screen change without a visible
  effect). A `disabled` control is not clickable by design: counted `unreachable`, severity `null`.

### Evidence

```
out/
  bughunt-web.json            one entry per element (the Row shape below)
  bughunt-web.md              the human report, generated from the JSON
  screens/NN-<screen>.png     one per screen, always
  screens/NN-<screen>.aria.yml
  screens/NN-<screen>-K-<element>-<outcome>.png   one per anomaly (≤ 40)
  console.log                 console errors, page errors, HTTP failures, dialogs
  run.txt                     the summary line + tool/head
```

`<slug>` = accessible name folded to `[a-z0-9-]`, ≤ 40 chars. The JSON carries `tool` (script@head)
and `run_id` so two reports never get confused.

## The report (`bughunt-web.md`)

Fixed order, checked by the B3 judge: title; the ISO date / head / bench / viewport line; the
`bughunt: …` summary; **the owner's scenarios first**; the deduplicated bugs table (most severe
first); one section per screen with a row per element; then what was not clicked (duplicates,
unreachable, out-of-pass routes).

A bug is a row with `severity != null`; two rows with the same `(screen, outcome, normalised name)`
fold into one bug with `occurrences=n`. The bug `key` contains no clock, id or count, so two passes
on the same head must produce the same key list (`jq -S '.bugs|map(.key)|sort'`).

## Screens (closed table, anti-drift guard)

`webapp/test/bughunt/lib.mjs` holds `SCREENS`: 21 route instances + the 11 Settings sections = 32
screens. Before any click, `run.mjs` reads the `routes` object of `webapp/src/App.svelte` and the
`id:`s of `$: SECTIONS` in `webapp/src/routes/Settings.svelte` and compares them with the table. A
route or a section that exists in the source and not in `SCREENS` stops the pass with exit 2 and
`bughunt: undeclared route /x` — **no click**. That is the SPA's equivalent of
`RoutesHaveADoorTest.kt`: the day a route is born, the hunt says so instead of ignoring it.

Out of this pass, named in the report with their reason: `#/party`, `#/rc/<token>`,
`#/shared/playlist/<token>`, `#/invite/<token>`, `#/onboarding`, `#/viz` (they need a second client,
a token or a fresh account).

## The three scenarios (`stress.mjs`)

They run **after** the pass, on a warm SPA, and fill the report's first section. A failure is a
`bloquant` bug even when the pass found nothing.

1. **Forward burst** — 20 taps on « Suivant » at 100 ms, then 2 s: a non-paused `<audio>` exists, its
   clock advanced ≥ 0.5 s over 1.5 s, the displayed title is the expected queue index, one more tap
   changes it, no `pageerror`.
2. **Alternating burst** — 20 taps Suivant/Précédent; the same five assertions.
3. **Every setting applied** — each switch, checkbox, the 2nd option of a select, the next radio,
   ArrowRight then ArrowLeft for a range (the value must change then come back), `zzz` + Escape for a
   text box. After each: `main` non-empty, no exception, and a `PUT/POST/PATCH` to `/api/` is
   followed by a reload that repaints the section. « Recharger l'application de zéro » is out of the
   pass and clicked **once** here, with the theme and the queue preserved.
4. **Session round trip** — « Déconnexion » → the login screen paints, no blank, no exception →
   log back in, the shell repaints. This is the owner's third symptom, the one he never described.

`node test/bughunt/stress.mjs --only '#/search'` runs the burst on a screen with no queue: it must
answer `reason=player-not-open` and exit 1 — the scenario's own negative control.

## Bounds

`--max-clicks 1200`, `--max-minutes 25`; over budget → `stopped=budget` and the real coverage, never
100 % by silent truncation. 40 screen screenshots + 40 anomaly screenshots, then
`screenshots_capped=1`. Never a coordinate: click by role + name, scroll by `scrollIntoView`.
Never a sleeping wait for availability: the 1 200 ms is an observation window; the screen is opened
by waiting on its own marker. One retry per click, only when the node was detached by a navigation.

## Le juge

The visual judge (Coder-Claude, the same agent and the same grid as T9's `mobile/tools/judge/rubric.md`,
unchanged: six criteria, seven blocking rules) reads the two reports and their screenshots and writes
**one verdict per application**. It does not click, run, fix or push: the walkers produce the map, the
judge says which lines of it are bugs.

- **Who / when.** A swarm schedule (`neezer-bughunt-judge`, Monday 06:30 `Europe/Paris`, three hours
  after the nightly `bughunt.yaml` pass) sends the prompt of `mobile/tools/bughunt/judge-trigger.md`
  (mode `bughunt`). A workflow cannot send a swarm task (no token), so the Lead's heartbeat is the
  fallback: a successful `bughunt.yaml` run whose artifacts have no verdict gets the same prompt by
  hand. A `run-id` already judged is never judged again (the judge reads recent tasks and stops).
- **What it reads.** `gh run download <run-id> -n bughunt-web` / `-n bughunt-native`: `bughunt-<app>.md`
  (the generated report), the `.json` register, `screens/*.png` (it must be able to **look** at them: no
  image, no verdict, it stops and says so).
- **What it writes.** `bughunt-verdict-<app>.md`: `Verdict : PASS|BLOCK|INCOMPLETE`, the six-criteria
  table **per screen** with a cited proof per cell (`—` where a screen cannot carry a criterion), then
  *Bugs confirmés* (severity kept, with the report line and the capture) and *Faux positifs*
  (what the proof does not support). It lowers or confirms a severity only with a sentence anyone can
  check on the capture. It also lists what only the images show and the harness cannot (a toast one
  character wide, a flat background behind a cover): those are findings of the judge, not of the walker.
- **`INCOMPLETE`** means the run produced no usable artifact (missing after two downloads, a screen
  never painted). It asks for **no code change**: rerun `bughunt.yaml`, resend the task.
- **What a confirmed bug becomes.** One fix task per `bloquant` / `sérieux` bug and one per screen for
  the `mineur` ones, routed by zone (plan §11); the fix PR carries the before/after report line of the
  element, and the next pass must show it `state-changed` / `navigated`.
- **Cost.** 8 to 12 minutes of Sonnet per application, once a week at most plus hand-sent runs; the
  same quota as T9's judge, so the two add up.

## What the first passes measured, and the rules they changed

Measured on the worker (4 vCPU, Chromium under SwiftShader) against `tools/perf_api.py --serve`
(8 000 titles, 60 playlists, Deezer off), viewport 412×900:

| | Pass A | Pass D |
|---|---|---|
| screens | 32 | 32 |
| elements found | 663 | 646 |
| clicked | 581 | 571 |
| coverage | 87.6 % (581/663) | 88.4 % (571/646) |
| duplicates | 1 029 | 954 |
| `nothing` | 86 (19 named) | 77 (7 named) |
| exceptions | 4 | 4 |
| blank / freezes | 0 / 0 | 0 / 0 |
| seconds | 1 080 | 1 187 (`--max-minutes 25` respected) |

The four scenarios PASS on both passes (20/20 taps, clock +1.50 s, 48 settings controls with 0
blank, logout/login round trip). The pass found a **real shipped bug**: `reduceMotion is not
defined` (`webapp/src/components/GlobalBackdrop.svelte:101`), thrown when a track with no art
clears the ground — listed in the report, **not fixed by this lot** (a bug is its own task, §11).

Eight rules came out of the first passes, each from a measured failure:

1. **The sibling rule is grounded in the DOM.** The aria tree alone cannot tell a list from a
   toolbar: every leaf under `main` shares "no child roles", and applying §4.4 to that grouping
   left the Album screen with **0 clickable elements** (20 heterogeneous action buttons collapsed
   to 3). A group now counts as a list only when the DOM repeats: ≥ 6 same-tag, same-class
   siblings somewhere in the member's first 4 ancestors. Measured on Réglages → Animations:
   47 `duplicates` on the 50-world picker, 3 clicked, as the acceptance asks.
2. **`blank` is measured on `.route-host`, not `main`.** `main` also holds the permanent shell
   (top bar, notices): after a route was blanked on purpose it still carried 120 characters of
   search bar, so the owner's own symptom could never fire. Nodes and text are read on the routed
   box, route unchanged; the planted blank then reads `blank=3`, `bloquant`, exit 1.
3. **A folded `<details>` is not in the accessibility tree.** The "Monde de cet écran" picker of
   Réglages → Animations hides its 50 worlds behind a closed disclosure: `revealAll` opens every
   closed `<details>` and scrolls the routed box to its end and back before enumeration (the
   plan's own inventory did one scroll per screen).
4. **A field answers a typed value, not a click.** A focus is not a state an aria snapshot sees,
   so a textbox clicked read `nothing` (a false `bloquant` on « Chercher un réglage »). The pass
   types `zzz` (a colour input gets `#123456`, `zzz` is a malformed colour) then Escape, and picks
   the 2nd option of a native select.
5. **A `nothing` is confirmed 800 ms later.** At 1 200 ms alone, 5 `nothing` lines differed between
   two passes; silence is now proven over 2 000 ms. The remaining variance is stated, not hidden:
   two full passes still differ by ~4 `nothing` **mineur** lines whose effect depends on the state
   a previous click left (a slider's value, a download Chromium refuses the second time); the
   `bloquant` / `sérieux` lists are identical.
6. **A 503 whose payload says `deezer-disabled` is the bench's state**, not a screen's defect
   (the proxy is off by design). It is read from the response body; a 503 without that code stays
   an error, and `/api/archive/backfill` (503 in the bench) is reported as the `sérieux` finding it
   is.
7. **A desktop hover quick action is `unreachable`.** The albums grid carries one hidden « Lire »
   per card (96 first measured): in the tree, never hit at its centre on a phone. The reachability
   probe decides in milliseconds instead of a 4 s click timeout, and the count is named in the
   report — coverage is never inflated.
8. **The burst re-injects its 12-track queue.** The pass clicks « Lancer mon Flow » and a
   playlist's « Lire »; by the time the scenarios run, the queue is no longer the bench's 12, and
   the first run failed for the wrong reason (`title-not-found`). The queue is written through
   `sessionStorage.__inject` (a direct localStorage write is overwritten by the app's own save on
   unload) and the page reloads before the burst.
