# Commands: tests, benches, builds

Reference notes for agents working on this area. Read [CLAUDE.md](../../CLAUDE.md) first: its process and rules apply here. Other areas: [docs/agents/](./).


```sh
# Python tests (no network). Discovery is driven by tests/__init__.py load_tests.
python -m unittest                                   # whole suite (~55s serially)
python -m unittest tests.test_deezer                 # one module
python -m unittest tests.test_webui.SomeClass.test_x # one test
# The 2026-11 discovery/history/mood/share lots, together (all offline):
python tools/partest.py tests.test_genre_radio tests.test_history tests.test_moods \
    tests.test_playlist_recs tests.test_playlist_share
python tools/partest.py tests.test_recap tests.test_playlist_folders tests.test_playlist_collab_core
                                                     # the 2026-11 recap / folders / collaborative core lots
python tools/partest.py tests.test_invites tests.test_admin_users tests.test_onboarding \
    tests.test_onboarding_import
                                                     # invites, guarded accounts, artist onboarding, account/import
python -m unittest tests.net.suite                   # network tests (hit real services; weekly in CI)

# Same suite across processes — ~12s on 4 cores. Takes the same test ids. What CI runs.
python tools/partest.py                              # whole suite
python tools/partest.py -j8 tests.test_webui
python tools/partest.py --coverage && coverage combine
NS_REQUIRE_FFMPEG=1 python tools/partest.py          # fail, don't skip, if ffmpeg is missing

# The upgrade path on the engines production runs (PostgreSQL/MySQL). Each case
# drops EVERY table of the named database, so the name must contain "test".
pip install -e .[postgresql]                         # or .[mysql]; MySQL 8's
                                                     # caching_sha2 also needs `cryptography`
NEEZER_TEST_DATABASE=postgresql://postgres:postgres@localhost:5432/neezer_test \
    python -m unittest tests.test_schema_external
NEEZER_TEST_DATABASE=mysql://neezer:neezer@localhost:3306/neezer_test \
    python -m unittest tests.test_schema_external
NS_REQUIRE_DATABASE=1 NEEZER_TEST_DATABASE=... python -m unittest tests.test_schema_external
                                                     # fail instead of skip without a URI (what CI sets)

# Dev install (a project .venv is expected)
pip install -e . && pip install lxml coverage        # == ci-requirements.txt

# Run the server
neezer-server                                     # serves on :5722
export FLASK_APP="neezer.web:create_application()"; flask run   # backend dev server

# Web UI (Svelte SPA)
cd webapp && npm install && npm run build            # -> neezer/webui/dist (gitignored)
cd webapp && npm run dev                             # hot reload; proxies /api -> localhost:5000
cd webapp && npm test                                # node --test: the shipped rhythm analyser on real audio, the animation core against
                                                     # its JavaScript oracles, the catalogue/skins/drivers, the genre trainers, the cover loader.
                                                     # No test framework — but run the npm install above first: the modules under
                                                     # test reach svelte/store via stores.js, and a pretest guard says so in one line.
cd webapp && node --test test/genre_station.test.mjs test/pan.test.mjs test/lyriccard.test.mjs
                                                     # the pure rules of the station context, mono/pan law and lyric card
cd webapp && npm run wasm                            # rebuild rhythm.wasm + rhythm-simd.wasm from webapp/rhythm, appcore.wasm from
                                                     # webapp/appcore and trainer.wasm + trainer-simd.wasm from webapp/trainer (Rust,
                                                     # target wasm32-unknown-unknown). All five binaries are COMMITTED; npm test
                                                     # fails if they were not built from the sources next to them.
cd webapp/appcore && cargo test --release            # the app core's own unit tests (folding, the estimators' edge cases)
cd webapp/trainer && cargo test --release            # the genre trainer's (artist folds never split an artist, a seed repeats a run)
cd webapp && node test/eval/rhythm-eval.mjs          # the analyser on 28 arranged records with ground truth (beats, kicks, tempo, drops)
cd webapp && node test/eval/family-eval.mjs          # ...which genre the live classifier names on each ([--explain <id>] rule by rule,
                                                     # [--features] the descriptors): analysed once, cached, the rules replayed in JS
cd webapp && node test/rhythm/run.mjs                # ...and end to end in headless Chromium: AudioWorklet, delivery, timing
cd webapp && node test/rhythm/run.mjs --trim -60     # ...on a (simulated) short output path: the automatic look-ahead at work
cd webapp && node test/party/run.mjs [--fmt flac]    # listen party end to end: real server, host + guest, clicks detected on what
                                                     # each one PLAYS, scored on the shared wall clock (seek, pause, skip)
cd webapp && node test/remote/run.mjs [--phone] [--shots <dir>]   # remote control end to end: real server, owner + controller
                                                     # (make a link in the sheet, claim it, drive, drop the network, cut)
cd webapp && node test/render/run.mjs --world piano --genre frenchcore --bpm 200   # render bench: contact sheet in test/render/out/
cd webapp && node test/render/run.mjs --size 480x270   # the whole catalogue, one sheet per world, for a visual audit
cd webapp && node test/render/run.mjs --check        # every world, held to the picture contracts (headless Chromium, no GPU needed)
cd webapp && node test/render/run.mjs --strip        # the players' bar strip, photographed over the blurred cover; prints the
                                                     # share of the canvas carrying alpha, its top/bottom rows, the exact colour
                                                     # and the SwiftShader cost (--eco / --light for the other grades)
cd webapp && node test/ui/run.mjs                    # the lists' main-thread cost in Chromium on a 8000-track library (needs npm run build):
                                                     # open the favourites, type into them, sort, open a 4000-track playlist
cd webapp && node test/queue/run.mjs [--phone]       # the queue's editing in headless Chromium on a 4000-track queue: drag a row (mouse, or real touch
                                                     # events), drag to the far end with the list scrolling under the finger, the keyboard, clear
cd webapp && node test/theme/run.mjs                 # the sleeve change is a 600 ms walk, not a cut: accent at 60 Hz, no
                                                     # uncovered ground on one or two skips, the podcast header tint, reduced-motion;
                                                     # DIST=<dir> serves a build made elsewhere so it never replaces neezer/webui/dist
cd webapp && node test/glass/run.mjs                # the ONE glass ladder (3 tiers): contrast >= 4.5:1 on 57 surfaces, one
                                                     # backdrop-filter at a time, zero in eco/reduced transparency, cost per surface
cd webapp && node test/lyrics/run.mjs               # the lyrics view in its two shapes: the wide player's separated column is NOT a
                                                     # card (transparent, no frame) and its current AND past/future lines keep 4.5:1
                                                     # on the painted backdrop, over a white and a black sleeve, in the three themes;
                                                     # the narrow player's card tab and the phone's sheet keep their glass
cd webapp && node test/vizstrip/run.mjs             # the visualizer strip's real height: 56 px desktop / 44 below / 40 mobile,
                                                     # canvas fills it, no overlap with the transport, the footer or the thumbs;
                                                     # and its picture on the three themes + reduced motion + no-WebGL2: alpha 0
                                                     # outside the capsules, one theme accent inside, backdrop identical, >= 3:1
                                                     # contrast, a live custom accent, and no canvas at all in eco
cd webapp && node test/backdrop/run.mjs              # the app's ground behind all seven routes: ONE blurred sleeve, 0 backdrop-filter,
                                                     # under the content, a 640 ms fade on a skip, one cover load, eco (no image,
                                                     # data-eco), contrast over a white sleeve; SHOTS=<dir> for PNGs, DIST=<dir> as above
cd webapp && node test/volume/run.mjs                # the player bar's volume: 8 px track, 22 px thumb centred (0.00 px), 44 px
                                                     # target, 14 % click -> 0.14, keyboard/mute, 1920/1440/1280/1181/641/390
cd webapp && node test/mobile/run.mjs                # the phone shell at 360/390/639/640/641 px: no sideways overflow, tab bar 64 + mini
                                                     # player 64 floating 8 above it, the library tab, the full-screen player, the theme kept
                                                     # (needs npm run build; PLAYWRIGHT=/path/to/playwright-core CHROMIUM=/usr/bin/chromium)
cd webapp && node test/player/run.mjs                # the sleep timer and the podcast speed on the audio elements the browser really plays (a fake page clock)
cd webapp && node test/output/run.mjs                # the audio path: routed by the full-screen player, the fault simulated (the element's clock 8% fast): convicted
                                                     # in one window, DIRECT on a fresh element and a fresh context, retried at a track change, forgotten when clean
cd webapp && node test/explore/run.mjs [--phone]     # the Explore screen: front page, into a genre, play a chart, back (Deezer's answers put in by the test)
cd webapp && node test/panel/run.mjs [--desktop]     # what is under the full-screen player: the swipe up (touch events), the desktop tab
cd webapp && node test/eq/run.mjs                    # the equalizer's saturation warning: a +16 dB curve warns, stays one
                                                     # second into silence with the same value, « Plat » clears it; a flat
                                                     # curve with a loud normalized master never warns; EQ off hides it at once
cd webapp && node test/repeat/run.mjs               # repeat one / all on a 4 s track: an ARCHIVED file served by the real server (byte ranges,
                                                     # tools/perf_api.py --audio) and a live first-play stream, with the trim and the crossfade:
                                                     # every pass reaches its end, none jumps back from the middle, no dead air
cd webapp && node test/skip/run.mjs                 # a crossfaded skip in a HIDDEN page whose timers are throttled (the phone behind a car
                                                     # radio): the next track must be heard at once, graph and direct paths
cd webapp && node test/offline/run.mjs              # downloads kept by the Android app: an old Blob moved in byte for byte, played off
                                                     # /__nsoffline/, a new download, the Réglages list (a stand-in app; needs npm run build)
cd webapp && node test/clicks/run.mjs               # what a click costs before the screen answers (Event Timing, 4x CPU throttle): the
                                                     # nav, opening and closing the full-screen player; PHONE=1, PROFILE="<action>" for
                                                     # its heaviest functions (build with `npx vite build --minify false` to read them)
cd webapp && node test/motion/run.mjs                # the motion table (docs/agents/transitions.md) on the real app, 1440×900 and
                                                     # 390×844 × full / reduced / eco: the tokens resolve to the plan's numbers, the
                                                     # eco guard (getAnimations() empty after a .btn hover AND a .btn toggle, a
                                                     # [data-motion="status"] spinner still 1 s), the CSS expressions lib/motion.js
                                                     # emits (calc() slides, a veil's color-mix), then every lot's
                                                     # scenarios/<lot>.mjs — each file on its own page, the same requests in the
                                                     # three levels. VIEWPORT=390, FILES=home-search, ONLY=full (one level),
                                                     # SELFTEST=1 (the canary: a fetch added at full motion MUST fail the request
                                                     # check), SHOTS=<dir>, DIST=<dir>, PYTHON=<venv python>
cd webapp && node test/vizdirect/run.mjs [FADE=1]   # the animations on the DIRECT output through pauses, skips and a reopening: they
                                                     # must still hear the music (the copy's dead-track bug)
cd webapp && node test/download/run.mjs              # a playlist downloading on a shared 256 kbit/s link (LINK=bytes/s) while a track is
                                                     # launched: it must start as fast as on an idle link, play without a stall, and the downloads finish
cd webapp && node test/import/run.mjs [--phone]       # a shared playlist link: pasted into the search, read, played, kept
cd webapp && node test/requests/run.mjs             # what playing costs in requests: a track the server refuses (Deezer off) is
                                                     # retried at a human pace, not 70 times a second; a missing cover is asked once
cd webapp && node test/fullscreen/run.mjs           # the desktop full-screen player from 641x900 to 1920x1080: every control on screen,
                                                     # inside its column, the title readable at 641-800, and the element a click at its centre lands on
                                                     # (the four browser tests above start tools/perf_api.py --serve; PYTHON=<venv python>, SHOTS=<dir> keeps pictures)
cd webapp && node test/failure/run.mjs               # an API 500/offline says its cause and offers Réessayer (one click = one request),
                                                     # menus ⋯ work from the keyboard, rows are 44 px on touch / 40 px with a mouse
cd webapp && node test/menu/run.mjs                  # the contextual menu on every surface: right-click / Shift+F10, items per surface,
                                                     # Escape + focus back, 0 request per open (16 surfaces, 1280×900 and 390×844)
cd webapp && node test/recap/run.mjs                 # the recap stories: 2 requests cold / 1 warm / 0 per card, 9 cards maximum,
                                                     # share poster 1080x1350 + real PNG download, 390/360/641/1440 px, auto-advance,
                                                     # all states and privacy; needs a build (DIST=/tmp/... for one outside dist)
cd webapp && node test/onboarding/run.mjs            # invitation -> public page -> account -> artists -> account step: 1 public inspect,
                                                     # 1 redemption POST, 36 tiles, 1 search GET, 1 like POST + 2 neighbours, the
                                                     # refresh, the 3 skippable options, 390/1440, and the no-artist escape on a 0-track server
cd webapp && node test/admin/run.mjs                 # Réglages → Utilisateurs on 200 accounts and 20 invites: 1 users + 1 invites GET,
                                                     # 1 GET per search/page, 1 POST/PATCH/DELETE per mutation, the once-shown link,
                                                     # the last-admin 409, the member's 0 request + 403, 390/641/1280 geometry
cd webapp && node test/podcast/run.mjs               # a show with chapters and transcripts: 0/1/0 chapter requests, 1 panel GET,
                                                     # 0 per segment, 200 lines, a hit seek, 1 poll per 10 s, the 100-chapter sheet,
                                                     # 390/641/1440 and the admin mark/progress card

# Desktop app (GPUI, desktop/): Omarchy Linux only, its own Cargo workspace
cd desktop && cargo run -p neezer-desktop            # the window (debug); Ctrl+Q quits
cd desktop && cargo build --release                  # -> target/release/{neezer-desktop,neezer-transcribe}
cd desktop && cargo test --workspace                 # the workspace suite (what CI runs)
cd desktop && cargo fmt --all --check
cd desktop && cargo clippy --workspace --all-targets -- -D warnings
cd desktop && xvfb-run -a ./target/debug/neezer-desktop --self-test-quit-after-first-frame
                                                     # headless: prints first_frame_ms=<n> after the first painted
                                                     # frame, then quits (what the desktop workflow runs)
cd desktop && cargo run -p nz-transcribe -- --version   # the linked whisper.cpp build (Vulkan included)
VULKAN_SDK=/path/to/vulkan-headers cargo build -p nz-transcribe   # whisper.cpp's CMake needs the Vulkan HEADERS,
                                                     # not only the loader (Arch: vulkan-headers + shaderc)
desktop/tools/build_engine_variants.sh --release      # the x86-64-v3 and x86-64-v4 engine children,
                                                     # `neezer-transcribe-v3` / `-v4`, beside the floor one:
                                                     # one cargo build each into target/engine-<level> with
                                                     # desktop/.cargo/engine-<level>.toml (cargo builds one
                                                     # copy of a dependency per graph). `v3`/`v4` as arguments
                                                     # to build only some of them.
cd desktop && cargo build --release && target/release/neezer-transcribe engine-program
                                                     # which engine child THIS CPU gets, one line: -v4 on
                                                     # AVX512F/CD/BW/DQ/VL, else -v3 on AVX2+FMA+F16C, else
                                                     # the floor binary
desktop/tools/check_cpu_floor.sh desktop/target/release/neezer-transcribe
                                                     # every floor at once: each binary's census, each child's
                                                     # ggml OWN objects gated at its level (v2/v3/v4), the floor
                                                     # binary transcribing under an emulated Nehalem
                                                     # (`qemu-user`; NZ_WHISPER_MODEL + NZ_WHISPER_AUDIO for
                                                     # the speech, the crate's fixtures), each child natively,
                                                     # then the dispatch under Nehalem/Haswell/Skylake-Server.
                                                     # A ggml built with -march=native SIGILLs here in a second.
cd desktop && cargo clean -p whisper-rs-sys && cargo build -p nz-transcribe
                                                     # after editing desktop/.cargo/config.toml: cargo does not
                                                     # rebuild a dependency for a build-script environment change,
                                                     # so an old target/ keeps the old instruction set once more

# Deezer CLI
neezer-cli deezer login-test                      # check the ARL works
neezer-cli deezer import <deezer-url|track|album|playlist <id>>
neezer-cli deezer sync                            # import playlists/favorites/new releases
neezer-cli audit list [-n 50]                        # the durable admin trail, newest first
neezer-cli deezer lyrics [--overwrite] [--limit N]  # archive synced lyrics for archived tracks
neezer-cli podcast chapters [--force] [--urls] [--limit N]  # extract chapters from archived podcast
                                                     # episodes (sidecar/tags, optionally publisher URL); resumable
neezer-cli podcast transcribe --episode <id>          # queue one archived episode (manual priority; repeatable)
neezer-cli podcast transcribe --all [--limit N]       # queue the backlog of marked shows (budgeted, resumable)
neezer-cli podcast transcribe --status                # queue counts, active progress, causes and budget
neezer-cli podcast transcribe --stop [--episode <id>] # stop queued/running work
neezer-cli transcribe rebuild [--limit N]             # recreate the DB index from the .transcript.json sidecars
python -m unittest tests.test_transcribe              # fake whisper.cpp, fake ffmpeg, real SQLite
NS_WHISPER=1 NS_WHISPER_BIN=/usr/local/bin/whisper-cli NS_WHISPER_MODEL=/models/ggml-small.bin \
    NS_WHISPER_AUDIO=/tmp/opencode/extrait-60s.wav \
    python -m unittest tests.test_transcribe.RealEngineTestCase   # opt-in, real engine, real speech
neezer-cli deezer analyze [--force] [--limit N] [--workers N]  # measure tempo + style for archived tracks
neezer-cli deezer bpm-audit [--limit N]             # Deezer's published BPM against the files' measured tempo, per style
python tools/tempo_eval.py --render /tmp/tempo          # the server's file tempo on the 28 arranged records + beatless tones
python tools/perf_api.py [--tracks 20000] [--serve] [--recap]   # every screen's /api route timed on a big synthetic library
python tools/perf_api.py --serve --covers cover.jpg             # the same, with a REAL cover on every album: the only way
                                                                # to measure the client's sleeve path (decoded images,
                                                                # atlas tiles, eviction) — without it /api/cover 404s
                                                     # (Deezer off): median ms, SQL statement count, bytes and gzip per route;
                                                     # the library lives in $TMPDIR/nsperf-* and is removed on every exit
                                                     # (SIGTERM included); a launch sweeps what a SIGKILLed one left;
                                                     # --recap seeds 37 000 listen events and times /api/recap/years + /<year>
                                                     # cold/warm/304 with exact totals; --onboarding seeds invitation tokens,
                                                     # a searchable artist and cached neighbours; --admin-users N --admin-invites M
                                                     # seed the accounts screen; --podcasts seeds a show with chapters and
                                                     # transcripts (external runner); --dist serves a build outside dist
neezer-cli deezer embed [--force] [--limit N]       # extract genre embeddings (needs onnxruntime)
neezer-cli deezer embed --self-test                 # check the mel front-end without a reference

# Docker (full stack: builds SPA + python image, runs entrypoint that creates admin + auto-sync)
docker compose up --build                            # web player at :5722/app, Subsonic at :5722/rest
docker build -t neezer:dev . && docker/smoke.sh neezer:dev   # what CI checks before pushing

# Transcription sidecar (docker/transcribe): no model in the image, model mounted :ro
tools/fetch-whisper-model.sh --model large-v3-turbo-q8_0 --dest ./models
tools/fetch-whisper-model.sh --model small --dest ./models   # NAS / tight RAM
docker build -t neezer-transcriber:dev docker/transcribe     # context is the sidecar dir
docker buildx build --platform linux/amd64,linux/arm64 -t neezer-transcriber:dev docker/transcribe
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml up -d transcriber
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml logs -f transcriber
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml \
    exec transcriber python3 /app/worker.py --check          # model/binaries/token, no API call
python3 docker/transcribe/test_worker.py -v                  # fake API + fake whisper/ffmpeg, no Docker needed
                                                     # (covers claim/audio Range/chunk/complete/fail/cancel/backoff;
                                                     #  the guide is docs/podcast-media.md)
```

