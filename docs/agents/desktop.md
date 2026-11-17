# Desktop app (Omarchy Linux, GPUI)

Reference notes for agents working on `desktop/`. Read [CLAUDE.md](../../CLAUDE.md) first: its
process and seven rules apply here. Other areas: [docs/agents/](./). The server contract this app
speaks is the web app's ([webapp.md](webapp.md), [server.md](server.md)); the audio rules are the
player's ([playback.md](playback.md)); remote control and the listen party are
[party-remote.md](party-remote.md); the native player it parallels is [android.md](android.md).

**Status (2026-11-17): D0, D1, D2, D3, D4, D5, D6, D7, D8a, D8b, D9, D10b, D11, D12, D13, D14a,
D14b, D15a, D16, D17, D18, M1, P1, P2, T1, T2, T3, T4 and V1 delivered, plus the D14a/D8b follow-up (silence
trim, saved sound at `AudioReady`, `409` told apart, the `auto` payload, the output list) and the
contract follow-up (Lyrics, GenreScore, `radio_next`, the podcast archive answer, the onboarding
route), plus the v1.6.1 scroll follow-up (the shared list's scroll handle, a page's tail bounded,
the credits page, the history's keys); the rest planned.**
This note is the plan the coding lots follow, in order. Each lot, when it lands, replaces its
plan paragraph by what was learned (rule, why, number) — a delivered lot is not a changelog.

## The need

A person on an Omarchy laptop (Arch + Hyprland, keyboard first, a 2560×1600 240 Hz panel, often on
Wi-Fi, sometimes tethered to a phone) wants Neezer as a real app: it starts at once, every click
answers in the same frame, it scrolls an 8 000-track library at the display rate, it plays gapless
and crossfaded through PipeWire, media keys and the bar's player widget drive it (MPRIS), it keeps
playing when the network drops, and it is entirely usable with the keyboard OR the mouse. It has
the SAME client features as the SPA (`webapp/`) and the Android app — not a subset — except the
visualisations, which are only the **bars** and the **oscilloscope**. On top, it transcribes
podcast episodes **locally**, on the GPU through Vulkan, in the server's segment format, as a
background job that never costs the music a sample.

## Research (sources) and what it decided

Every choice below was checked against several sources and then **built together on this machine**
(throwaway prototype in `/tmp`, never committed; Rust 1.99.1, AMD Radeon 780M, PipeWire 1.6.9).

| Topic | Sources | Decision |
|---|---|---|
| UI toolkit | [gpui-omarchy](https://github.com/huacnlee/gpui-omarchy) (README, `docs/design.md`, `src/motion.rs`, `Cargo.toml`), [gpui-kit](https://gpui-kit.com/), [Zed](https://github.com/zed-industries/zed) `crates/gpui` (`elements/animation.rs`), [gpui book: animations](https://matinaniss.github.io/gpui-book/animations/animation.html) | `gpui-omarchy = "=0.2.1"` + `gpui-kit = "0.7"` (locked 0.7.3). Omarchy theme followed live, square controls, `focus_scope` Tab traversal, `virtual_list`. No gpui-component facade: gpui-omarchy disables it on purpose; two design systems in one window is the "default-looking widget" rule 5 forbids. |
| Decoding | [symphonia 0.6](https://docs.rs/crate/symphonia/0.6.1), [symphonia-adapter-libopus](https://lib.rs/crates/symphonia-adapter-libopus), [heise on 0.6](https://heise.de/-11294999), [Psysonic](https://newreleases.io/project/github/Psysonic/psysonic/release/app-v1.34.11) (a Subsonic desktop client on the same stack) | symphonia 0.6.1 (`ogg`, `flac`, `opt-simd`; `mp3`+`isomp4`+`aac` for podcasts) + `symphonia-adapter-libopus` 0.3 (bundled libopus): symphonia has no native Opus decoder. |
| Output | [cpal 0.18 README](https://docs.rs/crate/cpal/0.18.2), [cpal pipewire host](https://docs.rs/crate/cpal/0.18.0/source/src/host/pipewire/mod.rs) | `cpal = { version = "0.18.2", features = ["pipewire"] }`, PipeWire host first, ALSA host as fallback. |
| Gapless / crossfade | [playback-rs](https://docs.rs/crate/playback-rs/0.4.3/source/docs.md) (one playing + one queued song), [Musicat discussion](https://github.com/basharovV/musicat/discussions/6) (decoder thread + ring buffer, no flow control), [Symphonia case study](https://cherbie-github-io.vercel.app/blog/symphonia-case-study) (back-pressure), [pleezer](https://docs.rs/crate/pleezer/0.14.2/source/src/player.rs) (Rust Deezer player) | Two voices (current + next) decoded ahead into a lock-free ring, mixed in the output callback; the crossfade/trim RULES are the web player's (`Player.svelte`, [playback.md](playback.md)). |
| MPRIS | [MPRIS 2.2 spec](https://specifications.freedesktop.org/mpris-spec/latest/), [mpris-server](https://docs.rs/crate/mpris-server), [ferrosonic](https://docs.rs/crate/ferrosonic/0.5.3/source/src/mpris/server.rs) (Subsonic TUI using it) | `mpris-server = "0.11.0"` (zbus 6) on a tokio `LocalSet`. Omarchy's media keys call `playerctl`, which speaks MPRIS. |
| Transcription | [whisper-rs](https://github.com/tazz4843/whisper-rs) (moved to Codeberg), `whisper-rs-sys` 0.15 `build.rs` (read), [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (Vulkan `GGML_VULKAN`, built-in Silero VAD `--vad`), [vad-speech-segments](https://gitea.mueller.network/extern/whisper.cpp/src/branch/master/examples/vad-speech-segments), the server's own `neezer/transcribe/` and `docker/transcribe/worker.py` | `whisper-rs = "0.16.0"`, feature `vulkan`; VAD through whisper-rs's `WhisperVadContext` (Silero GGML) — **not** `FullParams::enable_vad`, which is a no-op there because `WhisperState::full` calls `whisper_full_with_state`, where whisper.cpp does not run VAD; same model ladder and segment rules as the server. |
| Time stretch (podcast speed) | [signalsmith-stretch](https://crates.io/crates/signalsmith-stretch), [rubberband](https://crates.io/crates/rubberband) | `signalsmith-stretch` (MIT, no system library); the SPA's `playbackRate` keeps pitch, so must we. |
| Packaging | [ArchWiki Rust package guidelines](https://wiki.archlinux.org/title/Rust_package_guidelines), the `omarchy-app` packaging convention (an Arch package so it shows in the launcher) | `PKGBUILD` with `cargo fetch --locked --target host-tuple` in `prepare()`, `--frozen` build. |
| Packaging, the other four formats (P2) | [cargo-deb 3.8](https://github.com/kornelski/cargo-deb#readme) (`--no-build`, assets, `depends = "$auto"`), [cargo-generate-rpm 0.21](https://github.com/cat-in-136/cargo-generate-rpm) (packages `target/release` without building), [Debian policy: binary packages](https://www.debian.org/doc/debian-policy/ch-binary.html), [Fedora: RPM `Requires` from sonames](https://rpm-software-management.github.io/rpm/manual/dependencies.html), [AppImage manual packaging](https://docs.appimage.org/packaging-guide/manual.html) and [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy) (what it bundles, and `--exclude-library`), [Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html) and [flatpak-builder manifest](https://docs.flatpak.org/en/latest/flatpak-builder-command-reference.html) | One release build (`binaries` job) packaged four ways by the distribution's OWN tool (`dpkg-deb`, `rpmbuild`, `appimagetool`, `flatpak-builder`); each job installs and starts what it built. `cargo-deb`/`cargo-generate-rpm` were read and not used: both want their metadata in one crate's `Cargo.toml` and the former wants to run `cargo build` itself. |

**Measured on the prototype** (release build, this machine):

- The whole stack resolves and builds in ONE `Cargo.lock`: gpui-omarchy/gpui-kit 0.7.3 (gpui-pre
  0.3.9, wgpu 29), symphonia 0.6.1 + libopus adapter, cpal 0.18.2 (PipeWire), mpris-server 0.11,
  reqwest 0.13.5 (rustls, cookies, stream, gzip), tokio 1.54, rusqlite 0.43 (bundled SQLite 3.54.1),
  whisper-rs 0.16 (Vulkan). Full release build from scratch: ~2 min on 16 threads.
- A GPUI window with an omarchy panel and 64 animated bars on a `canvas`: **232 fps** on the
  240 Hz panel over 3 s, **80 MB RSS**.
- Decoding: Ogg Opus 320k stereo **524× real time** (60 s in 114 ms); FLAC **3 814×**.
- cpal PipeWire host: default sink 2 ch, 48 000 Hz, F32, buffer range 32–2048 frames. ALSA host also
  present.
- MPRIS: `org.mpris.MediaPlayer2.neezer_proto` registered on the session bus.
- whisper `ggml-base`, 330 s of speech: **Vulkan 44.9× real time (7.35 s), CPU 13.9× (23.7 s,
  4 threads)**, identical text; on 11 s: 12.4× vs 5.3× (the first GPU call compiles pipelines).
  Device: `AMD Radeon 780M (RADV PHOENIX)`, uma, fp16, `KHR_coopmat`.
- The repository's own crates build natively as path dependencies: `webapp/rhythm` (`viz_bars`,
  `viz_scope`, the analyser), `webapp/appcore` (sync estimators, track index, fold),
  `android/nscore` (`dsp::Chain`: gain, fade, 10-band EQ, bass).

**Pitfalls found by building, not by reading:**

- **gpui-omarchy 0.2.1 on crates.io depends on gpui-kit `^0.7.1`; its git `main` already moved to
  gpui-kit 0.9.0.** Asking for both 0.2.1 and gpui-kit 0.9 does not resolve (`gpui-pre =0.3.8` vs
  `=0.3.11`). Use the published pair; read the API in
  `~/.cargo/registry/src/*/gpui-omarchy-0.2.1/examples/` (`hello.rs`, `gallery/`), NOT the GitHub
  README, which documents 0.9 signatures (`scrollbar(…, window, cx)`, rem sizes). Moving to 0.9 is
  its own lot, once a gpui-omarchy release depends on it.
- **whisper.cpp's Vulkan build needs the Vulkan HEADERS** (Arch `vulkan-headers`), not only the
  loader: without them CMake stops at `Could NOT find Vulkan (missing: Vulkan_INCLUDE_DIR)`. It also
  needs `glslc` (Arch `shaderc`). On Linux `whisper-rs-sys` does not read `VULKAN_SDK` itself, but
  CMake's `FindVulkan` does: `VULKAN_SDK=<dir with include/ and lib/libvulkan.so>` works without a
  system package (CI uses that with Khronos `Vulkan-Headers` tag `v1.4.333`).
- **mpris-server's `Player` is a local server**: nothing answers until `player.run()` is polled on a
  `LocalSet`; the first property set otherwise panics inside the crate with `SendError`.
- symphonia 0.6 moved APIs (`Packet::track_id` is a field, `TrackType::Audio`,
  `codec_params.audio()`); 0.5 examples found online do not compile.
- Two zbus majors end up in the tree (5.19 via gpui's accesskit, 6.3 via mpris-server and gpui's
  notify-rust). Harmless; do not try to unify them.
- GPUI renders through wgpu/Vulkan on the SAME GPU whisper uses: transcription must yield to the UI
  too, not only to the audio (see *Transcription*).
- **Any ggml Vulkan enumeration ABORTS the process when no driver is usable.** `whisper_rs::vulkan::
  list_devices()` creates a `vk::Instance`; a machine without `vulkan-radeon`/`vulkan-intel`, or
  `VK_ICD_FILENAMES=/nonexistent`, throws `vk::IncompatibleDriverError` across the FFI boundary and
  the process is `terminate()`d (SIGABRT, exit 134 — measured). So no `--version`, no `probe`, no
  startup path may enumerate: T2 keeps it in the `engine` child, where the crash costs one child and
  becomes the `no_vulkan_device` cause.
- **whisper-rs-sys's CMake build caches its generated Vulkan shaders** inside its `OUT_DIR`. After an
  interrupted or concurrently rebuilt graph, the link fails on undefined
  `mul_mat_vec_id_iq2_s_…_data` / `im2col_3d_f32_f16_bda_len` symbols although the backend is fine:
  `cargo clean -p whisper-rs-sys` rebuilds them (and a stale CI cache can carry the same state).
  Measured again in T1: an *aborted first* build counts as interrupted — with no `VULKAN_SDK` CMake
  stops at `Could NOT find Vulkan (missing: Vulkan_INCLUDE_DIR)`, and the next attempt then fails at
  link, so set `VULKAN_SDK` (headers + `lib/libvulkan.so`) before the first build, not after.

## Architecture

### Workspace

```
desktop/
  Cargo.toml            # [workspace] resolver = "3", edition 2024, rust-version 1.89
  Cargo.lock            # committed
  crates/
    nz-model/           # the server's JSON as Rust types (serde, tolerant)        — no I/O
    nz-api/             # HTTP client, session, lanes (rule 1), errors, endpoints  — tokio + reqwest
    nz-cache/           # local SQLite: API cache, covers, play cache, downloads, settings, queue
    nz-player/          # queue/transport state machine, listen accounting         — pure, no I/O
    nz-audio/           # engine: source, decode, mix (gapless/crossfade), DSP, output, tap
    nz-viz/             # bars + scope geometry from webapp/rhythm                 — pure
    nz-mpris/           # MPRIS server
    nz-transcribe/      # STANDALONE: lib + bin `neezer-transcribe` (whisper, worker protocol)
  app/                  # bin `neezer-desktop`: GPUI shell, screens, keymap, wiring
  tools/dump_fixtures.py
  packaging/            # the five packages: see desktop/packaging/README.md
    arch/               # PKGBUILD, neezer-desktop.desktop, neezer-desktop.svg (shared)
    deb/ rpm/           # dpkg-deb, rpmbuild: metadata, build.sh, test.sh
    appimage/           # AppDir + AppRun + appimagetool
    flatpak/            # fr.neezer.Desktop.yaml, build.sh, test.sh
    lib.sh              # version, the soname table, Xvfb harness (all formats)
```

Path dependencies on the repository's crates, read-only — never fork their code into `desktop/`:
`rhythm = { path = "../webapp/rhythm" }` (analyser, `viz_bars::Bars`, `viz_scope::Scope`),
`appcore = { path = "../webapp/appcore" }` (`sync.rs` for the party, `tracks.rs`/`fold.rs` for list
search and sort), `nscore = { path = "../android/nscore" }` (`dsp::Chain`). A change those crates
need is made in place and keeps the web app's and Android's tests green (their benches compare
frame for frame).

### Threads

| Thread | Owns | Priority |
|---|---|---|
| main (GPUI) | windows, views, input, layout, paint | normal; never blocks: every I/O is a task |
| `nz-net` (tokio, 2 workers) | every HTTP request, MPRIS (LocalSet on its own thread), cache writes | normal |
| `nz-load-*`, `nz-preload-*`, `nz-seek-*` | one per load/preload/seek: open a source, probe it, hand the voice over | normal; owned and joined by the engine |
| `nz-decode-{gen}` | one per voice: read source, decode, resample, push into the voice ring | normal; owned and joined by the engine |
| `nz-audio-spool` | one per network body: pull it into its spool file, at most `read_ahead` in front of the decoder | normal; owned and joined by the engine |
| PipeWire callback (cpal) | mix the two voices, DSP, write the device buffer, copy to the tap | real-time: no allocation, no lock, no syscall |
| `nz-party` | one per listen-party guest session: reads the output clock, asks the `Planner`, hands chunks to `PartyChannel` | normal; owned and joined by `party_guest::Guest` |
| `nz-analysis` | tap → `rhythm` analyser → bars/scope state | `nice 5`; runs only while a viz is visible |
| `nz-bulk` | downloads, cover warming, offline sync | `nice 10`, idle I/O class |
| `neezer-transcribe` child processes | transcription supervisor + whisper engine | `nice 19`, `SCHED_IDLE`, idle I/O, SIGSTOP-able |

**Every thread that borrows `nz-net` is owned somewhere that joins it** (D13's *Pitfalls*): `nz-bulk`
by `Offline`, the engine's four families above by `nz_audio::Engine` (reached from the `NetRuntime`
global's `Drop`), and nothing else uses `Handle::block_on` — a `spawn`ed task is not borrowed, tokio
drops it at shutdown.

### Rule 1 on the device: lanes

Every network request carries a `Lane`. The gates live in `nz-api` and nothing bypasses them:

- `Lane::User` — what the person just did (the tapped track's stream, the screen being opened).
  At most 6 in flight, never waits for another lane.
- `Lane::Prefetch` — what comes right after (the next track's audio, the next screen's data, the
  playing track's art/lyrics/gain in the web app's `ladder.js` order). At most 1 in flight; starts
  only once the current track is playable and no `User` request has been in flight for 250 ms.
  Sends `X-NS-Background: 1`.
- `Lane::Bulk` — downloads, offline sync, cover warming, transcription audio. At most 1 in flight;
  waits for 1 s without `User` traffic; while the playing track is still streaming from the network
  it is capped by a token bucket at 25 % of the stream's measured throughput (min 64 kB/s).
  Sends `X-NS-Background: 1`.
- A background request answered `503` + `Retry-After` (the server's `_admit_by_priority`, or "not
  archived yet, queued") waits exactly `Retry-After`; it is never retried sooner.

CPU and GPU follow the same order: decode and output first, then the UI, then analysis, then bulk,
then transcription.

### Network errors (rule 3)

The SPA's `lib/api.js#req` is the contract; port it, do not reinvent it:

- Connection failure → `ApiError::Offline`; a cacheable GET returns the cached copy at once and
  revalidates in the background; a GET started while known offline does not retry.
- A GET started online retries on connection failure and on 502/503/504: 3 tries, backoff
  `min(400·2^n, 6000) ms + rand(0..250)`. Mutations never retry: the optimistic UI rolls back and a
  toast offers "Réessayer".
- `X-NS-Error` header → final (`ApiError::Provider { status, code, message, retry_in }`): the server
  already classified a Deezer failure; never re-ask.
- `401` → session ended: the login screen, unless the body is `{"error":"remote ended"}` (remote
  control loan ended: `ApiError::RemoteEnded { restored }`).
- `409` → **two different failures, told apart by the server's own header**: `409` +
  `Retry-After` is `ApiError::NotReady { retry_after }` (nothing on disk yet, a writer is at
  work — `neezer/webui/__init__.py#_not_archived`, the only 409 that sets the header), and a
  `409` WITHOUT it is `ApiError::Conflict { message, code, scope }` (a playlist whose `version`
  moved, a track that became available, a group that already exists, a finished file asked for a
  `?start=` slice). Only the first is retried as it was; the second is a verdict, and the screen
  re-reads what it was editing.
- Timeouts: connect 4 s; an API answer 12 s; a stream with no byte for 15 s reconnects (byte range
  when the file is finished, `?start=<s>` otherwise, see *Streams*).
- A screen that cannot load shows the cause and "Réessayer" (the `LoadFailed.svelte` equivalent),
  never an endless spinner.

### Caches (what was fetched once is kept)

One SQLite file `$XDG_DATA_HOME/neezer/desktop.sqlite` (WAL), schema versioned by
`PRAGMA user_version`, migrations idempotent and run at start (rule 7):

```sql
-- v1
CREATE TABLE IF NOT EXISTS api_cache(owner TEXT NOT NULL, path TEXT NOT NULL, body TEXT NOT NULL,
  fetched_at INTEGER NOT NULL, PRIMARY KEY(owner, path));
CREATE TABLE IF NOT EXISTS covers(url TEXT PRIMARY KEY, file TEXT NOT NULL, bytes INTEGER NOT NULL,
  used_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS play_cache(track TEXT NOT NULL, quality TEXT NOT NULL, file TEXT NOT NULL,
  bytes INTEGER NOT NULL, complete INTEGER NOT NULL, used_at INTEGER NOT NULL,
  PRIMARY KEY(track, quality));
CREATE TABLE IF NOT EXISTS downloads(owner TEXT NOT NULL, track TEXT NOT NULL, quality TEXT NOT NULL,
  file TEXT, bytes INTEGER, state TEXT NOT NULL, error TEXT, added_at INTEGER NOT NULL,
  PRIMARY KEY(owner, track));
CREATE TABLE IF NOT EXISTS download_sets(owner TEXT NOT NULL, kind TEXT NOT NULL, ident TEXT NOT NULL,
  title TEXT NOT NULL, added_at INTEGER NOT NULL, PRIMARY KEY(owner, kind, ident));
CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pending_progress(owner TEXT NOT NULL, episode TEXT NOT NULL,
  body TEXT NOT NULL, PRIMARY KEY(owner, episode));
```

v1 landed with three additions this plan could not express, each explained in *D3*: a
`download_members(owner, kind, ident, track)` table (a track in several sets, downloaded once),
`covers.pinned` (a download's art survives the cover LRU), and the indexes the LRU scans need
(`covers_used_at`, `play_cache_used_at`, `api_cache_fetched_at`, `downloads_state`).

- **API cache**: the paths `lib/apicache.js#isCacheable` accepts (same prefixes, same `LIVE_PATHS`
  exclusions), the response TEXT, keyed by user id so two accounts never see each other's copy.
- **Covers**: files under `$XDG_CACHE_HOME/neezer/covers/`, LRU capped at 300 MB; decoded images in
  memory LRU capped at 64 MB; decoded off the main thread, visible-first (`coverqueue.js`'s rule).
- **Play cache**: the streamed audio of played/prefetched tracks, `$XDG_CACHE_HOME/neezer/audio/`,
  LRU capped (default 2 GB, Réglages). Only `complete = 1` files are replayed from disk.
- **Downloads** ("hors ligne"): `$XDG_DATA_HOME/neezer/offline/`, chosen per playlist/album/show,
  never evicted by the LRU; a file is published only once complete (`.part` + rename, length
  checked). Nothing deletes a download but the person.
- Paths never come from the server: a file name is a hash of `(track, quality)`.

### Streams (the exact server contract)

- `GET /api/stream/<id>?q=OPUS_320|OPUS_128|OPUS_64` (absent `q` = FLAC; the server also accepts
  256/192). `<id>` is the track's `deezer_id` field (a Deezer number, or a UUID for uploads, YouTube
  rows and podcast episodes).
- A finished file answers byte ranges (`206`). A first play is generated as it is sent:
  `Accept-Ranges: none`, no length. Before seeking, ask `GET /api/stream/<id>/ready?q=` (cheap,
  never starts work): `ready` → byte range; `offset` → `?start=<seconds>` (one request, the server
  slices; `X-NS-Start` says where it began); neither → seek is deferred until the end of the
  buffered part (the web app's `seekplan.js` rule; [playback.md](playback.md) *A seek is answered by
  the server, or explicitly deferred*). `409` + `Retry-After` = not on disk yet.
- Prefetch of the next track sends `X-NS-Background: 1`; the server then answers `503
  {"queued":true}` + `Retry-After: 30` for a track not archived yet, which means "try later", not
  "failed".
- Quality, Réglages → Lecture: **Auto** (default) / Opus 320 / Opus 128 / Opus 64 / FLAC. Auto
  starts at 320 and steps down per measured stream throughput (EWMA over the last 3 streams):
  ≥ 600 kB/s → 320, ≥ 200 kB/s → 128, below → 64. Never MP3 (the server never sends it).
- `POST /api/listen {id, listened, next_id, context, shuffle}` on every track change with the
  seconds actually PLAYED (pauses excluded, `Player.svelte#flushListen`); skipped while offline.
- Episode position: `POST /api/podcast/progress {episode_id, position, duration, finished}` on
  pause, every 30 s while playing, and at quit; offline it lands in `pending_progress` and is sent
  on reconnection (last write wins).

### Auth and security

- `POST /api/login {"username","password"}` sets the session cookie; `GET /api/me` restores it. The
  cookie jar is a file `$XDG_STATE_HOME/neezer/session.json`, mode 0600, one per server URL.
- The client sends neither `Origin` nor `Sec-Fetch-Site`: `same_origin_guard` lets a request without
  both through (it exists to stop BROWSERS on other origins). Never forge them.
- TLS through rustls with the platform verifier. Plain `http://` is allowed (LAN installs are
  HTTP), with a warning in the login screen when the host is not a private address.
- Never log a cookie, a password, an ARL, a worker token, a lease. The ARL typed in Réglages →
  Sources goes straight to `POST /api/me/accounts/deezer` and is never stored on the device.
- What the person may use comes from `/api/me` (`user.access`, `user.sources`, `user.admin`, and
  `remote` when the session is a remote-control loan): the UI offers nothing the server would
  refuse, and the server stays the enforcement.

### Keyboard AND mouse

Everything is reachable both ways. GPUI `actions!` + one keymap file (`app/src/keymap.rs`), shown by
`?` in an overlay and listed in Réglages → Raccourcis. Every interactive element has a visible
focus border (gpui-omarchy's), Tab/Shift+Tab order follows reading order (`focus_scope`), and every
mouse action has a key:

| Key | Action |
|---|---|
| `Space` | play/pause (outside text fields) |
| `Ctrl+→` / `Ctrl+←` | next / previous |
| `Shift+→` / `Shift+←` | seek +10 s / −10 s |
| `Ctrl+↑` / `Ctrl+↓`, `m` | volume ±5 %, mute |
| `s`, `r` | shuffle, cycle repeat (the SPA's keys) |
| `/`, `Ctrl+K` | search |
| `g h` `g s` `g l` `g p` `g r` `g n` `g ,` | home, search, library, podcasts, releases, now playing, settings |
| `Alt+←` / `Alt+→`, `Esc` | back / forward, close the top sheet or go back |
| `j`/`k` or `↓`/`↑`, `Home`/`End`, `PgUp`/`PgDn` | move in a list |
| `Enter`, `Shift+Enter`, `a` | play from here, play next, add to queue |
| `Menu`, `Shift+F10` | the row's context menu (`trackmenu.js`'s keys) |
| `l`, `d` | like, dislike the focused or playing track |
| `t`, `Maj+T` | download the focused row, or the page's collection / take it back, cancel it or retry (D18) |
| `q`, `f`, `v` | queue panel, full-screen player, visualiser |
| `Ctrl+=` `Ctrl+-` `Ctrl+0` | zoom (rem size, gpui-omarchy) |
| `Ctrl+Q` | quit |

### Motion

GPUI `with_animation` + `Animation::new(d).with_easing(..)` (easing output must stay in [0, 1]).
Durations follow the Omarchy shell (gpui-omarchy `motion.rs`: control colours 120 ms, cursor 60 ms,
popup fade 140 ms, `OutCubic`) and add only what a player needs: screen push/pop 180 ms (slide 24 px
+ fade), now-playing expand/collapse 240 ms, queue reorder 160 ms, progress bars continuous. A
reduced-motion setting drops every slide to a 90 ms fade. Animation never delays input: a click acts
on press, the animation follows.

## Feature inventory (what "same features" means)

The SPA is the reference: each screen below ports the listed SPA files' behaviour and talks to the
same endpoints (`webapp/src/lib/api.js` lists them all; the server's routes are in
`neezer/webui/*.py`). Android-only behaviours that matter on a desktop are named.

| Area | SPA reference | Endpoints |
|---|---|---|
| Login, session, invite, onboarding (artists, account link/import) | `routes/Login`, `Invite`, `Onboarding`, `components/Onboarding*` | `/login`, `/logout`, `/me`, `/invites/*`, `/onboarding/*` |
| Home (hero, resume, mood cards, releases, recap card, flow) | `routes/Home`, `HomeHero`, `ResumeCard`, `MoodCard`, `RecapHomeCard`, `FlowPicker`, `FlowControls`, `FlowUpNext` | `/flow*`, `/recommend`, `/moods*`, `/releases`, `/discover/prefs`, `/podcast/progress` |
| Search (tracks, albums, artists, playlists, podcasts, transcripts, YouTube fallback, top result) | `routes/Search`, `TopResult`, `TranscriptHits`, `lib/searchrank.js` | `/search`, `/search/podcasts`, `/transcripts/search`, `/youtube/search` |
| Artist, album, credits, person credits | `routes/Artist`, `Album`, `Credits`, `TrackCredits`, `CreditsSheet` | `/artist/*`, `/album/*`, `/track/<id>/credits`, `/credits/person` |
| Library: playlists + folders, favourites, artists, albums, uploads, dislikes, unavailable | `routes/Library`, `LibraryNav`, `FolderSheet`, `MoveToFolderSheet`, `DislikedTracks`, `UnavailableList` | `/me/*`, `/playlist-folders*`, `/unavailable`, `/replace*`, `/tracks/delete` |
| Playlist: view (4 000 rows), edit, reorder, add/remove, members, share link, recommendations, export, import from link | `routes/Playlist`, `PlaylistTracks`, `PlaylistMembersSheet`, `ShareSheet`, `ExportSheet`, `routes/Import`, `SharedPlaylist` | `/playlist*`, `/playlists`, `/import/playlist*`, `/export/*`, `/services/*`, `/shared/playlist/*` |
| Track actions: menu, like, dislike, play next/queue, radio, add to playlist, credits, replace, delete | `ContextMenu`, `lib/trackmenu.js`, `PlaylistPicker`, `ReplaceSheet` | `/favorite*`, `/dislike`, `/radio/*`, `/replace*` |
| Player: queue (4 000), shuffle, repeat, sleep timer, quality, crossfade, trim, normalisation, EQ + presets, bass, mono/balance, output device, speed (podcasts) | `Player.svelte`, `PlayerBar`, `QueueList`, `SleepButton`, `QualityMenu`, `AudioEffects`, `TrimControl`, `SpeedButton`, `lib/stores.js` | `/stream/*`, `/gain/*`, `/gains`, `/audio/edges/*`, `/listen` |
| Now playing: full-screen, lyrics (synced), lyric card share, credits, more menu | `NowPlaying`, `DesktopNowPlaying`, `ImmersivePlayer`, `Lyrics`, `LyricCard`, `NowPlayingMore` | `/lyrics/*`, `/share/*` |
| Visualiser: **bars and oscilloscope only**, palette, intensity, fps cap, eco, scope orientation/colour, full screen | `Visualizer`, `VisualizerSettings`, `lib/viz/scenes/{bars,scope}.js`, `webapp/rhythm/src/viz_{bars,scope}.rs` | — |
| Discovery: flow, track/artist/genre radio, discovery slider, mute artist, releases + shelves + settings, recap (years, story, share), history, moods, playlist recommendations | `routes/Releases`, `ReleaseShelf`, `Recap`, `History`, `DiscoverySlider`, `lib/flow.js`, `lib/genrestation.js` | `/flow*`, `/radio*`, `/releases*`, `/recap*`, `/history`, `/moods*`, `/discover/*` |
| Podcasts: subscriptions, search/subscribe by URL, show, episodes (cursor), progress, auto-archive, markers, chapters, transcript panel + search, start/stop server transcription | `routes/Podcasts`, `Show`, `PodcastCard`, `PodcastEpisodePanel`, `ChapterList`, `ChapterSheet`, `TranscriptPanel`, `lib/podcastProgress.js` | `/podcasts`, `/podcast/*`, `/search/podcasts`, `/podcast/episode/*/{transcript,transcribe,markers,chapters}` |
| Genre studio + genre tree + explorer styles | `routes/Genres`, `GenreLibrary`, `GenreTree`, `GenreTagSheet`, `GenreBulkSheet`, `GenreExtractor`, `ExplorerStyles`, `webapp/trainer` | `/genre/*`, `/explorer*`, `/analysis*` |
| Offline: « Télécharger » on an album, a playlist, the favourites, a podcast, a show, a row's menu and the queue, with its five states; downloads per playlist/album/show, play cache, offline browsing, network indicator | `lib/offline.js`, `lib/playcache.js`, `lib/apicache.js`, `NetworkIndicator`, `lib/trackmenu.js`; Android `OfflineStore.kt`; Réglages → Téléchargements (`screens/settings/downloads.rs`) | `/stream/*`, `/cover/*`, `/download*` |
| Remote control (be controlled, control another player) | `lib/remote/*`, `RemoteSheet`, `RemoteHostChip`, `RemoteBanner`, `routes/RemoteClaim` | `/remote/*` |
| Listen party (host and guest) | `lib/party/*`, `PartySheet`, `routes/Party` | `/party/*` |
| Réglages: Lecture & son, Apparence, Animations, Téléchargements, Découverte, Sources (order, accounts, Deezer ARL, YouTube, access grants, groups), Compte (export services), Utilisateurs (admin), Serveur (archive, rules, cleanup, storage, quotas, migrate, analysis, credits/enrich/similar backfills, automatch, transcription + workers), Assistance (diagnostic log) | `routes/Settings`, `*Settings.svelte`, `AdminUsers`, `MyAccounts`, `SourceOrder`, `AccessSettings`, `ServicesSettings`, `TranscribeSettings`, `JobFailures` | `/settings`, `/sources`, `/access*`, `/groups*`, `/me/accounts*`, `/services*`, `/admin/*`, `/archive/*`, `/storage`, `/analysis/*`, `/transcribe/*`, `/automatch*`, `/youtube/*` |
| Desktop-only | — | MPRIS, single instance, desktop notification on track change (opt-in), local transcription |

Not ported, on purpose: the WebGL worlds, skins, projector and every scene other than bars and
scope; the service worker and the code splitting (a native binary has neither problem); the Android
car mode and APK updater.

## Transcription (`desktop/crates/nz-transcribe`)

A crate that builds and runs on its own (`cargo run -p nz-transcribe -- …`): the app is only one
of its callers.

- **Same output as the server.** Segments `{start_ms, end_ms, text}` in absolute milliseconds;
  600 s chunks with a 2 s overlap; a segment ending inside the previous chunk's overlap is dropped;
  clamped to the chunk; text cleaned exactly as `neezer/transcribe/engine.py#clean_text`
  (`[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]` → space, whitespace collapsed, `^\[[^\]]*\]$` dropped, 2 000
  chars max), at most 2 000 segments per chunk. The local file is the server's sidecar, byte for
  byte: `{"format":1,"episode_id","language","engine","model","engine_version","duration_ms",
  "created","updated","segments"}` written compact (`separators=(",",":")`, UTF-8, trailing
  newline), plus the `.srt` (`sidecar.py#srt_bytes`). Golden files produced by the Python functions
  hold the Rust port to that.
- **Two sinks.** (1) *Server worker*: an admin enrols this PC as a `client` worker
  (`POST /api/transcribe/workers {"name","kind":"client"}` → a token shown once, stored 0600 in
  `$XDG_CONFIG_HOME/neezer/transcribe.token`), then the crate speaks the existing protocol exactly
  as `docker/transcribe/worker.py`: `POST /api/transcribe/worker/claim` (204 = nothing to do) →
  `GET …/worker/audio/<eid>` (`Authorization: Bearer`, `X-NS-Lease`, Range to resume) →
  `POST …/<eid>/heartbeat {chunk_pct}` every 5 s (honour `cancel`; `pause` is only sent to `server`
  workers) → `POST …/<eid>/chunk {start_ms, duration_ms, segments, language, engine:"whisper.cpp",
  model, engine_version}` per chunk → `…/complete` or `…/fail {code, message}` or `…/release`.
  Results land on the server and every device sees them. (2) *Local*: any user (not only admins)
  transcribes an episode on this PC; the sidecar is stored in
  `$XDG_DATA_HOME/neezer/transcripts/<episode_id>.transcript.json` and the transcript panel shows it
  when the server has none.
- **Vulkan, with an explained CPU fallback.** Built with `features = ["vulkan"]`. At start the
  engine probes: Vulkan device list → whisper context with `use_gpu(true)` → a 1 s silent decode. If
  any step fails it falls back to the CPU and SAYS why, in French, with the cause and the expected
  speed: "Transcription sur le processeur : aucun GPU Vulkan utilisable (<cause>). Environ 3× plus
  lent." (measured ratio, 44.9× vs 13.9×). Causes are named: `no_vulkan_loader`,
  `no_vulkan_device`, `gpu_init_failed`, `gpu_out_of_memory`, `user_forced_cpu`.
- **Models** — the server's ladder and file names (`ggml-large-v3-turbo-q8_0.bin` when ≥ 2.5 GB of
  free memory, else `…-q5_0.bin`, else `ggml-small.bin`), plus the Silero VAD GGML model. Nothing is
  downloaded without the person's click in Réglages → Transcription locale, which shows each size;
  downloads go through `Lane::Bulk` to the PINNED URL as it is (`Api::download_url_to`: no session
  cookie sent, the host's `302` to its CDN followed — never through `download_to`, whose
  `{base}/api{path}` join made every model a 404 in v1.6.0), are checked against a pinned SHA-256
  list in the crate, and are validated like `engine.py#valid_model` (GGML magic, ≥ 1 MiB).
- **Audio** decoded in-process (symphonia `mp3`/`isomp4`/`aac`/`ogg`/`flac` + libopus, rubato to
  16 kHz mono): no ffmpeg dependency on the desktop.
- **It yields (rule 1).** Two processes: a supervisor (`neezer-transcribe worker|local`, HTTP,
  heartbeat, chunking) and an engine child (`neezer-transcribe engine`, whisper only, reads a PCM
  file, writes JSON) — the server's own split, so a pause is a `SIGSTOP` of the engine's process
  group while the heartbeat keeps the lease. The app pauses the engine whenever: the audio engine's
  buffered-ahead time falls under 2 s; a `User`-lane request has been in flight for 300 ms; the UI's
  p95 frame time over the last second exceeds 1.5× the display interval while an animation runs
  (GPU contention); the machine is on battery and "seulement sur secteur" is on (default on). It
  resumes after 3 s without a reason. All processes `nice 19`, `SCHED_IDLE`, idle I/O; whisper
  threads = `engine.py#threads` (min(4, cpus/2) on ≥ 16 cpus, else min(2, cpus/2)).
- **Control channel** app ↔ supervisor: JSON lines. App → `{"cmd":"pause"}`, `{"cmd":"resume"}`,
  `{"cmd":"stop"}`, `{"cmd":"local","episode_id","audio":"<path>","language":"auto"}`. Supervisor →
  `{"event":"engine","backend":"vulkan|cpu","device","reason"}`, `{"event":"progress","episode_id",
  "done_ms","duration_ms"}`, `{"event":"done","episode_id","segments"}`, `{"event":"error",
  "episode_id","code","message"}`, `{"event":"idle"}`. The UI shows running, progress, and each
  failure's cause (rule 2); an interrupted local job resumes at its last chunk after a restart.

## Memory — where the megabytes go (measured 2026-11-17)

The laptop this app runs on has 16 GB and the person watches `htop`. This section is the map:
what a Neezer window costs, which part of it is the app's data and which part is the
machine's graphics stack, and what is guarded by a test. Every number is a release build on
`Xvfb :105 -screen 0 2600x1700x24 -dpi 96` with `GPUI_X11_SCALE_FACTOR=1` (never the person's
session), against `tools/perf_api.py --serve` (8 000 tracks, 4 000 favourites, 60 playlists,
`--covers cover.jpg` when sleeves matter — without it every `/api/cover` is a 404 and a screen
paints placeholders), `/proc/<pid>/status` for RSS and `/proc/<pid>/smaps_rollup` + `smaps`
for PSS and the per-mapping breakdown. **RSS is not the app's data**: it is what the process
resident set holds, arenas and driver included. Count bytes when the question is "what does
the app keep" (`app/src/testmem.rs`).

- **The rule, the socle is not the app.** At the LOGIN screen — no request made, no list
  parsed — the release binary already sits at **195.8–202.3 MB RSS** here, and `smaps` says
  where: `libLLVM.so` **48.6 MB** (RADV's shader compiler, PSS 9.8 MB because it is shared),
  the glibc main arena `[heap]` **46.1 MB**, 181 anonymous malloc/driver mappings **36.9 MB**,
  the binary's own text and data **29.2 MB**, `libgallium` (llvmpipe/lavapipe) **13.3 MB**.
  D7's famous **103 MB** is the same app on a real Wayland session with a real GPU: two
  different machines' graphics stacks, not two versions. **Never compare a number measured
  under Xvfb with one measured on the panel**, and never compare with the login screen of a
  different run.
- **The rule, the bisect — D7 to master, one environment, login then home (RSS):**
  `a4c51f5` (D7) **197.8 → 199.6 MB**; `1c59508` (#238, the 146-icon superset) **195.8 →
  197.8**; `3afd1f0` (#241, D8a's browse screens) **198.5 → 203.1**; master (D8b, D13, D9,
  D14a) **201.5 → 206.3**. The whole walk from D7 to master is **+6.7 MB at home**, of which
  the icon superset is **−0.2 MB** (it embeds 53 632 more bytes of SVG and rasterises only
  what it draws). The "182/199/237 MB" of the lot reports are this environment's baseline,
  not a regression: **the +80–100 MB the plan was hunting was already there before D7**.
- **The rule, the window's area is paid in buffers.** The same build, same screen: **201.7 MB
  at 1280×800** and **282.9 / 283.3 MB at 2560×1600** — **+81.6 MB**, twice. A 2560×1600 RGBA
  surface is 16.4 MB, so that is ~5 of them: wgpu's swapchain plus the render targets GPUI
  keeps for the window. Nothing the app can share, and nothing to fix: a bigger window IS a
  bigger framebuffer. It is why every memory criterion is stated with its window size.
- **The rule, a screen's own share is small and paid once.** 1280×800, master, no covers:
  login **201.5**, home **206.3** (+4.8), artist **224.7** (+18.4), back to home **221.2**
  (−3.5), Bibliothèque **222.1**, 4 000 favourites **234.7** (+12.6 for the whole 4 000-row
  data model plus its windowed list). Of the artist page's +18.4 MB, `smaps` puts +12.0 MB in
  anonymous mappings, +3.5 MB in `[heap]` and +1.8 MB in the binary's pages — allocator
  working set, and it does not all come back because glibc keeps arenas. The **live** heap is
  tiny: `LD_PRELOAD=libjemalloc.so.2` with `stats_print` at a clean quit reports **9 708 144 B
  allocated** (9.3 MB) at the artist page, against 218 664 960 B of *mapped* arenas. When
  someone says "Neezer uses 300 MB", 9 MB of it is the app's objects.
- **The rule, navigation must cost nothing — and it cost 161 KB per screen.** 1 000 round
  trips between Accueil and Bibliothèque (2 000 screen builds): **204.4 → 526.1 MB,
  +321.7 MB (+161 KB per screen)** before the fix, **207.5 → 243.8 MB, +36.3 MB (+18 KB)**
  after it. The cause was the screen's reply channel: `events::Sender` never closed when its
  last clone dropped, so the drain task GPUI holds for `Receiver::next()` stayed parked for
  the life of the process, keeping the screen state it had captured. `Sender::drop` now closes
  the channel, the task answers `None`, and GPUI frees it. **Any new screen gets this for
  free** because every screen already answers through `events::channel`; a channel created
  any other way is a bug.
- **The rule, a sleeve is painted at the size it is SEEN.** GPUI uploads a painted image into
  one atlas tile **at the image's natural size**, keyed by the image's id, and nothing
  releases that tile before the window goes away (`gpui_pre`'s `Window::paint_image` /
  `drop_image`); the decoded copy lives in the App's asset cache, which also never evicts.
  A 1 000 px sleeve drawn in a 36 px row is therefore **4 MB of decoded heap and 4 MB of
  atlas for 1 296 useful pixels**, and a 4 000-row list that draws one per row would cost that
  per distinct sleeve — D9's +291 MB for "a sleeve per queue row" is that arithmetic (its queue
  drew glyph rows, so the number was a prediction, not a measurement), and it is the number this
  rule exists to keep off the queue. `nz-cache`'s `Covers::variant` writes a
  downscaled copy once (the bench's 1 753 091 B JPEG becomes **3 306 B at 128 px**, 4 480 B at
  384 px) and `cover::Class` decides the size: **`Class::Row` 128 px** for a screen whose
  sleeves are list rows, `Card` 384 px, `Page` 512 px. Decoded, 1 000² × 4 = 4 000 000 B
  becomes 128² × 4 = 65 536 B: **61×**. A screen that paints sleeves in rows is built with
  `Covers::rows()`; one that paints cards keeps `Covers::new()`. **Every list in the app now
  paints them** — the Bibliothèque (Favoris, playlist cards, album grid), a playlist's rows and
  banner, the queue and the 420 px panel — and the atlas tile is dropped when the row leaves
  (next bullet): the queue's 32 px rows and the library's 36 px rows cost **65 536 B decoded and
  65 536 B of atlas each, held for one screenful**.
- **The rule, what is measured is counted, not read from RSS.** `app/src/testmem.rs` counts
  bytes allocated minus bytes freed **on the test's own thread** (a `#[global_allocator]` that
  is `#[cfg(test)]` only, so the shipped binary is unchanged). The guard is
  `app.rs`'s `building_a_screen_a_thousand_times_keeps_nothing`: 2 000 screen builds keep
  **892 936 B** (446 B each, GPUI's task arena settling later than the test can see), budget
  **2 MiB** — 2.3× the measured weight and 160× tighter than the 161 KB-per-screen leak it
  caught. A screen that starts keeping a list, a response or a decoded image fails it at once.
  `nz-cache`'s `a_sleeve_is_stored_at_the_size_it_is_drawn` pins the resize: 128 px copy,
  idempotent, no upscaling, `None` when there is no art.
- **The rule, a sleeve is freed when the screen stops showing it — and the screen is what says
  so.** `cover::Sleeves` (one process-wide cache) keeps the decoded `Arc<RenderImage>` and drops
  the atlas tile with it, which `Window::drop_image` is the only way to do. What decides "shown"
  is the screen's own declaration (`Covers::show`), REPLACED by each frame's paint and published
  on the next render (`Covers::begin_frame`); an image no screen declares is freed after
  `IDLE_FOR` (2 s), the whole cache is bounded at `DECODED_BUDGET` (64 MB, `coverqueue.js`'s
  cap), and what is over the budget goes least-recently-declared first — a screenful, never the
  list. **Neither the paint nor the virtual list's closure is a witness**: `paint_image` uploads
  by image id, so an evicted image re-uploaded by a later paint would be invisible to us, and
  `v_list` calls its closure TWICE per frame — once for ONE row (measuring), once for the
  viewport — and the measuring row is really painted. A union of the two kept that first row's
  sleeve and atlas tile for the life of the process (20 images for 19 rows, found by the
  journey test); `a_measuring_pass_row_is_not_what_the_screen_shows` pins it.
  **Measured** (release, `Xvfb :214` 2600×1700 `-dpi 96`, `GPUI_X11_SCALE_FACTOR=1`,
  `perf_api --serve --covers` with a real 1 000 px JPEG on 1 333 albums, 8 000 tracks,
  4 000 favourites, 1280×800) — Bibliothèque, 4 000-row favourites, then 1 000 `PageDown`:
  | | Bibliothèque (60 playlists) | Favoris 4 000 | +1 000 défilements |
  |---|---|---|---|
  | before (master's rows drew placeholders) | 213.5 / 155.9 | 230.1 / 172.5 | 230.7 / 173.1 |
  | **with the sleeves** | **240.6 / 182.9** | **251.7 / 194.0** | **259.8 / 202.2** |
  The +21 to +27 MB is paid by the FIRST image the app ever paints, not by a screen: the same
  build on the Artistes tab (no sleeve at all) reads 250.9 / 193.2, and "1 000 défilements" over
  four full traversals went 263.3 → 263.3 → 263.3 MB (+8.1 MB once, then flat). The decoded stock
  is **20 images = 1 310 720 B** (counted, `Sleeves::resident`), and
  `a_thousand_renders_over_four_thousand_rows_keep_one_screenful` holds it there for 1 000
  frames over 4 000 rows, and `cargo test --workspace` is **362 app tests, every crate green**. RSS is not the app's data: at 2560×1600 the same screen is 327.7 /
  264.1 and **349.2 / 285.6** with the 420 px queue panel open (D7's +81.6 MB of window buffers
  is most of it). **Looked at**, on the isolated `Xvfb :214`, seven captures in
  `docs/desktop/neezer-62/`: the Bibliothèque's 60 playlist cards, the 4 000-row favourites
  before and after 1 000 `PageDown`, the queue in the 420 px panel at 1280×800 and 2560×1600, the
  favourites at 2560×1600, and a playlist's banner, rows and “Titres recommandés” strip.
- **The rule, one decode per screenful and no decode for what is below the fold.** A visible row
  asks for exactly one sleeve (`want`, at most `MAX_IN_FLIGHT` = 8 requests in flight), the file
  it gets is the 128 px copy (`Covers::rows`), and the journey test counts it: 18 visible rows
  → 18 requests, 18 decoded images of 65 536 B, **0 requests** for a row 100 below the fold, and
  a sleeve already on disk is never re-asked.
- **Pitfalls found by measuring**:
  - **A screenshot on a 2600×1700 software-rendered root can block `import -window root`**;
    drive a journey from the bench server's own access log (the route a screen asks for) and
    take pictures outside the measured path.
  - **`XResizeWindow` races the app's own initial size**: a resize issued before the window is
    ready is silently lost and the run measures 1280×800 while claiming 2560×1600. Resize
    after the first frame and assert the size you measured.
  - **An unmanaged Xvfb delivers no keyboard until the window is focused**: there is no window
    manager to give it the focus, so `XSetInputFocus` on the biggest ROOT CHILD (the app's
    window — an input-method helper is also a named child, and it is 1×1) must come before the
    first key, or every `g l` is a no-op. A resize MOVES the window to 0,0 and the app stops
    receiving clicks on this build: take the 2560×1600 pictures with the navigation already
    done, and keep the clicks before the resize.
  - **The bench must serve real cover bytes** to measure anything sleeve-shaped
    (`tools/perf_api.py --covers <jpeg>`): with Deezer unreachable every `/api/cover` is a 404
    and every screen paints a placeholder, which reads as "sleeves are free".
  - **This machine is shared.** Other sessions run `pkill Xvfb` and `pkill -f perf_api`: the
    isolated display is a copy of the binary under another name (`xserv105`) kept alive by a
    loop, and the bench runs as `bench.py` for the same reason. A run whose X server dies
    exits 0 and measures a dead process.

## The lots

Rules for every lot (they are part of each worker prompt below):

- One lot = one branch `desktop/<lot-id>-<slug>` (under Orchestrator: the worker's own session
  namespace, e.g. `<session-branch>/<lot-id>-<slug>`) = one PR into `master`; CI fully green (every job,
  including `security.yaml`'s findings) before merge; merge without asking.
- Only the files the lot names. A lot that needs a file another lot owns stops and reports.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` pass in `desktop/` before pushing.
- Numbers are measured, written next to the assertion and in the PR. A UI lot is RUN and looked at
  (window at 2560×1600, at 1280×800, and with the queue/side panel open at 420 px wide).
- French UI copy, no emoji, glyphs through the icon set of `gpui-kit-assets`.
- At the end the lot updates this note: its plan paragraph becomes the rule + why + number.

Dependency graph (→ = must be merged before):

```
D0 ─┬─ D1 ─┬─ D2 ─┐
    │      ├─ D3 ─┤
    │      └─ D4 ─┤
    ├─ D5 ────────┼─ D7 ─┬─ D8a ─┬─ D12, D16
    │     └─ D6 ──┤      ├─ D8b  │
    ├─ M1 ────────┘      ├─ D9 ──┼─ D10b, D11 ─ T4, D15a ─ D15b
    ├─ V1 ───────────────┘       │
    ├─ T1 ─┬─ T2 ─┐              ├─ D13, D14a, D14b, P1, P2
    │      └─ T3 ─┴──────────────┘
    └──────────────────────────────────────── D17 (last)
```

Waves that can run in parallel (disjoint files):

1. **D0** alone.
2. **D1, D5, M1, V1, T1** (five crates, disjoint).
3. **D2, D3, D4, D6, T2, T3** (D2/D3/D4 need D1; D6 needs D5; T2/T3 need T1 and touch disjoint files).
4. **D7** alone (it wires everything and creates every screen stub).
5. **D8a, D8b, D9, D13, D14a, D14b, P1** (each owns its own screen files).
6. **D10b, D12, D15a, D16** then **T4, D15b** (D10b, D11 and D15b are delivered; T4 remains).
7. **D17**.

### D0 — Workspace and CI (delivered 2026-11-16)

- **The rule**: `desktop/` is ONE workspace with ONE committed `Cargo.lock` (887 packages) and every
  version in `[workspace.dependencies]`; a lot writes `foo = { workspace = true }`, never a version.
  A crate proves its stack with a test that TOUCHES it, because a manifest alone links nothing that
  matters: `HostId::PipeWire` fails to compile when cpal loses its feature, and `neezer-transcribe`
  came out at **324 KB** with whisper.cpp left out of the binary until something called into it — it
  is **39 MB** now that `system_info()` does.
- **The rule, window side**: an app-level action is bound GLOBALLY (`KeyBinding::new("ctrl-q", Quit,
  None)`) and handled by `App::on_action`. A `key_context("Neezer")` on the root view is NOT enough:
  GPUI looks a keystroke up along the path of the FOCUSED node, and with nothing focused
  `focus_node_id_in_rendered_frame(None)` returns the dispatch tree's root — above the view — so the
  binding is never found and `Ctrl+Q` does nothing, silently. Screen bindings (D7) go on an element
  that `track_focus`es and holds the focus.
- **What it is now**: nine crates (`nz-model`, `nz-api`, `nz-cache`, `nz-player`, `nz-audio`,
  `nz-viz`, `nz-mpris`, `nz-transcribe`, `app`), one passing test each and the pins of *Workspace*
  above; `neezer-desktop` opens a themed window (`--self-test-quit-after-first-frame` prints
  `first_frame_ms=<n>`) and `neezer-transcribe --version` prints the whisper.cpp build it links.
  `.github/workflows/desktop.yaml` runs `fmt`, `clippy`, `test` (+ the real window under Xvfb),
  `build` (release, both binaries, uploaded) and `audit`.
- **Measured** (this machine: 16 threads, Rust 1.99.1, RADV 780M, Hyprland; CI: `ubuntu-24.04`,
  Xvfb + lavapipe):
  - `cargo test --workspace`: **12 tests, 0 failed**; `cargo fmt --all --check` and
    `cargo clippy --workspace --all-targets -- -D warnings` clean.
  - first painted frame: **94, 96, 97, 97, 101 ms** in release (median 97), 386 ms in debug; CI's
    debug build under Xvfb/lavapipe: **2 782 ms and 4 728 ms** on two runs — that number says the
    window paints on a machine with no GPU, it is not a performance target (D7's 300 ms is a release
    number).
  - release binaries: `neezer-desktop` **25 545 128 B**, `neezer-transcribe` **39 844 208 B**.
  - fresh release build 3 min 39 s here; CI **8 min 30 s cold, 6 min 3 s the run after, 2 min 13 s
    with the Rust cache warm** (warm: fmt 24 s, audit 22 s, clippy 47 s, release 2 min, test 2 min 10 s).
  - the window is real: `hyprctl clients` → class `neezer-desktop`, title `Neezer`, 1280×800; a
    `wtype`-injected `Ctrl+Q` exits 0.
- **What the scanners see of Rust**: Semgrep contributes nothing — its configured packs have no Rust
  rules (346 rules ran: python, js, kotlin, yaml, json, dockerfile), so the Rust surface is
  `cargo audit` on the committed lock plus Trivy's cargo scan (0 findings, 0 code-scanning alerts on
  this lock).
- **Pitfalls confirmed**: whisper.cpp's CMake `FindVulkan` is what reads `VULKAN_SDK` (Khronos
  `Vulkan-Headers` v1.4.333 in `$RUNNER_TEMP` with `lib/libvulkan.so` symlinked) — with it the apt
  list of *D0* above needed nothing added on `ubuntu-24.04`; `xvfb-run` also wants `xauth`; and
  `cargo test` does NOT build the binaries the launch steps run (`./target/debug/neezer-desktop: not
  found`, exit 127): one `cargo build --workspace` between the two.

### D1 — `nz-model` and contract fixtures (delivered 2026-11-16)

- **The rule**: `nz-model` is the server's JSON as types, PROVEN against the server's own answers.
  `desktop/tools/dump_fixtures.py` starts `tools/perf_api.py` (300 tracks, 5 playlists — one of 200
  titles, 120 favourites, a show with 8 episodes and a ready transcript, two years of listening
  history), logs in as `bench`, GETs one route per READ method of `webapp/src/lib/api.js` and writes
  each body verbatim: **102 fixtures, 393 872 B**, committed under
  `crates/nz-model/tests/fixtures/`. `tests/fixtures.rs` reads every file with the type its table
  names, then twice more — with an unknown field added to every struct-shaped object, and with every
  key the type does not truly need REMOVED (each key is tried and kept deleted when the payload
  still parses: **500 of 508 keys** are optional). CI regenerates the set from the commit under test
  and runs the same test on it, so a server payload change that would break the desktop fails the
  desktop job.
- **Why the real bench, not samples**: a hand-written sample only proves what its author believed.
  In one run the fixtures caught four things no plan had: an album's `year` is a four-character
  STRING from a browse card and an INTEGER from the database (same field, two types — hence
  `loose::opt_string`); `moods[].tracks` is a COUNT while `mood.tracks` is a LIST; a marker's
  `position` is MILLISECONDS while a chapter's `start` is seconds; and
  `genre/status.extractor.onnxruntime` is a BOOLEAN (the runtime is installed), not a version.
- **The rule, ids**: an id is a newtype over `String`, never an integer — the same `deezer_id` field
  carries `"3000000"` for a catalogue row and a UUID for an upload/YouTube/episode row, and
  `/api/stream/<id>` takes both. A field the server always sends stays REQUIRED (a rename must fail
  the test); everything else is `#[serde(default)]` + `Option`, and unknown fields are ignored
  (rule 7). Remote URLs stay relative (`cover`/`picture` are `/api/cover/<id>`).
- **The rule, the seed**: `--podcasts` and `--recap` are part of the command on purpose. Without
  `--recap`, `/history`, `/recap/years` and `/recap/<year>` answer an empty shell and three types are
  only ever proven against `[]`. Three read routes are empty until something has been WRITTEN (a
  listening position, a marker, a public link), so the script does that one write on its throwaway
  database first. What the seed still cannot fill is what needs an archived library: `/api/search`
  (its local half lists only rows whose file is on disk), `/api/flow`, `/api/radio/*`,
  `/api/moods/<id>` and `/api/recommend` answer empty lists there — their envelopes are proven, and
  their item types arrive with the screens that read them (D8a, D12). `perf_api.py --archived`, which
  needs numpy (not a dependency), is what would fill them — the two seeds below fill the two that
  were WRONG rather than merely empty.
- **The rule, a payload that is never EMPTY is never proven**: four fields were wrong because the
  bench always answered `null`/`[]` for them, and a type mismatch on an empty list parses. Two seeds
  close it now — `--seed-lyrics` (an archived track with a real `.lrc`, and a second with plain text)
  and `--seed-genre-scores` (three hand labels plus four v2-width embeddings, so the prototypes rank
  without a trained head) — and the fixtures they feed corrected:
  - `Lyrics` declared the words as a bare STRING; the server answers an OBJECT
    (`{"text", "synced": [{"time", "text"}], "source"}`) — `LyricsBody` now takes both.
  - `GenreScore` declared `{name, score, tag}`; the server sends the TUPLE
    `[label, score, head, proto]` (`genre.py#rank_scores`), `head`/`proto` possibly `null`.
  - `GenreTrack.tag` declared a `String`; `_tag_json` sends the tag OBJECT, so `GenreTag` is a type.
  - `GenreConstruction.missing` declared `Vec<String>`; each entry is `{"id", "bpm"}`.
  - `PartyState` declared the HOST's publish body (`{id, p, t, gap}`); `GET /api/party/<id>` answers
    the guests' PUBLIC timeline (`_public_state`: `track`, `playing`, `buf`, `anchor: {t, p}`,
    `xfade`, `next: {track, at, start, fade, gap}`, `norm`, `chunk`, `listeners`, `now`, `me`). The
    publish body is its own type (`PartyPublish`), and the party's clock is a FLOAT on the wire
    (`server_ms()` → `1794878678797.0752`): a `u64` there does not round, it fails to decode.
  The fixture test's `shrink` had to learn the same lesson: it truncates a list of ROWS to one row,
  but a fixed-arity array is a VALUE (a score tuple), and a 1-tuple is a different question.
  `party-state` is a write-then-read like the three below: the route answers an empty shell until
  its host published a timeline, so `dump_fixtures.py` hosts one on the bench first.
- **Measured** (this machine, debug build, `cargo test -p nz-model`): **32 tests, 0 failed** (28 rules
  + the fixture walks); the **105** fixtures deserialize in **23.3 ms** at the lot, stripping their
  optional keys in **185 ms**, and the whole run is **1.56 s** against the lot's 2 s budget. `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **What it is now**: `nz-model` holds 15 modules (`ids`, `track`, `album`, `artist`, `playlist`,
  `library`, `search`, `podcast`, `transcript`, `player`, `discovery`, `settings`, `remote`, `party`,
  `admin`) and the tolerant helpers (`loose::opt_string`, `Quality::wire/parse`); the `test` job of
  `.github/workflows/desktop.yaml` installs the server, regenerates the fixtures from that commit and
  re-runs `cargo test -p nz-model` on them, after the committed copy has passed.
- **Pitfalls**: regenerate the fixtures, never edit them (the script deletes a file the server no
  longer answers, so a removed route cannot keep passing). The bench's library is random: compare
  TYPES, never values. `perf_api.py` needs its project venv (`pip install -e .`); the bench itself is
  a throwaway SQLite in a temp directory and never touches a real install.

### D2 — `nz-api` (delivered 2026-11-16)

- **The rule**: rule 1 is enforced in ONE place, by SLOTS and a QUIET CLOCK, not by sleeps sprinkled
  at the call sites. A request declares a lane; `Gates::acquire` admits it or parks it, and the
  guard releases the slot when the answer is read. Three waits, each measured rather than assumed:
  User never waits (cap 6 in flight), Prefetch starts 250 ms after the last User request ENDED,
  Bulk 1 s after it — and a Bulk body is additionally paced by a token bucket at 25 % of the
  playing stream's throughput, floor 64 kB/s. A background request the server refuses
  (`503` + `Retry-After`) repeats ONCE, after exactly that delay; a foreground GET gets the SPA's
  three tries and `min(400·2ⁿ, 6000) ms + jitter`; a mutation gets none, ever.
- **Why the quiet clock starts at construction**, not at "no traffic ever": the acceptance test is
  "eight Bulk queued, one User issued → the User goes first", and that must be a fact, not a race.
  An app that only downloads (no tap yet) pays 1 s once, at start; a prefetched next track waits
  250 ms it would have spent decoding the current one anyway.
- **Why the cache is keyed by owner AND path**: two accounts share one disk. `is_cacheable` is
  `lib/apicache.js` ported path for path (10 prefixes, the live-path exclusions, the one
  `track/…/credits` regex), because a desktop that caches a different set either paints a stale
  admin screen or an empty playlist on a dead link. **One deliberate difference from the SPA**: a
  cache hit while the link is known dead does NOT revalidate in the background (the SPA's
  `refreshInBackground`) — that is one doomed request per screen open; recovery is the app's single
  reachability probe, which is also what makes "0 network attempts" countable (measured below).
- **What it is now**: `crates/nz-api` with `client` (`Api`, `ApiBuilder`, `Fresh`, `StreamResponse`),
  `lanes` (`Lane`, `Gates`, constants, `bulk_cap_from_stream`), `error` (`ApiError`, `cause_fr`,
  `backoff`), `session` (`SessionJar`: a `CookieStore` mirrored to
  `$XDG_STATE_HOME/neezer/session.json`, mode 0600, one entry per server origin, `Debug` that prints
  a count and never a value) and `cache_trait` (`ResponseCache`, `NoCache`, `is_cacheable`).
  `endpoints/` holds **248 typed functions** in thirteen modules (`auth`, `accounts`, `discovery`,
  `entities`, `library`, `playlists`, `podcasts`, `player`, `settings`, `admin`, `onboarding`,
  `remote`, `media`): of the SPA's 251 `api.*` names, every one but three — `upload` and
  `genreExtractorUpload` (multipart, see the pitfalls) and `streamReady`, which is `Api::ready`
  because that is the signature this lot pins. Same routes, same bodies, same query encoding
  (`encodeURIComponent`, byte for byte: an accent in a style name must land on the same route).
  `StreamResponse::chunk()` hands out `Vec<u8>` per chunk: reqwest 0.13 does not re-export
  `bytes`, and a lot may not add a version the workspace does not pin — the memcpy is the price.
- **Measured** (this machine, Rust 1.99.1; `cargo test -p nz-api`, **31 tests, 0 failed**, 0.07 s
  for the 21 unit tests, `tests/lanes.rs` **9 tests in 3.1 s**, `tests/bench.rs` **1 test in 5.5 s**
  including the server's startup):
  - **order**: with eight Bulk requests queued and one User request issued, the server's first
    arrival is the User one (`/api/user` index 0 of 9), the tap answered in **< 400 ms** (asserted),
    every background request carried `X-NS-Background: 1`, and no two Bulk requests overlapped.
  - **250 ms**: a Prefetch queued during a 200 ms User request arrives **≥ 250 ms after the answer
    was written** and **< 600 ms** (both asserted on the server's own timestamps).
  - **Retry-After**: a background `503 Retry-After: 2` produced **exactly 2 arrivals, 2.0 s apart**
    (asserted `≥ 2 s`, total `< 5 s`); a `POST` answered 503 was sent **once**; a `503` +
    `X-NS-Error: deezer-unreachable` was attempted **once** and surfaced `Provider { retry_in: 12 }`.
  - **cache**: a cacheable GET while the client knew the link was dead answered `from_cache: true`
    with **0 requests** reaching the server; the same request started *online* and refused (dead
    port) returns `Offline` in **< 400 ms** — one attempt, no backoff.
  - **real server**: `login` → `me` → `my_playlists` (**2** playlists, the seed) → `/version` →
    `stream/<id>/ready` (`ready: false`, nothing archived) → a second cookie-less client `401` →
    `logout` → `me` `401`. That whole round trip is 5.5 s, the bench's own boot included.
- **Pitfalls found by building**:
  - **`reqwest::Method` cannot be used in a `match` pattern** (`Method::GET` is a const of a type
    without structural equality): the retry policy compares with `==`, not `match (lane, method)`.
  - **`Notify::notified()` is lazy.** A future created but not polled has registered nothing, so a
    release landing between the state check and the await is lost and the waiter stalls until its
    deadline (or for ever on the User lane). The gates `pin!` + `enable()` it BEFORE reading the
    state; that is also why the 250 ms/1 s tests are exact instead of flaky.
  - **`tokio`'s `start_paused` needs the `test-util` feature**, and the workspace's tokio does not
    enable it. A lot may not edit `[workspace.dependencies]` (D0 owns it), so the feature comes from
    `nz-api`'s own `[dev-dependencies]`: the quiet periods are measured on a virtual clock, not
    slept through.
  - **A lane slot covers the request, not the body.** The stream releases it once headers arrive;
    a body is bounded by the player's read-ahead cap and, for Bulk, by the bucket — holding a slot
    for a 60 s FLAC would starve the six User slots for no reason.
  - **The `test` job's `cargo test` runs before its Python install.** The bench test starts
    `perf_api.py` itself, so the workflow's "Set up Python" + `pip install -e .` steps moved ABOVE
    "Test the workspace" (the file is outside this lot; T3 needs the same for its worker protocol).
  - **The two multipart routes are absent on purpose**: `POST /api/upload` and the genre extractor's
    model upload need `reqwest`'s `multipart` feature, which is not in `[workspace.dependencies]`.
    They land with the lots that own their screens (D14b, D16), together with that pin.
  - `Service Unavailable` from `_admit_by_priority` and the stream's `{"queued": true}` are the same
    status: the difference is the lane. A User-lane 503 is an ordinary `Http { 503 }` the GET may
    retry; a background one becomes `Busy { retry_after }` and is repeated once only.
  - **The party's two routes are one name apart and read opposite ways**: `POST /api/party/<id>/state`
    is the HOST's timeline (`PartyPublish` → the host view, typed `Party`), `GET /api/party/<id>` is
    the GUEST's public state (`PartyState`, typed now that the model has the right shape). Both were
    `Api::send`/`Api::get` with `Value` because the model's one `PartyState` was the publish body.
  - **A request body is a contract too, and an empty answer hides it.** `Api::radio_next` posted
    `{"seed": "3000123"}` where `radio_next` requires an OBJECT (`{"kind", "id", "artist"?}`,
    `400 invalid seed` otherwise) — the SPA builds that object in `lib/queryradio.js#radioSeed`, and
    the screen that needed a radio refill went through `Api::send`. `Api::radio_next` takes a
    `RadioSeed` now. And `Api::set_podcast_auto_archive` used `send_none`, throwing away a body:
    `podcast_auto_archive` answers `{**_channel(c), "queued": n}`, which is what lets a screen say
    « N épisodes en file » without re-reading the show. It returns `PodcastAutoArchive { queued,
    show }` (`serde(flatten)` keeps the channel payload). Both pinned in `tests/bench.rs`: the typed
    radio seed answers 200 where the bare string is `400 invalid seed`, and the archive switch
    answers 200 with the show's `title`/`episode_count` beside `queued`.
  - **A `409` is not one failure.** The first version read every `409` as `NotReady` ("nothing on
    disk yet"), which made a playlist's stale `version` arrive as "pas encore prêt" and be retried;
    `neezer/webui` uses the status for both. The header is the whole difference — `_not_archived`
    sets `Retry-After: 2`, and no other 409 does — so the classifier now returns `NotReady` only
    with that header and `Conflict { message, code, scope }` without it. `code` matters: the
    refusal sentences of Réglages → Utilisateurs (`last_admin`), Serveur (`unavailable`) and the
    transcription routes are 409s, and they are read through `ApiError::code()`. Pinned against the
    real server in `tests/bench.rs`: an unarchived seek is `NotReady { 2 }`, a `PATCH` with a stale
    version is `Conflict { scope: "content" }`, **one request each**.

### D3 — `nz-cache` (delivered 2026-11-16)

- **The rule**: ONE SQLite file in the data directory (`desktop.sqlite`), WAL, `synchronous =
  NORMAL`, a 5 s `busy_timeout`, versioned by `PRAGMA user_version`; a migration only ever ADDS
  (`CREATE … IF NOT EXISTS`), so replaying v1 is a no-op — which is both the criterion and the
  boot-time repair path. A file written by a NEWER build is REFUSED with its version: an older app
  replaying its v1 over it is how a column disappears. A 0-byte file, a v1 file and an existing
  store all open, and opening an existing store costs **0.5 ms** — invisible in the first-frame
  budget.
- **The rule, WAL**: a reader on a second connection answers in **0.18 ms** (debug; 0.25 ms
  release) while a writer holds `BEGIN IMMEDIATE`. That is the whole reason the journal is WAL, so
  the test holds the write lock open and reads underneath it; in rollback mode the reader waits out
  the writer. It is what lets `nz-bulk` mark a download while `nz-net` opens a playlist, and the
  only SQLite access rule that matters: this crate is called from those threads, never the GPUI
  one.
- **The rule, one statement**: the saved queue is ONE upsert — 4 000 entries (7 fields each,
  843 KB of JSON) in **8.8 ms release, 60 ms debug, 1 statement**. The SPA cuts at 1 000 because
  `localStorage` writes synchronously on its main thread; here the write happens on `nz-net`, so
  the cap is the queue's worst realistic case and 30 ms is a release number (like D7's 300 ms). A
  longer queue is kept as a WINDOW of the full cap, starting a quarter of it before the playhead
  (clamped to the queue's end), never truncated at the tail: what comes next is what the person
  is doing.
- **The rule, eviction**: LRU by `used_at` in two phases — reservations older than 1 h always,
  then complete entries until the cap holds. A reservation being written RIGHT NOW is never taken
  (rule 1: the fetch beats the cache), and its bytes are not counted until `complete` publishes
  it: `bytes` is the file's own length, read back from disk, never the caller's count. Covers and
  audio have their own directory and table, so no cap on either can reach a download; a
  downloaded album's art is additionally `pinned`, which is `lib/offline.js`'s own rule (a sleeve
  prepared for a flight must not be evicted by the 300 MB cover cap).
- **The rule, downloads**: a set and its tracks are ONE transaction; a track in three sets is
  downloaded ONCE (`downloads` keyed by `(owner, track)`) and removing a set takes back only what
  no other set wants. That needs a membership table the plan's DDL did not have
  (`download_members`): the set row alone cannot say which tracks it brought in, and deleting them
  with the set would empty another playlist's download. Files are published complete
  (`<digest>.part` → rename), a missing file downgrades its row to `failed` with that cause
  instead of being handed to the decoder, and an `active` row is still work after a restart.
- **The rule, names**: no path comes from the server. A file is `<FNV-1a-64(track, quality)>`, with
  `.FLAC`/`.OPUS_320`/… for audio and a sniffed `.jpg|png|gif|webp|img` for a cover (a `file://`
  URI handed to MPRIS is read by other programs). FNV-1a because `std`'s `DefaultHasher` may
  change between releases and a cache whose file names move re-downloads everything on an upgrade;
  4 000 tracks put a collision at ~4e-13.
- **The rule, the API cache is the page's**: the same prefixes, the same `LIVE_PATHS` exclusions,
  the same 400 newest entries, bodies kept as the wire's TEXT and keyed by owner. The filter sits
  on BOTH sides (`get` refuses a non-cacheable path as well as `put`), so a caller cannot
  resurrect a stale `/playlist/<id>/version`. `Cache` implements `nz_api::ResponseCache` (the
  trait lives in the client so `nz-api` never depends on SQLite), and a test asserts that the two
  ports of `isCacheable` — the client's and the store's — answer the same for 32 paths: the
  client decides what to ASK for, the store what to KEEP, and a disagreement is a screen empty
  offline or a stale admin one served from disk.
- **What it is now**: `Cache::open(dir)` (resolving the XDG cache directory) and
  `open_with_cache_dir` (the tests never write into the real `$XDG_CACHE_HOME`), `covers()`,
  `play_cache()`, `downloads()`, `kv()`, `queue()`, `pending_progress()` and `statements()`. Every
  read and write goes through one counted helper, which is what makes "one statement" an
  assertion instead of a claim. `reserve` returns `Result<PathBuf>` where the plan wrote
  `-> PathBuf`: a full disk is a cause a screen can print, not a panic. `pending_progress()` is the
  offline podcast position queue the schema already promised (last write wins per episode, sent
  then forgotten one at a time).
- **Measured** (this machine, Rust 1.99.1; `cargo test -p nz-cache`): **49 tests + 2 doc tests,
  0 failed**, 0.10 s debug and 0.04 s release; `cargo fmt --check` and `cargo clippy --workspace
  --all-targets -- -D warnings` clean.
- **Pitfalls found by building**: serde's imperfect derive adds `E: Default` to a generic struct
  with a `#[serde(default)]` field of type `Vec<E>`, refusing `QueueSnapshot<E>` for any entry
  type — `#[serde(bound(…))]` is the fix. Eviction cannot be one threshold: `SUM(bytes)` excludes
  the in-flight `.part`, so a store under its cap returned early and never dropped an abandoned
  reservation (the test caught it, as did a second one: an eviction loop must not equate "no bytes
  to free" with "nothing to clean"). The connection mutex is not re-entrant — holding a
  `cache.lock()` guard and then calling a store method deadlocks instead of panicking, which cost
  one 60 s test run. `PRAGMA journal_mode=WAL` must run OUTSIDE the migration transaction while
  `user_version` is set INSIDE it. rusqlite reports "no row" as an error: a `query_optional`
  helper is what keeps every lookup one readable line.

### D4 — `nz-player` (delivered 2026-11-16)

- **The rule**: the transport is ONE function, `PlayerState::apply(cmd, now_ms) -> Vec<Effect>`, and
  everything a person can do goes through it — the bar, the queue, MPRIS, a remote controller, the
  Flow. No I/O, no clock, no threads: `now_ms` is the caller's and is read for one thing (the listen
  ledger). A command that means nothing (a jump to a row the queue no longer holds, a `next` on an
  empty queue) changes nothing and returns NO effect, so "refused" and "did nothing" cannot drift
  apart. Effects come back in the order the app must apply them — `ReportListen`, `Load`,
  `PreloadNext`, `NeedMore`, `Persist`, `Changed` — each with exactly one owner; `Load` also carries
  a seek (`start_s`), so a remote drag reopens the engine at that offset instead of restarting at 0.
- **The rule, the queue row**: an `Entry` is `commands.js#slimTrack`, NOT `nz-model::Track`. This was
  the lot's one wrong contract: a queue crosses machine to machine, and `slimTrack` keeps `id` (the
  KEY) while dropping `deezer_id` for a library row, which `Track` requires — so a row taken from a
  library list and the same row after a remote round trip would not parse into the same type. The
  five flag fields are `Option<bool>` for the same reason: `slimTrack` omits a key the sender did not
  have, and only an absent/`false` distinction round-trips its bytes exactly.
- **The rule, identity**: the queue is keyed the way `trackkey.js` keys it — the row UUID, else the
  uuid5 of the Deezer id, i.e. the UUID the row WILL have (`ids.track_uuid` is deterministic), so a
  search result and the row it becomes are the same queue entry for de-duplication, un-shuffle and a
  remote `jump` by id. `track_key`/`uuid5`/`sha1` are ported in the crate and pinned to the SPA's own
  output: the fixture's uuid5 (`3000001` → `21408a08-d175-53a4-a02d-733a7bc83812`) is asserted, and
  `commands.js#resolveIndex`'s rule is the one `jump`/`remove`/`move` use (index when it still holds
  that id, else the nearest row carrying it, else nothing — a wrong jump is worse than none).
- **The rule, shuffle and removal**: `Arc<Entry>` rows, so a 4 000-row shuffle is a pointer copy (the
  queue itself becomes the un-shuffle order; measured **38.5 µs** release). One deliberate
  divergence: `stores.js#removeAt` leaves `_orig` alone, so removing a row and un-shuffling
  RESURRECTS it; here the removed row leaves the un-shuffle order too (test
  `un_shuffle_does_not_resurrect_a_removed_row`).
- **The rule, listening**: `now_ms` closes the mark, never a ticker, and only playing time counts —
  the state is constant between two `apply`s, so the wall time since the last one is playing time
  exactly. A restart (`prev` past 3 s, repeat `one`) keeps counting: the SPA resets there and loses
  the seconds heard before it. A backwards clock adds 0.
- **The rule, running dry**: a Flow/radio context is asked for more (`NeedMore`) once PER ROW, when
  that row becomes the last of the queue — not on every later command (an unasked-once top-up is a
  `/api/flow` storm), and the answer is `auto_extend` (append AND advance, `stores.js#autoExtend`);
  anything else stops with `playing: false`, as the SPA does. `clearUpcoming` turns autoplay off so a
  queue the person just cleared is not refilled.
- **What it is now**: six modules (`state`, `command`, `effect`, `shuffle`, `listen`, `lib` for the
  identity helpers), `Command` with one variant per wire op and a constructor per store method
  (`op()` gives the wire name back for a log or a permission check), `PlayerState::apply`,
  `track_ended`, `flush_listen`, `extend`/`replace_upcoming`/`auto_extend` (the SPA's non-remote
  store methods, which the Flow calls), `set_position`/`set_duration` (the engine's 4 Hz position and
  its own duration), `snapshot`/`restore` (the session, `orig` saved as POSITIONS so a 4 000-row
  queue stays a few kilobytes), and accessors for the bar.
- **Measured** (this machine, Rust 1.99.1; `cargo test -p nz-player -- --nocapture`):
  - **63 tests, 0 failed** — 16 lib, 40 rules, 5 wire, 2 perf. `cargo fmt --check` and
    `cargo clippy --workspace --all-targets -- -D warnings` clean.
  - 4 000-row queue, mean over a loop of 1 000 (`Instant`), budget 1 ms: `toggleShuffle` **38.5 µs**
    release — **29 µs** on a second run with other builds on the same cores — **158 µs** in a debug
    build idle and **570–695 µs** while the rest of the workspace was compiling; `move` **0.9–1 µs**;
    `removeAt` + its re-append **0.5–1.7 µs**. The test asserts the 1 ms budget in release and a
    **4 ms** bound in a debug build, the way `nz-viz` does: a debug build has no optimisations, this
    machine runs several of them at once, and a wall-clock bound that flips with the load teaches
    nothing. The first shuffle of a process pays one fresh 32 KB allocation (1.5–2.2 ms), so the loop
    warms up before measuring.
  - the fixture is the JavaScript's own output: **19 commands, 13 500 B**, produced by running
    `commands.js#commandFor` under Node (a module hook stubs only `settings.js`, which imports
    `svelte/store`), with the base commit and both blob hashes recorded in the file. Every command
    parses into its Rust variant AND serialises back to the same bytes; `seek` is the one op
    `commandFor` never produces (`controller.js#watchSeek` posts it), and `shuffle_play` is in the
    server's `remote.py#OPS` while the lot's list did not name it.
- **Pitfalls confirmed by building**: `#[serde(tag = "op")]` renames nothing by itself — without
  `rename_all = "snake_case"` the wire op came out as `"Next"` and only the round-trip test noticed.
  `play_queue` always carries `start` and `shuffle_play` never does, so they are two arg structs; one
  struct with an optional field adds a `"start": 0` the JavaScript never sends, and a fixture that is
  regenerated would then disagree with the code. `PreloadNext` is announced only when the row after
  the playing one CHANGES, so the first `Load` of a fresh state comes with none: the engine's second
  voice starts empty, and an app that waited for `PreloadNext(None)` before playing would wait
  forever. `clippy -D warnings` refuses `chunks_exact(64)` on a constant
  (`chunks_exact_to_as_chunks`) and an explicit lifetime on a `&Vec` return.

### D5 — `nz-audio` I: source, decode, output, gapless, seek (delivered 2026-11-16)

- **The seam is a ring, not an event.** A voice is one decoded track: an `rtrb` ring of 0.5 s of
  interleaved stereo f32 at the device rate, filled by one decode thread, drained by the output
  callback. When the callback finds the current ring empty **and its decoder ended**, it installs
  the preloaded next voice and keeps writing in the SAME buffer. Nothing starts a stream, waits on
  a timer or pads a frame at a track change. Measured: two FLACs cut out of one 2 s cosine decode
  **sample-identical** to the whole file (max difference `0.000000000` over 96 000 frames), seam
  Δ **0.000829** against a waveform step of 0.0576 — and frame 96 000 is silence, so the length is
  the sum of both exactly.
- **Nothing blocks the UI thread, nothing blocks the callback.** `load`, `preload_next` and `seek`
  bump a generation and return; a loader thread opens, probes and seeks; the decode thread
  resamples and pushes; the callback allocates nothing, locks nothing, waits for nothing. It reads
  atomics and preallocated buffers, takes commands from an 8-slot `rtrb` ring and sends notes
  through a 64-slot one that an ordinary thread turns into `EngineEvent`s (a `std::sync::mpsc`
  send allocates, so it can never be the callback's). The mixer is the truth about what is playing
  (`MixerShared::current_generation`), because after a gapless handover the engine's own "current"
  is the previous track: `seek` and `current_id()` read the mixer, or they aim at the wrong track.
- **A seek is the server's answer, in the server's three forms.** A finished local file is seeked
  with a file seek; a finished network file with `Range` requests (symphonia's `FormatReader::seek`,
  5 `Range` requests for a 10 s FLAC — its index search walks back and forth, and each restart
  discards the previous spool); a source that must be encoded is reopened with `?start=<seconds>`
  (exactly ONE request, asserted); a source that cannot answer yet is polled on `/ready`
  (1 s, 1.5 s while `streaming`) and announced as `SeekDeferred` until it lands — never a restart
  from 0. `Seeked` is emitted only after the mixer holds the new base, so the bar cannot move
  before the sound (`docs/agents/playback.md`, *A transport action always wins*).
- **What it is now**: `Engine::new(cfg, opener)` on PipeWire (ALSA fallback, a paced silence pump
  when neither exists), and `Engine::new_offline` returning an `OfflineSink` the caller renders —
  what the tests use, so a seam is an assertion and not a race with a sound card. `StreamOpener` is
  the app's only network seam: `open` returns a `ByteStream` (`len_hint`, `start_offset_s`), plus
  `ready` and `local`; `Quality` and `Lane` live in this crate and the app maps them onto
  `nz-api`'s. A network body is spooled to `EngineConfig::spool_dir` while it arrives, read as a
  blocking `Read + Seek` over the bytes received, and capped `read_ahead_bytes` in front of the
  decoder (rule 1: a prefetched next track stops instead of downloading itself whole — measured
  **65 536 B** with a 64 KiB cap on a 4 MiB body). A whole spool is renamed into place and announced
  as `EngineEvent::StreamComplete` so the app can publish it into the play cache; a `?start=` slice
  is never published (it is a window, not the track).
- **Measured** (this machine: 16 threads, Rust 1.99.1, PipeWire 1.6.9; `cargo test -p nz-audio`):
  - click to first sample: local FLAC median **1.1 ms** (max 1.1, budget 50); over loopback on a
    body generated as it is sent median **1.1 ms** (max 1.1, budget 250), 10 runs each. (`perf_api
    --audio` seeds WAV files, and this crate's format set is FLAC, Ogg Opus, MP3 and MP4 — a WAV
    stream is not something the app plays, so the loopback budget is measured on a local server
    that reproduces `/stream`'s headers, including `Accept-Ranges: none` and `X-NS-Start`.)
  - seek in a finished 48 kHz FLAC to 6.0 s: landing **6.000000 s** (clock 6.001000 a millisecond
    later), first sample matching the generated cosine to **0.000000**; the same over HTTP with a
    byte range, same landing.
  - Ogg Opus: a 1.000 s file decodes to exactly **48 000 frames** (pre-skip and final granule
    honoured; ignoring the pre-skip is +312 frames = 6.5 ms). 44.1 → 48 kHz FLAC: **1.000000 s**,
    first sample 0.488459 (the resampler's delay dropped, not left as leading silence).
  - soak: **0 underruns in 600 s** with a thread allocating in a loop on the same cores, 10 ms
    buffers (CI runs 20 s; `NZ_SOAK_SECS` sets it).
  - engine memory: **+0.4 to +2.9 MB** RSS over 5 s of playback across runs (two rings = 384 KB,
    spool capped); the device latency comes from cpal's playback timestamps (480 frames here).
  - real PipeWire: `default_output`, 48 000 Hz, 2 ch, the 10 ms period (480 frames), 0 underruns
    over 3 s (throwaway binary, not committed); `cargo test --workspace` green.
- **Pitfalls confirmed by building**:
  - **rubato 5's `output_frames_max()` is not `output_frames_next()`.** Emitting the maximum after
    every call adds a tail of stale samples: one second at 44.1 kHz produced **54 954** frames
    instead of 48 000. And the opposite trap: the last partial chunk still produces a full FFT
    block, and dropping its overflow cut **246 frames** off the second. Count the real input frames
    and trim the output to `round(fed × ratio)`.
  - **A pull-driven test races the decoder.** Rendering 72 000 frames in a microsecond found no
    audio at all (the decode thread had not started), and reading `position()` right after an event
    showed the old track until the mixer drained the install command. Pump in real time; make
    `Seeked` wait for the clock.
  - **Dropping a `VoiceConsumer` does not stop its decoder**: `rtrb` keeps accepting pushes until
    the ring is full, so a replaced voice is stopped through its `VoiceStatus` flag.
  - **A seekable network source makes symphonia's probe scan the trailing metadata**, which seeks to
    the END of the body before the first sample (seen as a pull of the file's tail on a 60 s
    track). `HttpSource::is_seekable()` answers false while `decode::prepare` probes — one
    `probe_handle` flag, cleared after the probe — so the probe is head-only while the reader still
    seeks for real; a finished-file seek then issues only the `Range` requests the container's own
    index search needs.
  - cpal 0.18: `SampleRate` is a `u32` alias (`.0` does not compile), `DeviceTrait::name()` became
    `description().name()`, and an underrun arrives as `ErrorKind::Xrun` in the error callback.
  - symphonia 0.6: `AudioDecoderOptions` is `#[non_exhaustive]`, and the Opus decoder must be
    registered in a custom `CodecRegistry` (the default one has no Opus).

### D6 — `nz-audio` II: DSP, crossfade, trim, speed, tap (delivered 2026-11-16)

- **The rule, the shape of the sound**: **a chain per voice, a chain on the sum.** During a
  crossfade two tracks are audible at once and each has its own loudness, so the normalisation gain
  and the fade envelope belong to the VOICE; the ten-band EQ, the bass shelf, its compressor and the
  safety limiter belong to the MIX, exactly where the page puts them (`graph.js`: `srcA → normA →
  fadeA` into `input → eq → bass → bassComp → limiter`). Both are `nscore::dsp::Chain` — the Android
  chain, whose own tests pin the Web Audio biquad formulas, the quarter-circle fade and the two
  compressors — so nothing of the maths is re-implemented and a setting sounds the same in the
  browser, on the phone and here. With everything flat the chain is a plain gain and the mix is
  bit-transparent (asserted).
- **The rule, the handover**: **the current track changes at the START of the fade**, not at its end
  — the incoming voice takes the generation, the clock (`start_at`) and the `Started` event the
  moment the two begin to blend, so the bar, the queue and MPRIS name what is being heard
  (`Player.svelte#adoptIncoming`). The old voice is only *stopped* when its ramp reaches zero (or it
  ends), and a transport action during the ramp wins at once: the new track is current in the next
  buffer and the overlap is cut in the web player's 60 ms (`crossfade::SKIP_CUT_SECONDS`).
- **The rule, the trim**: a voice carries `trim.end_s` and the mixer **stops pulling at it**, even
  though its file goes on — so the 3 s of digital silence a master carries after its last note is
  never played, and the preloaded next voice starts in the same buffer. The bounds are the page's,
  accepted only when they are plausible (`trim.rs`): ≤ 10 s at either end, a measured file length
  within 2.5 s of the track's metadata duration, an end at least 0.2 s inside the file, and a start
  worth a seek above 0.25 s. A trim ends a voice HARD; a *duration* does not (see the pitfalls).
  **The app primes them ahead of the play** (`AppState::load`/`preload`, D14a's follow-up): the
  playing track's bounds at USER priority, the next one's at PREFETCH, and the next one's VOICE is
  built only once its answer has landed — the head is skipped when the voice is built, so a voice
  built at 0 plays the silence the trim exists to remove. A failure is logged and forgotten (the
  next play asks once more), and a `ready:false` is remembered for the session as "not measured".
  Measured on a 5.5 s fixture (0.5 s of silence, 2 s of tone, 3 s of silence): untrimmed the first
  sample is **0.0000** and the head window is silent; trimmed, the voice starts at **0.500 s**, the
  first rendered window peaks at **0.2500**, and the track ends at **2.500 s** — `nz-audio`'s
  `a_measured_head_and_tail_of_silence_are_both_skipped`, and the app's own chain in
  `state::tests::the_servers_bounds_reach_the_engine_before_the_next_voice`.
- **The rule, the gain**: the server's `/api/gain/<id>` value is Deezer's LOUDNESS, not a gain. The
  adjustment is `-(loudness + 18.4)` dB (deemix's ReplayGain transform) plus the level's offset
  (`off` 0, `low` +2, `medium` +5, `high` +8), clamped to −24…+12 dB; a track with no measurement is
  never given an invented one, and `off` is exactly nothing. The app hands it over with the rest of
  the track's metadata BEFORE the voice is built (`Engine::set_track_meta`), which is what keeps the
  first track of a session normalised like the next one.
- **The rule, the speed**: 0.5–3× through `signalsmith-stretch`, pitch kept, **episodes only**
  (`TrackMeta::podcast` decides, not the app's current setting: music always plays at 1×), and the
  stage runs on the DECODE thread, in front of the voice ring, never in the callback. The timeline
  stays media time: the clock advances by `frames × speed`, so a 2× episode shows its real duration.
- **The rule, a cancelled decode says nothing**: the engine stops a voice the instant the person
  changes track, seeks or quits, and the read that voice was in the middle of then fails (a body
  closed by the server, `corps interrompu`, `Interrupted`). That failure is the person's own click,
  not a verdict about the track: `decode::fail` returns WITHOUT an `EngineEvent::Error` when the
  voice was stopped, and a load or seek superseded by a later one is silent too (its `mark` is
  stale). D10b saw the opposite on every fast next/next — an error toast per track change. The app
  has the second half of the rule: an `EngineEvent::Error` naming a track that is no longer current
  is logged, not toasted (`state.rs`, `a_late_error_for_another_track_is_not_a_toast`), and only a
  failure of the track that IS playing keeps its cause and its « Réessayer ».
- **The rule, the tap**: post-DSP and **pre-mono/balance** (`pan.js`: the analysers keep the original
  stereo), an `rtrb` ring of 8 192 frames (170 ms), pushed by the callback with
  `push_partial_slice` and DROPPED when the analysis thread is behind — a hidden window costs the
  music nothing. `TapReader::dropped()` says how many frames were lost.
- **Files**: the five modules the lot named (`dsp.rs`, `crossfade.rs`, `trim.rs`, `stretch.rs`,
  `tap.rs`) plus the crate's integration points — `mixer.rs` (where the callback's chains and the
  fade state live), `engine.rs` (the settings handle, `TrackMeta`, the tap), `output.rs` (the mixer's
  new ends), `decode.rs` (the sink the resampler and the stretch stage share), `voice.rs` (per-voice
  meta), `tests/dsp.rs`. No other lot edits `nz-audio`, so this stayed inside the wave's rule.
- **Measured** (this machine: 16 threads, Rust 1.99.1; `cargo test -p nz-audio`):
  - **61 tests, 0 failed** (`54` in the crate + `7` end-to-end in `tests/dsp.rs`); `cargo fmt
    --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean.
  - **crossfade, equal power**: two generated tones (440 and 610 Hz, 0.25), a 1.0 s fade; 23 windows
    of 50 ms across it, **worst deviation 0.00 dB** from the pre-fade power, **0** windows below
    −0.5 dB (criterion ±0.5 dB), and the last track alone is at **+0.00 dB** of it.
  - **trim**: a 5 s file (2 s of tone + 3 s of silence) ends at **2.000 s** with the server's bounds
    (event at 2.015 s, clock stopped at 2.000 s) and is still playing at **2.600 s** without them.
  - **a skip during a fade** (1.7 s of overlap left): only the new track is audible after **50–80 ms**
    — the 60 ms cut plus one buffer.
  - **EQ**: +6 dB on the 250 Hz band lifts a 250 Hz tone by **5.999 dB** measured on the samples
    (criterion ±0.1 dB), and the band's transfer function is `Biquad::magnitude` to 1e-5.
  - **output callback**, every effect on (10 bands, bass +9, normalisation high, mono, balance −0.2,
    two voices with gains and fades, the tap and the pan stage), 480-frame buffers: **p99 215 µs in
    debug, 107 µs in release** of a 10 000 µs period (criterion 25 %).
  - **speed**: a 2.000 s episode at 2× is **48 000 frames** (1.000 s) and 64000 at 0.75×, the 440 Hz
    tone stays **440.0 Hz**, and the engine reads **1.000 s of media after 0.5 s of output** while the
    same file as music reads **0.500 s**.
  - **tap vs speakers**: with mono on, speakers **0.1768/0.1768**, tap **0.4000/0.1000** — the tap
    keeps the stereo image the fold removes.
- **Pitfalls confirmed by building**:
  - **`Stretch::flush` silently does nothing when the buffer is shorter than `2 × output_latency`**
    (the C++ `SignalsmithStretch::flush` returns false). Flushing with exactly `output_latency`
    frames lost the tail: a 2.000 s episode at 2× came out at **47 296 frames instead of 48 000**
    (measured), i.e. 14 ms of the last word gone. Size the flush buffer at `2 × latency` and the
    length is exact.
  - **A phase vocoder cannot start sample-exact.** Its own latency is `input_latency +
    output_latency` = **5 760 frames (120 ms at 48 kHz)**: the first output frame reconstructs the
    silence before the track. At FULL scale the window sum overshoots by **+4.7 dB for one 10 ms
    window** (1.715 for a 1.0 source); at speech level (0.1) the peak is 0.1715 — a podcast sits 10
    to 20 dB below full scale and the master limiter is behind this stage for a hot file. Do not
    "fix" the onset by dropping the latency: it would cut the first 120 ms of the episode.
  - **`nscore`'s chain carries a static makeup gain** (`Compressor`: `−0.6 × curve(0)` = **+0.285 dB**
    at the limiter's settings, as Web Audio's `DynamicsCompressorNode` does). An EQ measured straight
    through it reads +6.285 dB for a +6 dB band; measure the LIFT against the same chain with a
    transparent band (0.001 dB), which cancels it.
  - **The D5 clock over-counted a gapless handover by the frames before the switch** (it advanced the
    whole buffer after `start_at` had reset it). The mixer now advances per span, so after a 4-frame
    buffer with a 2-frame handover the new track reads **2 frames**, not 4 — one D5 assertion changed
    with that number.
  - **A duration is not a trim.** Retiring a voice when its *container duration* is reached cut the
    last samples of a file whose decoder ran a hair short (Opus reports to the granule). Only an
    explicit trim end retires a voice; the duration is only where a crossfade has to fit in front of.
  - **The repository's crates are now COMPILED by the desktop job.** `nz-audio` depends on `nscore`
    (and through it on `webapp/rhythm`), so the sound the phone and the browser get is compiled into
    the desktop binary instead of a copy of it — and a change in either can break `desktop`. They are
    not workspace members: `cargo fmt --check` never reformats them and clippy never lints them, so
    their hand-formatted sources (whose exact bytes the committed `rhythm.wasm` hashes) stay
    untouched, which is why the job keeps `cargo fmt --check` and not `--all`.


### M1 — `nz-mpris` (delivered 2026-11-16)

- **The rule**: `MprisHandle` is the app's only D-Bus surface, and MPRIS is optional to the app
  running. `Mpris::start` owns one thread (current-thread runtime + `LocalSet`), does not return
  before the bus name is ours, and a missing session bus, a name another instance owns, or a
  `mpris-server` failure leave an inert handle (`is_serving()` false, one logged reason): the
  player plays, the bar and the media keys do not. Updates reach that thread on an unbounded
  channel — the GPUI thread never waits on D-Bus — and commands come back with `try_send`, so the
  D-Bus thread never waits on the app either.
- **The rule, position side**: `Position` is **never** announced (MPRIS 2.2: it does not appear in
  `PropertiesChanged`). `set_position_s` writes it with `Player::set_position` (documented silent)
  and emits `Seeked` only when the reported position departed from what a client extrapolates, past
  a 1.5 s tolerance: at the app's 4 Hz a tick may be 0.9 s late for free, while any gesture is
  announced. A new track and a pause freeze the base, so neither looks like a seek — and a new
  track also writes `Position` 0 before its first tick, or the bar shows the previous track's
  progress for a quarter second.
- **The rule, identity side**: `mpris:trackid` is derived, not passed in. A Deezer number is its own
  path element (`/fr/neezer/track/3135556`); anything else (the UUIDs of uploads, YouTube rows and
  episodes) is sanitised to `[A-Za-z0-9_]` **and** gets an FNV-1a fingerprint, because sanitising
  alone merges `a-b` and `a_b` and a stale `SetPosition` would then move the next track. Staleness
  is checked against that path, so a click from a client that saw the previous track is dropped at
  the D-Bus thread. `mpris:artUrl` is a **percent-encoded** `file://` URI of the cached cover: a
  space, a `#` or an accent in a cover name would otherwise truncate or break the URL.
- **Measured** (this machine, `playerctl` 2.4.1, private `dbus-run-session`):
  - 100 position ticks over **10.0 s** produce **0** signals (0 `PropertiesChanged` on the player
    interface, 0 `Seeked`), counted by the test through zbus and confirmed with `dbus-monitor` on a
    real bus: **2** `PropertiesChanged` in 10 s, both the initial track+status publication, none
    from the ticks. One real seek (0 → 42.5 s) produces exactly **1** `Seeked` carrying
    `42 500 000 µs`, and `Position` reads it back; a paused position repeated 20 times emits
    nothing.
  - `playerctl -p neezer metadata` → `xesam:title Balancing the Dark`, `xesam:artist Kaya
    Project`, `xesam:album Songs from the Inside`, `mpris:length 312000000`, `mpris:trackid
    '/fr/neezer/track/3135556'`. `playerctl -p neezer play-pause` flips the status to `Paused` and
    arrives as `MprisCommand::PlayPause`; `next` moves the metadata to the next track.
  - The whole contract test runs itself under `dbus-run-session` (a private bus, so a developer's
    own player is never shadowed): `cargo test -p nz-mpris` passes 9 tests in **12.0 s**, and the
    no-bus child proves startup is never held by a missing bus.
- **Pitfalls confirmed**: the server really is local — nothing answers until `Player::run()` is
  polled on a `LocalSet`, and a property set before that panics inside the crate with `SendError`
  (so `start` blocks on a bounded readiness handshake instead). zbus 6.3 proxies cache properties
  **lazily by default**: a test that reads `Position` after `PlaybackStatus` gets the first
  `GetAll` snapshot — build the proxy with `CacheProperties::No`. `GetAll` hands back the whole
  property map with each `a{sv}` value one variant level down and the player's fields inside the
  `Metadata` dict, so `Get("Metadata")` and the map are not interchangeable. `playerctl` is not on
  the `ubuntu-24.04` runner image: the CI test asserts the identical surface through zbus and runs
  `playerctl` wherever it is installed (measured here). zbus 6 already runs on tokio, so no
  `mpris-server` feature and no manifest change is needed.

### V1 — `nz-viz` (bars and oscilloscope, pure) — delivered 2026-11-16

- **The rule**: the geometry is the page's OWN Rust, never a port. `rhythm::viz_bars::Bars` and
  `rhythm::viz_scope::Scope` are the arithmetic `lib/viz/core.js` calls on the browser side, so
  `nz-viz` only feeds them what the page feeds them, in the same order: the mono sum `(l+r)/2` into
  `rhythm::Analyzer` at `LEVEL_SPECTRUM` (the depth `viz/modes.js#levelFor` picks for these two
  scenes; SMART spends the tempo/grid ladder for nothing), one `Bars::update` + one `Scope::update`
  per ANALYSIS frame (94 Hz at 48 kHz, dt = hop/sr), and a plain read at the display rate. Copying
  the maths would let the desktop and the browser draw different pictures from the same track.
- **The rule, frame layout**: the analyser's `LAYOUT` table is private; the only published form is
  the `rhythm_layout()` string, and the page parses it precisely so neither side can hard-code an
  offset. `nz-viz` parses the same string through the same entry point, once per process
  (`OnceLock`), and asserts the parsed length against `frame_len()`.
- **The rule, time**: `bars(n, dt)` / `scope(cols, dt)` are drawn at the display rate and carry the
  time since the previous call, which is spent ONCE, on the newest analysis frame — `approach()`
  composes, so a 240 Hz panel and a 60 Hz one agree to 2.7e-3 of a bar. Below the analysis cadence
  (94 Hz) a paint can only draw the newest target for the whole interval: 1.2e-2 of a bar at 60 Hz,
  bounded and drift-free, which is the price of a dropped frame, not a bug.
- **The rule, scope precision**: a desktop takes the exact path. `ScopeCfg { search_ms: 40, exact,
  fine }` — the page reserves `exact`/`fine` for its high/ultra tiers because a phone cannot afford
  the per-column min/max, but a 512-column trace on a 1548 px lane drawn as one value per column is
  visibly polygonal. The window stays 4096 samples (the page's low/medium `buffer`, 85 ms for a
  42 ms timebase) and the palette is `lib/coverpalette.js#analyse` ported field for field, plus
  `Palette::from_theme(accent, surface, light)` for the Omarchy strip's one flat colour.
- **What it is now**: `desktop/crates/nz-viz` depends on `rhythm` (path) and nothing else;
  `VizState` (`new`, `push_frames`, `bars`, `scope`, `scope_state`, `set_scope_config`, `idle`),
  `BarsViz` (levels then peaks, contiguous, `MAX_BARS` cap), `ScopeViz` (the 4096-sample per-channel
  tap, the `[lo, hi]` trace per column, the trigger lock), `palette` (`analyse`, `zone_colours`,
  `from_theme`).
- **Measured** (this machine, Rust 1.99.1; `cargo test -p nz-viz --release -- --nocapture`):
  - **21 tests, 0 failed**; `cargo clippy --workspace --all-targets -- -D warnings` clean.
  - against the SHIPPED `webapp/src/lib/audio/rhythm.wasm` in Node, over a fixture recording the two
    builds generate bit for bit (a 220 Hz square + deterministic noise): **worst relative difference
    1.2e-7** over 4 sampled frames × 8 bars (levels and peaks) × 8 scope columns × 2 channels — the
    residue is libm's `exp` on the two backends, so the desktop draws the page's picture.
  - `bars(64)` + `scope(512)` + the analysis that lands in the same paint: **0.054–0.072 ms mean,
    0.08 ms p95** per paint at 240 Hz (budget 0.3 ms; debug, unoptimised, 3.96 ms).
  - 60 Hz against 240 Hz painting: bars 1.2e-2, scope traces **identical** (0.0 — a column is a
    function of the window and the trigger, and the window sequence is the same).
- **Pitfalls found by building**: `cargo fmt --all --check` in `desktop/` also formats every local
  PATH DEPENDENCY, and `webapp/rhythm`/`webapp/appcore`/`android/nscore` are hand-formatted on
  purpose (rhythm's committed `.wasm` carries an FNV hash of its exact source bytes, so reformatting
  them fails `webapp`'s tests): the job now runs `cargo fmt --check`, which still checks every
  member. The analyser withholds its first `LOOKAHEAD` output frames, so the tap must not assume one
  frame per push. `Bars`/`Scope` expose raw pointers only (three justified `unsafe` slice views), and
  `Bars::input_ptr` is 256 floats wide while a frame carries 120 bands — clamping to `MAX_BANDS`, not
  to the frame's count, is what keeps that copy sound. The palette's `colors`/`dominant`/`vivid` are
  picked from a histogram sorted by presence with JS's stable tie order: a Rust sort that is not
  stable colours a two-colour sleeve differently from the browser.

### D7 — App shell and wiring (delivered 2026-11-16)

- **The rule, the shell composes**: one `RootView` (`app.rs`) owns the header, the sidebar,
  the player bar, the layers (queue panel, toasts, `?`) and the screen table; screens are
  entities from `screens::build` and keep their own state. Every route of the inventory is
  registered — **22 routes and 32 stub files** — and every widget file a later lot names is
  declared in `widgets/mod.rs`, so D8a→D16, T4 and P1 touch only their own files. A stub
  renders its title and « Bientôt »: every navigation target is reachable and measurable from
  the first day.
- **The rule, one runtime for the process's life**: `Application::run` may drop its launch
  closure after calling it, and a `tokio::Runtime` captured only there loses its workers. The
  measured symptom was a login that stayed « Connexion… » forever: the request task existed
  and was never polled (a startup task printed, a later one did not). The app keeps the
  runtime in a GPUI `Global` (`NetRuntime`) and one event channel carries every worker's
  [`AppEvent`] to the shell; the shell drains it in a foreground task, so no worker touches
  GPUI state. That global also OWNS the threads that borrow the runtime: its `Drop` joins the
  offline worker AND the audio engine's threads (loaders, decode voices, spoolers) before the
  runtime goes, because a `Handle::block_on` on a shutting-down runtime panics its thread (see
  D13's *Pitfalls*). The engine is reached through a registry of weak handles
  (`wiring::engine::attach` / `shutdown_process`), since gpui drops the shell's state — the
  engine's other owner — before the globals.
- **The rule, the keyboard is the mouse**: every control dispatches the action the keymap
  binds (`cx.listener` → `window.dispatch_action`), and the action handler is the shell's one
  transport path (`PlayerState::apply`). The single-letter/space bindings live in
  `Neezer && !Input`: GPUI evaluates the predicate against the whole focus path, so a space
  typed into any field is a space (gpui-base's editor puts `Input` on the path). The shell
  also owns the focus anchor: after the login screen leaves, `refocus` gives the keyboard
  back to the root — without it the scoped bindings are looked up above the view and do
  nothing, silently (the same failure D0 documented for `Ctrl+Q`).
- **The rule, explicit icons**: `gpui_kit::assets::Assets` is the **default** bundle (a dozen
  component glyphs), and an icon outside it renders NOTHING while keeping its hit box — the
  first real run showed a player bar with only Play and a sidebar without Bibliothèque or
  Nouveautés. The shell registers `icon_assets!(ShellAssets, [...])` with exactly the glyphs it
  draws plus gpui-base's own, and a test walks the list through `AssetSource::load` so a
  missing icon fails CI instead of disappearing.
- **The rule, single instance**: `fr.neezer.Desktop` is claimed **in the background** (2 s
  budget for a hung bus): the blocking version delayed the first frame by up to its timeout,
  and a 250 ms budget made two launches both think they were first — the reason two windows
  existed during development. A second launch calls `Raise` on the owner (a D-Bus method on
  the registered object) and quits; no bus is not a reason not to start.
- **Measured** (this machine: 16 threads, Rust 1.99.1, Hyprland, Omarchy; release build):
  - **cold start to first painted frame**: fresh XDG dirs **153, 157, 158, 159, 163 ms**
    (median 158), warm state **111–120 ms**; the first-ever run (fontconfig cache cold) 350 ms.
    All under the 300 ms criterion; `--self-test-quit-after-first-frame` is unchanged.
  - **RSS**: **103 148 KB** at the home screen after login, **103 544 KB** with a track playing
    (release, `/proc/<pid>/status`, a 4 000-entry restored queue); the 8 000-row list plus a
    4 000-row queue cost **+24 304 KB** in the app's own data model (a unit test prints it) —
    the lists are windowed, so the widget count does not follow. Idle stays under 120 MB.
  - **journeys**: 38 app tests, 0 failed — the keyboard one types the login, presses `Entrée`,
    `g l`, `Space`, `Ctrl+→`, `Alt+←`, `q` through the real keymap and asserts the
    `PlayerState` transitions; the mouse one clicks Play and Next through real hit testing at
    1280×800; a dead server gives `Auth::Failed`, a readable cause and one toast.
  - **theme**: the shell reads every colour through `cx.omarchy()`; a harness test applies a
    second Omarchy theme and asserts the tokens changed, and gpui-omarchy's watcher is what
    calls `refresh_windows()` on `omarchy-theme-set`.
  - **visual**: the login and the shell were looked at at 1280×800 logical (Omarchy palette,
    French copy, no emoji, real Lucide glyphs). The 2560×1600 and 420 px panel re-shots were
    not taken: this machine has no isolated compositor or software Vulkan, and the user's
    session must not be driven (no focus/workspace/clicks). The layout is fixed rails
    (200 px sidebar, 420 px panel, 72 px bar) plus a flexible centre and rem type, so it
    scales; D17 owns the full-size pass.
  - release binary **42 443 784 B** (the shell + reqwest/rustls + bundled SQLite + symphonia/
    libopus + zbus; D0's 25.5 MB was the empty window).
- **What it is now**: `app/src/` — `main` (args, runtime global, single instance, window),
  `app` (RootView, header/sidebar/layers, action handlers), `state` (`AppState`, `Auth`,
  effects → engine/requests/queue, `entry_from_track`), `theme`, `keymap` (the complete table
  and the `?` data), `actions` (one action per verb, including later lots'), `nav` (routes,
  history, per-page scroll), `toast`, `load_failed`, `net_indicator`, `player_bar`, `login`,
  `wiring/{net,engine,viz,mpris,instance,events}` plus the empty stubs the later lots fill;
  `screens/` (22 routes, 32 stub files), `widgets/` (28 modules). The engine's only network seam
  is `wiring/engine.rs` (`StreamOpener` over `nz-api`, blocking reads on the loader thread,
  `StreamComplete` published into `nz-cache`'s play cache); MPRIS starts off-thread and its
  commands rejoin `PlayerState::apply`.
- **Pitfalls found by building**:
  - **A GPUI test may not be woken by a real runtime thread**: `gpui-pre`'s deterministic
    scheduler panics when a tokio worker wakes a foreground task
    (`Detected activity on thread "tokio-rt-worker"`). The journey tests therefore drive a
    **current-thread** runtime on the test thread (`runtime.block_on(sleep(1ms))` between
    `run_until_parked`), which makes every wake happen where the test expects it.
  - **`VisualTestContext::update` takes `(&mut Window, &mut App)`**, unlike `TestAppContext`'s
    one-argument `update`; and `render_to_image()` needs a `HeadlessRenderer` the test platform
    does not install (`no HeadlessRenderer configured`), so screenshots are not a CI artefact
    here.
  - **`impl IntoElement` captures every input lifetime in edition 2024**: a render helper that
    takes `&mut Context` keeps that borrow alive in its return type until `+ use<>` is added;
    without it `player_bar::render(self.state.read(cx), cx)` does not borrow-check.
  - **The playhead is a 4 Hz foreground tick.** In an X11 run with the window on a workspace
    the compositor did not show, MPRIS reported `Playing` and the engine had opened the stream,
    but the tick (and so the bar) did not advance. The transport itself was proven by the
    stream request and the status; whether GPUI throttles foreground timers for an inactive
    window is D17's to measure, and the tick must keep running if it does.
  - `cargo test --workspace` locally needs `NZ_PYTHON` pointing at a Python that can
    `import neezer` (CI installs the server before the test job); `VULKAN_SDK=/tmp/vk` (headers
    + `lib/libvulkan.so`) is what builds whisper.cpp outside CI.

### D8a — Browse: home, search, artist, album, credits

- **Files**: `screens/{home,search,artist,album,credits}.rs` and new files under
  `app/src/widgets/` it creates (`track_list.rs` — the shared windowed list — `cover.rs`,
  `context_menu.rs`, `card.rs`). D8b uses the widgets once merged; until then it may only read them.
- **Acceptance**: search answers at each keystroke from cache then network (debounce 150 ms), never
  more than 1 request in flight per field; the artist page paints from cache in one frame when
  cached; requests per screen equal to the SPA's (`webapp/test/requests/run.mjs` counts) or fewer;
  scrolling a 4 000-row list at ≥ 230 fps on this machine (frame times logged with
  `NZ_FRAME_LOG=1`, p95 < 4.2 ms); context menu by `Menu`/`Shift+F10` and right click.
- **The rule, a page column is as tall as its CONTENT, and the scroller around it is bounded**
  (v1.6.1). GPUI measures a scroll container from the BOXES of its DIRECT children
  (`Div::prepaint`'s `content_size`), never from what those children paint past their own
  bottom edge — so a `flex: 1 1 0%` page column is exactly one viewport tall, `scroll_max` is
  0, and the wheel, the touchpad, `Page suivante` and the arrows do nothing while every
  section below the fold is clipped, silently. That was Accueil v1.6.0: **no scroll at all**,
  at every window size and with the 420 px panel open. Measured by the regression test
  `the_home_scrolls_with_the_wheel_and_the_keys` (`cargo test -p neezer-desktop screens::home`)
  and `a_content_sized_column_scrolls_and_a_flex_1_one_does_not` (`widgets::page_scroll`, which
  pins the failure: 1 600 px of travel with `column()`, **0** with `flex_1`): **0 → 902 px** of
  travel on a 1280×800 window, **2 017 px** in the 660 px column a 1280 window keeps with the
  sidebar and the 420 px queue panel open, one wheel notch **78 px**, `Page suivante` **640 px**
  (0.8 of the window), an arrow **64 px** (4 rem). Scrolling costs nothing measurable: 40
  `Page suivante` on a release build under Xvfb give a p95 element-tree build of **0.29 ms**
  against **0.25 ms** idle (the 160 ms present interval is the software rasterizer, not the app).
  One
  implementation, `widgets/page_scroll.rs` (`PageScroll::container` for the scroller,
  `column()` for the page, `own()` for the keys): Nouveautés, le Récap, Bienvenue and the
  search results page were built the same way and are fixed with it. The keys are app-level
  interceptors that CONSUME the stroke (the shared `ListPageDown`/`ListNext` actions would
  otherwise move a row list in the same breath), so only a screen with **no text field**
  installs them: Accueil (it keeps `j`/`k` for its recomposed list), while Réglages, search,
  Bienvenue and le Récap keep their own list/deck keys and scroll with the wheel.
- **The rule, a windowed list's scroll handle is the SCREEN's, or the list does not scroll at
  all** (v1.6.1). `v_virtual_list` builds a `VirtualListScrollHandle` of its own on every frame
  and the wrapper reads THAT handle's offset, so a list that is not handed the handle its screen
  keeps begins every paint at 0: the wheel, the touchpad, `Page suivante` and the scrollbar
  beside the list did NOTHING, while `scroll_to_item` moved a cursor nobody could see.
  `widgets/track_list::element` (the shared list of the album, an artist's titles, a person's
  credits, the search results, the home's reco, Nouveautés) and `screens/history.rs` both built
  their list and never called `.track_scroll`: measured by
  `the_album_track_list_scrolls_with_the_wheel_and_the_keys`,
  `the_person_credits_list_scrolls_with_the_wheel_and_the_keys` and
  `the_keys_scroll_the_history` — **0 → 57 px** for one wheel notch (one 56 px row),
  `Page suivante` **566–623 px**, `Fin` the last row, at 1280×800 and in the 660 px pane; live
  under Xvfb, `Page suivante` ×3 on a 14-title album changes **26 491 px** of the window.
  `.track_scroll` belongs to the widget, never to its callers: a list built without it looks
  right and scrolls never.
- **The rule, a page's TAIL is bounded and its list keeps the window** (v1.6.1). A tail is a
  second list at the bottom of a page: the provider's half under an album's tracks (« Disponible
  sur Deezer ») and the suggestions under a playlist (« Titres recommandés »). Unbounded, the
  tail is the page — twelve 44 px suggestions took the whole 800 px page and left a 4 000-title
  playlist's rows **0 px** tall, and a Deezer band of twenty titles squeezed the album's list to
  nothing. Bound by `card::TAIL_SHARE` (40 % of the page) or `RECS_MAX` (208 px, four rows) and
  `overflow_y_scroll`, the tail scrolls inside itself: the album's own tracks keep **336 px**
  (six rows) with a 20-title band at 1280×800 and in the 660 px pane
  (`the_album_track_list_scrolls…`, `the_suggestions_strip_bounds_itself_and_the_rows_scroll`).
- **The rule, un titre crédité est une PAGE** (v1.6.1). Le corps d'un écran de crédits de titre
  n'avait aucun conteneur de défilement : un titre crédité sur douze rôles (trois personnes
  chacun) peignait ses derniers rôles, son label et son ISRC sous la fenêtre, inatteignables à la
  molette, au pavé tactile comme au clavier. `PageScroll::container` + `column()` + `own()`
  (aucune liste, aucun champ : les touches sont celles de la page) : **852 px** de course pour un
  titre à 30 rôles, un cran de molette **78 px**, `Page suivante` **640 px**, `Fin`/`Début` aux
  extrémités (`the_track_credits_page_scrolls_with_the_wheel_and_the_keys`). Les crédits d'une
  PERSONNE gardent les touches de leur liste : la liste EST le conteneur de cette page.
- **The rule, the history scrolls from the keyboard** (v1.6.1). L'historique n'a pas de curseur
  (un clic ouvre un titre, rien n'est « sélectionné ») et n'avait aucun gestionnaire de touches :
  sur un journal de 37 000 écoutes, la molette était le seul chemin. `Page suivante`,
  `Page précédente`, `Début` et `Fin` déplacent donc la VUE (`scroll_to_item` sur les lignes que
  la dernière image a construites), avec `ScrollStrategy::Center` : `Top`/`Bottom` sont
  « amener dans la vue » (course minimale) et laissaient `Page suivante` à **176 px** au lieu
  d'une page (`the_keys_scroll_the_history` : **398 px** mesurés, 5 à 6 lignes).
- **Pitfalls found by measuring**:
  - **A live Xvfb run cannot prove the WHEEL.** XTEST button 4/5 is not a notch on Xvfb: GPUI's
    X11 backend drops a scroll press flagged `POINTER_EMULATED` and the virtual pointer has no
    scroll valuator (`gpui-pre-linux`'s `XinputButtonPress`). Keys are real XTEST input and a
    capture proves them; the wheel is held by a `#[gpui::test]` dispatching the exact
    `ScrollWheelEvent` the backend builds from a notch (`ScrollDelta::Lines(point(0, -3))`).
  - **A scroll container's `max_offset` is POSITIVE travel**, while the offset itself is
    negative going down (`ScrollHandle::set_offset` docs): the two signs are one place where a
    page "scrolls the wrong way" or "does not scroll" (`Set asked=562px -> 0px` was exactly
    this).
  - **The window the bench drives is the NEWEST root child named `Neezer`.** A run whose
    `stop()` did not land leaves an identical 1280×800 window on the shared Xvfb at the same
    place: `wait_window` picks the biggest, the focus goes to the stale one, and every key,
    screenshot and pixel count then measures a dead app (measured: two runs of
    "changed = 0 px" on a list that scrolls). Snapshot the root's windows before the launch,
    wait for the id that appears after it, and give each run its own `XDG_*` tree and no
    `DBUS_SESSION_BUS_ADDRESS` (two instances of one session bus fight for the window).

### D8b — Library and playlists (delivered 2026-11-16)

- **The rule, a screen never takes the keyboard focus.** GPUI looks a keystroke up along the path
  of the FOCUSED node; a screen that called `window.focus` on itself left the window with nothing
  focused the moment it was unmounted, and every global binding — Space, `q`, `g h` — silently
  stopped. D7's own journey test failed the first time this lot's screen focused itself, which is
  how it was found. So a screen keeps the shell's anchor and registers its verbs on the APP with a
  weak handle (`App::on_action` + `Entity::downgrade`, exactly how D0 bound `Ctrl+Q`): they fire
  for the screen that is alive, and a dropped screen's handle answers `Err`. `q` over a real
  library screen is a regression test (`the_queue_panel_still_opens_over_the_library`).
- **The rule, `Alt+↑/↓` without editing the frozen keymap.** `keymap.rs` has no row-move binding
  (D7 freezes it), so the lot installs `alt-up`/`alt-down` → `MoveRowUp`/`MoveRowDown` ONCE per
  application (`install_row_keys`, a `Global` marker) in the shell's own `Neezer && !Input`
  context. The gesture therefore works over a list and never inside a text field; it is advertised
  on the screen itself ("Alt+↑↓ pour déplacer") because the `?` overlay cannot be extended.
- **The rule, a list paints from the disk first.** `/me/favorites`, `/me/playlists`,
  `/me/artists`, `/me/albums` and `/me/local` are cacheable paths, so the screen parses the cached
  body in the frame it is built (`Cache::get` + the model type) and asks the server right after —
  the SPA's `swr`. A 4 000-title favourites list is therefore **6.26 ms from cache**: 2.36 ms to
  parse and project, 3.90 ms to first paint (release, this machine; the acceptance asked < 100).
- **The rule, only what is visible is built.** Every title list is a `v_virtual_list` with uniform
  56 px rows; a grid paints one screenful (60 cards) and says "Afficher plus"; the rows carry the
  column names once (TITRE · ALBUM · DURÉE), which is also what keeps the album from reading as a
  longer artist line. `w_full()` on a row is REQUIRED: without it the row is content-sized,
  `flex_1` distributes nothing, and the album column lands under the artist (seen at 1280×800).
- **The rule, an optimistic write carries the version — and a 409 IS the conflict here.** A
  reorder is coalesced into ONE `PUT /order` 350 ms after the last move (the SPA's debounce), a
  removal is by ROW INDEX (a playlist may hold the same title three times), and a `409` re-reads
  once and says "la playlist a changé ailleurs". `nz-api` classifies EVERY `409` as
  `NotReady` (the stream's "nothing on disk yet"), so the desktop reads that variant as the version
  conflict on these mutations — the only `409` `playlist.py` answers. A `Conflict` variant carrying
  the body's `scope` would say it once for every screen (noted for `nz-api`'s owner).
- **The rule, an export is a DOWNLOAD, not a link.** A desktop has no browser holding the session
  cookie, and a playlist's ZIP is gigabytes: `Api::download_to(lane, path, destination, progress,
  cancel)` streams the body into `<file>.part` on `Lane::Bulk` (rule 1: it yields to playback) and
  renames it only once complete, with progress on screen and "Annuler" removing the partial file
  (rule 3). The file goes to `~/Téléchargements` (or `<data>/Exports` in a test). The public
  capability link is kept: it needs no session, which is what makes it shareable.
- **Measured** (this machine, release, `cargo test -p neezer-desktop`): **66 tests, 0 failed**
  (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` clean). The
  favourites' cache open: **2.36 ms parse+project, 3.90 ms first paint, 6.26 ms total** for 4 000
  titles on an 8 000-track library. Model memory for the worst case (8 000 favourites + a
  4 000-row playlist, both parsed and projected): **+16 772 KB** over the bare process. The app's
  own RSS (release, the real 8 000-track bench): **182 MB at Accueil**, **186 MB on the playlist
  shelf**, **198 MB with the 4 000-title favourites** (+13.8 MB for the list) and the same with a
  4 000-row playlist — the lists are windowed, so the model is the only thing that grows.
  `nz-api`'s new download has 3 tests against `perf_api.py` (a real CSV written and renamed, a
  cancellation that leaves nothing, a 404 that writes nothing), and the app has a journey that
  clicks the export panel's "Télécharger" and reads the file back.
- **Looked at, on an isolated Xvfb** (`DISPLAY=:99`, `WAYLAND_DISPLAY` unset, the real bench:
  8 000 tracks, 4 000 favourites, one 4 000-title playlist) at **1280×800**, **2560×1600** and with
  the **420 px** side panel open, keyboard (`g l`, `j`, `Entrée`) and mouse: the library, its folder
  tree, a playlist, the import screen. Eight captures are the lot's artefacts. The pass fixed three
  things a test cannot see: the playlist title floated to the right of its facts (the banner had two
  flex columns instead of one), the ALBUM column had no header and read as a longer artist line (now
  TITRE · ALBUM · DURÉE plus `w_full()` on the rows), and a clicked tab jumped back (the route is
  the source of truth: `pick_tab` does a `nav.replace`).
- **Pitfalls found by building**: (1) `App::on_action` returns `&mut App`, not a `Subscription` —
  handlers cannot be removed, so each holds a weak entity and goes inert; inside it `cx` is `&mut
  App`, and `WeakEntity::update` is the way back to a `Context<T>`. (2) A gpui-omarchy `Button`'s
  `debug_selector` is not recorded (its own element swallows it): wrap it in a `Div` to find it with
  `debug_bounds` in a test. (3) `InputState::set_value` needs a `Window`, so a task that clears a
  field sets a flag the next paint reads. (4) The screen is built by `screens::build` without a
  `Window`, so its `InputState`s are made on the first paint (`ensure_fields`). (5) `Nav::replace`
  is the only way a tab click may move: setting the screen's own `tab` is reverted by the next
  `sync_tab` (the route is the source of truth — found on the real server, at once). (6) A
  `directories::UserDirs` download folder must never be used from a test: `AppConfig::data_dir`
  wins when it is set.

### D9 — Now playing, queue, MPRIS wiring (delivered 2026-11-17)

- **The rule, the words are the MODEL's**: `widgets/lyrics.rs` used to parse `/api/lyrics/<id>`'s
  JSON itself, because `nz_model::Lyrics` declared the words as a bare string while the server sends
  the object (`neezer/deezer/lyrics.py`). The fix is in `nz-model` (`LyricsBody`), the widget calls
  `Api::lyrics_on(Lane::Prefetch, …)` and keeps only the two screen rules the server does not own:
  the lines are sorted before they drive the timer, and an empty document is « aucune parole », not
  an empty panel.
- **The rule, the lyrics are scheduled on their own timestamps.** The shell repaints four times a
  second (the bar's clock), so a line read off that tick flips 0–250 ms late — the SPA's own
  `activeLyricIndex` exists for that reason (`lib/lyrics.js`). Here one wake-up is armed for the
  NEXT line's timestamp, the position it reads is the engine's own audible clock (`nz-audio`'s
  `Engine::position` already subtracts the device latency), and a state notification re-arms it
  (dropping the previous task cancels it, GPUI's `clearTimeout`) so a seek or a pause moves the
  wake-up instead of leaving it behind. Measured: a paint 1 ms before a boundary shows the previous
  line, a paint on the boundary shows the new one — **0 ms of error at the timestamp, one frame
  (4.2 ms at 240 Hz) of scheduling**. The position read is the ENGINE's while it plays and the
  shell's playhead otherwise, which is what makes a restored track (position 95.4 s, engine not
  started yet) still ask for and show its words.
- **The rule, the lyrics come on the `Prefetch` lane, on the network runtime.** The words belong to
  the stream they follow (rule 1: `lib/ladder.js` lists them after the audio), and the request runs
  on `nz-net` — its quiet clock is a `tokio::time` deadline, and polling it from the GPUI executor
  panics with "no reactor running" (measured in the widget's first test). The widget only awaits the
  answer on the foreground task. One request per track, cacheable (`/lyrics/` is in
  `is_cacheable`), so a second visit costs **0 requests**.
- **The rule, a 4 000-row queue builds the viewport's rows.** `virtual_list` with one 56 px size per
  entry: an 800 px window builds **17 rows** whatever the queue holds (15 visible + 2 overscan),
  and a paint costs **35.9 µs in release** (314 µs debug) against the 240 Hz frame budget of
  4.17 ms — **1.2 %**. Only what is AFTER the playhead is editable (the SPA's rule), the playing row
  is told apart by a tint and its weight, and every edit is the state machine's (`jump`, `removeAt`,
  `move`, `clearUpcoming`). The row's own ✕/⌃/⌄ appear on hover (a 4 000-row queue of ✕ is noise)
  and always for the keyboard's cursor row.
- **The rule, three widths measured on the real window.** The stage is `[paroles][pochette+transport]
  [file]` from **1600 px** of content (the SPA's ROOMY), `[pochette][carte]` from **980 px**, one
  column below. The content width is the WINDOW minus the shell's fixed rails (200 px sidebar, 420 px
  queue panel) — measuring the window instead put a two-column layout in 660 px and the sleeve
  overlapped the card. At 2560×1600 the sleeve is 340 px and the lyrics column 300 px; at 1280×800
  (1080 px of content) the card's queue sits beside the transport; at 1280 with the 420 px panel open
  (660 px) the card stacks under the transport and shows the LYRICS — the panel already has the queue,
  and duplicating it was two scroll positions for one list.
- **The rule, the sleep timer runs where the audio is, not where the screen is.** The timer is a
  process-wide slot plus a detached foreground task, so leaving the now-playing screen does not
  cancel bedtime; its deadline is compared with the WALL CLOCK (a suspended laptop still ends where
  the clock says) and the last **15 s fade** is a multiplier on the ENGINE's volume — the person's
  own level is never touched and nothing has to be put back. **15 s, not the 10 s this paragraph
  used to say**: `webapp/test/sleep.test.mjs` and `docs/agents/webapp.md` pin 15, and "the same
  features as the SPA" wins. It fires a PAUSE (queue and position kept) and "end of the track" holds
  the crossfade back, then stops on the next track, paused. Measured: the fade is 1 at 15 s, 0.25 at
  7.5 s and 0 at the deadline; a clock that jumped 10 minutes past a 30-minute timer fires on the
  next tick; a spent timer pauses where it was, tested end to end.
- **The rule, MPRIS publishes ONE track's metadata and a stopped transport.** A new track is
  announced once (its length, artists, album and `mpris:trackid`), position ticks never re-announce
  it, and an EMPTY queue says `Stopped` with the last metadata kept — `playerctl status` used to
  read "Paused on the last track" for ever. Proven on a private `dbus-run-session`: the wiring's
  `publish` puts the four `xesam` fields and `Playing` on the bus, `playerctl -p neezer play-pause`
  (what Hyprland's binding runs) and a direct `PlayPause` call both arrive as
  `AppEvent::Mpris(PlayPause)`, and an emptied queue reads `Stopped`.
- **The rule, quality applies at the NEXT track and says so.** The badge opens the app's five rungs
  (Auto/FLAC/320/128/64 — `PlaybackQuality` has no 256/192; a menu that offered them would save a
  value the player cannot load), the choice is saved in `kv` (`playback.quality`, the SPA keeps it on
  the device) and the toast says « au prochain titre »; `AppState::load` reads it when it opens the
  next source. Podcast speed is per show (`channel_id`, else the album's Deezer id) and is applied by
  a watcher on the current entry, so an episode started from another screen still gets its show's
  rate; music is never touched (the engine re-checks `TrackMeta::podcast`).
- **Measured on this machine** (release binary, `Xvfb :94 -dpi 96` + `GPUI_X11_SCALE_FACTOR=1`,
  2560×1600 and 1280×800, a real server from `tools/perf_api.py --serve --archived` with a
  4 000-row queue, a 234 s archived FLAC with 39 `.lrc` lines, and four screenshots (2560×1600 and
  1280×800, each with and without the 420 px queue panel) kept with the session):
  - RSS, `/proc/<pid>/status` VmRSS: **267 MB** at 2560×1600 on the empty now-playing screen,
    **306 MB** with the 4 000-row queue, the lyrics and the sleeve (**+39 MB**); **206 MB** at
    1280×800, **214 MB** with the queue (**+8 MB**), **212 MB** with the 420 px panel. The shell's
    own 2560 baseline is the bulk of it (D7 measured 103 MB at 1280 on a build without D14a's
    Réglages); the screens' own share is the queue, the lyrics and one decoded sleeve.
  - **A sleeve per queue row WOULD cost +291 MB** (mechanical: each 1 000 px JPEG decoded for a
    32 px slot, twenty rows visible) — which is why D9 drew one glyph per row. The lot that
    brought the lists their sleeves keeps the arithmetic off the queue instead: a row paints the
    128 px copy (`Covers::rows`), 65 536 B decoded and 65 536 B of atlas, freed when the row
    leaves (see *Memory*, below).
  - `cargo test --workspace`: **128 app tests** (the D9 ones: 6 lyrics, 6 sleep, 3 speed, 1 quality,
    4 queue, 4 now-playing, 4 MPRIS wiring) and every crate green; `cargo fmt --check` and
    `cargo clippy --workspace --all-targets -- -D warnings` clean. `cargo test --release` fails in
    **D14a's** `screens/settings/ui.rs` (its hit points come from `debug_bounds`, recorded only with
    `debug_assertions`; D9's own mouse tests are `#[cfg(debug_assertions)]` for the same reason).
- **Pitfalls found by building**:
  - **A click on a row's ✕ also played the row behind it**, and because the remove ran first the
    jump landed on whatever slid into that index (`row_remove` then `jump(4)` → the NEXT track).
    GPUI bubbles mouse events: the inner handler needs `cx.stop_propagation()`, not just priority.
  - **`debug_bounds` returns `None` in release**: selectors are only recorded with
    `debug_assertions`. A mouse or layout test that uses them must be gated, or `cargo test
    --release` fails for a reason that is not the code's.
  - **`flex_1` + `max_w` did not clamp the main column** in this GPUI snapshot (the column took the
    whole row and pushed the queue card off-screen): a fixed `w(px(560)).max_w_full()` on the column,
    centred by the row, is what the layout obeys.
  - **The app opened a window in the person's session once**: `WAYLAND_DISPLAY`, `DISPLAY` and
    `HYPRLAND_INSTANCE_SIGNATURE` are inherited from the shell, and GPUI prefers Wayland when
    `WAYLAND_DISPLAY` is set. Every isolated run must `env -u WAYLAND_DISPLAY -u
    HYPRLAND_INSTANCE_SIGNATURE -u XDG_SESSION_TYPE DISPLAY=:NN`.
  - **This Hyprland (0.56, Aquamarine) has no headless backend**: `WLR_BACKENDS=headless` is ignored
    and the DRM backend cannot take a seat the person's session owns. Xvfb is the isolated display
    that works here, at `-dpi 96` plus `GPUI_X11_SCALE_FACTOR=1` (without the latter the app sees a
    2560×1600 window as 1280×800 and the layout is measured at half the size).

### D10b — Visualiser UI (bars, oscilloscope) (delivered 2026-11-17)

- **The rule, the page's arithmetic and our quads.** The geometry is `nz-viz`'s (V1, the page's own
  `rhythm::viz_bars`/`viz_scope`); what this lot owns is the DRAWING: one GPU quad per bar (body,
  inner core, falling peak) and one per scope column (`Window::paint_quad`, no tessellated path).
  The per-frame cost it controls is therefore measurable on its own: **p50 111–115 µs, p95 135–136 µs,
  max 1.2–2.2 ms** for 52 bars at 1 280 × 800 and 512 columns × 2 lanes of scope (release, this
  machine, `NZ_FRAME_LOG=1`) — **2.7–3.3 % of the 4.17 ms a 240 Hz frame has**, which is the number
  that says the next panel rate is affordable.
- **The rule, the cadence is the panel's by default.** `viz.fps` follows `stores.js` (Écran/30/45/60)
  with one deliberate difference: the desktop's default is the DISPLAY, because the acceptance is
  240 Hz. At "Écran" each paint asks for the next frame (`request_animation_frame`, GPUI's own frame
  source); at 30/45/60 — and in eco, which forces 30 whatever the setting — one sleeping pacing task
  spaces the frames, so the whole window is not re-rendered for nothing. `Animation::with_max_fps` was
  rejected on purpose: it obeys `reduce_motion` by freezing a repeating animation, and a frozen
  visualiser is not the feature.
- **The rule, nothing runs when nothing is visible.** The screen (and the Réglages preview) set
  `VizService::set_visible` on every paint and hand it back in `Context::on_release`; the analyser
  thread clears the tap and sleeps 50 ms while hidden, and the widget asks for no frame when the
  player is paused. Asserted on a real tap: **0 frames analysed over 40 rendered buffers while
  hidden**, then frames again once a screen asks for it
  (`the_analyser_does_no_work_when_no_visualiser_is_on_screen`).
- **The rule, the settings are the SPA's keys, and the panel is the same controls.** `viz.mode`,
  `viz.palette` (Pochette = `coverpalette.js#analyse` on the decoded sleeve, via GPUI's `RenderImage`
  — BGRA, downsampled to 64 × 64 before the analysis; Thème = `Palette::from_theme`), `viz.intensity`,
  `viz.fps`, `viz.eco`, `viz.scope.orientation`, `viz.scope.colour`. One `controls()` function, used by
  the screen's sheet (420 px) and by the Réglages section. **Deliberate difference**: in the SPA eco
  turns every animation off; here it keeps the scene at 30 fps with the flat grade (one fill per bar,
  no halo/cores/breath/peaks), because the lot's acceptance measures a 30 fps eco.
- **What it is now**: `widgets/bars.rs` (`Bars` entity: `set_prefs`, `set_active`, `frames`,
  `bar_count`, `Layout::{Strip,Full}`), `widgets/scope.rs` (`Scope`: lanes, graticule, min/max trace,
  halo+beam, trigger marker, `SCOPE_POINTS = 512`), `screens/viz.rs` (the full-bleed stage, the title
  card, the control bar that fades after 3 s, the settings sheet, the notices — full-screen for a
  cause, a quiet line at the bottom for a pause — and the keys `1`/`2`/`c`/`Maj+F` installed ONCE for
  the process, dispatched to a `WeakEntity` so the screen never takes the focus),
  `screens/settings/viz.rs` (`Prefs` + load/save, the palette helpers, `Sleeve`, `FrameLog`,
  `controls`, and the `Section` with a live preview).
- **Measured** (release, `Xvfb :95`/`:96` at 2 560 × 1 600, `GPUI_X11_SCALE_FACTOR=1`, a real FLAC in
  the play cache, `NZ_FRAME_LOG=1`, this machine):
  - the app's own frame log, 1 280 × 800, bars: **fps 24.0 and 27.9** over two runs, `paint_us`
    p50 **115/111 µs**, p95 **136/135 µs**, max 2.2/1.2 ms. At 2 560 × 1 600: **2.4 fps**, which is
    the software rasteriser, not the app (see the environment note).
  - `cargo test -p neezer-desktop`: **12 new tests** (the seven preference/geometry ones, the scene
    keys, the sheet, the analyser stop, the three-size layout) and the whole suite green; `cargo fmt
    --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean.
  - the screen was LOOKED AT: Xvfb :96 captures of the WINDOW (never the root: Xvfb keeps the pixels
    of a destroyed window, and two app instances overlapped on the first attempts) at **2 560 ×
    1 600** — the bar row, the oscilloscope, and the settings sheet over it — and at **1 280 × 800**
    (the row with the title card and the control bar) — kept as
    [barres 2 560](../desktop/d10b/barres-2560x1600.png),
    [barres 1 280](../desktop/d10b/barres-1280x800.png),
    [oscilloscope 2 560](../desktop/d10b/oscilloscope-2560x1600.png) and
    [la feuille](../desktop/d10b/reglages-2560x1600.png). The 420 px panel was opened on the run
    (the layout assertion covers the three widths); its capture did not survive the harness.
- **The environment limit, said plainly**: this machine's Xvfb has no DRI3, so Mesa cannot present a
  Vulkan surface ("No DRI3 support detected - required for presentation") and GPUI paints through a
  software path. **The 240 Hz / ≥ 230 fps criterion cannot be demonstrated here** — the fps above is
  the rasteriser's ceiling at 4 Mpx, exactly as D0 recorded for the first frame. The app-side number
  that can be held is the paint cost (2.7–3.3 % of the 240 Hz budget); the panel number belongs to a
  real session, D17's pass.
- **Pitfalls found by building**:
  - **`Window::paint_quad` paints in WINDOW coordinates.** `canvas`'s prepaint hands an element's
    `bounds`, and painting `bounds.size`-relative quads (forgetting `bounds.origin`) drew the bar row
    at the window's corner, *over the sidebar*: seen on the first real run, fixed by adding the
    origin. A geometry bench cannot see this; only the window can.
  - **A cancelled decode is not an error.** Pressing play while the restored queue was still loading
    produced `EngineEvent::Error … « lecture interrompue »` (a toast) — `decode.rs` returns it when
    its ring is closed by the next voice. The toast belongs to `state.rs` (D7/D9), not to the
    visualiser, and the follow-up fixed both halves: `decode::fail` says nothing for a voice that
    was stopped, and a late error for a track that is no longer current is logged, not toasted
    (`state.rs`, *The rule, a cancelled decode says nothing*).
  - The settings section needs one dispatch line in `screens/settings/mod.rs`
    (`SettingsSection::Viz => viz::body(...)`) and the "in the player" strip one `child()` in
    `screens/now_playing.rs` (D9's file). Both are outside this lot's files; the screen's own sheet
    carries the same controls, and the widget exposes `Layout::Strip` for the strip.

### D11 — Podcasts (delivered 2026-11-17)

- **Files**: `screens/{podcasts,show}.rs`, `app/src/widgets/{episode_panel,chapters,markers,
  transcript_panel}.rs`, captures under `docs/desktop/d11/`.
- **The rule, the episode list is the page's content**: the head never takes more than 45 % of the
  page, and it FOLDS the admin block before it reaches that cap. At 1280×800 the page is 656 px and
  the cap 295 px; the full hero (236) + actions (44) + the transcription card (220) + the transcript
  search (40) is 540 px, so the card used to be cut through a switch exactly where the list started
  — the complaint that opened this lot. `head_fit(page_height)` decides both: a compact hero
  (96 px sleeve, no description, 1.5 rem title) under 900 px of page, and the admin block as ONE
  summary row (« Réglages de l'émission · Archivage auto désactivé · 1 sur 8 transcrits ») that the
  person opens, until the head with it fits the cap (a page of 1200 px, i.e. a 1344 px window). The
  opened block scrolls inside the head (measured on the bench: the wheel moves its card into view
  while the list keeps 4 rows).
- **The rule, a page is a cursor, never a fetch per episode**: `GET /api/podcast/<id>?limit=50`
  paints the first page and nothing per row: **4 API requests** for the whole screen (the page, the
  show's markers in one read, the transcription settings for an admin, and the one-time
  `/podcast/progress` merge) against the bench, and « Charger plus » is the only thing that sends
  the cursor back (asserted on the stub's queries: `limit=50`, then `limit=50&cursor=…`, and 0
  requests under `/podcast/episode/` before a panel opens). The list is windowed: a page of 50
  builds the ~13 rows the window shows.
- **The rule, the panel is a column beside the list, and its body is the scroller**: every wrapper
  is a FLEX container. A plain block with `flex_1` gives its child no definite height, so the
  virtual list's `h_full` resolved to 0 and the episodes painted NOTHING beside an open panel while
  the panel's own body ran under the player bar instead of scrolling (both seen at 1280 and 2560;
  `the_episode_list_survives_the_panel_beside_it` is the regression test). Under 1040 px of content
  the panel replaces the list with « Retour à la liste »; on any width its ✕ folds it (a mouse has
  no Échap).
- **The rule, progress is written on pause, every 30 s, at quit, and once the link is back**: one
  keeper for the process (`attach_keeper`, started by the first podcasts/show screen), a position
  under `PROGRESS_MIN = 8 s` is a mis-tap and never written, the last 15 s are the server's own
  « finished », and the device's copy (`podcast.progress.<owner>`) paints before the server's merge. The
  speed is per show (`podcast.speeds`, the `channel_id`, else the album's Deezer id) and is applied
  by D9's watcher, so an episode started from another screen still gets its show's rate.
- **The rule, the transcript panel follows and searches where it is**: `follow_to` re-arms on the
  playing episode's frame only (a pause leaves the reader where they scrolled, `pause_follow`
  /`resume_follow`), a page of 200 lines is read once and re-read when the playhead leaves it, a
  poll is ONE request with `limit=1` and only while a job is running, and a search folds accents
  (`fold_text`) before it highlights. The server job is started, stopped and relaunched from the
  panel with each failure's own French cause (`engine_crashed`, `too_long`, `queue_full`,
  `already_running` read as a job, not an error).
- **T4's slot, ready and empty**: `transcript_panel::LocalTranscript` / `LocalSource` /
  `set_local_source` / `local_for` are the seam. When the server answers `status: "none"` and a
  reader is installed, the panel shows the machine's sidecar with a « transcrite sur cet appareil »
  badge and searches it in memory (`Search::locally`); `nz-transcribe` is untouched — T4 fills the
  reader and nothing else.
- **Measured** (this machine: 16 threads, Rust 1.99.1; the debug binary on a private `Xvfb :118`
  with `-dpi 96` + `GPUI_X11_SCALE_FACTOR=1`, a private `dbus-run-session`, its own XDG dirs and
  `tools/perf_api.py --serve --podcasts --tracks 8000`; RSS from `/proc/<pid>/status`):
  - `cargo test --workspace`: **every crate green**; `cargo test -p neezer-desktop` is **344 tests,
    0 failed** (7 of them the heavy benches their parent runs in a child process). `cargo fmt
    --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean. **35 of those tests
    are this lot's** (11 `screens::show`, 3 `screens::podcasts`, 5 `chapters`, 4 `episode_panel`,
    3 `markers`, 9 `transcript_panel`).
  - **requests**: podcasts **1** (`/api/podcasts`) + the keeper's one-time `/podcast/progress`;
    show **4** (page, markers, transcription settings, progress); a panel opens with 2 more
    (chapters, transcript), a transcript search 1, a page change 1.
  - **RSS**: **261 008 kB** on Podcasts, **261 912 kB** on the show (+904), **267 368 kB** with the
    episode panel open and its transcript page (+5 456), **270 580 kB** with the 420 px queue panel
    too (+3 212), **357 956 kB** at 2560×1600 (the window's own surfaces). `testmem`: the screen's
    model for the worst realistic case — 50 episodes, a 200-line transcript page and the 100 hits
    of a show search — is **98 877 B**: nothing per row beyond the model.
  - **a frame** of the 50-episode show, windowed, at the default window: **14.2 ms median in a
    debug build** (the release budget is 4.2 ms for 240 Hz; D13 quotes the release number, this lot
    had no release build to hand — the assertion is the loose debug bound).
  - **looked at**, 7 captures in `docs/desktop/d11/`: podcasts at both sizes, the show (compact head,
    folded admin block), the panel beside the list, the panel with the 420 px queue panel (the
    narrow « Retour à la liste » branch), and 2560×1600 with and without the queue panel. Both
    screens cap their content column at 1800 px, so a 2560 px window is margins rather than a
    two-metre episode row. The pass fixed three things no test saw: the list blank beside an open
    panel, the panel's body overflowing under the player bar, and a switch cut in half above the
    first episode.
- **What it is now**: `screens/podcasts.rs` (the subscriptions in one read, a 300 ms debounced
  catalogue search whose cards SUBSCRIBE, a pasted Deezer link, `j`/`k`/`Début`/`Fin`/`Entrée`
  through `track_list::own`), `screens/show.rs` (`PAGE = 50` + cursor, `head_fit`, the keeper and
  its `Watch` state machine, chapters/markers/transcripts, the batch transcription card, the speed
  menu, the 64 px windowed rows, `Exit`/Échap interceptors), `widgets/episode_panel.rs` (stacked
  chapters → transcript → description → facts, `Act::{Seek,LoadChapters,RetryChapters,AddMarker,
  DeleteMarker,ToggleDescription,ToggleAllChapters,Transcript,Close}`), `widgets/chapters.rs`
  (normalise/sort/dedupe, the playhead's chapter, the inline cap), `widgets/markers.rs` (one read
  per show, optimistic add/remove reconciled with the server's answer), `widgets/transcript_panel.rs`
  (pages, follow, search, the job's state card, T4's local branch).
- **Pitfalls found by building**: (1) a GPUI flex item with `flex_1` inside a BLOCK parent has no
  definite height — `h_full`/`flex_1` under it collapse to 0 (the blank list) or grow to content
  (the panel running under the player bar); make every wrapper on the path a flex container. (2)
  The window's own single-instance and MPRIS names are per SESSION BUS: a second isolated bench
  without `dbus-run-session` reads itself as a second instance, calls `Raise` on the other worker's
  app and quits — the symptom was an app that disappeared between two clicks. (3) Under Xvfb with no
  window manager the app-level key bindings (`q`, `g p`) did not fire while text into a focused
  field did; the suite's journey tests are what prove the keyboard, the bench drives the mouse.
- **The rule, the switch counts what it queued** (the contract was fixed by the follow-up):
  `POST /api/podcast/<id>/auto_archive` answers `{…channel, "queued": n}` (`archive_show`), and
  `Api::set_podcast_auto_archive` returns that body typed (`PodcastAutoArchive { queued, show }`).
  The screen says « Archivage activé : N épisodes en file » — 0 says there was nothing to queue —
  arms its poll only when something went in, and takes the switch's own value from the answer:
  no second read of the page. The workaround it replaces was `self.load(cx)` plus a generic
  « Archivage lancé », because `send_none` used to drop the count.

### D12 — Discovery (delivered 2026-11-17)

- **The rule, a station never runs dry**: the player announces `Effect::NeedMore` on the queue's
  LAST row, which is already a gap, so `widgets/flow.rs` keeps `UPCOMING_MIN = 2` titles after the
  playing one and asks while they are still queued. Measured (stub server, `cargo test -p
  neezer-desktop`): a Flow start is 1 request (2 titles: the playing one and its next), then ONE
  per title consumed — 6 requests for 5 titles — and the queue never held fewer than 2 upcoming
  when a beat looked at it. The `n` the server accepts is 1 or 2 (`discover.py#flow_next` clamps
  it), so a "the next two ahead" buffer costs exactly the SPA's one-request-per-title.
- **The rule, one attempt, then the cause**: one request in flight, `MIN_EXTEND_MS = 5 s` between
  two, a `(source, queue length)` key asked ONCE when its answer added nothing (the SPA's
  `exhausted`, which is what stopped a request storm there), and a failing choice retried
  `5 s / 15 s / 45 s` for the same row, then given up on with the cause on a toast (rule 3).
  Asserted on the stub's counters: a second beat right after the start asks nothing, an exhausted
  source is never asked twice at that length, and four `failed` calls are the end of the line.
- **The rule, the beat**: ONE task on the app's own entity reads the player every second while a
  station plays and every 20 s otherwise (`beat_period`, asserted), and it never repaints on its
  own. It is installed by the first D12 screen opened (`flow::ensure`); the shell is D7's file, so
  a session that never opens one of these four screens does not top up a radio started from an
  artist page — one line in `app.rs` closes that seam, reported rather than taken.
- **The rule, the level**: `widgets/discovery_slider.rs` writes the device copy at once
  (`discover.level`, the SPA's own key) and `PUT /api/discover/prefs` once per SETTLE (450 ms):
  twenty clicks in three seconds are one request, with the last value — a click is a value, never
  a drag of requests. The server's answer is adopted into the same copy, so the phone and the
  desktop agree.
- **The rule, Nouveautés**: `GET /api/releases` (the device's copy paints first), `GET
  /api/discover/prefs`, and `POST /releases/seen` once per fetched feed when it carries unseen
  releases — the badge keeps THIS visit's count, so a back navigation does not call the same
  releases new twice (asserted: 4 routes and the page's sleeves, nothing else). The two shelves
  come from the payload's `close_to`, and a card's click costs 0 requests: `ReleaseShelf` is a
  ROUTE in the SPA and `nav.rs` is frozen after D7, so a card opens the album and a release row
  folds its titles out in the shared windowed list.
- **The rule, the Récap**: 2 requests cold (`/recap/years` then the latest year), 1 per year change,
  **0 per card** — the whole year is one bounded payload. `←`/`→` are APP-level keystroke
  interceptors (a screen may not take the focus, D8a's rule, and `keymap.rs` is frozen), so the deck
  is navigable with nothing focused; `j`/`k`, `Page préc./suiv.` and `Début/Fin` come through
  `track_list::own`. The 7 s auto-advance runs only under 900 px and never under reduced motion
  (the SPA's own two conditions), and the year is NOT rewritten into the route (a `replace` would
  rebuild the screen and buy a second `/recap/years`).
- **The rule, the Historique**: one page per request with the keyset cursor, « Charger plus »
  appends and a duplicate id is dropped even if a page overlapped (asserted: two pages of the same
  body stay 2 events), the filter is the server's own `outcome` word (asserted on the query), and
  the rows are windowed at the SPA's exact geometry (44 px a day header, 72 px an event, so the
  virtual offsets need no measurement). A row plays and turns into its radio
  (`lib/actions.js#playThenRadio`), which is also what feeds the Flow service.
- **The rule, a new account lands on Bienvenue**: `state.rs#landing_route` sends a signed-in person
  to `Route::Onboarding` while `user.onboarding` is `"artists"` or `"account"` — the SPA's own
  `onboardingRequired = !REMOTE && stepFor($user.onboarding) !== "done"` — and to Home otherwise. A
  remote-control loan never onboards, and an account whose field is absent (an older server) lands on
  Home rather than on a step it cannot finish. Before this line the screen existed and NOTHING led
  to it: no key, no click, no login.
- **The rule, Bienvenue**: a like is optimistic AND one request; offline it waits in a queue that is
  flushed in order, and « Continuer » refuses while any of it is unsent (a finished onboarding that
  lost the choices is worse than one that waited). The tiles are real `Toggle`s — `Entrée` and
  `Espace` keep an artist exactly as a click does — and the first step has 4 tab stops (asserted:
  « Continuer », the search field, the two tiles; a disabled « Voir plus » is not one). The account
  step's import is a JOB the screen polls every 1.5 s with its progress and each failure's cause.
- **Measured** (this machine: 16 threads, Rust 1.99.1; `cargo test -p neezer-desktop`):
  - **162 tests, 0 failed** (26 of them this lot's: 12 pure rules + 8 headless UI). `cargo fmt
    --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean.
  - **RSS**, debug build under Xvfb + software Vulkan (the same bench as D8a, `/proc/<pid>/status`):
    **236 724 kB** on Nouveautés, **236 756 kB** on the Récap (+32), **236 756 kB** on the Historique,
    **236 852 kB** with the 420 px queue panel open (+96). The discovery screens cost ~130 kB over
    the shell — they hold one page of events, one year, one feed: nothing per-row (D8a's 237 MB is
    the shell's own, and the software Vulkan driver is most of the difference with D7's 103 MB).
  - **visual**: Nouveautés (with and without the 420 px panel open), the Récap's cards 1 and 4, and
    the Historique were looked at at **1280×800**, and Nouveautés at **2560×1600**, on an isolated
    Xvfb display (never the user's session; the window is resized through XTest because Xvfb has no
    window manager — D7/D8a's recorded limit). The screens cap their content column at 1800 px, so a
    2560 px window is margins, not a two-metre line of text. The shots are committed under
    `docs/desktop/d12/` (`nouveautes-1280`, `nouveautes-2560`, `nouveautes-panneau-1280`,
    `recap-1280`, `recap-carte-4-1280`, `historique-1280`), against `tools/perf_api.py --serve
    --recap --tracks 400`.
- **A contract that was wrong**: `nz-api`'s `Api::radio_next(seed: &str)` sends `{"seed": "<id>"}`
  while the route requires an OBJECT (`neezer/webui/__init__.py#radio_next` answers `400 invalid
  seed`), so the radios' refill goes through the client's generic `Api::send` with the SPA's own
  body (`Player.svelte#nextBatch`). A typed `RadioSeed { kind, id }` in `nz-api` is the 3-line fix
  its owner can make; nothing else uses the function today.
- **Pitfalls found by building**:
  - **`Toggle::on_change` is not a GPUI listener** (`Fn(bool, &ClickEvent, &mut Window, &mut App)`),
    so a tile holds a WEAK entity: a screen that is gone answers nothing.
  - **A test that waits on the REQUEST, not the answer, races**: D8a's menu test waited for the
    stub's `/api/search` count and pressed `Entrée`/`Menu` on results that had not landed — 1 run in
    6 failed once this lot's 41 extra tests loaded the machine (10/10 green before them). It now
    waits for the list, which is what those keys act on.
  - **`Entity::update` must not read the entity again**: `ask` was written against the `Entity` and
    panicked `cannot read AppState while it is already being updated` when the beat called it from
    inside `update`. The service's functions take `&mut AppState` + `&mut Context<AppState>`.
  - **`add_window_view` hands back `&mut VisualTestContext`**, not a value: `&mut cx` there is a
    `&mut &mut` and clippy's `needless_borrow` is right.
  - **`cargo test`'s tab walks need a laid-out frame**: `Window::focus_next` before the first
    `window.draw()` finds no stops, and `--test-threads` paranoia aside, the count must be read on
    a painted frame (D14a recorded the same for `Window::focus_next`).
  - **A `→` press needs an interceptor, an `Entrée` on a chip needs a button**: `card::chip` is a
    `div` with a click handler, so the year chips, the history filters and the two navigation chips
    became `toggle`s/`button`s — a mouse-only control in a discovery screen is a keyboard hole.
  - The bench (`tools/perf_api.py`) cannot seed an archived+measured library in ONE run:
    `--archived` inserts `track_analysis` rows a second time when `--recap` already wrote them
    (`UNIQUE constraint failed: track_analysis.track_id`), so `/api/flow/next` answers an empty list
    on it and the live Flow panel was proven by the tests, not by a screenshot.
- **What it is now**: `screens/{releases,recap,history,onboarding}.rs` and
  `widgets/{flow,discovery_slider,recap_story}.rs`; `widgets/mood_card.rs` stays empty on purpose
  (D8a's Accueil draws its own mood tiles, so the widget this lot was allotted has no caller — a
  delivered stub with nothing to say is not worth an unused module).

### D13 — Offline: the downloads worker and its screen (delivered 2026-11-16)

- **The rule, one worker and one lane**: downloads get no queue of their own. `nz-bulk` (one
  thread, `nice 10`) drains `nz_cache::Downloads`, and every byte it asks for is a `Lane::Bulk`
  request — so rule 1 is the gates' and not this file's: 1 s of quiet, one in flight,
  `X-NS-Background: 1`, and a body paced by the token bucket. Measured on an **800 kB/s** shared
  token-bucket link (loopback alone measures the kernel, not the rule): the same stream reads
  **797 529 B/s alone and 574 370 B/s while 200 tracks download — a 28.0 % drop** (budget 30 %;
  25.7–29.6 % over runs, the spread being the two sockets' buffers) and the armed cap reads
  **127 508 B/s**, a quarter of the *degraded* stream it measured (510 kB/s) — the feedback the plan
  asks for. With a real 60 s FLAC on the audio engine the same run renders **6.00 s of audio with 0
  underruns** and fetches **200/200** tracks. Rule 1 has a second half here: the meter. A cap of
  "25 % of the stream" needs somebody to MEASURE the stream, so `offline::note_stream_bytes` (fed by
  the player's own reader) publishes an EWMA over 250 ms windows and sets
  `bulk_cap_from_stream`; with no byte for 4 s the cap is released (a finished stream has no
  throughput to yield to).
- **The rule, a failure is never a verdict**: only the server's own classification (`X-NS-Error`, a
  4xx) or a full disk marks a row `failed`, with the CAUSE stored and printed one row each; a dead
  link and a `503 + Retry-After` put the row back to `pending` and the worker waits. Measured: a
  track refused twice with `503 {"queued":true}` then served **completes with `failed == 0` and 3
  arrivals**; a `404` keeps `HTTP 404 : …` and « Réessayer les échecs » re-asks it. Restart is the
  store's own `active` row: after a fresh worker start the published track costs **0 requests** and
  the interrupted one is fetched again.
- **The rule, the screen never reads SQLite**: the worker publishes one `Snapshot` (version, worker
  state, progress, sets, failures with causes, settings, both cache sizes) and the screen compares
  the version and skips the frame. `nz-cache`'s access rule and rule 4 in one shape: the UI thread
  takes a mutex and clones. That snapshot carries only what a row prints — the labels of the
  single-track rows, never the whole map (four thousand entries cloned on every publication).
- **The rule, a long list is paged**: 20 sets and 8 failures a page. Painting 400 rows in one frame
  cost **405 ms** (debug) — a hitch, not a list — and no per-row control is free: a `Button` per row
  costs about **5× more inside the shell's `focus_scope` than outside it** (166 ms against 31 ms for
  the same 28 rows, debug). Paging bounds the element count, so the cost does not follow the library.
- **Measured** (this machine: 16 threads, Rust 1.99.1; `cargo test -p neezer-desktop`):
  - `wiring::offline`: **18 tests** (the two acceptance benches, the restart, the cut link and the
    session seam among them; the four heavy ones run in a process of their own, see the rule above);
    `screens::settings::downloads`: 4, one of them the layout journey below; `widgets::download_badge`: 2.
  - the screen at 2 560×1 600, 1 280×800 and 420 px: the reading column is **896 px** (56 rem) at
    x=832 / x=192 and fills 420 px at 420, inside the viewport at all three; a page of 20 sets + 8
    failures paints in **3.9 ms median / 4.7 ms p95 in release** (45 / 57 ms in a debug build,
    which is why the assertion quotes the release budget) and costs **+5.2 MB RSS** in release
    (+7.3 MB debug) for a 200-set, 200-failure workload.
  - keyboard alone: Tab walks the page in reading order and Espace presses each control (the test
    sees the four quality chips at tabs 3–6 and the « hors-ligne » switch at 7); mouse alone: a real
    hit test on the FLAC chip changes the default quality.
  - the screen was LOOKED AT, not only asserted: the debug binary on a private `Xvfb :131` (never the
    person's session), a stub `/api/me`, a seeded store (215 downloads, 6 failures) and the window at
    **2 560×1 600**, at **1 280×800** and with the queue panel open at 420 px: the column keeps its
    896 px, the cards keep their order, the status reads « Terminé — 6 échecs » with `215 / 221`,
    and every failure prints its cause (three screenshots are attached to the lot's PR).
- **What it is now**: `wiring/offline.rs` (`Offline` — `start`/`ensure`/`configure`, `choose`/
  `choose_track`/`drop_set`/`drop_track`/`retry_failures`, `pause`/`resume`, `set_quality`/
  `set_offline_only`/`set_play_cache_cap`, `clear_play_cache`, `local`/`is_downloaded`, `snapshot`,
  and the free `attach`/`local_path`/`note_stream_bytes`/`keep_downloaded`), `Snapshot`/`Failure`/
  `Running`/`WorkerState`, the SPA's own keys (`offline.quality`, `offline.onlyDownloaded`,
  `cache.limit`) and the four-rung picker; `screens/settings/downloads.rs`
  (`Screen::new(section, state, cx)` — the module's own shape, `settings/mod.rs`'s arm — five cards,
  a pager, the failure list, the cache gauges);
  `widgets/download_badge.rs` (`chip` for a row that repaints anyway, `Badge` for one that does not).
- **Deliberate difference from the SPA**: the download picker offers four rungs (FLAC, Opus
  320/128/64), not six — the server answers `OPUS_256`/`OPUS_192` but `nz-audio`'s ladder has no rung
  for them, so offering them would download a file this app cannot describe.
- **The rule, offline is the OPENER's answer**: a downloaded track plays with the link cut because
  `NetOpener::local` (`wiring/engine.rs`) asks the play cache first and then
  `cache.downloads().local(owner, track)` — the file is found whatever quality the bar shows, since
  the bytes on disk are the only ones there are. Measured (`a_downloaded_track_plays_with_the_network_cut`):
  one request to download, then a client whose only address is a dead port starts the engine, renders
  **0.2000 of level with 0 underruns**, and asks the network nothing.
- **The rule, the app starts the worker at the session, not at the screen**:
  `AppState::on_restored`/`on_logged_in` call `start_offline()` (`state.rs`), so a queue left by a
  previous run resumes on the next launch. Measured (`a_restart_mid_download_resumes_the_set`): the
  worker is stopped mid-body (**1 row `active`, 1 published, 39 left**), a NEW worker over the same
  store finishes all 40 with **0 failures**, and nothing is fetched twice.
- **The rule, a heavy bench gets its own process**: `cargo test` runs a binary's tests in threads of
  ONE process, and the screens assert on `/proc/self/status`'s RSS. A bench streaming 200 tracks and
  decoding audio beside them cost D14a's settings page **+4 272 KB instead of +228 KB** and pushed
  D14b's 250-row test past its 4 MB bound. The four heavy benches are therefore `#[ignore]`d bodies
  run by a thin parent that spawns `current_exe() --exact --ignored`: still in `cargo test
  --workspace`, and every number is measured in the process it belongs to.
- **Pitfalls found by building**: `simulate_keystrokes` runs key BINDINGS only — `tab` moves the
  focus, but pressing the focused control needs a real `KeyDownEvent`/`KeyUpEvent` through
  `simulate_event` (without it a keyboard test presses nothing and passes vacuously). A screen
  mounted bare has no Tab binding at all: Tab lives in gpui-omarchy's `focus_scope`, so a screen test
  needs the shell's wrapper. `debug_bounds` records selectors only in a build with
  `debug_assertions` (a `cargo test --release` run cannot assert geometry). And a screen's frame
  cost must not be judged in a debug build: GPUI's layout and tab-stop bookkeeping are ~26× slower
  there (45 ms against 3.9 ms for the same page), which is why the budget is quoted in release and
  only loosely bounded in debug.
  - **A thread that borrows the runtime is JOINED before the runtime goes — and a panic in a
    worker thread is a bug, never something to catch.** `nz-bulk` drives every download with
    `Handle::block_on`, and tokio PANICS (it does not return an error) when a timer is polled on a
    runtime that is shutting down: `runtime::time::entry::TimerEntry::poll_elapsed`, « A Tokio 1.x
    context was found, but it is being shutdown. » Master's CI carried exactly that
    (`thread 'nz-bulk' panicked at tokio-1.54.1/src/runtime/time/entry.rs:534`, run 43964917017)
    while the bench that produced it still reported `ok`: the runtime was dropped under a worker that
    still had 200 tracks queued, and a `catch_unwind` around `block_on` turned the panic into « le
    réseau est coupé » and into a passing test. The cause is a lifetime, not an error: the worker's
    thread is now owned (`Worker`), `Offline::shutdown` joins it, and the `NetRuntime` global joins it
    in `Drop` — which Rust runs BEFORE the `Arc<Runtime>` field drops. The stop is a
    `tokio::sync::watch` raced against every `block_on` (`select!`), so a close abandons a request in
    flight instead of waiting for the next chunk. Measured mid-body with 40 tracks queued
    (`closing_the_app_mid_download_stops_the_worker_without_a_panic`): the close returns in
    **220 µs**, leaves **1 row `active`** and its `.part`, the run has **0 failures** and the
    process records **0 panics**; the 200-track share bench still measures 28.1 % drop with the cap
    at 127 545 B/s, so nothing got cheaper by accident. The same order is owed by every other thread
    that borrows the runtime, and the AUDIO ENGINE owed it next (`desktop/app/src/wiring/engine.rs`
    and `desktop/crates/nz-audio`): see the bullet below.

  - **The engine owns its threads too — a loader, a decode voice, a spooler is JOINED, never
    abandoned to the runtime.** `StreamOpener` drives `nz-api` with `Handle::block_on` from four
    families of thread that the engine spawns (load `nz-load-*`, preload `nz-preload-*`, seek
    `nz-seek-*`, decode `nz-decode-*`, spool `nz-audio-spool`), and none of them was joined: closing
    the window during a track, or while the next one preloaded, dropped the runtime under them.
    Measured before the fix, with the app's own drop order, on the same test
    (`closing_the_app_mid_track_stops_the_engine_without_a_panic`, child process, panic hook
    installed): `thread 'nz-preload-next' panicked at tokio-1.54.1/src/runtime/time/entry.rs:534` —
    « A Tokio 1.x context was found, but it is being shutdown. » — parked where a Prefetch request
    waits on its lane's `timeout_at`. The rule and the shape:

    - the ENGINE owns what it spawns (`loaders`, `voices` with their stop flags, `spoolers`), and
      `Engine::shutdown` stops the opener first (a `tokio::sync::watch` raced by every `block_on`,
      `select!`), then the voices, then joins the loaders, the decode voices and the spoolers, then
      the event bridge. Idempotent; `Inner::drop` calls it as the last resort;
    - a join SKIPS the caller's own thread: the last owner of the engine can be a loader, and
      `join` on yourself is `EDEADLK` (measured — the probe's first failure was that panic);
    - a spooler a SEEK superseded is `abandon`ed, not joined: it may be inside a socket read of
      bytes nobody wants, and joining it hung `seek_in_a_finished_network_file_uses_a_byte_range`
      for ever (measured);
    - the runtime's owner reaches the engine through a process-wide registry of WEAK handles
      (`wiring::engine::attach` / `shutdown_process`), because gpui drops the shell's state before
      the globals — the engine's only strong owner is gone by then. `spawn_audio` also stops the
      previous engine first, so a sign-in during a track never leaves a player on the old session;
    - no `catch_unwind` anywhere: the panic was a lifetime, and a swallowed panic is how it reached
      master's log with a green suite.

    Measured after the fix (same bench, 120 kB/s link, a 60 s FLAC): the close returns in
    **0.50–0.76 ms**, records **0 panics**, and the process has **0 engine threads** left when it
    returns — the assertion `left_at_close.is_empty()` is the decisive one, since it is what the old
    code could not satisfy. The same run proves the two bodies in flight never became play-cache
    entries, so an interrupted stream is not a track. Gapless, seek and crossfade are untouched:
    `cargo test -p nz-audio` (54 unit + 7 dsp + 4 network tests) is green, byte-range seek included.

### D14a — Réglages (user sections) — delivered 2026-11-17

- **The rule, one page**: Réglages is ONE entity (`settings::Screen`) with the SPA's
  category strip at the top (`SettingsSection::USER`, plus the two an administrator
  gets) and a « Chercher un réglage… » field that narrows it by label, accent- and
  case-blind. Each section is a file with one `body(screen, window, cx)` function; the
  two ADMIN sections are D14b's own screens and this page hands them the whole area
  (`delegated: Option<AnyView>`), while a section no lot has filled yet keeps D7's
  « Bientôt ». Switching a tab calls `nav.replace`, so `Alt+←` leaves Réglages instead
  of walking ten tabs back.
- **The rule, the sound changes the moment it is set**: the device settings are the
  SPA's `localStorage` in `nz-cache`'s `kv` (`audio.eq`, `audio.bass`,
  `audio.normalization`, `audio.mono`, `audio.balance`, `audio.fade.*`,
  `audio.presets`, `audio.output.device`, `ui.reduce_motion`, and the stream quality
  already in `playback.quality`), and every control writes the engine the instant it
  moves — D6's setters are atomic stores the output callback reads at the top of its
  next buffer. The stream quality is the one exception and its card says so ("au
  prochain titre").
- **The rule, the sections that do not fit get panes**: at 1280×800 with the queue
  panel open the content column is 660 px. Lecture et son is three panes (Son,
  Égaliseur, Transitions), Sources three (Ordre, Mes comptes, Accès), so each pane
  fits that window without a wheel — asserted by the headless layout test below.
- **The rule, one reply channel**: a request is spawned on `nz-net` and answers
  through the screen's own `events::channel::<Reply>()`, drained by one foreground
  task, so no worker touches GPUI state (D7's rule). Each section asks its routes
  ONCE (`loaded`): **8 GETs** for a person's whole tour (Découverte 2, Sources 3,
  Compte 1, Assistance 2; Lecture, Apparence and Raccourcis ask nothing), **+1** for
  an admin's Découverte (`/releases/settings`) and **+2** for their Sources
  (`/settings`, `/deezer/status`), and **0** on a revisit.
- **The rule, the sink list is the machine's, and `nz-audio` publishes it**:
  `nz_audio::output_devices()` lists the first host of `HOSTS` that has anything —
  PipeWire's own `node.description`s, or ALSA's devices on a box with no session
  manager — so the names are exactly the strings `probe` opens by and a chosen sink
  survives a restart. It replaced the card's own `pw-dump` subprocess: **2 real
  sinks in 13.0 ms** instead of 29 ms and 246 KB of JSON, no parsing, and a headset
  plugged in since the engine started is seen (a fresh host and registry snapshot).
  Two filters are the difference between a chip row and a device dump (rule 5):
  only the FIRST host that has devices is listed (PipeWire's two sinks beside
  **nine** ALSA plumbing lines — `Discard all samples…` — was the measurement), and
  the session manager's aliases (`default_sink`, `default_output`) are dropped
  because the « Sortie par défaut » chip already IS that choice. The same list is
  what `probe` walks, and `probe` was fixed with it: a NAMED device now matches on
  any host before any host's default is used, so a sink only ALSA reports opens on
  ALSA instead of silently becoming PipeWire's default. `set_output_device` hands
  the mixer to the new stream (nothing stops — proven below), and « Sortie en
  cours » is re-read 400 ms later, once PipeWire has it.
- **Measured** (this machine, Rust 1.99.1; `cargo test -p neezer-desktop`):
  - **102 tests, 0 failed** for the app (D14b's included), 38 of them this lot's
    (33 rules + 5 headless UI).
    `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`
    clean.
  - **layout**: all **10 user sections** (the three other lots' stubs excluded) at
    **2560×1600**, **1280×800** and **660×800** — the laptop with the 420 px queue
    panel open: the root is the window (±1 px), the body never leaves the window
    sideways, never grows past it vertically and never starts off-screen. 30 renders,
    no panic. **Every pane is walked too, not only the first** (Playback's three,
    Sources' three): a card added to a pane is exactly what breaks the rule above,
    and the follow-up's « Tronquer les silences » card is asserted that way at all
    three sizes.
  - **keyboard**: the Son pane has **25 distinct tab stops** and the order comes back
    to its first one (walked with `Window::focus_next`, the call gpui-omarchy's Tab
    makes), Shift+Tab retraces it, and the search field narrows the strip to
    « Découverte » and back. Page Down / Page Up scroll the page itself and Home / End
    jump to its ends (Assistance is the long one), on the page's own focus — four keys
    only, so `j`/`k` stay the lists'.
  - **mouse**: the Apparence tab, the Mono switch and the « Normale » chip are
    hit-tested at 1280×800 and move the same state a key would (the route changes
    with `replace`, `values.mono` flips, the level is saved).
  - **Memory**: **20 560 B** retained per tour of the ten user sections with a
    **4 000-row queue** in place (bytes allocated minus bytes freed on the test's
    own thread, `app/src/testmem.rs`'s counting allocator; the same byte count on
    **30 runs out of 30** under an 8-process load, 260 280 B for the first tour
    through caches the warm-up had not built yet, budget **64 KiB**). The page
    holds no per-row data, so the worst case stays the queue the shell owns. Of
    those 20 560 B, ~2 KiB per `Screen` is retention the page does not own: the
    `drain` task GPUI keeps for the screen's reply channel outlives it because
    `events::Sender` sets no `closed` flag when dropped (a separate probe read
    520 B per `Screen::new` + drop with no window). Small, linear per section
    switch, and NOT this lot's file — reported, not fixed here.
  - **Why not RSS anymore**: `VmRSS` is the process's, and the suite runs its
    121 tests in parallel threads of one process: the same page read **+8 404 KB**
    and **+2 604 KB** in CI under load (PR #236, two red runs), and a neighbour's
    480 MB buffer turned one local run into **+480 944 KB**. A budget there is a
    budget on the scheduler and on glibc's arenas, not on the page. Bytes counted
    on one thread cannot move with a neighbour, so the budget got **32×** tighter
    and still fails on a real leak (a deliberate 4 MiB `mem::forget` in a section
    read **8.4 MB** and failed at once).
  - **requests**: the table above is asserted by a test; a revisit asks nothing.
- **Pitfalls found by building**:
  - **A memory guard that reads `VmRSS` guards the test scheduler, not the
    screen.** One process runs every test of the crate in parallel threads, and
    RSS also moves with glibc's arenas and with any neighbour's big buffer: the
    same ten sections that keep 20 560 B read +228 KB alone, +2 604 KB and
    +8 404 KB in CI (PR #236), +480 944 KB beside one test that allocates
    480 MB. Measure **bytes retained by the measured thread**: a
    `#[global_allocator]` that adds `layout.size()` to a const-init
    `thread_local!` on alloc and to another on dealloc (`app/src/testmem.rs`, a
    `#[cfg(test)]` module and a `#[cfg(test)]` `#[global_allocator]`, so the
    shipped binary links the system allocator and its size and first frame are
    unchanged — 45 979 912 B and the same `.text` before and after), then
    `allocated - freed` between two readings, the minimum of five tours. Two
    const-init `Cell`s mean the allocator itself never allocates, so it cannot
    recurse, and a neighbour thread's bytes are invisible. Assert the **minimum**
    of several tours (a page that keeps something per row adds it to every tour,
    so the minimum still fails) and write the measured number next to the budget.
  - **`.id()` returns `Stateful<Div>`**, not `Div`, and edition 2024 refuses `use<>`
    on a generic `impl IntoElement` return: the shared builders return
    `Stateful<Div>` (or `AnyElement` when generic).
  - **A GPUI test window that re-focuses its anchor on every paint steals the
    keyboard back.** The Tab walk saw 2 stops and bounced until the harness view
    anchored ONCE (as `RootView` does): the same walk then found 25 stops that wrap.
  - **`Window::use_keyed_state` may only be called during layout/paint**: the search
    and ARL fields are created in `render` (per window, so a draft survives a tab
    switch), not in `Screen::new`, which the route table calls without a window.
  - **D7's "a tokio worker must not wake a GPUI foreground task" bit again**: the
    sink read is a `spawn_blocking` whose reply woke the screen from a blocking-pool
    thread and panicked the deterministic scheduler. The read is now gated on an
    engine existing (`audio.is_some()`), which is also the honest rule: no output, no
    sinks to choose.
  - **A paused `OfflineSink` never consumes the mixer's commands.** `Engine::send`
    pushes into a bounded `rtrb` queue that only `render` drains, so a test that
    loads a voice and waits for `Started` without rendering never gets it (measured:
    5 s of waiting, no event, the install command dropped after its 500 ms retry).
    Pump the sink while waiting — which is also what makes the clock advance only
    for audio that was really rendered.
- **Two seams this lot reported, landed by the follow-up** (both live in `state.rs`,
  D7's file: a screen cannot reach a track change):
  - **Tronquer les silences** is now a real switch in the Transitions pane, with the
    SPA's threshold slider (−70…−25 dB, default −45). `AppState::load` asks the
    playing track's bounds at USER priority and `preload` the next one's at
    PREFETCH, then sets `TrackMeta::edges` before a voice is built; the session
    keeps 512 answers and asks once per (track, threshold). `set_trim` changes the
    sound at once: off drops the bounds of what is playing, a new threshold throws
    the old answers away. Measured: the two requests leave in that order, one ask
    each, `db=-45`; the engine's meta for the next track is `{0.5, 2.5, 5.5}`; the
    first rendered window peaks at **0.2500** and the clock starts at **0.500 s**
    (details in *D6 — the rule, the trim*). The switch is a control that ACTS
    (rule 2): one real click on it at 1280×800 turns the trim off, writes
    `audio.trim = false` on the device and turns it back on — and the headless
    layout walk asserts the card's geometry in the Transitions pane at
    **2560×1600**, **1280×800** and **660×800**.
  - **The saved sound** is applied at `AppEvent::AudioReady`, not when this page is
    built: the EQ (10 bands), the normalisation level, the bass lift, mono, the
    balance and the crossfade reach the engine at a cold start, and the saved
    output device is opened too (`set_output_device`, whose failure is logged there
    — the engine reports a real device that refuses through `EngineEvent::Error`).
    Measured: an engine that starts `eq(0) = 0`, `Off` reads **6.0 dB on 31 Hz,
    9.0 dB of bass, `Medium`, mono, −0.30** after the one event.
  - **A switch stops cutting the music**: `changing_the_output_hands_the_mixer_over_and_the_music_keeps_playing`
    (`nz-audio`) plays a real FLAC and switches sink mid-track — **0.597 s → 1.088 s**
    across the handover, **0 underruns**, same track, no `Ended`/`Error`. On CI (no
    sound server) the pump has no device to switch and the test asserts the honest
    `Err` instead.
  - **Counted**: the desktop workspace is **246 app tests + 6 child-process
    benches**, **75 in `nz-audio`** (the head+tail trim and the sink handover are
    two of them) and **38 in `nz-api`** (22 unit, 9 lanes, 3 real-server bench —
    the `409` pair and the `auto` payload among them); **634 in the whole
    workspace**. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D
    warnings` and `cargo test --workspace` clean.
- **A contract that was wrong, fixed**: `nz-api`'s `set_account_auto(id, bool)`
  posted `{"auto": true}`, while `myaccounts.my_accounts_options` requires
  `{"auto": {kind: bool}}` and answers `400 auto invalide`. The function now takes
  the kinds the account announced and builds that map, so the Sources screen calls
  it instead of hand-building the body through `Api::send`. Pinned against
  `perf_api.py`: the typed call is **200** with every announced kind at the value
  asked, one kind alone leaves the others untouched, and the old bare boolean is
  **400 `auto invalide`**. `link_deezer`'s doc was corrected too: the server only
  distinguishes `"browser"` from every other value (a typed ARL).

### D14b — Réglages (admin sections) (delivered 2026-11-16)

- **The rule**: an admin screen is a column of CARDS, and every card that can
  start work owns its own JOB. The start route returns at once, the card polls
  its own `*_status` route on the app's `nz-net` runtime at the SPA's own
  cadence (archive 1.5 s, migration 2 s, MusicBrainz and credits backfills
  1.5 s), stops when the job says it is done, and then says what it did or the
  CAUSE of each failure — the tally of reasons, then the rows, then « Copier le
  détail » (`JobFailures.svelte`'s contract). Measured: a running archive is
  polled **once per 1.5 s, 2 calls for a job that ends after the first poll**,
  and an idle Serveur tab makes **0 further requests** over half a second.
- **The rule, one sequential read pass**: opening Serveur is **10 reads** on
  `Lane::User`, one after the other, each card painting as its answer lands — so
  a slow link never sees ten requests compete (rule 3), and one dead route costs
  its card, not the page. The SPA's Serveur tab makes eleven, because
  it also reads the Deezer status and the release settings that this app keeps
  in the Sources and Découverte sections. Measured against a real socket:
  **10 requests, 2 242 B of JSON** (the harness's bodies; Utilisateurs: **2
  requests, 573 B**).
- **The rule, the token is shown once**: enrolling a transcription worker mints
  the token with the row, shows it in a box that says so, offers « Copier le
  jeton », and **keeps it nowhere else** — reopening the screen cannot bring it
  back because the server never answers it again. Revoking sends the DELETE for
  that worker's own UUID path, and an id that is not a UUID (a hostile payload)
  never becomes a route. The SPA has no worker screen at all: these three routes
  (`GET|POST /api/transcribe/workers`, `DELETE …/<id>`) existed on the server
  with no client, and this lot is their first.
- **Why some payloads are read as raw JSON, and the contract corrections**:
  - `/enrich/backfill` answers the shared `libjob` snapshot **plus** its
    coverage, but `nz_model::EnrichStatus` (D1) typed only the coverage half, so
    the desktop reads `EnrichLive` locally (`#[serde(flatten)] JobStatus +
    coverage`) instead of losing `done/failed/failures`. A test pins the payload.
  - The worker routes have no `nz-api` function (they never existed in
    `api.js`), so the screen uses the seam D2 left public — `Api::get` and
    `Api::send` — rather than editing a crate this lot does not own.
  - The inventory's Réglages → Serveur line also names **automatch** and the
    **similar backfill**: in the SPA they live in Bibliothèque → Titres
    indisponibles (`AutomatchPanel`) and in the genre studio (`SimilarRebuild`,
    D16), not in Réglages; this lot does not duplicate them. `POST /api/upload`
    (multipart) is a library action too, not a Serveur control.
- **The rule, no invented dates**: the payloads carry naive server-local
  timestamps and the workspace has no timezone crate, so these screens print
  the server's own wall clock (`Vu le 16 nov. 2026 à 20:15`) instead of a
  relative time that would be off by the UTC offset. A trailing "il y a 3 h"
  needs a clock the client does not have; the absolute date is exact.
- **The rule, keyboard-only as well as the mouse, and never by taking the
  focus**: Tab reaches every control through `focus_scope`, and the page takes
  **Page Down / Page Up** as app-level actions (`actions!(settings_page, …)`,
  bound once, handled through a **weak handle**). The screen never focuses
  itself — D8b's trap: a screen that takes the focus breaks the shell's focus
  anchor when it is unmounted, and every scoped key (`Espace`, `Ctrl+→`) dies
  with it. A regression test visits Serveur, comes back to the shell and plays
  with `Espace`. Home/End are deliberately NOT bound: they belong to every text
  field's caret. A sheet is the only place focus moves, and it gives it back to
  whoever had it (the shell's anchor included) when it closes.
- **What it is now**: `screens/settings/server.rs` (archive, migration,
  MusicBrainz, credits, archive rules, cleanup preview + run, storage + cache
  flush, server transcription settings, workers, upload quota) and
  `screens/settings/users.rs` (25-row keyset pages capped at 250, six filters,
  account sheet with promote/disable/delete-typed, invitations with
  create/renew/revoke and the link shown once), plus the two dispatch arms in
  `screens/settings/mod.rs`. Both screens keep their own `Inbox` (an mpsc the
  GPUI thread drains) because `AppEvent` belongs to D7's shell and may not grow
  a variant per screen; the requests themselves go through `nz-api`, never a
  second HTTP path.
- **Measured** (this machine, Rust 1.99.1, debug build, `cargo test -p
  neezer-desktop screens::settings`): **25 tests, 0 failed, 1.7 s** (the 1.5 s
  poll is most of it); `cargo fmt --check` and `cargo clippy --workspace
  --all-targets -- -D warnings` clean. The screens' own worst case — **250
  accounts + 200 invitations = +1 380 KB RSS** in a unit test (the caps the
  screens enforce; the widgets are built per paint, so the count does not
  follow). The whole process's RSS on these screens could not be measured here:
  this machine has **no Xvfb and no Xephyr** and Hyprland's headless backend
  dies with `CBackend::create() failed!` (measured), and the user's session must
  not be driven — the same wall D7 hit. The layout is a wrapping column capped
  at 64 rem, so 660 px (1280 with the 420 px panel open) and 1 940 px (2560)
  scroll the same cards; D17 owns the full-size screenshot pass.
- **Pitfalls found by building**:
  - **A GPUI background timer does not fire in the test harness** (it did not
    advance within 10 s), so a poll that sleeps on `cx.background_executor()` is
    untestable and, worse, unmeasured. Every timer of these screens (job polls,
    the search debounce) runs on `nz-net` instead and answers through the same
    inbox as the requests — one clock, driven by the same loop the D7 journeys
    use.
  - **`StatefulInteractiveElement` is what carries `on_click` and
    `overflow_y_scroll`**, not `InteractiveElement`: a bare `div()` cannot be
    clicked or scrolled, it needs `.id(...)` first. The modal veil, the scrolling
    sheet and every row are stateful for that reason.
  - **`InputState::focus_handle` needs the `Focusable` trait in scope** — the
    inherent method the docs imply does not exist on `InputBaseState`. Without
    it, a field cannot be left alone while the person is typing in it.
  - **A number field and its chips drift** unless the render puts the server's
    value back into the field: clicking « 180 min » left the text box at 120
    until `sync_field` wrote the new value (never while it holds the focus).
  - **`Box::leak` in a render is a leak per paint**: the archive rules' element
    ids were built with `format!` per frame at first. Static ids per rule
    (`event_id`) fixed it; clippy would not have caught it.
  - **A timestamp is hostile input**: `&iso[11..16]` panics when a multi-byte
    character sits where the clock belongs. Every slice is `str::get`, and a
    time is printed only when it is `HH:MM` digits (`date_fr`/`datetime_fr`,
    pinned by a test with `éééé`).

### D15a — Remote control (delivered 2026-11-17)

- **The rule, a command IS a player-store call**: `remote.py#OPS` and `commands.js` are the same
  vocabulary as `nz_player::Command`'s serde tags, so the desktop DESERIALIZES a wire command into
  the state machine's own type and applies it through `AppState::command` — the one transport path a
  click, a key and MPRIS already take. The owner's hand is never taken: their taps work as always
  and the last action wins. A `set` command is the one op that is not a player command; it takes the
  `kv` path Réglages uses, for the five settings this build can honour (`player.quality`,
  `podcast.speeds`, `offline.quality`, `offline.onlyDownloaded`, `cache.limit` — `viz.*`, `fx.*`,
  `fade.*` are D10b's and D14a's keys, and the server only accepts a key the player PUBLISHED).
- **The rule, two loops and one foreground tick**: the network task polls
  `GET /api/remote/host/<device>/poll?since=&chan=` every 700 ms while somebody drives and 1.5 s
  otherwise, publishes `POST /api/remote/host/<device>` when the shape changed or every 15 s
  (heartbeat), and stops the moment the server says no link is left; the GPUI tick (120 ms while a
  link is live, 2 s otherwise) applies what arrived and hands back the state to publish. The split is
  not optional: applying touches `PlayerState` and the engine, and a `tokio::time` deadline polled
  from the GPUI executor panics (D9 measured it). The queue is sent only when a POINTER FINGERPRINT
  (`queue_fingerprint`: `seq` + the order of the row `Arc`s) moved, so a 120 ms tick never clones
  5 000 rows.
- **The rule, nothing at all until this device has been lent**: `remote.hosted` in `kv` is written
  the first time a link is made here; a launch that finds it false starts NO request and NO task
  (not even the tick's timer), and a launch that finds it true asks `GET /api/remote/links` and
  resumes.
  Measured: without it, D8a's own "a screen open fans nothing out" test read **8 requests instead of
  7** at login (PR #236's discipline), and a desktop that never used remote control paid a request
  per launch for a feature nobody used.
- **The rule, rule 1 by lane**: every request here is `Lane::User`. It is the one lane that never
  waits and that never holds the single `Prefetch` slot the next track's audio needs; `Bulk` would
  put a tap behind a whole download body (1 s of quiet plus one slot shared with `nz-dl`), which is
  how a remote control would start slowing the music down. The payloads are debounced and coalesced,
  the queue goes only on a change, and both loops STOP (not slow down) when nothing is lent.
- **The rule, a loan keeps nothing**: on a session whose `/api/me` carries `remote`, `start` drops the
  audio engine, sets `Api::set_caching(false)` and `adopt`s the owner's state by REBUILDING the
  player from a `Snapshot` while DROPPING every effect (`adopt_effects`) — no load, no listen report,
  no queue written to this device. The controller applies a tap locally at once and holds that view
  until the owner's player has RUN it (`ack`, 2.5 s at most), so a tap never flickers back; the
  position runs on the owner's published line `{p, t}` with the clock skew measured as the round
  trip's midpoint.
- **What it is now**: `wiring/remote_host.rs` (`device_id`/`device_name`, `HostStatus`,
  `Published`, `Session` — `poll_path`/`publish_body`/`absorb_poll`/`absorb_publish`, `Pending`,
  `decode_pending`, `queue_fingerprint`, the service with `ensure`/`start`/`stop`/`resume`/`tick`,
  `settings_snapshot`/`apply_setting`, `link_url`, `expiry_label`, `LinkRow`, and the session's
  entry `start`); `wiring/remote_controller.rs` (`Grant`, `op_level` (the UI's hiding, the server's
  `POLICY` being the enforcement), `Status`/`Phase`, `Anchor`, `skew_from`, `Read`, `parse_queue`,
  `adopt`, `token_from_input`, and the service's `command`/`tick`/`run`); `screens/remote.rs` (the
  owner's sheet: four levels with their blurbs, four durations, the « Pour » label, the invite with
  its URL, the live links with their expiry and connected count, « Couper » per link and « Tout
  couper » armed once; plus the claim form and the loan's panel with its transport and queue). The
  chip « Contrôlé par … / Couper » is `app.rs`'s (it must be on every route, never over the
  transport), the sidebar's « Contrôle distant » row is its way in, and `state.rs` gained the one call
  `remote_host::start` in `on_restored`/`on_logged_in` — the three files this lot touched outside its
  own, with `neezer-50`'s blessing; `screens/remote.rs` is reached from that row and by the
  `RemoteClaim` action D7 declared and nothing handled.
- **Measured** (this machine, Rust 1.99.1; the real server is `webapp/test/remote/server.py`, four
  WAV tracks, the fixture the SPA's own `test/remote/run.mjs` drives):
  - **command → sound: best 6–15 ms, worst 710–718 ms** over three taps phased across the poll cadence
    (the owner's loop polls every 700 ms), both under the lot's 1.2 s; the 120 ms foreground tick
    brings the app's own worst case to ~840 ms. The command is applied to a real `PlayerState` and
    the asserted effect is `Effect::Load { autoplay: true }` — that is the sound.
  - `cargo test -p neezer-desktop`: **25 tests of this area** (15 host, 10 controller) plus the
    screen's 4, 0 failed; the e2e also proves the ack (`ack` = the seq the player ran), the restart
    (`need_queue: true` on a new launch's publish, then false), and the cut (the controller's next
    read is `401 remote ended`).
  - the app, on a private `Xvfb :137` at 2560×1600 (screenshots kept with the session): the owner's
    sheet renders, the link made through the API is listed with « Expire dans 1 h », the live line
    and the chip say « Contrôlé par Le salon · Lecture seule » as soon as a controller reads, and
    `q` opens the 420 px queue panel OVER that screen — the screen takes no focus (D8b's rule), so
    the shell's bindings keep working. RSS 244 MB at 2560×1600 on this screen.
  - **resume after a real restart**: a launch seeded with `remote.hosted` and a live link answered
    `/api/remote/state` with `online: true` and a published state WITHOUT anybody opening the
    screen — the SPA's `initRemoteHost` at boot, proven on the app's own store.
- **Not done, and why**: no screenshot of the LOAN panel on this machine. A controller app needs a
  claimed cookie in its own store; the second instance kept coming up on the login screen while the
  first held the display, and GPUI under Xvfb has no DRI3 here (the window sometimes never
  appears — D8a hit the same wall and CI runs the window under lavapipe). The loan's logic is
  covered by the protocol test (a real client claims and drives), by `adopt`'s own test (it changes
  the player and executes NO effect) and by the level table's.
- **Pitfalls found by building**:
  - **`need_queue` is not "the channel is empty", it is "the last publish left the queue out"**: the
    server adopts the `qv` of a publish that CARRIES the queue, so a first publish with it answers
    `need_queue: false`. The test asserted the opposite and was wrong — a restart's cue is a publish
    WITHOUT the queue.
  - **A `MutexGuard` across an `.await` makes the future non-`Send`**, so `nz-net` refuses it: the
    loop's error branch has to scope its status update in a block.
  - **`virtual_list` takes `Rc<Vec<Size<Pixels>>>`, `gpui_kit::base::VirtualListScrollHandle`, and
    it hands back `Entry`s, not `Arc<Entry>`** — the queue screen's own types, not the player's.
  - **`use_keyed_state`'s initialiser returns the STATE, not an `Entity`** (`cx.new` there yields
    `Entity<Entity<InputState>>`), and `Button` has no `debug_selector` (D8b's note): a control a
    test must find is wrapped in a `Div`.
  - **`pkill -f "server.py"` kills the shell that runs it** when the pattern is in its own command
    line (measured twice, with 143s): keep the server in the foreground of a background shell and
    hold its stdin with `sleep`.

### D15b — Écoute partagée (delivered 2026-11-17)

- **The rule, the guest schedules on the OUTPUT clock, not on a timer.** A party guest receives a
  LINE (`{anchor: {t, p}}` — the media position `p` at server instant `t`) and six-second chunks of
  the album, and must have each chunk come out of THIS machine's speakers at the instant the host's
  does. `nz-audio` grew one channel for it (`crates/nz-audio/src/party.rs`, `Engine::party()` →
  `PartyChannel`/`Segment`): a segment says "these PCM frames are heard over `[from_s, to_s)` of the
  OUTPUT STREAM", the callback writes whichever samples cover the positions it is filling, and the
  device's own `playback` timestamp is what says where that stream is (`Clock::output_anchor`,
  `Engine::output_clock`). No second output stream, no second DSP chain: the party's audio goes
  through the same EQ, bass, limiter, volume and tap as a track, and `PartyChannel::take_over` stops
  the person's own voices (their queue and position are kept) while somebody else's music plays.
  **Sample-exact**: a click scheduled at stream 2.000 s is rendered at frame 96 000, 0 frames off,
  and still 0 frames off with +6 dB of EQ, bass and normalisation engaged.
- **The rule, a line is FITTED, and the fit restarts on two events.** The host's position read steps
  per audio callback and the read lands somewhere in a timer callback, so the host publishes the
  MEDIAN of the recent `position = c + time` readings (`appcore::sync::AnchorFit`, the page's own
  Rust) and restarts it on a new track and on a device-mapping step. That second one is not a
  detail: the mapping `at − pos` moves (Chromium steps it by a render burst; a card's reported delay
  settles the same way), and a line drawn across a step is off by the step for seconds. `Clock` now
  counts those steps (`output_generation`) and `Snapshot::generation` restarts the fit — the same 4 ms
  threshold as `appcore`'s `OutputClock`.
- **The rule, the seam is where drift dies.** The device's crystal runs tens of ppm from the server's,
  so a guest that scheduled a chunk 30 s early landed **1.05 ms** off it (measured); a guest places
  only what is 7 s ahead (`SCHED_AHEAD_S`) and re-places every chunk from the line in force when it
  places it, so the error is bounded by 7 s of drift and reset at every seam. Measured over ten
  minutes of listening at +35 ppm: **worst |error| 0.452 ms between seams, 0.000 ms at a seam, 0
  corrections**. A 20 ms mapping step: 0.000 ms after the two readings that believe it.
- **The rule, one correction rule** (`timeline.js#correctionFor`, ported): ≤ 3 ms nothing, ≤ 30 ms at
  the next seam (`ClearFuture`: the sounding chunk plays out, the future is placed again), behind by
  more skip forward now, ahead by more fall silent and resume the same audio — past 2.5 s ahead it is
  the host going BACK and the guest goes with it. With a pull scheduler "behind" and "ahead" are the
  same operation (`Clear` + re-place from the line), and it never repeats what the room heard.
- **The rule, a cut is not a stop.** A poll that fails keeps the line it has, so the music carries on
  through the outage, extrapolated; a link that comes back **after** the look-ahead ran out rejoins
  with |error| **0.000 ms** within one tick plus the 100 ms lead, and the view says « Connexion
  perdue — la musique continue » rather than pretending (rule 3). Chunk fetches are `Lane::User`, two
  in flight (`engine.js#MAX_INFLIGHT`), four ahead, and a chunk the server is still cutting is
  retried after the delay it asks for.
- **The rule, the host is only WATCHED.** `wiring/party_host.rs` reads `Engine::position()` (which
  already subtracts the device latency — the number the page has to model by hand) four times a
  second, publishes `{track, playing, buf, t, p, xfade, next, norm}` only when the line MOVED or the
  plan changed (`host.js#measure`'s `need`, field for field), a heartbeat every 10 s otherwise, one
  publish in flight, coalesced. The next handover is announced with `at`/`start`/`fade`/`gap`, and
  `gap` is LEARNT from the player's own previous handovers (EMA 0.5, 0.15 s cut / 0.02 s crossfade by
  default). A `404` means the server restarted (its registry is in memory): it takes the same id back
  with `{resume}` so the link in everybody's hands keeps working.
- **The rule, an app restart does not strand the guests.** `party.host` in `kv` names the party this
  device hosts, and `state.rs`'s `on_restored`/`on_logged_in` call one `party_host::resume_at_launch`
  next to `remote_host::start`; a device that never hosted asks NOTHING (rule 3). Measured on a real
  restart with a party open: the same id came back, `v` went 17 → 19 (the full republish plus a
  heartbeat) and the guest's link kept working.
- **What it is now**: `wiring/party_guest.rs` (`ServerClock` — the NTP probes both sides use,
  `Timeline`/`position_at`/`correction_for`/`to_stream`, the `Planner` and its `Plan`, `decode_chunk`
  — Ogg Opus via symphonia + libopus with rubato for a 44.1 kHz device, the poll loop, the chunk
  cache, the scheduler thread), `wiring/party_host.rs` (`HostStatus`/`Listener`, `track_info`,
  `next_plan`, `Gaps`, `Line`, `decide`/`Tick`, the publish loop, `resume_at_launch`),
  `screens/party.rs` (the two panes: « Animer » — link, copy, audience with each guest's own estimate,
  clock quality, « Terminer la party » armed once; « Rejoindre » — link or id, peek, name, then the
  live pane: sleeve, title, the room's playhead, the host's state, the next track, volume, « Décalage »
  and « Quitter »), the « Écoute partagée » row in `sidebar()` (`app.rs`) and its glyph (already in the
  bundle). `nz-audio` gained `party.rs`, `Clock::advance_stream`/`note_output`/`output_anchor`/
  `output_generation`, `Engine::party()`/`output_clock()` and `PartyChannel`; the mixer mixes the party
  before its DSP chain and takes the output over while a party plays.
- **Measured** (this machine, Rust 1.99.1; `cargo test -p neezer-desktop`): **21 tests of this area**
  (11 `party_guest`, 10 of them the planner's ten-minute/cut/step benches and the two real-server ones,
  9 `party_host`, 1 screen), `cargo test --workspace` **778 tests, 0 failed**, `cargo fmt --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` clean. **Host ↔ guest, end to end through
  two real engines** (`nz-audio` offline sinks, a real ffmpeg-cut Opus chunk per six seconds, a click
  track): **26 clicks compared, guest minus host p50 0.0 ms, worst 0.0 ms** — the placement adds no
  error of its own; what the clock estimate adds is the ten-minute figure above. **A real server**
  (`webapp/test/party/server.py`, `NZ_PYTHON`): one chunk is **175 860 B of Ogg Opus → 6.0500 s** at
  48 kHz, its click at **0.500 s** with **0.005** of level before it.
- **Looked at, on an isolated `Xvfb :119`** (`-dpi 96`, `GPUI_X11_SCALE_FACTOR=1`, a real
  `perf_api.py --serve --archived` on :5000, two app instances under their own `dbus-run-session` — a
  second one otherwise reads itself as a second launch and quits): the host side at **2560×1600**, at
  **1280×800**, and with the **420 px** queue panel open at both; the guest's live pane at 1280×800
  and with the panel open. Five captures are the lot's artefacts. The pass fixed what no test sees:
  the intro sentences ran under the card's edge (a `flex_1` text without `min_w_0`), a card's actions
  were centred instead of full-width, and « Décalage » sat off its own buttons.
- **Pitfalls found by building**:
  - **A generation must be a generation.** `Snapshot::generation` first carried
    `output_clock().at_s.to_bits()`, which moves on every callback: the fit restarted every tick,
    never reached its three readings, and the host published NOTHING while looking healthy. The counter
    the callback bumps on a real step (`|Δ(at − pos)| > 4 ms`) is what `anchor.js` waits for.
  - **The newest placement is where the room is.** `Planner::sounding` took the FIRST covering
    placement; consecutive chunks overlap by the server's 50 ms, so it reported the *previous* chunk's
    line and a phantom **0.67 ms** drift after 19 s. It takes the last one now.
  - **A schedule table that truncates re-places the same chunk for ever.** Pushing a seventh segment
    into a six-slot table dropped the one just pushed; the next tick saw it missing and placed it
    again — the same chunk every 100 ms, and silence where it should have been. The planner prunes what
    played out and never exceeds the mixer's own table.
  - **An offline sink renders as fast as the test asks**: a test that fed `Instant` to the fit while
    the engine advanced 10 ms per render broke the line on every reading (D5's pitfall, again). The
    benches drive a synthetic clock in the engines' own domain.
  - **The clock route is `/party/<id>`, and the probe appends `/clock`**: passing `/party/<id>/clock`
    asked for `/clock/clock`, got a 404 and left « Synchronisation de l'horloge… » on screen for ever
    while the party worked.
  - **`desktop/app` is TWO levels below the repository root**, not three (`desktop/crates/nz-api` is
    three). The real-server test silently resolved its script outside the worktree and then waited out
    its whole 180 s deadline before skipping; it now fails fast when the child dies and when the script
    is missing.
  - **`gen` is a reserved keyword in edition 2024** — the seqlock's local had to be renamed.
  - **No QR code, deliberately**: the SPA's sheet shows one for the phones in the room; this screen
    shows the link with « Copier » instead, because a desktop has no camera to scan its own screen
    with. A guest pastes the link or the id (D15a's controller pastes its token the same way).
  - **Two names for one screen**: `nav.rs`'s `Route::Party.title()` is « Soirée d'écoute » (D7) and the
    sidebar's row is « Écoute partagée »; both are French, and D17's copy pass should settle on one
    (`nav.rs` is not this lot's file).
  - **The login POST failed under `Xvfb` while every other request worked** (`ApiError::Offline`), so
    the visual pass seeds `$XDG_STATE_HOME/neezer/session.json` with a cookie taken by `curl`; it is an
    environment artifact of the isolated display, not a party code path.

### D16 — Studio de genres et analyse côté client (livré 2026-11-17)

- **La règle, un studio, pas un second client** : les cinq onglets de `routes/Genres.svelte` —
  Étiquetage, Bibliothèque, Arbre, Modèle — plus l'Explorer (`components/ExplorerStyles.svelte`,
  que la page place sur Recherche : D16 possède `genre_explorer.rs` et pas `search.rs`, le studio
  le porte donc comme cinquième onglet ; un déplacement, pas une différence). `Étiquetage` est le
  flux « confirmer, pas taper » : un candidat, les neuf genres les plus probables sur `1`…`9`,
  `Entrée` pour le premier, `z` pour annuler, `j`/`k` pour avancer/reculer, `/` pour chercher
  (le champ se rejoint avec Tab). L'`Arbre` est le DAG du serveur : multi-parents, création,
  renommage, suppression, coupe d'un lien, annulation, remise à l'arbre d'origine — chaque écriture
  répond l'arbre ENTIER (`_tree_answer`) et l'écran peint cette réponse, jamais sa propre
  supposition. Le `Modèle` entraîne ici, montre le verdict de santé (`health.js` : bon / à
  surveiller / peu fiable), envoie la tête (`PUT /genre/model`) puis suit le reclassement.
- **La règle, les nombres et la tête sont ceux de la page, liés NATIVEMENT** : `webapp/measure` et
  `webapp/trainer` sont des dépendances de chemin de `app/` (`measure = { path = "../../webapp/measure" }`).
  La mesure locale fait `measure::measure` sur les canaux SOURCE plafonnés à 600 s (le serveur
  mesure le signal source), `lowband::analyse`, puis `tempo::analyse` sur le mono 44,1 kHz
  rééchantillonné par rubato (retard du filtre retiré) ; elle n'envoie que des nombres
  (`POST /analysis/<uuid>/measures`), le serveur décide. L'entraînement fait les plis, les bags,
  la projection et la température du crate, avec le ménage AVANT (`health.js#prepare` : doublons
  par `vectorKey`, un enregistrement étiqueté deux fois écarté, un artiste plafonné à 40 % d'un
  genre, un genre sous 5 titres laissé de côté) et une seule largeur de matrice (v3 3840 quand
  TOUTES les lignes l'ont, sinon le préfixe v2 2560). Deux calculs dans le studio ne peuvent pas
  diverger de la page : c'est le même code.
- **La règle, le fond cède (règle 1)** : un thread `nz-analysis`, `setpriority(0, 0, 10)` (les
  appels `setpriority`/`getpriority` sont déclarés localement : le workspace ne pin pas `libc`),
  toutes ses requêtes sur `Lane::Bulk`, et un drapeau `Control::yield_now` que l'écran lève tant
  qu'un titre démarre (`current_time() < 8 s`). L'entraînement a son thread, le même nice, le même
  drapeau. Rien ne touche l'état GPUI : tout revient par `wiring::events`.
- **La règle, l'écran est ATTEIGNABLE** : D7 avait enregistré la route sans lui donner
  d'entrée. D16 ajoute donc, hors de sa liste de fichiers mais avec l'accord de l'orchestrateur,
  un « Genres » dans `sidebar()` (`app.rs`, administrateurs seulement, comme la webapp) et le
  raccourci `g e` (`keymap.rs`, avec sa ligne dans la table des raccourcis qui alimente `?` et
  Réglages → Raccourcis). Rien d'autre dans ces deux fichiers ; D15a les modifie aussi (entrée
  Contrôle distant), d'où un rebase qui garde les deux côtés.
- **La règle, le clavier ne prend pas l'ancre** : l'écran ne focalise rien (D8a). `1`…`9` et `z`
  sont liés GLOBALEMENT (`KeyBinding::new("1", GenrePick(1), None)`) et livrés au studio VIVANT par
  un handle faible, exactement comme les touches de liste de `track_list::own` ; `Entrée`,
  `j`/`k`, `Début`/`Fin` passent par `track_list::own`. Mesuré : avec `Some("Neezer && !Input")`
  les touches n'arrivaient JAMAIS à l'écran dans le banc (aucune ancre focale hors coque) ; avec
  `None`, oui. Les deux chemins appellent les mêmes méthodes (`label`, `undo`, `skip`).
- **Ce que c'est maintenant** : `screens/genres.rs` (les cinq onglets, l'état, les réponses, les
  requêtes) et sept widgets — `genre_label.rs` (le modèle d'étiquette, le port de
  `lib/genre/rank.js` épinglé à la sortie Node de la page, les chips), `genre_tag_sheet.rs`
  (l'étiquette d'UN titre, ouverte du candidat/du lot/de l'arbre), `genre_bulk_sheet.rs` (l'aperçu
  du lot : un désaccord arrive décoché, seuls les titres cochés partent), `genre_tree.rs` (la liste
  aplatie du DAG, un nœud dessiné UNE fois sous son premier parent, les autres nommés sur la
  ligne), `genre_extractor.rs` (l'état de l'extracteur, son autotest, l'installation d'un `.onnx`
  par CHEMIN — pas de sélecteur natif sans dépendance), `genre_explorer.rs` (les styles parents,
  les cinq crans, la découverte par POST puis sondage GET seulement) et `wiring/analysis.rs`
  (le runner local et le module `training`).
- **Mesuré** (cette machine : 16 threads, Rust 1.99.1, PipeWire 1.6.9 ; `cargo test -p neezer-desktop`) :
  - **5 tests du studio** (4 journeys + le modèle de données) et **4 tests d'analyse** ; le
    workspace entier reste vert. `cargo fmt --check` et
    `cargo clippy --workspace --all-targets -- -D warnings` propres.
  - **requêtes** : ouvrir le studio = **2** (`/genre/status`, puis `/genre/candidates`) ; un
    onglet = **1** (`/genre/tree`) ou **4** (`/analysis/plan`, `/analysis/backfill`,
    `/genre/embed`, `/explorer`) ou **1** (`/explorer/styles`) ; une revisite = **0** (asserté).
    Un statut peint depuis le cache n'empêche plus la page de candidats (`candidates_asked`) —
    le piège exact de la règle cache-d'abord de D8a.
  - **mémoire du studio** : 220 étiquettes + 40 candidats × 24 scores = **677,3 Ko** (modèle de
    données, budget 1 Mo asserté) ; l'écran ne tient JAMAIS la bibliothèque — la liste de
    8 000 titres est celle de la coque, fenêtrée par D8a. Sous Xvfb + lavapipe, l'app entière au
    studio mesure **~247 Mo RSS** (le pilote Vulkan logiciel en fait l'essentiel ; D8a : 226–237 Mo).
  - **analyse de 100 titres pendant la lecture** (banc `NZ_ANALYSIS_BENCH=1`, vrai PipeWire, vrai
    ffmpeg, 100 FLAC de 20 s) : **128,9 s, 1,29 s/titre, 0 échec, 0 underrun** (`EngineEvent::
    Underrun`, compté par le banc), thread à **nice 10**.
  - **visuel** : 2560×1600, 1280×800 et **660×800** (panneau latéral de 420 px ouvert) regardés sur
    un Xvfb isolé : la page est centrée, bornée à 100 rem, les rails de la coque restent en place,
    les cinq onglets peignent, la taille des cibles et les textes français tiennent ; aucun emoji.
- **Pièges trouvés en construisant** :
  - **Le contrat `GenreScore` de `nz-model` était faux** : le serveur envoie `scores` comme
    `[[label, score, head, proto]]` (`genre.py#_scores_json`), pas un objet `{name, score, tag}`.
    Le lot de contrat l'a corrigé (`GenreScore` est un tuple sur le fil, `GenreTrack.tag` un OBJET
    `_tag_json`) et l'écran a perdu ses deux analyseurs : `Candidate.scores` est
    `Vec<nz_model::GenreScore>`, la feuille d'étiquette reçoit `nz_model::GenreTrack` typé et
    `genre_tag_sheet::scores_of` ne fait plus que projeter vers la rangée de la page. La fixture
    `genre-track` porte maintenant un vrai classement (le banc n'avait aucune étiquette, d'où le
    `[]` qui avait laissé passer le type faux).
  - **`nz-api` n'a ni `/genre/share/export`, ni `/explorer/styles`, ni `/explorer/discover`** : ils
    se lisent avec `Api::get`/`Api::send` (publics, avec la lane) — aucune route inventée. Le seul
    multipart, `POST /genre/extractor`, vit dans `genre_extractor::upload` avec la feature
    `multipart` posée sur `app/` : D2 disait qu'elle arriverait « avec D14b, D16 » ; elle est
    arrivée ici, côté app, et `nz-api` reste JSON-only. `POST /upload` reste à D14b.
  - **`bind_keys` d'un écran ne suffit pas** : sans ancre focale (un banc d'écran seul), une
    liaison contextuelle n'est jamais trouvée (D0) ; les touches du studio sont globales et le
    handler est le handle faible de l'écran vivant.
  - **`App::update_global` demande `BorrowAppContext`** dans le champ ; sans l'import, la méthode
    n'existe pas (`no method named update_global`).
  - **Le rééchantillonneur du tempo perd le retard du filtre** (147 trames à 1024, 3,3 ms) : le
    tracker lit des onsets, pas une durée ; le banc l'asserte à 5 ms. L'engine, qui doit garder la
    durée, réalimente ce retard.
  - **glibc `getpriority` rend le nice directement**, et remonter le nice exige `CAP_SYS_NICE` :
    le test de priorité nice son propre thread et le laisse mourir (un worker de test laissé à
    nice 10 ralentirait toute la suite).
  - **`the_whole_settings_page_costs_a_few_megabytes`** échoue parfois sous charge (D14a le sait) ;
    `a_right_click_opens_the_row_menu` (D8a) est du même genre : ces deux-là passent seuls, le
    studio n'y touche pas.

### T1 — `nz-transcribe` core (segments, sidecar) — delivered 2026-11-16

- **The rule**: the crate re-implements `engine.py#normalize_segments`/`clean_text` and
  `sidecar.py#build/dumps/srt_bytes` **byte for byte**, and its proof is a golden corpus written by
  *the Python functions* — `desktop/tools/transcribe_golden.py` — never by the Rust code, because a
  fixture the port generated would only prove the port agrees with itself. `tests/golden.rs` walks
  `tests/golden/<case>/` and compares `segments.json`, `transcript.json` and `transcript.srt`.
- **Why**: an episode transcribed on a laptop and the same episode transcribed on the server land in
  the same archive and the same segment table, and the SPA's transcript search folds over whichever
  arrived first. Anything less than the same bytes is a file that changes when rewritten, a line
  duplicated at a chunk seam, or a line moved by one millisecond. Three Python details a "reasonable"
  port gets wrong, each with its case: `round()` rounds **half to even** (`1500.5 → 1500`,
  `1501.5 → 1502`, so the port needs `round_ties_even` — measured: changing that one call to `round`
  fails case `10-half-millisecond-rounding`, 1500 → 1501); the engine's control class
  `[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]` keeps `\t\n\r` for the whitespace pass to collapse while the
  sidecar's `ord < 0x20` folds them too; and a **document keeps a tag-only line** (`[Musique]`) that
  the engine dropped, because on disk it is text the person can search.
- **Measured**: **16 cases, 62 files, 209 907 bytes** of fixtures (the 2 001-segment case is 131 922
  bytes of them and covers the cap; the rest are kilobytes). `cargo test -p nz-transcribe`: **28 lib
  tests in 0.30 s + 2 golden tests in 0.01 s**, the corpus covering the overlap cutoff, clamping at
  both edges, chunk 0 keeping its head, tag-only, control characters, Unicode (CJK, Arabic, an emoji
  with its variation selector, a decomposed accent), the 2 000-character cut counted in characters,
  2 001 segments → 2 000, an empty chunk (0 bytes of SRT, not one newline), fractional offsets, and
  a document built from raw server lines. An hour of dense French speech is **100 695 B of JSON and
  93 492 B of SRT** (asserted against the server test's own 100–250 kB band). The golden comparison
  itself is 62 file reads: nothing to time.
- **What the stubs pin** (T2/T3 own the bodies): `Cargo.toml` declares every dependency T2 and T3
  need (`rubato`, `reqwest`, `tokio`, `directories` join the whisper/audio set), so no later lot
  edits the manifest and the lock grows only in `nz-transcribe`'s own dependency list. The empty
  modules carry only facts that cross a process or a wire boundary, and that two lots would
  otherwise renegotiate: `FORMAT_ENGINE`, the five fallback causes with their French sentence, the
  model ladder and its two thresholds, `HEARTBEAT_SECONDS = 5`, `RESUME_AFTER_MS = 3 000`,
  64 bytes of `--pcm` per millisecond, the nine control-channel words, and the seven worker routes,
  built only from an id `is_episode_id` recognises so no id can escape its path segment.
- **Regenerating**: `python3 desktop/tools/transcribe_golden.py` writes the 62 files;
  `--check` writes nothing and exits 1 when one moved (measured by adding one space to a fixture).
  The desktop job does not run it yet: that is one line in the `test` job of
  `.github/workflows/desktop.yaml`, next to D1's `dump_fixtures.py` step, and outside this lot's
  files. Until it lands, the job catches a **Rust** drift (`cargo test --workspace` runs the
  committed corpus) and a **Python** drift only through `--check`, by hand.
- **Pitfalls**: Python's `format` check is `payload.get("format") != 1` and `True == 1` in Python,
  so the port accepts a JSON `true` there instead of inventing a stricter rule; `clippy -D warnings`
  rejects `assert!(CONST > CONST)` (`assertions_on_constants`) — pin such orderings with
  `const { assert!(…) }`; and the two cleaners differ on purpose (see *Why*), which is why the
  golden cases carry `sidecar_segments` apart from the engine's output.

### T2 — Engine (whisper Vulkan, models, VAD, audio, fallback) — delivered 2026-11-16

- **The rule, enumeration**: ggml's Vulkan enumeration ABORTS the process when no driver is
  usable, so it happens in a CHILD of the same binary and a crash becomes a named cause. Measured:
  `VK_ICD_FILENAMES=/nonexistent neezer-transcribe probe` → `{"backend":"cpu","reason":
  "no_vulkan_device"}`, exit 0, in **91 ms** (one child spawn); the same enumeration in-process is
  SIGABRT 134. `--version` survives the same dead ICD (exit 0, pinned by a test), and `file` still
  transcribes on the CPU with the cause written into its document. The child is found by name
  (`neezer-transcribe`) beside the current executable, because under `cargo test` the current
  executable is the libtest harness, which refuses `--vulkan-devices`.
- **The rule, fallback**: the ladder is observed, never guessed — device list → whisper context with
  `use_gpu(true)` → a one-second silent decode (the first GPU call compiles whisper.cpp's shaders),
  once per process; a failure at any rung falls back to the CPU with `gpu_init_failed`, or
  `gpu_out_of_memory` when the model is bigger than the device's free VRAM. **Vulkan 72.2× real time
  against CPU 13.1× on the same 330 s** (`ggml-base`, warm, best of two passes, measured here:
  Vulkan 4.57 s, CPU 25.18 s and 13.3× on the CPU-seconds it spent). The same GPU run measured
  **45.0×** while the machine was at load 25: a CPU starved of cores cannot feed the 780M, which is
  why the CPU half is scored on its CPU-seconds and the GPU number is quoted with its load.
- **The rule, VAD**: **`FullParams::enable_vad` is a NO-OP in whisper-rs 0.16.** `WhisperState::full`
  calls `whisper_full_with_state`, and whisper.cpp 1.8.3 only runs VAD in `whisper_full` /
  `whisper_full_parallel` (read in `whisper-rs-sys-0.15.0/whisper.cpp/src/whisper.cpp`: lines 7750
  and 7778 are in those two, none is in `whisper_full_with_state`). The crate runs Silero itself
  (`WhisperVadContext`) and puts the timestamps back with whisper.cpp's own interpolation table
  (integer for integer), and **keeps it OFF by default**: that path is CPU-only and unbatched —
  measured **23.6 s of CPU for 11 s of speech** (2.1× real time, 4 threads) against **0.24 s** of
  Vulkan decoding, i.e. 98× the cost of the audio it removes, for the same text on `jfk.wav`. A
  one-hour episode would spend two hours in VAD before whisper saw a sample (rule 1). `--vad <path>`
  turns it on, and a VAD model that was named but cannot load fails the run instead of being
  ignored.
- **The rule, audio**: decoded in-process, never by `ffmpeg`: symphonia for Ogg Opus, FLAC, MP3 and
  MP4/AAC (the workspace's feature set has no WAV), `hound` for WAV, mono `(l + r) / 2` like
  `ffmpeg -ac 1`, rubato's FFT resampler to 16 kHz with its delay dropped and the last partial block
  trimmed to the ratio. Measured on a 5.000 s tone: MP3 **5025 ms** (its encoder delay, 1152 samples
  at 44,1 kHz), M4A **5000**, Ogg Opus **5000**, FLAC **5000** — the criterion is ±50 ms. A window
  (`decode_range`) keeps an hour-long episode's memory to the window, which is how T3 gives the
  engine one 600 s chunk.
- **The rule, a window is `ms × 16` samples, never `ms`:** `decode_range` passed milliseconds where
  the collector counts 16 kHz samples, so a 600 s chunk decoded **13.6 s** of audio and every
  transcript after the first was shifted in silence, with no error. T3 found it on a real file:
  `(5000, 1000)` of a 3 s file returned **16 000 samples instead of 0**. The tests now pin both
  halves — a window past the end is 0 samples, and a middle window is `samples[2000×16 .. 2500×16]`
  of the whole decode and says 880 Hz where the file says 440 Hz before 2 s — on WAV, MP3, M4A, Ogg
  Opus and FLAC.
- **The contract T3 consumes**: `neezer-transcribe engine --pcm <raw f32le 16k mono> --model …
  --lang … --out …` writes `engine::Document`, i.e. whisper's own `-oj` keys
  (`result.language`, `transcription[].offsets/text`) plus `backend`, `device`, `reason`, `model`;
  the offsets are the chunk's own, so the supervisor's `normalize_segments` gives the same absolute
  lines the server would store. `file <audio> --out …` is the whole episode: decoded, chunked like
  the server (600 s, 2 s overlap), normalised per chunk, absolute offsets. Both write `.tmp` then
  rename. `probe` prints the `{"event":"engine",…}` line T4 shows. The supervisor's own seam is
  `audio::write_pcm_range(audio, start_ms, length_ms, out) -> Result<u64, Error>`: it decodes that
  window straight into a `--pcm` file and returns the **duration written in milliseconds**, so a
  short file (killed child, truncated episode) is visible instead of silent.
- **Models are pinned**: the server's ladder (`TURBO_FREE_BYTES`, `ORDER_FREE`/`ORDER_TIGHT`,
  `MIN_MODEL_BYTES`, the GGML magics), `valid_model`'s three refusals, the choice by `MemAvailable`,
  and the **six** files' sizes + SHA-256 the settings screen may download (VAD included), with a
  SHA-256 written here (the workspace has no hashing dependency) and held to the FIPS 180-4 vectors
  including the million-`a` case.
- **Measured** (this machine: 16 threads, Rust 1.99.1, RADV 780M; `cargo test -p nz-transcribe`):
  **43 lib tests in 0.4 s**, the **7 acceptance tests in `tests/engine.rs` in 5.5 s** with
  `ggml-base` (they skip with a printed reason without `NZ_WHISPER_MODEL`/`NZ_WHISPER_AUDIO`, and
  fail if the variable names a missing file, so a broken CI cache cannot pass), and the 2 golden
  tests. `jfk.wav` gives the **same `transcription` array** on Vulkan and on the CPU, and in CI on
  `llvmpipe (LLVM 20.1.2, 256 bits)` too.
- **CI**: the `test` job fetches `ggml-tiny.bin` and `jfk.wav` (at the `v1.8.3` tag this build
  vendors) into an `actions/cache` keyed by their SHA-256, verifies both with `sha256sum -c`, and
  runs the suite with `NZ_REQUIRE_VULKAN=1` so the Vulkan half is proven on `lavapipe` — the
  software device `mesa-vulkan-drivers` (already in `APT_DEPS`) provides, not skipped. **`lavapipe`
  needs `GGML_VK_VISIBLE_DEVICES=0`**: ggml's Vulkan backend keeps only `DISCRETE_GPU` and
  `INTEGRATED_GPU` devices, and lavapipe reports `CPU`, so with the driver installed and working the
  probe still answered `no_vulkan_device` on the runner until the variable named device 0
  (measured in CI, run 43897207709).
- **Pitfalls found by building**: the shader-cache trap below hit again — an aborted first build
  (no `VULKAN_SDK`) left `whisper-rs-sys`'s `OUT_DIR` in a state where the next build *compiles*
  and then fails at link on `get_rows_iq2_xxs_data`; `cargo clean -p whisper-rs-sys` with
  `VULKAN_SDK` set rebuilds it (5 min 41 s here). `--version` calls `print_system_info()`, which is
  not the compile-time string T1's comment claimed (it prints a `ggml_vulkan:` device row when a
  driver exists) — it survived a dead ICD here, and the test now pins it. whisper's segment
  timestamps are in **centiseconds**, and with VAD the mapping is integer arithmetic on that unit:
  mixing units moves every line by 90 %.


### T3 — Worker protocol and supervisor (delivered 2026-11-16)

- **The rule**: the heartbeat is its own task, so a pause is never a lost job. The engine is a child
  in its own process group and a pause is `SIGSTOP` of that group; the lease keeps renewing while
  the engine is stopped. Measured: **3 heartbeats during a 15 s pause**, the engine's
  `/proc/<pid>/stat` state `T`, and a `cancel` from the server ended the job in **< 0.4 s** with a
  300 ms beat (5 s in production). A pause therefore yields the CPU *and* keeps the job.
- **The rule, the protocol is a port, not an invention**: `protocol.rs` follows
  `docker/transcribe/worker.py` value for value — the backoff schedule (1, 2, 4, 8, 16, 30 s, capped,
  a `Retry-After` raises it up to 120 s), `429`/`5xx` retried and nothing else, `fail` truncated at
  24/240 **characters**, 800 kB of segment text per chunk, a 1 GiB and 20-request audio download that
  resumes with `Range`. Two additions a claim needs before a byte of it is used: the id must be a
  UUID, the lease a single token, and `audio_url` must be **this** episode's own route, never another
  host (the client is handed a URL by the server; following it would be an SSRF).
- **The rule, two pause bits**: the app's pause (rule 1: buffer low-water, a `User` request, a slow
  frame) and the server's are separate flags. Measured the hard way: one bit, written by the
  heartbeat's `pause: false`, resumed a `SIGSTOP`ped engine **within one beat** — a `client` worker
  never receives `pause` (`job.heartbeat` only answers it for `server` workers), so the answer must
  not clear the app's decision.
- **The rule, no decoder and no progress here**: a chunk's PCM comes from `PcmSource`, whose
  production implementation is T2's `audio::write_pcm_range` (a window, not the whole
  episode: 38 MB of PCM for a ten-minute chunk). The supervisor never decodes audio and never
  invents a percentage: `chunk_pct` is whisper.cpp's own `progress = N %` or 0.
- **What it is now**: `worker.rs` (token on disk, `0600`-checked and never printable — `Debug` says
  `<jeton>`; the typed client of the seven routes), `protocol.rs` (payloads, backoff, `fit_segments`,
  `is_episode_id`), `control.rs` (the nine words as serde-tagged `Command`/`Event`, one flushed JSON
  line per event), `supervisor.rs` (`Halt`, `PcmSource`/`AudioPcm`, `EngineChild` with its signals,
  the claim loop, the local job, the `<episode>.partial.json` resume file), and the
  `worker`/`local` subcommands. A local transcript lands in
  `$XDG_DATA_HOME/neezer/transcripts/<episode_id>.transcript.json` + `.srt` (T1's `sidecar::write`).
- **Measured** (this machine, Rust 1.99.1; `cargo test -p nz-transcribe`): **13 tests against a fake
  server in 19 s** (the 15 s pause is most of it) and the crate's **67 lib tests in 0.5 s**. The full
  claim → audio → chunks → complete is **6 requests** (claim, audio, 3 chunks, complete) for a
  3-chunk episode, 12 lines stored; a `429 Retry-After` + a `500` are retried to exactly the
  schedule's attempt count (**3 claims, not a storm**). Against the REAL server
  (`NZ_PERF_API=1`, `perf_api.py --podcasts`, `bench` enrolling a `client` worker):
  **6 chunks, 12 lines, the row `ready` with `source: client`, `worker: banc-t3`, in 0.9 s**.
- **The rule, arm a verdict on the event, never on the clock**: a fixture that cancels after
  one 300 ms beat, or sends a `stop` after a 500 ms sleep, can land before the engine exists on a
  loaded runner. The job is then released without ever spawning, and the test reads a `pid` nobody
  wrote — the `a_cancel_stops_within_one_heartbeat` failure of PR #237 ("the engine wrote its pid:
  No such file or directory"), reproduced at **iteration 1** under 8 spinners on one core. Both
  tests now wait for the fake engine's `pid` (it writes it before it blocks), arm the cancel or send
  the stop on that event, and assert the cancel→idle time: **62-420 ms measured** over 65 passes
  (25 of them pinned to one core), **50/50 passes each, 10 of them pinned to one core** under that
  load. The same event
  replaced the pause test's 600 ms sleep. The `pid` itself appears in **3 ms** idle and **0.6 s**
  under that load (the child runs `nice 19`, so it is scheduled last), which is what justifies the
  fixtures' 10 s fail-fast guard — 16x the worst measurement, and it never becomes a pass.
- **Pitfalls found by building**: a pause needs `process_group(0)` on the child **and**
  `kill(-pgid, SIGSTOP)` — the `libc` crate is not available to this lot, so the four signal numbers
  and `kill`/`nice`/`localtime_r` are declared locally (the app is Linux-only); a stopped process
  ignores `SIGTERM` until it is continued, so `terminate` is CONT → TERM → KILL; `nice 19` goes
  through `pre_exec`; the idle loop polls the control channel every 25 ms instead of `select!`ing,
  because a `local` command has to run a whole job between two claims. **A `SIGSTOP` is not
  synchronous with `kill`**: a test that reads `/proc/<pid>/stat` right after it sees `R` and calls
  a working pause broken — read it in a loop for a second (measured: the state flips within 10 ms).
  And the seam's first caller is what finds the seam's bugs: `audio::decode_range` took `start_ms`
  as a **sample** count, so a chunk at 600 s decoded from ~13.6 s and every line after the first
  chunk would have been silently misplaced. The window test that catches it is a file that is loud
  for half a second then silent (a window at 600 ms must be silence).

### T4 — Transcription in the app (delivered 2026-11-17)

- **The rule, the app does not link the engine.** `nz-transcribe` gained two features, `engine`
  (whisper.cpp, Vulkan, the in-process decoder) and `worker` (the protocol client, the supervisor),
  both `default` and both required by the `neezer-transcribe` BINARY (`required-features`).
  `neezer-desktop` depends on the crate with `default-features = false`: `cargo tree -p
  neezer-desktop` contains **0** whisper/ggml package, the release binary stays the shell's size
  (**58 618 624 B** at this commit against **58 127 616 B** at `master`, **+491 008 B / +0.84 %**),
  and `strings` finds no `ggml_backend`/`whisper_full` symbol in it. T4 also ships the vocabulary apart (`backend.rs`:
  `Backend`, `FallbackCause`, `FORMAT_ENGINE`) so the screen can say "Vulkan" and "no_vulkan_device"
  without the engine.
- **The rule, one child and threads that borrow nothing.** The supervisor is `neezer-transcribe`
  in its own process group at `nice 19`; the app owns it through one service (a snapshot behind a
  mutex, a stdin writer, a stdout reader, a 250 ms governor). Neither the reader nor the governor
  touches `NetRuntime` (one blocks on a pipe, one on its own clock), so `shutdown` sends `stop`,
  closes the pipe, kills the group (CONT→TERM→KILL) and JOINS both; the fake-supervisor test
  reads the child's pid and asserts `/proc/<pid>` is gone. `NetRuntime::drop` calls it, one line.
- **The rule, the governor is measured, not guessed.** A job pauses on: a discharging battery when
  « Seulement sur secteur » is on; an underrun, or a playhead that advanced under half the wall
  time in 250 ms (a starving decoder); a paint interval over **25 ms** on the « Très basse »
  priority (a visible hitch on the same GPU). It resumes after **3 s** with no reason — the
  supervisor's own `RESUME_AFTER_MS` — because one slow frame is one pause, never a start/stop per
  chunk (the first GPU call compiles whisper.cpp's pipelines).
- **The rule, models are a click and a pin.** Nothing downloads by itself: `Api::download_url_to`
  on `Lane::Bulk` with progress, then the pinned **size and SHA-256** (`nz-transcribe`'s table),
  then `valid_model`; a file that fails is deleted and its cause printed. The person's chosen model
  is authoritative (`model_in`), the ladder only picks when nothing is chosen. The VAD file is pinned
  but NOT offered: T2 measured it at 98× the audio it removes on a CPU (rule 1).
- **The rule, a pinned URL is requested as it is.** `Api::download_to` builds `{base}/api{path}`, so
  handing it the absolute model URL asked the neezer SERVER for `/apihttps://huggingface.co/…` and
  every download in Réglages → Transcription locale was a **404** (v1.6.0). The two entry points are
  now separate on purpose: `download_url_to` takes an absolute **`https`** URL (a `http`, `file`,
  `ftp`, relative or credentialed one is refused before a byte moves, and the client's `https_only`
  refuses a redirect that would downgrade), uses a client with **no
  cookie jar** (a neezer session never leaves its server, and a third-party `401` must not clear it),
  sends no `X-NS-Background` (a CDN does not read our headers) and follows the **`302`** Hugging Face
  answers with — the body only counts at the end of that chain — while holding the same lane slot and
  token bucket. Measured: the 5 models + the VAD answer `200` after the redirect and their
  `Content-Length` is the pinned byte count (tiny `77 691 713`, VAD `885 098`), and a real
  `ggml-tiny` + VAD download lands with `sha256sum` equal to the table. A `404`/`410` is said as what
  it is — the file moved UPSTREAM, nothing installed, retry later or pick another model — and an
  unreachable host says « votre serveur Neezer n’est pas en cause » (`download_failure_fr`). The
  regression test drives a loopback server through a `302` and asserts the request target is the URL
  itself: with the old call it is `/apihttp://…`, failing.
- **The rule, audio is fetched once.** An offline download is used as it is; anything else is
  streamed once to `$XDG_CACHE_HOME/neezer/transcribe/<episode>.opus` at Opus 128 (the engine turns
  it into 16 kHz mono anyway) and kept for a retry. The `.partial.json` T3 writes after each chunk
  is the resume point: the panel reads it and says « un travail interrompu s'est arrêté à X min ».
- **The rule, the panel's local branch.** The panel reads `$XDG_DATA_HOME/neezer/transcripts/`'s
  sidecar through `sidecar::load`, so the local text has the server's segment shape; `at`/`covers`/
  `loaded` follow the LOCAL lines (the playhead highlights them, a click seeks), the search folds
  accents in memory, and a job's phase, percent, pause cause and failure are on a card above the
  server's. A virtual list needs a definite height: stacked in the episode panel (an unbounded
  scroll column) it collapsed to 0 and the lines were there but invisible — the floor is eight rows.
- **Worker sink.** Enrol writes the device token 0600 and never shows it again; revoke deletes it
  locally even when the server cannot confirm; the switch runs the server queue on this PC. One
  child: the supervisor's worker mode also accepts `local` commands in its idle loop, so local and
  server jobs share one engine.
- **A T2 bug T4 found**: with `params.language = null` AND `detect_language = true`, whisper.cpp
  auto-detected "en" and then decoded only timestamp tokens — **0 segments** on `jfk.wav`, while
  `--lang en` gave the two lines. The server's default language is `auto`, so the worker would have
  written empty transcripts. `"auto"` as the string (the branch whisper.cpp itself detects on) is
  the working path; `language_auto_still_transcribes` now asserts the text.
- **Measured** (this machine: 16 threads, Rust 1.99.1, RADV 780M; `cargo test -p neezer-desktop`):
  - **Vulkan vs CPU**, 176 s of speech (`jfk.wav` × 16), `ggml-base`, best of two warm runs:
    **Vulkan 57.8× real time (3.05 s) against CPU 11.6× (15.23 s), a 5.0× factor**, identical text
    (the same lines as `--lang en`). The CPU fallback sentence quotes 5×, the largest measured
    here (T2 saw 3.2–3.4× on shorter inputs).
  - **0 underrun** while a real transcription ran: 10 s of playback rendered in real time through
    the app's own engine (offline sink) during a **CPU** transcription of 176 s — the slower path,
    the worst case for rule 1: transcription active on **1000/1000 samples** of the window,
    **0** underrun.
  - **the snapshot** a screen holds for the worst realistic state (six models, a download, a job,
    six failures, the worker): **1 646 B** by `testmem`'s counting allocator (budget 16 KB; 258 B of
    them are the failure sentences).
  - **tests**: `cargo test --workspace --locked` green; `cargo test -p neezer-desktop` **428
    passed, 0 failed, 11 ignored** (19 `wiring::transcribe`, 4 of them `--ignored`: the two réseau
    ones and the two measurement benches; 5 `screens::settings::transcribe`, the panel's local
    case); `cargo test -p nz-transcribe` **69 lib + 8 acceptance** (the two measurement benches —
    Vulkan/CPU and 0 underrun — are `--ignored`, their numbers above); `nz-api` **27**.
  - **RSS** (release, Xvfb + private D-Bus, `/proc/<pid>/status`): **196 760 kB** at Accueil
    against **196 616 kB** for `master`'s binary at the same screen (**+144 kB**); **199 616 kB**
    on this page and **287 016 kB** at 2560×1600 (the window's own surfaces, D11/D12's number). The
    feature holds a snapshot, not a model: the engine is another process.
  - **looked at** at **1280×800**, **2560×1600** and with the **420 px** queue panel open: the
    settings page with Vulkan detected, the models and their sizes, the priorities; the episode
    panel showing a local transcript following the playhead; a job's card. Captures under
    `docs/desktop/t4/`.
- **Pitfalls found by building**: the service's `attach` dropped the runtime assignment in an
  early-return edit, so every `spawn` went nowhere while the job card appeared — a click that
  "does nothing" was a missing field, not a lane; the governor thread checked a GLOBAL stop flag
  and a test-local service never joined (60 s hang), so the flag lives in `Shared`; an async
  `Done` can beat the job it clears, so a test publishes the job before the command; `ElementId`
  has no `From<(&str, &str)>` and `Toggle`/`Button` expose no `debug_selector` (D8a's pitfall
  again); `models::choose` replaces a person's explicit small model with the ladder's best, which
  `model_in` now refuses. **A dead child looked alive**: `ensure_child` answered "already running"
  from a handle whose process had exited, and the next `local` command was written into a closed
  pipe — the card sat at 0 % for minutes while an idle supervisor slept beside it. `ensure_child`
  now `try_wait`s, the reader's EOF clears the child and its stdin, and a write failure publishes a
  `control_channel` failure instead of vanishing. **`print_progress` was off**, so whisper's
  `progress = N %` never reached the supervisor: `chunk_pct` was always 0 (the server heartbeat too).
  It is on in `Engine::params`, and each new percent becomes a `Progress` event while the chunk
  runs.

### P1 — Arch packaging and release (delivered 2026-11-16)

- **The rule**: the app ships as ONE Arch package, `neezer-desktop`, built from the tree it lives in.
  `desktop/` links `webapp/rhythm`, `webapp/appcore` and `android/nscore` by path, so there is no
  source tarball to point `source=(...)` at: the PKGBUILD follows the Omarchy apps' convention
  (`source=()`, `build()` builds the checkout) and the release job checks the `v*` tag out before
  running `makepkg`, so the artefact always comes from the tagged tree. `prepare()` is the ArchWiki
  Rust guideline verbatim (`cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"`),
  `build()` is `cargo build --frozen --release` into `$srcdir/target` under
  `RUSTUP_TOOLCHAIN=stable`, and the tag's version reaches `pkgver` through
  `NEEZER_DESKTOP_VERSION` (`v1.3.1` → `1.3.1`): a tag can never ship a package named after the
  workspace's 0.1.0.
- **The rule, `!lto` is load-bearing**: makepkg's default `lto` option appends `-flto=auto` to
  `CFLAGS`/`CXXFLAGS`/`LDFLAGS` (`/usr/share/makepkg/buildenv/lto.sh`), which turns whisper.cpp's
  and aws-lc's static archives into GCC LTO bitcode; `rust-lld` cannot read it and drops every C
  object, so the link fails on `undefined symbol: aws_lc_0_47_0_AES_set_encrypt_key` /
  `whisper_full_n_segments_from_state` (measured twice, once per binary). `options=('!debug' '!lto')`
  — the ArchWiki's "mixed Rust/C/C++ project" case; the Rust half keeps its own thin LTO.
- **The rule, `depends` follow `ldd`, not the plan**: `neezer-desktop` links `libpipewire`,
  `libasound`, `libxcb`, `libxkbcommon` and `libxkbcommon-x11`, and `neezer-transcribe` links
  `libvulkan`; `dbus` is not needed at all (zbus speaks the D-Bus wire protocol), and `fontconfig`
  is a **build** dependency only — `yeslogic-fontconfig-sys` asks pkg-config for `fontconfig.pc`
  (freetype2 comes with it) but rustc's `--as-needed` drops `libfontconfig` from the linked binary,
  so `ldd` is clean and it belongs in `makedepends`. `namcap`'s `E:` lines named exactly the two
  MISSING runtime packages (`libxcb`, `libxkbcommon-x11`) and a third (`hicolor-icon-theme`, for the
  icon hierarchy); `wayland` is dlopen'ed by `wayland-sys` (absent from `ldd`, called "may not be
  needed" by namcap) and stays, with the reason in the PKGBUILD. `cargo` is not an Arch package name
  either — `rust` provides it — so only `rust` is in `makedepends`: naming both makes `makepkg -si`
  stop on "two providers available for cargo" on a machine that has neither yet.
- **The rule, one artefact**: `!debug` in `options` (the Rust profile already strips), so the release
  attaches exactly `neezer-desktop-<ver>-x86_64.pkg.tar.zst`.
- **What it is now**: `desktop/packaging/arch/PKGBUILD` (both binaries, the `.desktop`, the icon, the
  licence; `package()` installs `neezer-transcribe` beside `neezer-desktop`, which is how T2 finds
  its child), `neezer-desktop.desktop` (`StartupWMClass=neezer-desktop`, the `app_id` of `main.rs`;
  `Icon=neezer-desktop`, the web app's own mark, so the launcher entry is recognisable) and
  `neezer-desktop.svg`. `.github/workflows/release.yaml` gains an `arch` job: `archlinux:latest`,
  the declared `makedepends` + `depends` installed first (a typo in `depends` fails there),
  `makepkg` as an ordinary user (it refuses root), `desktop-file-validate`, `namcap` on the package
  AND the PKGBUILD with `E:` lines failing the job, the artefact uploaded and attached on `v*` only.
  Like `android.yaml`, the job also runs on pull requests touching `desktop/packaging/**`, so this
  YAML is proven before the first tag instead of by it.
- **Measured** (this machine: 16 threads, Rust 1.99.1, Omarchy/Arch, makepkg 7.1.0):
  - `makepkg -f` from an empty `$srcdir`: **8 min 56 s** wall, 33 min 20 s CPU (`CARGO_BUILD_JOBS=4`);
    the release build inside is 8 min 52 s.
  - package **28 020 486 B (26.72 MiB)** compressed, **87.16 MiB** installed —
    `neezer-desktop` 41 716 736 B, `neezer-transcribe` 49 642 888 B.
  - `namcap` on the package: **0 error**, 8 warnings (two `ld-linux` "unused library" from its
    known noise, four "implicitly satisfied" — glibc, libgcc, libstdc++, libpipewire — and the
    `pipewire`/`wayland` "may not be needed" the two comments above explain). `namcap` on the
    PKGBUILD: nothing. `desktop-file-validate`: clean, before and after installation.
  - installed binary, first painted frame on an isolated **2560×1600 X server**:
    **156, 122, 136 ms** (median 136; D7's criterion is 300 ms).
  - RSS at the login screen: **175 244 kB** at 2560×1600, **175 340 kB** at 1280×800 — an Xvfb X11
    run, and Mesa says `No DRI3 support detected`, so it is NOT comparable with D7's **103 MB** on
    Wayland/Hyprland; P1 adds no screen and the packaged binary is D7's code.
  - `pacman -U -dd` into a throwaway root installs exactly the five paths (two binaries, the
    `.desktop`, the icon, the licence), and `neezer-transcribe --version` from that root reports the
    RADV 780M device and `whisper.cpp 1.8.3 (backend vulkan compilé)`.
  - the `arch` job in CI (`archlinux:latest`, cold container and cold crate cache): **11 min 08 s**
    end to end, `namcap` gate green, the `files` job correctly skipped on a pull request.
- **Pitfalls found by building**:
  - makepkg runs `build()` with the distro's hardening `CFLAGS`/`CXXFLAGS`/`LDFLAGS`, and its `lto`
    option is what broke the link (see the rule above); the failure looks like a C library problem
    (`undefined symbol` reported by `rust-lld`), not a flag problem, and it hits BOTH binaries.
  - A dependency the binary does not link can still be needed to BUILD it: the CI's Arch container
    stopped at `yeslogic-fontconfig-sys` ("Package 'fontconfig' not found") while `ldd` on this
    machine was clean, because this machine happens to have fontconfig installed. `fontconfig` is a
    `makedepend`; a `depends` list checked only with `ldd` says nothing about the build.
  - `makepkg` refuses to run as root, so a container job needs a `builder` user, a `chown` of the
    mounted workspace and a `su -` login shell (so cargo finds its registry).
  - `actions/checkout`'s node runtime works in `archlinux:latest` without installing node, but the
    image has no `git` until `pacman -Syu` has run: install the build chain BEFORE checkout. Runtime
    dependencies belong in that same list — it is what proves every `depends` name exists.
  - `namcap` prints `<pkgname> E: …` and **exits 0 even with errors** (measured): a gate has to
    grep for `(^| )E:` or it passes every finding.
  - `makepkg`'s `-debug` subpackage would be a second artefact; `!debug` (plus the profile's
    `strip = true`) keeps one, and the release glob names `[0-9]` after `neezer-desktop` so it can
    never pick up a `-debug` package.
  - The package still contains whisper.cpp's `__FILE__` paths under `$srcdir`
    (`makepkg` warns `Le paquet contient une référence à $srcdir`): diagnostic strings only, from
    GGML's logging macros. It is a known cosmetic reproducibility gap, not a defect of the
    install.


### P2 — Five packages, one build (delivered 2026-11-17)

- **The rule**: every `v*` release ships the desktop app for **five** Linux
  destinations — Arch, Debian/Ubuntu, Fedora/RHEL, a bare AppImage, a Flatpak
  bundle — and every one of the four new ones is **installed and started** by the
  job that built it before it is attached. A package nobody ran is a file, not a
  package.
- **The rule, one build**: `release.yaml`'s `binaries` job builds
  `neezer-desktop` and `neezer-transcribe` once on `ubuntu-24.04` and uploads
  them; `deb`, `rpm`, `appimage` and `flatpak` download that artefact and package
  it with the distribution's OWN tool (`dpkg-deb`, `rpmbuild`, `appimagetool`,
  `flatpak-builder`). The `arch` job keeps building the checkout with `makepkg`
  (P1): an Arch package that did not come out of `makepkg` is not the package P1
  described. **Measured**: the four packaging jobs add 3–6 min each to a release
  run and the build itself is unchanged.
- **The rule, three refs, each for one thing.** The `binaries` job builds the
  TAGGED tree (its checkout keeps `ref: <tag>`); the four packaging jobs take
  their RECIPES from the commit the workflow itself runs at (`github.sha`, no
  `ref:`) and their VERSION from the tag (`TAG`); the attached files come from
  the artefact. A release replay (`workflow_dispatch -f tag=v1.6.0`) is exactly
  why: `v1.6.0` predates `desktop/packaging/{deb,rpm,appimage,flatpak}`, so a job
  that checked the tag out failed on `desktop/packaging/version.sh: No such file
  or directory` in all four (measured, run 44150836135) while `binaries` and
  `arch` — which must read the tag — were green. A tag that carries the recipes
  behaves the same way, because then `github.sha` IS the tag.
- **The rule, the sidecar travels with the app.** Every format installs
  `neezer-transcribe` in the SAME directory as `neezer-desktop` — `/usr/bin`,
  `/app/bin`, `usr/bin` inside the AppDir — because `wiring/transcribe.rs#program`
  resolves `current_exe().parent()` before `PATH`. A package that forgets it
  installs an app whose every episode says « Le binaire neezer-transcribe est
  introuvable ». `neezer_check_sidecar_installed` asserts it in each test.
- **The rule, `depends` are read from the binaries, once.** `lib.sh` holds the
  ten sonames the two binaries may NEED (`readelf -d`) and the Debian package
  that provides each; `neezer_check_needed` **fails the build of every format** on
  an unknown one, and the Debian `Depends:` is generated from that same table, so
  a new library can never be silently shipped. `rpmbuild` derives its own
  `Requires` from the same sonames, which is why the RPM lists no dependency by
  hand — `libpipewire-0.3.so.0()(64bit)`, `libvulkan.so.1()(64bit)`, and dnf
  resolves those to package names on Fedora and RHEL alike.
- **The rule, the glibc floor is measured, not written.** `neezer_glibc_floor`
  takes the highest `GLIBC_x.y` from `readelf --version-info` and it lands in
  `Depends: libc6 (>= …)`. The release builds on Ubuntu 24.04, whose floor is
  **2.39**: Debian 13 and Ubuntu 24.04 are the oldest systems the `.deb` installs
  on, and the README says so. A local build on Arch (2.42) produces a package
  that says 2.42 — the truth about its own binaries.

- **What it is now**: `desktop/packaging/{deb,rpm,appimage,flatpak}/` (a
  `build.sh` and a `test.sh` each, plus `control.in`, `neezer-desktop.spec`,
  `AppRun`, `fr.neezer.Desktop.yaml`), `desktop/packaging/lib.sh` (version,
  soname table, glibc floor, Xvfb harness), `desktop/packaging/version.sh` and
  `desktop/packaging/README.md`. All four reuse `arch/neezer-desktop.desktop` and
  `arch/neezer-desktop.svg`: one launcher entry, one icon, five packages.
  `app/src/main.rs#flatpak_app_id` reads `FLATPAK_ID` for the window's app id, so
  the sandbox's window matches the launcher entry Flatpak exports.
- **Flatpak permissions**, each with its reason in the manifest: `wayland` +
  `fallback-x11` (the window), `pulseaudio` (sound through pipewire-pulse and the
  ALSA devices), `filesystem=xdg-run/pipewire-0` (cpal's PipeWire host opens that
  socket directly), `device=dri` (the GPU, for GPUI's surface AND whisper),
  `share=network` (it is a client), `own-name org.mpris.MediaPlayer2.neezer`
  (playerctl and the bar's widget), `talk-name org.freedesktop.Notifications`
  (the opt-in track change). **No filesystem grant**: the app opens no file of
  the person's — measured, `--socket=pulseaudio` mounts `pulse`, not
  `pipewire-0`, and nothing else was asked for.
- **Measured** (this machine, 16 threads, glibc 2.42 on Arch; the two binaries
  built in a **Ubuntu 24.04 user-space root** — `unshare`/`bwrap`, no docker, no
  sudo — because Arch's glibc is newer than every current distribution's, so a
  package built from Arch binaries installs nowhere):

  | Format | File | Size | Installed and started in | First frame |
  |---|---|---|---|---|
  | `.deb` | neezer-desktop-1.6.0-x86_64.deb | **22 831 992 B** (21.8 MiB) | Debian 13 (trixie), `apt install ./…` | **82 ms** |
  | `.rpm` | neezer-desktop-1.6.0-x86_64.rpm | **20 238 549 B** (19.3 MiB) | Fedora 45, `dnf install ./…` | **136 ms** |
  | `.AppImage` | neezer-desktop-1.6.0-x86_64.AppImage | **32 750 072 B** (31.2 MiB) | this machine, `APPIMAGE_EXTRACT_AND_RUN=1` | **128 ms** |
  | `.flatpak` | neezer-desktop-1.6.0-x86_64.flatpak | **21 972 128 B** (21.0 MiB) | this machine, `flatpak --user install` | painted |
  | `.pkg.tar.zst` (Arch, P1) | neezer-desktop-1.6.0-1-x86_64.pkg.tar.zst | **34 498 908 B** (32.9 MiB) | `pacman -U` | — |

  These are the five files **attached to the release v1.6.0** by the replay
  (`workflow_dispatch -f tag=v1.6.0`, run 44159184701, tag left untouched at
  `a1d71917`), downloaded again from the release and installed a second time on
  their own X server: the `.deb` on Debian 13, the `.rpm` on Fedora 45, the
  AppImage here, the Flatpak with `flatpak --user`. A local build gives the same
  file within a few hundred bytes (timestamps inside the archive).

  The two binaries are **60 437 392 B** and **38 692 288 B** (99 MB installed;
  the `.deb`'s `Installed-Size` says 96 860 KiB). All four printed
  `first_frame_ms` on their own X server (`:172`, `:183`, `:185`, `:192` —
  `neezer_xvfb_start`, a free number from 140, never the session's display), and
  every one ran `neezer-transcribe --version` from the installed copy. The
  Flatpak's sandbox listing showed `pipewire-0`, `pulse`, `wayland-1`, `bus` and
  `/dev/dri` (`card1`, `renderD128`), and its sidecar found the RADV 780M inside
  the sandbox: `ggml_vulkan: 0 = AMD Radeon 780M Graphics (RADV PHOENIX)`.

- **Pitfalls found by packaging, not by reading**:
  - **`actions/upload-artifact`/`download-artifact` do not carry the executable
    bit**: the four packaging jobs received the two binaries 0644 and every one
    refused them (measured on #283, all four jobs at once). The four jobs `chmod
    +x` after the download, and `neezer_check_binaries` now tells "absent" from
    "not executable" so the next person reads the cause instead of a silence.
  - **`DISPLAY` is unset on a runner**, so `[ "$DISPLAY" = ":140" ]` under
    `set -u` aborted the first test with `unbound variable`: the harness reads
    `${DISPLAY:-}`, because a machine with no display at all is the normal case
    for a CI job, not an error.
  - **A GPUI app aborts when fontconfig resolves no font**: `failed to resolve
    font '.SystemUIFont' or any of the fallbacks` (panicked, exit 134) in a bare
    Debian container. Any desktop has a font, a `--no-install-recommends`
    container does not. The `.deb`/`.rpm` now `Recommends` DejaVu and the two
    tests install one — a package that starts only on a machine that happens to
    have fonts is a package that does not start.
  - **`--socket=pulseaudio` does NOT expose `pipewire-0`** (measured inside the
    sandbox: `pulse` yes, `pipewire-0` no). cpal opens PipeWire's own socket
    first, so the Flatpak needs `--filesystem=xdg-run/pipewire-0` or it silently
    runs on its ALSA fallback.
  - **Debian renamed the same library twice** (`libasound2` →
    `libasound2t64`, `libpipewire-0.3-0` → `-0t64` on trixie, and `dpkg-shlibdeps`
    would write the BUILD machine's name into the control file): the `Depends:`
    line carries the alternative, which installs on both.
  - **An inherited `WAYLAND_DISPLAY` steals the test's display**: GPUI prefers
    Wayland whenever the variable is set, and a test run from a Wayland session
    then reports `failed to connect to Wayland compositor`. `neezer_xvfb_start`
    unsets it — the same rule as the memory measurements (our own server, never
    the person's session).
  - **A Flatpak launcher entry must be named after the app id**, and the window
    must report that same id: `rename-desktop-file` + `rename-icon` +
    `copy-icon` in the manifest, `FLATPAK_ID` in `main.rs`, and the one
    `StartupWMClass` line rewritten in `flatpak/build.sh`. Miss it and the
    sandboxed window has no launcher to group with.
  - **`appimagetool`'s `continuous` tag is a moving build**, so `build.sh` pins it
    by SHA-256 and fails on a different file; bumping the pin is a deliberate
    act. Its build 370 warns that an SVG `.DirIcon` "is hard to parse, so file
    managers and thumbnailers cannot be expected to handle it" and will refuse it
    in a future release, so the AppDir's icon is a PNG: the web app's own
    committed `webapp/public/icon-512.png`, which `compare -metric AE` puts at
    **0.19 px** of difference from `neezer-desktop.svg` (the same mark, no
    rasteriser to install). `flatpak-builder`'s flag for a machine without the
    FUSE helper is `--disable-rofiles-fuse` (the documentation page shows a newer
    name that this version rejects).
  - **`cargo-deb` and `cargo-generate-rpm` were read and not used**: both want
    their metadata in one crate's `Cargo.toml` (`neezer-desktop` is not the crate
    that knows about `neezer-transcribe`, and the workspace has nine), and
    `cargo-deb` wants to run `cargo build` itself (`--no-build` exists but its
    asset paths are tied to `target/release/` of that one crate). The four
    packages stay in `desktop/packaging/`, which is where the Arch package
    already was.

### D17 — The whole journey, measured (delivered 2026-11-17)

- **The rule, "fini" is a run, not a claim.** The acceptance is one command on the worst
  realistic input, and every number below comes from it: the app (release build) against
  `tools/perf_api.py --serve` — 8 000 tracks, 4 000 favourites, a 4 000-row playlist, a show,
  1 333 albums with a real 1 000 px sleeve — either directly or through a **degraded link with no
  root** (`desktop/tools/journey/throttle.py`: 300 ms RTT, 1 Mb/s, 2 % of chunks paying one
  retransmission timeout), on **its own Xvfb** (`:130`+, never the person's session), each screen
  opened by `--route <token>` so a run is the same run every time. The tools are
  `desktop/tools/journey/{throttle,drive,bench,checks,soak,frame_stats}.py` and `frame_bench.sh`;
  their README states the method. **A benchmark that cannot be re-taken by the next person is not a
  measurement.**
- **The rule, the bench talks to its own machine.** Every URL `desktop/tools/journey/bench.py` asks
  is `perf_api.py` or its proxy, both bound to loopback, so `local_url` refuses anything that is not
  `http` on `127.0.0.1`/`localhost`/`::1` before urllib sees it (urllib also speaks `file://`, and a
  URL that came back in a response must not turn the bench into a file reader), and `local_open` is
  the one place that builds a request — one line to audit, its `# nosemgrep` naming that guard. The
  login password is `BENCH_PASSWORD`, the throwaway account `perf_api.py` seeds for this run and
  nothing else.
- **The rule, a screen is reached by a path.** D17's first audit answered one question for every
  variant of `Route`: *can someone who has never seen this screen open it?* One could not —
  `Route::Queue` existed, the `q` panel drew the same screen, and **nothing in the app ever pushed
  the route**, so the full-screen queue was unreachable. `g q` is its way in now, and
  `Route::access()` (`nav.rs`) states, for every route, whether it is reached by a sidebar entry, a
  shortcut the `?` overlay prints, a link on another screen, or the app itself (login,
  onboarding); `screens::every_route_of_the_table_has_a_way_in` walks the whole table and fails on a
  shortcut the overlay does not list. The other 30 routes were already reachable (the eight
  Bibliothèque tabs by their strip, Genres by `g e` **and** the admin's sidebar row, Récap and
  Historique from Nouveautés).
- **The rule, the playhead is not the UI's.** D7 reported the 4 Hz position tick freezing with the
  window hidden, and it was true: the tick was a GPUI `spawn`ed task, a hidden window stops the
  frame loop, and `playerctl position` answered the same value for as long as the window was away.
  Three things changed. **MPRIS is published by its own thread** (`wiring::mpris::PositionClock`,
  250 ms, reading the mixer's clock through `Audio::position`, writing the silent `Position`
  property) — measured with the window unmapped: **2.00 s advanced visible, 2.00 s hidden**, the
  same 2 s of audio. **The listen ledger reads the engine's clock before every command**
  (`AppState::sync_playhead_from_engine`), so a `next` pressed with the window hidden no longer
  under-counts the seconds heard. And the shell's tick only redraws. The clock's own test
  (`the_position_clock_moves_without_a_window`) runs the real loop with no window at all.
- **The rule, opening the queue is enough to use it with the keys.** D9 left the focus to a click,
  so `q` opened a panel that ignored `j`, `Alt+↑` and `Suppr` until a row was clicked. The first
  paint of `screens::queue::Screen` takes the focus (`window.focus`, once), which is what Tab would
  do; `opening_the_queue_is_enough_to_use_the_keys` drives `Fin`, `Alt+↑` and `Suppr` without ever
  focusing the window itself. **And a row drags**: an upcoming row carries `on_drag`/`on_drop` and a
  drop is `Command::move_to` — the same edit `Alt+↑` makes, so the gesture cannot move the playing
  row or a played one (`move_row` refuses both). One command, two gestures.
- **The rule, a sleeve is painted at the class of its box — including the two places that were
  not.** The now-playing stage painted the archived 1 000 px file through `img()`, and the player
  bar drew a disc glyph and claimed it was "until the cover widget exists". Both go through
  `cover::view` now: the stage at **`Class::Page` (512 px)** for a box of at most 340 px, the bar at
  **`Class::Row` (128 px)** for its 44 px slot, each with its own `Covers` (shown, freed and
  frame-swept like every list). Counted, not guessed: **1 048 576 B** of decoded pixels per stage
  sleeve instead of **4 000 000 B**, **65 536 B** per bar sleeve, and the archive's own 1 753 091 B
  JPEG becomes a 512 px copy the first time it is drawn.
- **The rule, an older server hides its features.** A server that predates a feature answers 404 for
  its routes; a client that finds out by opening the screen shows a menu entry that cannot work.
  `nz-api`'s `Feature` (Party, Remote, Genres, Transcription, Folders) is asked once per session with
  **one `OPTIONS` per feature on the `Prefetch` lane** — a route that exists answers 200, or 405 for
  another method (Folders has no GET), an unknown one 404 — and the shell does not draw the entry it
  is missing and replaces a screen reached by a link with « … n'est pas sur ce serveur » and the
  reason. A doubt (offline, a 5xx) hides nothing. Measured against the bench: the five probes exist
  (`an_optional_route_is_told_apart_from_a_missing_one`), and the whole set costs **5 requests,
  1.6 kB** per session.
- **The rule, the bench's library must be one the app can actually play.** Building the benchmark
  found three bugs in `tools/perf_api.py`, all of which made a measurement say the wrong thing:
  `--archived --recap` aborted with `UNIQUE constraint failed: track_analysis.track_id` (the
  listening seed had measured the 300 titles its events play, and `make_archived` inserted them
  again — it **updates** them now); `--archived` wrote a style without `style_confidence`, so the
  Récap read an archived library as **0 % measured** (and `--archived`'s own coverage check failed at
  0.90); and the archived rows all pointed at **silent WAVs**, which the desktop cannot decode — a
  real server's archive holds FLAC/Opus and the benchmark now points the rows it plays at real FLACs
  **inside the archive** (the server serves nothing outside it). The store keeps a row id as **32 hex
  characters without the dashes the API prints**, which cost one debugging round: an `UPDATE` keyed
  by the dashed form matched nothing and every stream answered 503.
- **Measured** — every number below from `desktop/tools/journey/bench.py`, `checks.py` and
  `soak.py` on this machine (release build, Xvfb `:130` 2600×1700, `GPUI_X11_SCALE_FACTOR=1`), with
  `perf_api.py --serve --archived --podcasts --seed-lyrics --recap --onboarding --covers`:

  **Requêtes et octets par écran** (première visite, app store vide ; `_startup` = `/api/me`
  seul, mesuré à part). Le lien dégradé est le même serveur derrière `throttle.py` à
  300 ms RTT / 1 Mb/s / 2 % : une ligne `≤` est une borne basse — la fenêtre de silence de
  ce passage s'est refermée avant les pochettes de cet écran, et le chiffre n'est donc pas
  complet. Les 5 `OPTIONS` des sondes de fonctionnalités atterrissent dans chaque fenêtre
  (5 requêtes, 1,6 kB).

  | écran | 1280 req | 1280 octets | 2560 req | 2560 octets | dégradé req | octets |
  |---|---:|---:|---:|---:|---:|---:|
  | `home` | 19 | 44 kB | 25 | 197 kB | 31 | 203 kB |
  | `search` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `library:playlists` | 62 | 1.5 MB | 62 | 1.5 MB | 68 | 1.5 MB |
  | `library:favorites` | 14 | 672 kB | 28 | 1.0 MB | 3 | ≤7 kB |
  | `library:artists` | 3 | 20 kB | 3 | 20 kB | 9 | 26 kB |
  | `library:albums` | 63 | 1.5 MB | 63 | 1.5 MB | 69 | 1.5 MB |
  | `library:folders` | 3 | 11 kB | 3 | 11 kB | 9 | 17 kB |
  | `library:uploads` | 3 | 7 kB | 3 | 7 kB | 9 | 13 kB |
  | `library:dislikes` | 3 | 7 kB | 3 | 7 kB | 9 | 13 kB |
  | `library:unavailable` | 3 | 7 kB | 3 | 7 kB | 9 | 13 kB |
  | `podcasts` | 3 | 4 kB | 3 | 4 kB | 9 | 9 kB |
  | `releases` | 4 | 13 kB | 3 | 12 kB | 10 | 18 kB |
  | `recap` | 19 | 290 kB | 19 | 290 kB | 25 | 296 kB |
  | `history` | 5 | 85 kB | 16 | 365 kB | 11 | 91 kB |
  | `genres` | 4 | 12 kB | 4 | 12 kB | 10 | 18 kB |
  | `import` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `party` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `remote` | 2 | 2 kB | 2 | 2 kB | 8 | 8 kB |
  | `now-playing` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `queue` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `viz` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `settings:lecture-et-son` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `settings:apparence` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `settings:visualiseur` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `settings:telechargements` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `settings:decouverte` | 4 | 5 kB | 4 | 5 kB | 10 | 11 kB |
  | `settings:sources` | 6 | 9 kB | 6 | 9 kB | 12 | 15 kB |
  | `settings:compte` | 2 | 3 kB | 2 | 3 kB | 8 | 9 kB |
  | `settings:transcription-locale` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `settings:assistance` | 3 | 4 kB | 3 | 4 kB | 9 | 9 kB |
  | `settings:raccourcis` | 1 | 1 kB | 1 | 1 kB | 7 | 7 kB |
  | `settings:utilisateurs` | 3 | 5 kB | 3 | 5 kB | 9 | 11 kB |
  | `settings:serveur` | 11 | 13 kB | 11 | 13 kB | 17 | 19 kB |
  | `artist:1000185` | 6 | 34 kB | 6 | 34 kB | 12 | 39 kB |
  | `album:2000435` | 4 | 29 kB | 4 | 29 kB | 10 | 35 kB |
  | `credits:track:3000329` | 2 | 3 kB | 2 | 3 kB | 8 | 9 kB |
  | `credits:person:1000185` | 2 | 2 kB | 2 | 2 kB | 8 | 8 kB |
  | `playlist:00d27951-1bf7-446e-bfce-5f02457019c7` | 24 | 545 kB | 13 | 288 kB | — | — |

  Ce que la table dit, en clair : l'ouverture de l'app coûte **1 requête, 1,4 kB**
  (`/api/me`) ; un écran qui liste la bibliothèque coûte **1,5 MB de pochettes** (60
  playlistes ou albums, chacune une copie 384 px) ; les Favoris passent de **672 kB à
  1,0 MB** quand la fenêtre passe de 1280 à 2560, parce qu'une fenêtre large peint plus de
  rangées ; et la moitié des écrans (Réglages, File, Lecture en cours, Visualiseur,
  Import) ne demandent **rien** de plus que `/api/me`.

  **Clic → son**, mesuré sur les deux liens (le temps est l'horloge de l'app : `load` au
  geste, `started` au premier échantillon ; les octets sont ceux du proxy) :

  | état | local | 300 ms / 1 Mb/s | ce qui se passe |
  |---|---:|---:|---|
  | en cache (appareil) | 83 ms | **87 ms** | le fichier est déjà dans `nz-cache`, 0 octet d'audio demandé |
  | archivé (transcodage serveur prêt) | 88 ms | **2 273 ms** | le serveur sert un fichier fini ; le lien paie la tête de lecture (≈ 200 kB) |
  | froid (le serveur génère) | 87 ms | **4 688 ms** | le serveur transcode en l'envoyant : la génération s'ajoute au lien |

  Les trois passent par la même `Command`, celle qu'un clic sur une rangée envoie. Sur le
  lien dégradé, 2,3 s et 4,7 s sont ce qu'un mobile à 1 Mb/s paie pour un titre jamais
  entendu, et 87 ms ce qu'il paie pour un titre déjà écouté — la règle 3 en un tableau.

  **Temps d'image** (release, Xvfb `:160`, 2600×1700, lavapipe) : au repos **9 images**
  tracées en 1,2 s, intervalle p50 **32 ms**, construction d'arbre p95 **0,23 ms** ;
  pendant 40 `Page suivante` sur 4 000 favoris **33 images**, intervalle p50 **92 ms** (le
  rasteriseur logiciel), construction p95 **0,19 ms**, max **0,50 ms** ; visualiseur
  ouvert, même ordre. **Le taux d'images de Xvfb est un plancher** : c'est lavapipe, pas le
  panneau. Le chiffre du panneau vient de `frame_bench.sh`, que la personne lance
  elle-même (la commande est dans la PR) — ce script ne lance rien.

  **RSS** (la table ci-dessus, par taille de fenêtre) : app au repos **214–219 MB à
  1280×800**, **297–335 MB à 2560×1600** — l'écart est celui de la fenêtre, pas de l'app
  (*Memory* ci-dessus) ; lecture en cours avec la file ouverte **~218 MB**, visualiseur
  **~215 MB**. Rien de tout cela n'est les données de l'app : le tas vivant reste ~9 MB.

  **Une heure de lecture** (`soak.py`) : lecture en boucle d'un FLAC de 30 s **pris dans le
  cache de lecture de l'appareil** (donc zéro réseau pour l'audio), **40 téléchargements
  bulk** en file et un vrai `neezer-transcribe file` (ggml-tiny, Vulkan) en parallèle sur la
  même machine, pendant 3 600 s : **0 sous-alimentation** sur 369 relevés, **125
  démarrages de piste** (la boucle), 14 téléchargements bulk en file (2 terminés dans
  l'heure — le débit bulk est plafonné à 25 % du flux, règle 1), RSS 214 → 224 MB, aucun
  arrêt de la lecture. Le détecteur de gel du banc lit les `started` (une boucle = un `started`), parce
  que l'échantillonneur d'état à 30 s s'aligne sur la boucle de 30 s et lit toujours la
  même phase : un relevé de position seul aurait dit « gelé » à tort.

  **Chemin de mise à jour** : une base desktop d'une version antérieure s'ouvre
  (`nz-cache`'s tests : un fichier v1, un fichier de 0 octet, une base existante, 0,5 ms),
  et un serveur sans une route masque la fonctionnalité — `an_optional_route_is_told_apart_from_a_missing_one`
  la prouve contre le vrai serveur, et un passage du banc derrière un proxy qui répond 404
  pour `/api/party/*` montre l'entrée « Écoute partagée » disparue et la phrase « L'écoute
  partagée n'est pas sur ce serveur » à la place de l'écran.

- **Pitfalls found by measuring** (each cost a round):
  - **GPUI uses Wayland when it can, whatever `DISPLAY` says.** The first bench run set `DISPLAY=:130`
    and inherited `WAYLAND_DISPLAY`, and the window opened on the developer's Hyprland session while
    the bench waited for it on Xvfb. `App::env` strips `WAYLAND_DISPLAY`/`WAYLAND_SOCKET`/the
    Hyprland signature and sets `XDG_SESSION_TYPE=x11`, and `drive.assert_isolated` refuses to run
    with `WAYLAND_DISPLAY` set.
  - **`XGetImage` on a 24-bit root answers four bytes per pixel**, BGRX: handing that stream to
    `convert -depth 8 bgr:-` fails on a truncated tail. The bench drops the fourth byte.
  - **A log the app appends to cannot be emptied by deleting it**: the handle writes to the unlinked
    inode. Truncate the path instead.
  - **Keys pressed before a list's rows land play nothing** (`Intent::PlayFrom` on an empty list),
    and the run then measures a session that never started: the bench waits for the link to go quiet
    before pressing, and retries the gesture.
  - **`EngineEvent::Started` fires once per voice**: replaying the row that is already playing
    measures the previous play's start (a 6 ms "click to sound" that never happened). A cold/archived
    pair needs a different row played in between, or a fresh process.
  - **The app prefetches the row AFTER the playing one**, so a "cold" target must be a row it has not
    prefetched — row 0, from a fresh store, is the honest cold click; row 1 is warm before the click.
  - **A queue that walks into a row the bench did not archive stops the sound**: the first soak sat
    silent for 780 s with 0 underruns, which measures nothing. The soak now sets **repeat one** on a
    known-good FLAC and recovers a stall by going back to row 0 — not by pressing `Entrée` on the
    same row again.
  - **The app's own probes land inside a screen's request window** (`/api/me`, then the five
    `OPTIONS`): the table below separates `_startup` and `_probes`.
  - **Under Xvfb the present rate is the software renderer's** (lavapipe: 10–30 fps whatever the app
    draws). The frame numbers below are the app's own frame-build time and the X server's delivered
    interval; the panel's rate comes from `frame_bench.sh`, run by the person on their display.

### D18 — Les téléchargements dans les écrans (livré 2026-11-17)

- **Le besoin** : rien ne se téléchargeait depuis les écrans. D13 avait livré le worker, la page
  Réglages → Téléchargements et le badge, et `screens/show.rs` son bouton « Tout télécharger » ;
  partout ailleurs la seule action « Télécharger » était l'ARCHIVAGE serveur (`POST /api/download`),
  qui n'a rien à voir avec l'hors-ligne. La parité manquait donc à l'endroit où le geste se fait :
  l'album, la playlist, les favoris, l'émission, le menu d'une ligne et la file.
- **La règle, UNE dérivation pour tous les écrans** : `widgets/download_control.rs` porte les cinq
  états (`Idle`, `Queued{done,total}`, `Running{done,total,bytes,size}`, `Done{tracks}`,
  `Failed{cause,failed,done}`), leurs libellés français, leur phrase d'action
  (Télécharger / Annuler / Réessayer / Retirer), le chip d'une ligne et le `Watch` (le snapshot lu
  une fois par version publiée, redessiné seulement quand `version` bouge, 300 ms). Aucun écran ne
  re-dérive un état : album, playlist, favoris, émission, file et le menu d'une ligne appellent les
  mêmes fonctions, donc le même mot ne peut pas dire deux choses.
- **La règle, l'état vient du SNAPSHOT — et il ne MENT pas.** Lire `is_downloaded` par ligne serait
  une requête SQLite sur le thread GPUI (`nz-cache` : une connexion derrière un mutex, donc une
  lecture d'écran peut attendre la transaction d'écriture du worker et figer une image). L'état
  vient donc de `Snapshot` : les échecs sont par titre, le titre en vol est publié, et un titre
  téléchargé seul est un « ensemble » de un (D13). Ce que le snapshot ne peut pas PROUVER n'est pas
  inventé : une ligne d'une collection téléchargée PARTIELLEMENT lit « Télécharger », et c'est le
  contrôle de la collection qui dit `12 / 40`. Sans coût : le store ignore un titre déjà publié
  (`add_set`'s `ON CONFLICT DO NOTHING`), donc ré-appuyer ne redemande rien.
- **La règle, une playlist téléchargée reste COMPLÈTE** : `screens/playlist.rs` re-`choose` son
  ensemble à chaque ouverture quand il compte moins de titres que la page (`sync_download_set`) —
  le worker ne relie que les nouveaux, donc une playlist de 4 000 titres qui en gagne un coûte UN
  téléchargement et ZÉRO requête pour les 3 999 autres. Aucune autre app ne le fait : la SPA
  télécharge des titres, pas des listes, donc un ajout après coup y est oublié.
- **La règle, le clavier et la souris sont le même chemin** : `t` (`DownloadSelection`) et `Maj+T`
  (`RemoveDownload`) sont liés une fois (`keymap.rs`) et chaque écran les enregistre sur l'app avec
  une poignée faible (D8b : un écran ne prend pas le focus). Le bouton du contrôle, l'entrée du
  menu et la touche finissent dans le MÊME gestionnaire ; `download_control::row_verb` est
  l'unique implémentation du verbe d'une ligne, donc une ligne téléchargée propose « Retirer » et
  pas « Télécharger ».
- **La règle, supprimer se demande deux fois** : « Tout supprimer » (Réglages → Téléchargements)
  arme le bouton au premier clic et supprime au second, avec le compte de ce qu'il a retiré. Un
  téléchargement est permanent ; c'est le seul contrôle destructeur de l'app.
- **Mesuré** (cette machine, 16 threads, Rust 1.99.1 ; `cargo test --workspace` :
  **448 tests de l'app, 0 échec**, 11 `#[ignore]` lancés par leur parent, toutes les autres crates
  vertes ; `cargo fmt --check` et `cargo clippy --workspace --all-targets -- -D warnings` propres) :
  - **requêtes par collection** : un album de 3 titres coûte **3 requêtes `/api/stream/<id>`**
    (une par titre, chacune une fois), et une SECONDE demande du même album **0** — mesuré sur une
    vraie socket (`stub` du banc, `downloading_an_album_costs_one_request_per_title_and_nothing_the_second_time`).
  - **lecture pendant 200 téléchargements** (les bancs de D13, relancés avec ce lot) : le flux qui
    joue lit **798 949 B/s seul, 565 297 B/s pendant 200 téléchargements (29,2 % de baisse, budget
    30 %)**, le seau est armé à **127 745 B/s**, et **6,00 s d'audio rendues avec 0 sous-alimentation**
    pendant que **200/200 titres** arrivent (`a_two_hundred_track_download_keeps_the_playing_streams_share`,
    `a_two_hundred_track_download_while_a_track_plays_has_no_underrun`) — le lot n'ajoute AUCUN
    accès réseau : il ajoute un état lu dans un mutex.
  - **clic → son** : **1,1 ms de médiane, 1,1 ms de maximum**, en local comme par le réseau
    (`nz-audio` : `click_to_first_sample_from_a_local_file_is_under_50_ms`,
    `click_to_first_sample_over_the_network_is_under_250_ms`) — le worker est un thread `nice 10`
    et la voie Bulk, donc un titre lancé démarre sans l'attendre.
  - **mémoire comptée** (`app/src/testmem.rs`, pas de RSS) : un écran paie **un `Rows` par version
    publiée**, soit **271 B** pour une collection de 4 000 titres (1 en vol, 2 échecs), et **235 706 B**
    pour le pire cas que le snapshot exprime — 4 000 titres choisis UN PAR UN
    (`a_four_thousand_row_list_keeps_a_few_kilobytes_and_not_its_rows`). Un `Watch` ou un `Rows`
    vide ne coûte rien : aucun écran sans téléchargement ne change.
  - **états du bouton** : les cinq états sont testés en GPUI (`the_album_download_control_walks_its_five_states`),
    chaque état portant son libellé, son verbe et sa CAUSE ; `t` et `Maj+T` sont testés sans souris
    (`t_and_maj_t_queue_and_take_back_the_album`) et le menu d'une ligne via `row_verb`.
- **Regardé, pas seulement asserté** : neuf captures dans `docs/desktop/d18/`, sur MON Xvfb
  (`:171`, `-dpi 96`, `GPUI_X11_SCALE_FACTOR=1`, jamais l'affichage de la personne), avec le banc
  `tools/perf_api.py --serve` et un magasin amorcé (40 titres « Téléchargé », ou 38 + 2 échecs) :
  l'album et la playlist à 1280×800 et dans le panneau 420 px, les favoris à 2560×1600, l'état
  « Téléchargé · 40 » + « Retirer » + un chip par ligne, l'état « 2 échecs · 38 sur l'appareil ·
  HTTP 404 : … », et Réglages → Téléchargements aux deux tailles. **Trois défauts trouvés par les
  captures, qu'aucun test ne voyait** : (1) le contrôle poussait « Exporter » hors de la fenêtre à
  660 px — les barres d'outils de la bibliothèque, de la playlist et de l'album passent en
  `flex_wrap()` ; (2) **Réglages → Téléchargements affichait encore « Bientôt »** : D13 avait livré
  l'écran mais D14a a refactoré la page en `delegated` APRÈS, et la section n'a jamais eu son bras
  — elle l'a maintenant, et `the_downloads_section_is_its_screen_and_not_the_stub` le tient ;
  (3) la carte d'un podcast n'avait qu'un glyphe orphelin en bas — il est sur la ligne des
  épisodes.
- **Ce que c'est maintenant** : `widgets/download_control.rs` (`State`, `label`/`detail`/`action`/
  `short`/`fraction`/`busy`/`on_device`/`glyph`, `set`, `collection`, `track`, `Rows`, `key`,
  `row_verb`, `chip`, `control`, `Watch`), `widgets/context_menu.rs` (`Item::Download`,
  `RetryDownload`, `CancelDownload`, `RemoveDownload` et le vocabulaire `Download`), les six écrans
  (album, playlist, bibliothèque/favoris, émission, podcasts, file) et `screens/settings/downloads.rs`
  (« Tout supprimer », l'emplacement des fichiers, les libellés de collection Favoris et File).
- **Différence assumée avec la SPA** : le menu d'une ligne n'a pas de sous-menu « Télécharger en… » ;
  la qualité est celle de Réglages → Téléchargements (ou du contrôle de la page), comme le worker la
  range. Le bouton d'ARCHIVAGE garde son mot séparé (« Archiver », contre « Télécharger » pour
  l'hors-ligne) là où la SPA écrit « Télécharger » pour les deux avec un titre qui les distingue.
- **Pitfalls trouvés en construisant** : `Offline::snapshot()` CLONE (sets, labels) — l'appeler par
  ligne aurait cloné 4 000 entrées par ligne et par image, d'où `Rows` (résolu une fois par version)
  et `Watch::get` qui ne relit que si la version a bougé. La version elle-même se lit par
  `snapshot().version`, donc chaque écran paie UN clone par 300 ms quand rien ne bouge (quelques
  kilo-octets, six écrans) ; un accesseur `version()` dans `nz-offline` le supprimerait. Un écran en `&self` (le menu d'une
  émission) ne peut pas appeler `Watch::get` : il lit `self.rows`. `cx.entity().read(cx)` depuis un
  rendu de la même entité PANIQUE (« cannot read … while it is already being updated ») : l'état
  « armé » de « Tout supprimer » se passe en paramètre. Et `cargo fmt --all` REFORMATE les crates
  partagées du dépôt (`webapp/rhythm`, `appcore`, `nscore`, `trainer`, `measure`) dont les octets
  exacts sont dans le hash du `rhythm.wasm` committé : c'est `cargo fmt --check` (sans `--all`) que
  la CI lance, et `git checkout -- webapp/ android/` rattrape le `--all` d'un agent.

## Worker prompt template

Each coding worker receives this text with `<LOT>` replaced by the lot's id and title:

> Tu implémentes le lot `<LOT>` de l'app desktop Neezer. Lis d'abord `CLAUDE.md` (processus et
> règles non négociables), puis `docs/agents/desktop.md` en entier, puis les notes que le lot cite
> et les fichiers SPA/serveur qu'il nomme. Ne touche QUE les fichiers que le lot liste ; si tu as
> besoin d'un autre fichier, arrête-toi et signale-le. Respecte les contrats (API Rust, routes,
> payloads) exactement comme écrits ; si un contrat est faux face au code réel, corrige la note dans
> la même PR et dis pourquoi. Branche `desktop/<lot>-<slug>` depuis `master` (sous Orchestrator : dans l'espace de
> branches de ta session). Avant de pousser, dans
> `desktop/` : `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
> `cargo test --workspace`. Mesure chaque critère d'acceptation, écris les nombres à côté des
> assertions et dans la PR (avec les commandes). Pour un lot d'interface, lance l'app et regarde-la
> à 2560×1600, 1280×800 et avec le panneau latéral à 420 px. Ouvre la PR (résumé, sources, mesures,
> risques), pousse jusqu'à ce que CI soit entièrement vert (y compris les journaux de
> `security.yaml`), fusionne sans demander. Enfin remplace le paragraphe de plan du lot dans
> `docs/agents/desktop.md` par ce que tu as appris (règle, pourquoi, nombre mesuré).
