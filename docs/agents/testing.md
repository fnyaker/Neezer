# Tests: philosophy and suites

Reference notes for agents working on this area. Read [CLAUDE.md](../../CLAUDE.md) first: its process and rules apply here. Other areas: [docs/agents/](./).


**A TEST EXISTS TO SAY HOW THE THING BEHAVES IN REAL CONDITIONS.** Not to say that there is a test.
Three perfect frames of a sound nobody has ever recorded prove nothing, because no sound is perfect
— so a test that draws its input is testing its own drawing, and the bar is: understand the music,
then simulate it the way it is actually made. This applies to everything in this repository, and
every claim a test makes should be a MEASURED number written down next to the assertion.

`webapp/npm test` (node --test, no dependency to install) is the SPA's suite. Its audio half drives
the **shipped binary** (`test/rhythm.test.mjs`) — real audio, pushed in the worklet's own 128-sample
quanta, through the whole Rust chain — because every fault listed above passed a suite that drove one
module at a time with material chosen to suit it. `test/eval/rhythm-eval.mjs` asks the broader
question on 28 arranged records (`test/songs.mjs`: an intro with no kick, a build whose snares
accelerate, the silence before a hardcore drop, a breakdown with no drums, a waltz, a live drummer
drifting a percent either side of the click) in four scenarios, cold and seeded; `test/rhythm/run.mjs`
asks it through the AudioWorklet and the engine's delivery in headless Chromium. Its animation half (see **How the animations are
tested** in [animations.md](animations.md)) pins what can be decided without a GPU — the catalogue, the skins, the shader contracts,
the drivers' musical time — and leaves the pixels to the render bench (`test/render/run.mjs
--check`), which is not part of `npm test` because it needs a browser. Run it after touching a world.

`test/synth.mjs` is the instrument: it writes **samples**, not spectra, and analyses them with an
iterative radix-2 FFT and a Hann window scaled exactly as `AnalyserNode.getFloatFrequencyData`
scales it. Drawn spectra have no leakage, no phase, no intermodulation and only the harmonics
somebody remembered to draw, and every one of those absences hid a fault: a lead's filter transient,
a hat's skirt, a pad beating against itself. Every source is the production recipe for the sound it
names — a pitched sine swept and driven into a waveshaper and a clipper, three detuned saws through
a moving low-pass and a **24 dB/octave** mix low-cut (the order is the part that matters: a lead's
low end is a filter transient, not a fundamental, and 6 dB/octave does not remove a transient), a
kick sub layer that outlasts the punch, a real lookahead limiter, rolls shortened to fit their
subdivision, a reverse bass modelled as the sidechained note it is rather than a gated one.

Three rules hold it together, and each was bought:
- **Keep it physical.** Every knob corresponds to something a producer does. A test that only passes
  because the generator is unrealistic is worse than no test — and when the instrument is wrong, fix
  the instrument FIRST: making the hi-hat a real hi-hat did not stop it being convicted, which is
  what turned a mystery into a stated fault in the feature extractor, where the fix belonged.
- **Never move the material to make the code pass.** If the two are genuinely inseparable, say so in
  the test with the measured evidence and assert the layer that *does* resolve it.
- **Score both ends on the same window.** A detection belongs to the kick it is nearest to and is
  scored only if that kick is inside the window. Scoring detections over one interval and ground
  truth over another invents faults at the seam — two of the three outliers in the whole suite were
  a kick rendered at exactly 30.000 s being dropped from the ground truth while its perfectly good
  detection at 30.016 s stayed. For the same reason the bench never analyses past the music: a
  window straddling the end of the buffer is a step function, and the FFT of a step is broadband
  splatter weighted to the bottom, which the kick detector convicts and is right to.

Renders and analyses are memoised per options object, which is what keeps the suite at ~30 s.

All proxy/web tests run offline with mocks: `tests/test_deezer.py` (mock provider),
`tests/test_webui.py` (`MockGW` + `MockApi` cover every `/api` route), `tests/test_detach_guard.py`
(the spy client: what the app may ask Deezer), `tests/test_deezer_resilience.py` (a fake HTTP
adapter that times out / answers garbage, pinning the rules above: the breaker opens and stops
costing sockets, retries stay inside their budget, and no transport failure ever condemns a track or
a show). Note that its circuit breaker is a process-wide singleton — reset it in `setUp` when a test
trips it. `tests/test_deezer_stream.py` runs the audio path for real, where every other test replaced
the download with one writing zeros (which the archive then kept as a FLAC): the key and the cipher
against vectors from Node's md5 and OpenSSL's bf-cbc, the body over a loopback HTTP connection that
delivers it in pieces aligned with nothing — Content-Length and chunked, cut short, refused, and
gated to prove the first block is handed on before the rest has even been sent — through
`open_live_stream` and `/api/stream` to a tagged archive. `tests/net/` hits real services and is
CI-only.
`tests/test_webui.py::GenreStudioTestCase` covers the studio's API without onnxruntime (which is
the normal install): the vectors it reads are written by `struct`, so the whole tagging/training/
shipping path is exercised on a stock server. It also covers the extractor's upload/delete round
trip and that the operator's `embed_model` is never deleted. Its "a head that does not fit is
refused" test deliberately logs a traceback — that is the refusal working.
`tests/test_remote.py` pins the remote control's grant: every `/api` endpoint classified (a new route
fails it until it is), each level reaching exactly its endpoints and commands, no level reaching the
links or the host's side, a cut / an expiry / the owner's new password ending the lent sessions on
their next request, the controller's own account handed back, stale commands dropped, ids that
could steer a request path refused, a decompression bomb refused. `webapp/test/remote.test.mjs`
drives two real player stores (stores.js imported twice) and requires the controlled one to end up
where the controller's tap showed it, for every transport and queue button.
`tests/test_party.py` pins the listen party's capability model (only what the host plays is
reachable, a guest never learns another's listener id) and cuts real noise audio with ffmpeg to
check every chunk lands at zero lag and every seam's overlap is sample-identical (skipped without
ffmpeg). `webapp/test/latency.test.mjs` pins the output clock on contexts modelled on what Chromium
measured (bursty `currentTime`, a timestamp that repeats, a mapping that steps and drifts), the
element model, the automatic look-ahead's rule and the DelayNode's ramps on a recording
AudioContext. `webapp/test/party.test.mjs` drives the guest scheduler in virtual time against a recording
AudioContext (`test/partymock.mjs` replays the automation the way the audio thread does) and asserts
on what is AUDIBLE at each server instant: seams, re-anchors, pause/resume without replay, predicted
handovers, crossfades, late chunks, mapping steps, repeat-one, skips to the announced next / mid-fade
/ through a loading state, rewinds, and a seeded walk through all of them asserting that only the
host's track is ever audible once the guest has heard of a move; the host's rules against a model
of what Player.svelte's store and element really do on a skip and a handover; and the clock estimator under
simulated asymmetric, heavy-tailed queueing and 60 ppm skew, held to the measured numbers.
Add a test alongside these when touching the proxy or `/api`.

**What makes a Python test worth keeping** — each rule below replaced a test that passed while
the code under it was broken:
- **Wait on the event, never on the clock.** The watcher tests wait for a processing pass to
  finish with nothing pending (`_settle`), the cache tests move a fake clock, the daemon test waits
  for its socket. A fixed sleep is both too long and, on a loaded CI machine, too short — and an
  assertion that something did NOT happen is only meaningful once the thing that could have done
  it has run.
- **Count, don't time.** "The import is fast" is a statement budget (`count_statements` in
  `tests/test_deezer.py`), not `elapsed < 15`: exact, machine-independent, and an N+1 query moves
  it by thousands. The time bound it replaced could not see a re-sync rewriting every row.
- **Pin the clock a value is derived from — a cache key's day, a decaying weight's age — and write
  that instant as a literal.** Both flavours bit on the night of 2026-11-16. `tests/test_moods.py`
  cached its catalog per (person, day) while `DAY = moods.now().date().isoformat()` was read once at
  import and the route resolved "today" itself: CI run 43983638806 started at 23:57:23 UTC,
  imported the module on 16 November and failed at 00:00:58 with `Counter({'SELECT': 2})` — one
  login read plus the rebuild — and the ids assertion behind it would have compared yesterday's
  day-seeded draw with today's. `tests/test_explorer_styles.py` compared a star's weight with a
  play's at exactly 2.5 while `listening.activation` fades a play by `(hours + 1) ** -0.5`: one
  second of `setUp` spent on a loaded runner gives `(1 + 1/3600) ** -0.5 = 0.9998611400395998`.
  Both fixtures now pin every clock the value reads (and stamp the star's date, whose model default
  is the wall clock) to one literal instant, so the gap is zero by construction; a guard test
  (`assertEqual(moods.now(), FROZEN)`) fails the moment a pin is removed — at any hour, where the
  count only moved once a day. The counts stay exact: a route that rescans moves them to 2 and 5,
  asserted against that mutation.
- **Attribute a job's calls to the thread that ran it, and keep the promise global.** A test that
  counts "what this import asked" cannot read one flat list: the app boots background workers
  (`neezer/web.py:93` starts `credits-resume`, asleep for 45 s), a `partest` batch outlives that
  delay, and the guard's class-wide `DeezerProvider.dz` patch makes a worker left behind by an
  earlier test report into the current spy. `tests/test_detach_guard.py` then saw
  `gw.get_tracks_data` in a window where the import had only read its lists
  (`trackcredits._Backfill._answers` → `provider.get_tracks` → `gw.get_tracks_data`), reproduced by
  starting that worker inside the window — the call arrives on another thread — and confirmed by
  the same CI job logging "Credits backfill crashed … Cannot use uninitialized Proxy"
  (run 43990956743): the worker outliving its database. The cure is attribution, never a wider
  allowance: the job's own calls (its thread — `libraryimport.run` is synchronous) are held to
  `LIBRARY_READS`, and the promise the test exists for, no worker reads the ACCOUNT, is asserted
  for every thread over the whole window. A real account read added on purpose outside the job —
  from another thread, or one the job never makes — still fails it.

  `_MAX_OUTAGE_RETRIES + 1`): write the literal. And keep a test's own timeout below the cap it is
  checking, or "it waited the cap out" passes as "it did not wait".
- **Real ffmpeg, physical answers** (`tests/test_ffmpeg_integration.py`): the command lines run
  against a real ffmpeg on lavfi-generated signals whose answer is known (a 1/80-scale 1 kHz sine is
  −41.07 LUFS; kicks at 174 BPM read 174). Transcripts of stderr pin the parsing; only this pins that
  the command still means what the parser assumes — it is how the untrimmed trailing silence was
  found. It is also where the phantom tempo of a held note was recorded, as an `expectedFailure`
  with its numbers, until the attack gate fixed it — now it pins the gate from both sides: held
  tones and a chord measure no tempo, a kick train mastered flat (tail at −2.2 dB) keeps its 150.
- **A canonical id already owned by another account** (`tests/test_playlist_collab_edges.py`):
  the second importer must get a derived id and leave the first row's tracks, members and version
  untouched; the same suite pins the 60/h creation and 30/10 min member-write limits.
- **Playwright routing bypasses Chromium's HTTP cache** (`webapp/test/recap/run.mjs`): a
  304/revalidation bench must run without `page.route` interceptors, and `request.headers()`
  hides `If-None-Match` where `allHeaders()` shows it. Count covers **by the URL the screen's
  payload names**, never globally: the shell's playlist thumbnails otherwise inflate the number.
- **A bench that checks a seek must answer byte ranges** (`webapp/test/podcast/run.mjs`): a
  `/api/stream` fulfilled with one 200 body and `Accept-Ranges: none` let the player read 10:00
  for a tick, then reset to 0:00 when the metadata landed. Answering 206 with `Content-Range`
  kept `currentTime` at 10:00 and advancing — the same contract the real server serves. A seek
  assertion on a whole-body stub measures the stub, not the player.
- **Real chapter fixtures** (`tests/test_chapters.py`): ffmpeg writes a 1 h MP3 carrying ID3
  `CHAP`/`CTOC` and an M4A carrying `chpl`; the mutagen extractor is measured on both (0.25 ms per
  file) instead of trusting a hand-built tag dict. The same suite pins that a corrupt tag, a
  symlinked sidecar outside `Podcasts/` and a hostile chapter URL all answer `none`, never a 500.
- **The engine production runs cannot be rehearsed on SQLite** (`tests/test_schema_external.py`):
  the SQLite suite rebuilds database files with `sqlite3`; it never parses a PostgreSQL or MySQL
  migration, never measures its backfill, and never reads the shape through `information_schema`.
  The external module boots a real PostgreSQL 16 and MySQL 8, replays `20261114..20261121` twice
  on populated tables, upgrades seeded `20261113` and `20261118` databases, and drops the shape to
  exercise the boot repair. It drops every table between cases and refuses a database whose name
  lacks "test". Measured: 7 tests in 0.88 s on PostgreSQL 18.6 locally; the MySQL leg is CI's
  proof. `NS_REQUIRE_DATABASE=1` turns an absent URI into a failure, so the database job can never
  pass by skipping.
- **Check the test by breaking the code.** Every test added for these rules was run against a
  deliberate mutation of the line it guards; a test that survives its mutant is decoration.
- **Isolate optional providers.** A test that builds a source order or a list must switch off the
  optional providers it did not mean to include. The YouTube extra is installed on some machines
  and in the image, not in the minimal CI job, so its presence silently changed a fallback rank
  (`tests.test_source_order`).
- **Tests that write files write them in their own temp directory**: `tests/assets/folder` is
  scanned by several modules, from several processes under `partest`.
- **A test's fixtures carry no more privilege than the test needs, and no fixture credential
  travels in a payload.** The fake `whisper-cli`/`ffmpeg` are `0o700`: the test runs them as the
  user that wrote them, so the group and the world never reach them (CodeQL
  `py/overly-permissive-file`).
  Every account `tools/perf_api.py` seeds takes one published fixture password
  (`BENCH_PASSWORD`, the `bench` login every bench already types), and **no credential travels in
  the READY payload**: that line goes down a pipe, and CI keeps what a pipe carried (CodeQL
  `py/clear-text-logging-sensitive-data`). A bench that creates the account itself makes the
  password up per run instead of reading one out of a payload (`webapp/test/onboarding/run.mjs`
  fills the invite form with `randomUUID()`). And a fake that decides which requests it answers
  matches the URL's HOST, never a substring — `urlsplit(url).hostname == "media.deezer.com"`,
  because `?media.deezer.com` in a query is not the edge (CodeQL
  `py/incomplete-url-substring-sanitization`).
- **A bench that writes in the temporary directory removes it, and proves it does.** On most
  machines `/tmp` is a tmpfs — a leftover is RAM, not disk. On 2026-11-16 hundreds of `nsperf-*`
  libraries from `tools/perf_api.py` (5 MB for a small seed, 2.3 GB at 20 000 tracks) filled a
  shared tmpfs to 19 GB of 24 until the user quota answered EDQUOT for everyone: the tool deleted
  its directory only when it reached the end of its own code, and every browser bench stops it
  with SIGTERM. It now removes the library on every exit it can catch (`atexit` for a return, an
  exception and `sys.exit`; handlers for SIGTERM, SIGINT and SIGHUP) and writes an `owner` file
  naming its pid. SIGKILL cannot be caught, so each launch sweeps the `nsperf-*` directories whose
  owner is gone — never one a living process holds (a serving bench keeps its SQLite database open:
  **4 fds** measured on an idle one, and the 60 s grace on a directory with no owner file covers a
  peer between its `mkdtemp` and its own owner file). Launchers therefore stop a bench with SIGTERM
  and keep SIGKILL as the fallback (`stopServer` in `webapp/test/harness.mjs`, the desktop benches'
  `Drop`), and a bench that spawns its own temporary file — `webapp/test/requests/run.mjs`'s WAV —
  removes that too. Measured 2026-11-17: **20 of 20 SIGTERM launches left 0 directories, 20 of 20
  SIGKILL launches left exactly one and the launch after each left 0**, and a bench kept alive while
  another started kept its library and kept answering `200 /api/version`. `tests/test_perf_api_leaks.py`
  (7 tests, 4.1 s) pins all of it, each of its three safety rules checked against its own mutation.

## A timing benchmark is skipped where the machine is shared, never deleted

**A BOUND MEASURED ON A SHARED RUNNER MEASURES THE RUNNER** (owner decision, 2026-11-24: « on vire
les tests de perf en CI »). Six of the suites' timing tests failed only that way, and every one of
them passed on a rerun of the SAME commit — the proof that it was the machine: `theBarsSceneFitsIn
OneFrame` p95 **16.91 ms** against 16.7 on attempt 1 of run 45248683801, `theScopeSceneHoldsThirtyFramesASecond`
**33.67** then **40.40 ms** against 33.4, the 200 MiB loopback fetch **36 MB/s** against 40,
`a_guest_hears_the_host_within_a_millisecond` **−1 650 ms** (the host's own descheduling, not the
code), the vizcore scope `5 of 7` rounds, and `theTapToSoundDelayIsMeasuredOnOneClock` "trial 11
never rendered a sample". Each now declares itself skipped where the hardware is shared: `CI` set
(GitHub sets it everywhere) in `webapp/test/vizcore.test.mjs`, `desktop/crates/nz-lan/tests/fetch.rs`
and `desktop/crates/nz-party/src/tests/end_to_end.rs`; the software rasteriser / generic emulator
fingerprint in `VizDeviceTest` and `NowPlayingDeviceTest`.

The BODY is never touched and no assertion is removed: `npm test` without `CI`, `cargo test` on a
workstation and a real phone still run every one of them, and a skip is printed, not silent. This is
the one place a green check does not mean "measured"; the number that replaces it is the run on
dedicated hardware that the area note records. Skipping is for timing comparisons only — a
functional test that happens to be slow still runs on CI.

## CI tests what ships

Owner, 2026-11-25 (« revérifie entièrement les workflows, je ne veux plus voir ce genre de
soucis »): two regressions in a row shipped in v1.6.5/v1.6.6 with every job green — R8 stripped the
TLS verifier, then a crash at the first cover render — because the artifact a person installs was
built and never RUN in the form it ships. The rule below is now the bar for every workflow, and the
table is the inventory it is held to: for each workflow, what a person downloads and what CI actually
does with that same file.

**A shipped artifact is tested in its shipped form.** The APK, the image, the binaries and the
packages are launched and walked as a person gets them (release / minified / signed, the real image,
the installed package), through the first user path, never only built; a `v*` tag may not publish an
artifact in a form CI never ran.

| Workflow | What ships | What the workflow does on the shipped form |
|---|---|---|
| `tests` | the Python server (inside the image and the packages) | `tools/partest.py` on two Python versions (image + minimal), and the migrations replayed on a real PostgreSQL 16 and MySQL 8 (`NS_REQUIRE_DATABASE=1`, so an absent engine fails instead of skipping). Providers' real endpoints are the weekly `network` run. |
| `webapp` | the SPA build (`neezer/webui/dist`, bundled into the server and the image) | `npm test` (node, the shipped WebAssembly analyser) then `npm run build` — the dist the server serves. The browser benches (`webapp/test/{clicks,ui,requests}/run.mjs`) are run by hand on the area they cover, not by this workflow. |
| `docker` | `ghcr.io/fnyaker/neezer` (amd64 + arm64) | builds amd64 and runs `docker/smoke.sh` (boot, admin bootstrap, `/app/` serves the SPA, a real `/rest/ping` login, a wrong password refused); then **pulls the published tag back (linux/amd64) and re-runs the same smoke**, so the manifest a person pulls is proven, not a lookalike. |
| `android` | `neezer.apk` (the WebView app) | release-variant unit tests, lint, the structural verify (signature v1/v2/v3, 4-byte alignment, package id), and now **the release APK installed and driven on a headless emulator**: setup screen → point it at `tools/perf_api.py` over the emulator's `10.0.2.2` bridge → « Se connecter » → MainActivity → the WebView fetches the SPA (a `/app/*.js` request in the server's log), empty crash buffer. The bench server is plain HTTP: this app carries no verifier of its own, so the TLS path is `mobile`'s. |
| `desktop` | the four release binaries (packaged by `release`) | fmt, clippy, `cargo test --workspace`, the **debug window** under Xvfb + lavapipe, the **release binary's window** (`--self-test-quit-after-first-frame`), and the CPU floors on the engine children those binaries run. |
| `mobile` | `neezer-native*.apk` (minified release, signed with the stable key) | the Rust and JVM suites, **`release-journey.sh` on the signed-in minified APK** (eight doors, car-mode settings, empty crash buffer), the **general R8 `-keep` check** (every module's rule, unrenamed in `mapping.txt` and present in the shipped dex), and the instrumented suite on the debug build with **the bench servers its `assumeTrue` gates need** (tone server on 4711, bench on 5610), so 8 tests measure instead of skip. |
| `bughunt` | nothing (the harness) | `webapp/test/bughunt/run.mjs` on the built SPA and the bench server, plus `mobile/tools/bughunt/explore.sh` on the **release APK of the last green `mobile.yaml`** (downloaded, never rebuilt). A report, never a gate: `workflow_dispatch` and a nightly `schedule`, no `pull_request`; the artifact is uploaded `if: always()`. |
| `release` | the Arch, deb, rpm, AppImage and Flatpak packages, the four binaries, `docker-compose.yml` | each package is **installed and its app started** under Xvfb (deb/rpm/AppImage/Flatpak already, Arch now via `pacman -U` + lavapipe); the binaries' CPU floors and dispatch run on the packaged files; the docker image is `docker.yaml`'s. |
| `network` | nothing | `tests/net` against the real third-party services — Last.fm, ListenBrainz, LRCLIB, and Deezer/YouTube link imports — on a weekly cron, once, as a red run instead of a `continue-on-error` nobody read. |
| `security` | nothing | CodeQL, Semgrep, Bandit, Trivy, Gitleaks — every job `continue-on-error`, so its check can be green with findings: the finding is read and fixed, the check is never trusted as the answer. `cargo-audit` is the exception (never `continue-on-error`). |

Two gaps are stated rather than hidden. The SPA's browser benches are not in a workflow: they need a
server and Chromium and are run on the area a change touches (the `webapp` section of
[commands.md](commands.md)). The one exception is the bughunt walker, which has its own nightly
`bughunt` workflow — it is a report to read, not a gate: it never runs on a pull request. And `tests`/`network`/`security` ship no artifact of their own, so they
are the supply side of the rule, not a place to look for a released file.
