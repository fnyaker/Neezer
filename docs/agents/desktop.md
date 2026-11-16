# Desktop app (Omarchy Linux, GPUI)

Reference notes for agents working on `desktop/`. Read [CLAUDE.md](../../CLAUDE.md) first: its
process and seven rules apply here. Other areas: [docs/agents/](./). The server contract this app
speaks is the web app's ([webapp.md](webapp.md), [server.md](server.md)); the audio rules are the
player's ([playback.md](playback.md)); remote control and the listen party are
[party-remote.md](party-remote.md); the native player it parallels is [android.md](android.md).

**Status (2026-11-17): D0, D1, D2, D3, D4, D5, D6, D7, D8a, D13, D14a, D14b, M1, T1 and V1
delivered; the rest planned.**
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
  packaging/arch/       # PKGBUILD, neezer-desktop.desktop, icons
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
| `nz-decode-{a,b}` | one per voice: read source, decode, resample, push into the voice ring | normal |
| PipeWire callback (cpal) | mix the two voices, DSP, write the device buffer, copy to the tap | real-time: no allocation, no lock, no syscall |
| `nz-analysis` | tap → `rhythm` analyser → bars/scope state | `nice 5`; runs only while a viz is visible |
| `nz-bulk` | downloads, cover warming, offline sync | `nice 10`, idle I/O class |
| `neezer-transcribe` child processes | transcription supervisor + whisper engine | `nice 19`, `SCHED_IDLE`, idle I/O, SIGSTOP-able |

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
| Offline: downloads per playlist/album/show, play cache, offline browsing, network indicator | `lib/offline.js`, `lib/playcache.js`, `lib/apicache.js`, `NetworkIndicator`; Android `OfflineStore.kt` | `/stream/*`, `/cover/*`, `/download*` |
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
  downloads go through `Lane::Bulk`, are checked against a pinned SHA-256 list in the crate, and are
  validated like `engine.py#valid_model` (GGML magic, ≥ 1 MiB).
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
    ├─ T1 ─┬─ T2 ─┐              ├─ D13, D14a, D14b, P1
    │      └─ T3 ─┴──────────────┘
    └──────────────────────────────────────── D17 (last)