The project's subagents are declared in `opencode.json`, with their prompts under
`.opencode/prompts/` (`docs/agents/orchestration.md` is the routing guide):

```sh
opencode debug agents                        # what OpenCode loaded: ids, modes, permissions
opencode debug config                        # which config files are in effect
python -m unittest tests.test_agent_docs     # config, prompts and area notes stay in sync
```

Note: the upstream `tests/` are unittest-based; there is no pytest config.
`tests/__init__.py` makes the suite fast in two test-only ways, each guarded by a test: it swaps
argon2 for cheap parameters (hashing was half the wall clock; `tests.managers.test_manager_user`
pins the production defaults), and it compiles each distinct werkzeug URL builder once per process
instead of once per rule per app (~95% of every `create_application`; `tests.test_harness` checks
every rule's URLs against a fresh werkzeug compilation). CI runs nine workflows:
### CI: the Actions cache budget is a hard constraint

Measured 2026-11-18: `gh api /repos/fnyaker/Neezer/actions/caches` → **30 entries, 16.9 GB**, for the
**10 GB** GitHub allows per repository. Past the limit GitHub evicts in LRU order, so a run that asks
for its cache often finds it gone: that is where `mobile`'s 8-minute Rust rebuilds, the 29-minute
`Arch package` and `desktop.yaml`'s variance came from, not from the workflows' own logic. Every
cache experiment run in that state measures eviction.

The rule, then: **the number of entries and their size are part of the design.** Two jobs that build
the same thing share one entry (`shared-key`), and a job that only reads a cache should not write a
second copy of it.

- `desktop.yaml`'s `floor` shares the `test` job's entry (`shared-key: test`) — same debug profile,
  same workspace. It also holds `target/engine-v3|-v4`, so when `test` writes the entry the children
  are rebuilt: ~6 min, on a job that is not the workflow's longest.
- `desktop.yaml`'s `build` and `release.yaml`'s `binaries` share one release entry
  (`shared-key: desktop-release`) instead of keeping 1.35 GB each.

- `tests.yaml` — three jobs, not a version matrix: **image** (Python 3.13, what the Docker image
  ships, with numpy, `NS_REQUIRE_FFMPEG=1`, under coverage, and the image's OWN ffmpeg binary —
  the job reads the pinned `FROM mwader/static-ffmpeg…` line out of the Dockerfile and copies
  `/ffmpeg` from it) and **minimal** (Python 3.10, the oldest `setup.cfg` accepts, bare install),
  both via `tools/partest.py`, packed by the previous run's timings (Actions cache) — and
  **database** (`matrix`: PostgreSQL 16 and MySQL 8.4 service containers, the engine's extra
  installed, `NS_REQUIRE_DATABASE=1` and `tests/test_schema_external.py`). A test that needs
  ffmpeg or numpy must RUN in the image job — a skip there is the suite testing less than it
  claims.
- `webapp.yaml` — `npm ci`, `npm test`, `npm run build`.
- `desktop.yaml` — the GPUI workspace in **independent jobs** (`fmt`, `clippy`, `test`, `floor`,
  `build`, `audit`, no `needs` between them): the workflow lasts its LONGEST job, never the sum, so
  work that shares nothing with the suite gets its own job rather than a place at the end of
  another. Two measured rules:
  - **Install the system packages from a cache of their `.deb` files**
    (`.github/actions/cached-apt`, `Dir::Cache::archives` — apt-get(8), FILES). The same 116 MB came
    down at **87 and 120 kB/s** in two jobs and **12.4 MB/s** in a third of run 44278965583, so
    `System dependencies` was the workflow's critical path — **median 22.2 / 20.4 / 16.4 min** per
    job over 8 successful runs — for a package set that never changes. Same run, same mirror: what
    varied was the CONNECTION, so the point is not to depend on it. The list still lives in the
    workflow that installs it (release.yaml keeps its own): the action hashes it as the cache key (and NOT the runner
    image: in run 44309607752 the four jobs reported three different `ImageVersion`s, so an
    image-keyed cache would never be shared), and its cold path forces IPv4 and retries a stalled
    fetch (`Acquire::ForceIPv4`, `Acquire::Retries`, `Acquire::http::Timeout`). Two traps, both
    measured and both fatal to the cache: apt creates `archives/partial/` and `archives/lock` as
    ROOT, so caching the directory made every save die on `tar: … Permission denied` — the path is
    the glob `apt-archives/*.deb`; and the archive served the same 116 MB at 20.8 MB/s to one job
    and 90.9 kB/s to another of the same run, which is the connection, not the mirror.
  - **The CPU-floor proof is its own job** (`floor`). The two engine children plus
    `check_cpu_floor.sh` were the tail of `cargo test + window` (medians **5.83 + 4.15 min**) and
    share nothing with the suite but the target directory. Moved, not lightened: same steps, same
    binaries, same assertions. Its Rust cache has its own key on purpose — the two jobs hold
    different artifacts (`target/engine-v3`, `-v4`), and one shared entry would be overwritten by
    whichever job finished last.
- `mobile.yaml` — the Rust core (`mobile/rust`), the APK (UniFFI bindings, Gradle, lint, sizes) and
  the headless emulator, in three jobs; the emulator `needs` the APK, so the workflow lasts both.
  **`cancel-in-progress` is for pull requests only** (`${{ github.event_name == 'pull_request' }}`),
  and that is a bug fix, not a preference: it was unconditional, so every push to `master` killed the
  run in flight — and **a cancelled run never reaches its cache's post-save step**, which is exactly
  why run 44278965693 logged `Cache not found for input keys: mobile-apk-…` and recompiled 8 min of
  Rust. A push to `master` is the commit a release comes from: it runs to the end.
  The rest of this workflow's cost is not in this file: the repository's Actions cache budget is
  exceeded (30 entries, **16.9 GB for the 10 GB** GitHub allows, `gh api /repos/…/actions/caches`),
  so entries are evicted in LRU order and a run that asks for its cache often finds it gone. That is
  the next lot, and it is where the 8-minute Rust rebuilds and the 29-minute Arch rebuilds come from.
- `docker.yaml` — a PR builds amd64 only (no QEMU) and runs `docker/smoke.sh` on it; master and
  `v*` tags smoke-test amd64, then build and push both architectures. **ffmpeg in the image is a
  static build** (one file, no shared library): the apt layer that installed Debian's package was
  172 MB of the 281 MB compressed image, for a server that only needs audio. The smoke test checks every filter and
  encoder the app drives BY NAME and writes-then-reads every format it serves; bump the pin (it is
  the multi-arch index digest) and those two lists are what to re-check.
- `network.yaml` — `tests/net` (real third-party services), weekly and on demand.
- `android.yaml` — the native app APK, uploaded as a run artifact / attached to `v*` releases as
  the one asset `neezer.apk` (a release that carried the pre-rebrand `nsupysonic.apk` fails the
  job). `testReleaseUnitTest` and **`lintDebug`** both run before the build: `assembleRelease`
  only runs `lintVitalRelease`, which ignores an error-level check such as AndroidX's
  `UnsafeOptInUsageError` (a missing `@OptIn(UnstableApi::class)` on a declaration using a
  `@UnstableApi` symbol compiles and ships), so the job asks for the full debug lint explicitly.
  The job then refuses to publish an APK it cannot vouch for: `unzip -t` (a truncated file is not
  an APK), `apksigner verify --min-sdk-version 23` must report the **v1, v2 and v3** schemes —
  without `--min-sdk-version` apksigner skips the JAR check whenever the APK's own minSdk is 24 or
  more and reports `v1 scheme: false` even on a v1-signed file —, `zipalign -c -p 4` (an unaligned
  stored entry is the other way an APK opens as "not a valid application"), `aapt2 dump badging`
  must say `org.nsupysonic.app` with a versionCode above 0, and after the upload the asset is
  downloaded again and `cmp`-ed against the built file.
- `release.yaml` — on a `v*` tag, attaches `docker-compose.yml` (image pinned to the tag's version, since the checked-in file follows `:latest`) and `.env.example` to the GitHub release; `android.yaml` attaches the APK to the same one, and whichever runs first creates the release.
- `security.yaml` — advisory scanners side by side (CodeQL, Semgrep, Bandit, Trivy over every lockfile + Dockerfile, Gitleaks over the history); each job is `continue-on-error`, so a finding never gates a merge. SARIF goes to the Security tab.
