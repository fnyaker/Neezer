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
- **Pin the clock a cache is keyed on, and write its day as a literal.** A value cached per
  (person, day) is compared on a day, and the route resolves that day from the wall clock while a
  fixture usually captures it once at import: a suite that starts before midnight and reaches the
  route after it compares two different days, and both symptoms are real — a rebuild inside a
  statement count, and a different day-seeded draw. `tests/test_moods.py` had exactly that:
  `DAY = moods.now().date().isoformat()` at module scope, the route calling `moods.catalog(user)`
  itself. CI run 43983638806 started at 23:57:23 UTC, imported the module on 16 November and
  failed at 00:00:58 — `Counter({'SELECT': 2})` where 1 was the login's user read and the second
  was the rebuild, and the ids assertion behind it would have compared yesterday's draw with
  today's. The fixture now pins the one clock `moods.catalog` reads (`grep 'now()'
  neezer/moods.py`: one call) to a literal instant, and a guard test (`assertEqual(moods.now(),
  FROZEN)`) fails the moment that pin is removed — at any hour, where the count only moved once a
  day. The counts stay exact: a route that rescans moves them to 2 and 5, asserted against that
  mutation.
- **Never compute the expectation from the constant under test** (`cleanup.GB`,
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