```

Waves that can run in parallel (disjoint files):

1. **D0** alone.
2. **D1, D5, M1, V1, T1** (five crates, disjoint).
3. **D2, D3, D4, D6, T2, T3** (D2/D3/D4 need D1; D6 needs D5; T2/T3 need T1 and touch disjoint files).
4. **D7** alone (it wires everything and creates every screen stub).
5. **D8a, D8b, D9, D13, D14a, D14b, P1** (each owns its own screen files).
6. **D10b, D11, D12, D15a, D16** then **T4, D15b**.
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
  needs numpy (not a dependency), is what would fill them.
- **Measured** (this machine, debug build, `cargo test -p nz-model`): **31 tests, 0 failed**; the 102
  fixtures deserialize in **23.3 ms**, stripping their optional keys in **185 ms**, and the whole
  run is **1.56 s** against the lot's 2 s budget. `cargo fmt --all --check` and
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
  GPUI state.
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

### D8a — Browse: home, search, artist, album, credits (delivered 2026-11-16)

- **The rule, keys**: a screen does NOT take the shell's focus. D7's `RootView` keeps ONE anchor
  and binds every action there; a screen that focused a handle of its own took the bindings away
  with it as soon as it was replaced — measured: `g l`, then `Alt+←`, did nothing, because the
  focused node belonged to the Home screen that no longer existed and GPUI then dispatches ABOVE
  the view. Two consequences, both fixed: `RootView::sync_screen` re-anchors on EVERY screen change
  (2 lines in `app.rs`), and the list keys are APP-level listeners (`track_list::own`/`deliver`)
  routed to the screen that owns a list, which is also why they work with nothing focused. A
  screen restates `.key_context("Neezer")` on its root so a test can drive it without the shell.
- **The rule, cache then network**: `card::cached` reads the device's copy ONCE, at construction —
  the SPA's `api.swr` first half. A screen that paints from it issues **0 requests before its first
  frame** and exactly one after it (asserted on a stub server's counters). This is the one SQLite
  read on the GPUI thread, and it costs 0.18–0.5 ms (D3's WAL measurement); the alternative, a
  tokio hop, costs a skeleton frame on every revisit, which rule 3 forbids.
- **The rule, one request per settled term**: a term is asked `DEBOUNCE_MS` (150 ms) after the last
  keystroke and only ONE `/search` is in flight per field (`ask` queues the newer term behind the
  answer) — typed "kaya" letter by letter sends **1 request** for `kaya`, then 1 for `kayas`, never
  four; the YouTube fallback has its own 800 ms beat and the passage search 300 ms. The answers of
  the last 16 terms are kept in memory, so a term typed back paints in one frame and still
  revalidates.
- **The rule, the shortlist**: the `Tout` tab lists the first 10 ranked titles and the whole page
  scrolls; `Titres` gives the list its own height and its own scroll. One scroller per tab, never
  two nested ones.
- **The rule, rows are data**: a `TrackList` holds the `Track`s, the queue rows they make (built
  ONCE when the answer arrives, never on the click) and a visible range; only that range is built
  per frame (`virtual_list`). Measured on 4 000 rows: `set_rows` **2.77 ms** and **+13.9 MB** of
  data model (debug), a click copying the queue into the player **2.59 ms** (debug), and the
  row-building work of a frame while paging: **p50 109 µs, p95 501 µs, max 792 µs** over 82 frames
  (`NZ_FRAME_LOG=1`), against the 240 Hz panel's 4.17 ms — the frame's cost is not the list.
- **Measured, per screen** (stub server counters, `cargo test -p neezer-desktop`): home **6**
  requests (`/releases`, `/me/artists`, `/discover/prefs`, `/recommend`, `/moods`, `/recap/years`)
  plus the login; an artist page **2** (`/artist/<id>`, `/artist/<id>/discography`), **1 more** when
  `Titres` opens, and none to the provider when the library does not hold the artist; an album
  **1** (`/album/<id>`), plus `/album/<id>/on-deezer` only when `library`; a search 1 per settled
  term + 1 fallback. RSS of the release app under Xvfb + lavapipe: **199 MB** at the login screen,
  **226 MB** at home, **236 MB** at the search results, **237 MB** at an artist page (the software
  Vulkan driver is most of the difference with D7's 103 MB on the real GPU; the numbers are
  comparable to each other, not to D7's).
- **The rule, a cover is bytes**: `nz-api` gained ONE method, `Api::bytes(lane, path)` — the only
  file outside this lot's list, and the reason is in its doc comment: every route answered JSON or a
  typed value, so a screen had no way to fetch a sleeve without building its own HTTP client, which
  is what that crate exists to prevent. Sleeves are visible-first and bounded
  (`Covers::want`, 8 in flight, `coverqueue.js`'s rule), kept by `nz-cache`, and a second visit
  makes 0 requests for them.
- **The rule, the menu**: one `Intent::Menu` from three gestures (right click on the row, the row's
  `⋯`, `Menu`/`Maj+F10`), drawn inside the list's own box so the panel lands on its row without
  measuring the window. While it is open it INTERCEPTS keystrokes (`Échap` closes, `↑`/`↓` move,
  `Entrée` picks, anything else closes it): a keystroke interceptor, because `Échap` is the shell's
  `CloseTop` and a screen-level handler would never see it. The items are only what D8a can carry
  out; adding to a playlist and replacing a track open D8b's sheets and are NOT listed — a menu
  entry that does nothing is a bug (rule 2).
- **What it is now**: `screens/{home,search,artist,album,credits}.rs` and four widgets —
  `track_list.rs` (the windowed list, `Intent`, `PlayRequest`, the app-level key routing, the frame
  log), `cover.rs` (`Covers`, file lookup, placeholder tiles), `context_menu.rs` (`Menu`, the item
  sets), `card.rs` (the browse kit: card, section head, chips, skeletons, `Phase`, `cached`, the
  test stub server and its harness). `search.rs` also carries the `lib/searchrank.js` port
  (`fold`, `match_score`, `recording_key`, `rank`, `top_result`), pinned to the JavaScript's own
  output: `fold("Été")` is `ete`, `match_score` of the plain and the live "Bohemian Rhapsody" is
  **1.0 and 0.648**, an identical video is dropped, and a video under `YOUTUBE_MIN` never leads.
- **Pitfalls found by building**:
  - **A column that overflows without scrolling clips its list away.** The search page put the
    windowed list under the best-result card with no scroller; the rows were painted below the
    window, `list.visible()` said `0..0`, and a click never reached a row. Found by the right-click
    test, not by looking: `cargo test` said "no row answered", the screen looked fine.
  - **`App::on_action` listeners run in the BUBBLE phase, after node handlers, and re-entering the
    window from inside a dispatch fails SILENTLY** (`window.update` returns an error nobody reads).
    Measured: the handler was found, the intent went nowhere. `deliver` therefore defers by one
    effect flush.
  - **`overflow_y_scroll`/`overflow_x_scroll` live on `StatefulInteractiveElement`**, so a
    scroll container needs an `.id(…)` first — and `context_menu::clipped` aside, a plain `div`
    has no such method at all.
  - **A `VisualTestContext` right click needs a mouse MOVE first**: hitboxes are hover-tested
    against the window's pointer, and `simulate_mouse_down` alone leaves it wherever it was.
  - **`TestAppContext` has no `advance_clock`** (the `TestApp` façade does): a debounce test drives
    `cx.cx.dispatcher.advance_clock(…)` and then `run_until_parked`.
  - **The icon bundle is closed** (`main.rs`'s `ShellAssets`): an unregistered glyph renders
    nothing. This lot draws no Heart/Radio/Download, so the actions that need one are text labels
    and the menu's icons come from the registered set.
  - **The contract, corrected**: `nz_model::Discography` has no `featured` group, although
    `routes/Artist.svelte` reads `disco.featured` — the server never answers it, so the artist page
    has no "Apparaît dans" shelf (the SPA's is always empty). Noted, not invented.
  - A track's credits page cannot show the TRACK's title: `Route::Credits { kind, id }` carries the
    id only, and `nav.rs` is frozen after D7. The SPA opens the sheet from a row and has the track.
    The page says "Crédits du titre" and lists the roles; a title would need a route field.
  - **Visual pass**: 1280×800 was looked at (login, home, search with results, artist, album, the
    row menu, the credits page, and the 420 px queue panel), on an isolated Xvfb display — never the
    user's session. The 2560×1600 shot was NOT taken: the app opens a 1280×800 window and this
    machine has no window manager on the isolated display to resize it (D7 recorded the same limit;
    D17 owns the full-size pass).

### D8b — Library and playlists

- **Files**: `screens/{library,playlist,import}.rs`, `app/src/widgets/{folder_tree,playlist_picker,
  sheets}.rs`. It uses D8a's widgets (`track_list::TrackList`/`element`/`own`, `card::*`, `cover::*`,
  `context_menu::*`) and takes its list keys through `track_list::own`, as every browse screen does.
- **Acceptance**: favourites 4 000 and the 4 000-row playlist open in < 100 ms from cache (measure);
  reorder by drag AND by `Alt+↑/↓` with an optimistic update and a rollback toast on failure;
  folders, members, share link, export, import from link with background progress (rule 2).

### D9 — Now playing, queue, MPRIS wiring

- **Files**: `screens/{now_playing,queue}.rs`, `app/src/wiring/mpris.rs`, `app/src/widgets/{lyrics,
  sleep_timer,quality_menu,speed}.rs`.
- **Acceptance**: synced lyrics follow the position within 50 ms; queue of 4 000 scrolls at display
  rate; media keys work under Hyprland (`playerctl` and the keyboard's keys); the sleep timer ends
  with a 10 s fade; quality change applies at the next track and says so.

### D10b — Visualiser UI (bars, oscilloscope)

- **Files**: `screens/viz.rs`, `app/src/widgets/{bars,scope}.rs`, `screens/settings/viz.rs`.
- **Acceptance**: in the player and full screen at 240 Hz: ≥ 230 fps, CPU of the whole app with
  bars on < 12 % of one core (eco mode: 30 fps cap, < 5 %); the analysis thread stops when no viz is
  visible (assert 0 analyser calls when hidden); settings: palette (cover/theme), intensity, fps cap,
  scope orientation and colour, eco.

### D11 — Podcasts

- **Files**: `screens/{podcasts,show}.rs`, `app/src/widgets/{episode_panel,chapters,markers,
  transcript_panel}.rs`.
- **Acceptance**: progress synced as in *Streams* (pause, 30 s, quit, offline queue); speed per show
  remembered (`podcast.speeds`); transcript panel follows playback and searches; server
  transcription start/stop with its state; episode list paged by cursor (no unbounded fetch).

### D12 — Discovery

- **Files**: `screens/{releases,recap,history,onboarding}.rs`, `app/src/widgets/{flow,
  discovery_slider,mood_card,recap_story}.rs`.
- **Acceptance**: flow and radios top the queue up before it runs dry (no gap); releases badge and
  "vu"; recap story navigable by keys; onboarding complete with keyboard only.

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
  - `wiring::offline`: **11 tests**, the two acceptance benches included (3 s + 3 s of paced
    streaming, 6 s of real-time audio); `screens::settings::downloads`: 4, one of them the layout
    journey below; `widgets::download_badge`: 2.
  - the screen at 2 560×1 600, 1 280×800 and 420 px: the reading column is **896 px** (56 rem) at
    x=832 / x=192 and fills 420 px at 420, inside the viewport at all three; a page of 20 sets + 8
    failures paints in **3.9 ms median / 4.7 ms p95 in release** (45 / 57 ms in a debug build,
    which is why the assertion quotes the release budget) and costs **+5.2 MB RSS** in release
    (+7.3 MB debug) for a 200-set, 200-failure workload.
  - keyboard alone: Tab walks the page in reading order and Espace presses each control (the test
    sees the four quality chips at tabs 3–6 and the « hors-ligne » switch at 7); mouse alone: a real
    hit test on the FLAC chip changes the default quality.
- **What it is now**: `wiring/offline.rs` (`Offline` — `start`/`ensure`/`configure`, `choose`/
  `choose_track`/`drop_set`/`drop_track`/`retry_failures`, `pause`/`resume`, `set_quality`/
  `set_offline_only`/`set_play_cache_cap`, `clear_play_cache`, `local`/`is_downloaded`, `snapshot`,
  and the free `attach`/`local_path`/`note_stream_bytes`/`keep_downloaded`), `Snapshot`/`Failure`/
  `Running`/`WorkerState`, the SPA's own keys (`offline.quality`, `offline.onlyDownloaded`,
  `cache.limit`) and the four-rung picker; `screens/settings/downloads.rs` (`Screen::new(state, cx)`
  for `SettingsSection::Downloads`, five cards, a pager, the failure list, the cache gauges);
  `widgets/download_badge.rs` (`chip` for a row that repaints anyway, `Badge` for one that does not).
- **Deliberate difference from the SPA**: the download picker offers four rungs (FLAC, Opus
  320/128/64), not six — the server answers `OPUS_256`/`OPUS_192` but `nz-audio`'s ladder has no rung
  for them, so offering them would download a file this app cannot describe.
- **Two seams this lot could not land inside its own files** (written down rather than guessed):
  1. `NetOpener::local` (`app/src/wiring/engine.rs`) still answers the play cache only, so a
     downloaded track is found by `Offline::local` but not yet by the engine: `local_path(track)` is
     the one-line call it needs (after `cache.play_cache().lookup(...)`).
  2. Nothing starts the worker at launch: `AppState::on_restored`/`on_logged_in` (`app/src/state.rs`)
     need one `wiring::offline::attach(self)`, or a queued download waits for the Réglages screen to
     open. Both files belong to D7 and no wave-5 lot owns them; the calls are `pub` and tested.
- **Pitfalls found by building**: `simulate_keystrokes` runs key BINDINGS only — `tab` moves the
  focus, but pressing the focused control needs a real `KeyDownEvent`/`KeyUpEvent` through
  `simulate_event` (without it a keyboard test presses nothing and passes vacuously). A screen
  mounted bare has no Tab binding at all: Tab lives in gpui-omarchy's `focus_scope`, so a screen test
  needs the shell's wrapper. `debug_bounds` records selectors only in a build with
  `debug_assertions` (a `cargo test --release` run cannot assert geometry). And a screen's frame
  cost must not be judged in a debug build: GPUI's layout and tab-stop bookkeeping are ~26× slower
  there (45 ms against 3.9 ms for the same page), which is why the budget is quoted in release and
  only loosely bounded in debug.

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
- **The rule, the sink list is the machine's**: `nz-audio` publishes no enumeration,
  so the output card reads `pw-dump` (`media.class == "Audio/Sink"`, one
  `node.description` each — exactly the string `probe` matches, so a chosen sink
  survives a restart) on a blocking thread, sorted so a node's creation order never
  moves a chip under the pointer, and falls back to the device in use when the tool
  is absent. `set_output_device` hands the mixer to the new stream (nothing stops),
  and « Sortie en cours » is re-read 400 ms later, once PipeWire has it. Measured:
  `pw-dump` **29 ms median over 7 runs, 246 KB** of JSON (15.8–43.4 ms), one read per
  visit.
- **Measured** (this machine, Rust 1.99.1; `cargo test -p neezer-desktop`):
  - **102 tests, 0 failed** for the app (D14b's included), 38 of them this lot's
    (33 rules + 5 headless UI).
    `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`
    clean.
  - **layout**: all **10 user sections** (the three other lots' stubs excluded) at
    **2560×1600**, **1280×800** and **660×800** — the laptop with the 420 px queue
    panel open: the root is the window (±1 px), the body never leaves the window
    sideways, never grows past it vertically and never starts off-screen. 30 renders,
    no panic.
  - **keyboard**: the Son pane has **25 distinct tab stops** and the order comes back
    to its first one (walked with `Window::focus_next`, the call gpui-omarchy's Tab
    makes), Shift+Tab retraces it, and the search field narrows the strip to
    « Découverte » and back. Page Down / Page Up scroll the page itself and Home / End
    jump to its ends (Assistance is the long one), on the page's own focus — four keys
    only, so `j`/`k` stay the lists'.
  - **mouse**: the Apparence tab, the Mono switch and the « Normale » chip are
    hit-tested at 1280×800 and move the same state a key would (the route changes
    with `replace`, `values.mono` flips, the level is saved).
  - **RSS**: **+228 KB** for the ten user sections on the second pass with a
    **4 000-row queue** in place (`VmRSS` 57 936 → 58 164 KB; two runs 228 / 236 KB),
    the window's and the first pass's cost excluded — the page holds no per-row data, so
    the worst case stays the queue the shell owns. The first pass through the sections
    reads 1 016 KB alone and 5 960 KB beside the rest of the suite: that is the glyph
    cache being filled with strings nobody had shaped yet, which is why the test warms
    up once and measures the structures, never the font cache.
  - **requests**: the table above is asserted by a test; a revisit asks nothing.
- **Pitfalls found by building**:
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
- **Two seams outside this lot, reported and not shipped** (both live in `state.rs`,
  D7's file: a screen cannot reach a track change):
  - **Tronquer les silences** needs `TrackMeta::edges` primed per track
    (`GET /api/audio/edges/<id>?db=`), for the current track and the next, in
    `AppState::load`/`preload`. The option is absent from the page rather than dead
    (rule 2).
  - **The saved sound is applied when this page is built** (and again if the engine
    arrives while it is open); a session that never opens Réglages runs with the
    engine's defaults. The fix is ~15 lines in `AppEvent::AudioReady`: read the same
    `kv` keys and call the same `Values::apply`.
- **A contract that was wrong**: `nz-api`'s `set_account_auto(id, bool)` posts
  `{"auto": true}`, while `myaccounts.my_accounts_options` requires
  `{"auto": {kind: bool}}` and answers `400 auto invalide`; the screen sends the right
  body through `Api::send`. `link_deezer`'s doc names `"shared"`/`"personal"` while
  the server only distinguishes `"browser"` from every other value (a typed ARL) —
  harmless, but D2's note should say so.

### D14b — Réglages (admin sections)

- **Files**: `screens/settings/{users,server}.rs`.
- **Acceptance**: users and invitations; archive/rules/cleanup preview then run; storage; backfills
  with progress and failure causes (`JobFailures` equivalent); server transcription settings and
  worker list (create shows the token once, revoke); everything a long job does runs as a job with
  progress (rule 2).

### D15a — Remote control

- **Files**: `app/src/wiring/remote_host.rs`, `app/src/wiring/remote_controller.rs`,
  `screens/remote.rs`.
- **Contract**: host = `POST /api/remote/host/<device>` (state + `qv`) and
  `GET /api/remote/host/<device>/poll?since=` (polling, never a held connection), commands applied
  through `PlayerState::apply` ([party-remote.md](party-remote.md)); controller = claim a link
  (`POST /api/remote/claim`), then the app runs with no audio engine and every `Command` sent;
  links list/cut; the owner's chip "Contrôlé par N appareils".
- **Acceptance**: a controller (the SPA in Chromium) drives the desktop: command to sound < 1.2 s on
  loopback (poll period), and the desktop drives the SPA; the level's limits respected (server
  enforces, UI hides).

### D15b — Listen party

- **Files**: `app/src/wiring/party_host.rs`, `app/src/wiring/party_guest.rs`, `screens/party.rs`.
- **Contract**: host publishes the anchor `{t, p}` from the OUTPUT clock (cpal timestamps — better
  than the browser's), the next-track announcement and `gap`; guest plays `/party/<id>/chunk/<track>
  /<k>` (6 s Opus chunks) scheduled on the device clock with `appcore`'s `sync.rs` estimators and
  the correction rule of `party-remote.md`.
- **Acceptance**: the repository's party bench (`webapp/test/party/run.mjs`) with the desktop as
  host and as guest: offset ≤ 5 ms median, ≤ 15 ms p95.

### D16 — Genre studio and client analysis

- **Files**: `screens/genres.rs`, `app/src/widgets/genre_*.rs`, `app/src/wiring/analysis.rs`.
- **Contract**: the SPA's genre studio over `/genre/*`, `/explorer*`; client-side analysis and
  training use `webapp/measure` and `webapp/trainer` natively (path deps), as BULK work that yields
  like the transcription.
- **Acceptance**: label, tree edit/undo, bulk tag with preview, share import/export; analysis
  backfill of 100 tracks runs reniced with 0 underruns while playing.

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

### T4 — Transcription in the app

- **Files**: `screens/settings/transcribe.rs`, `app/src/wiring/transcribe.rs`, the transcript
  panel's local-source branch in `widgets/transcript_panel.rs` (coordinate: D11 owns the file; T4
  runs after D11).
- **Acceptance**: Réglages → Transcription locale: engine status (backend, device, fallback cause),
  model choice and download with size and checksum, enrol/revoke this PC as a worker (admin),
  battery rule, current job with progress and failures with causes; playing music while a job runs:
  0 underruns and UI p95 frame time unchanged within 10 % (measure both with and without the job);
  a local transcript shows in the panel and is searchable.

### P1 — Arch packaging and release

- **Files**: `desktop/packaging/arch/{PKGBUILD,neezer-desktop.desktop,neezer-desktop.svg}`,
  `.github/workflows/release.yaml` (a job that builds the package in `archlinux:latest` and
  attaches `neezer-desktop-<ver>-x86_64.pkg.tar.zst` on `v*` tags).
- **Contract**: `makedepends=(rust cargo cmake clang vulkan-headers shaderc)`, `depends=(pipewire
  libxkbcommon wayland vulkan-icd-loader fontconfig freetype2 dbus)`, `optdepends=('vulkan-radeon:
  transcription GPU AMD' 'vulkan-intel: transcription GPU Intel' 'nvidia-utils: transcription GPU
  NVIDIA')`; `prepare()` `cargo fetch --locked --target host-tuple`; `build()` `cargo build --frozen
  --release`; installs both binaries, the `.desktop` (`StartupWMClass=neezer-desktop`) and the icon.
- **Acceptance**: `makepkg -si` on this machine installs it, it appears in the Omarchy launcher, and
  `namcap` reports no error.

### D17 — The whole journey, measured

- **Goal**: the definition of done for the app, on the worst realistic input.
- **Acceptance**: on `perf_api.py --serve` (8 000 tracks, 4 000 favourites, 4 000-track playlist) and
  on a real server over a throttled link (`tc netem` 300 ms, 1 Mb/s, 2 % loss): every screen's
  request count and bytes in a table; click-to-sound for a cached, an archived and a cold track;
  frame time p95/max during navigation and scrolling; RSS idle/playing/with viz; 0 underruns over
  one hour with downloads and a transcription running; the upgrade path (an old desktop DB opens; an
  older server without a route hides the feature). Results replace the plan paragraphs in this note.

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
