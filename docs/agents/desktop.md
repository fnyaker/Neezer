# Desktop app (Omarchy Linux, GPUI)

Reference notes for agents working on `desktop/`. Read [CLAUDE.md](../../CLAUDE.md) first: its
process and seven rules apply here. Other areas: [docs/agents/](./). The server contract this app
speaks is the web app's ([webapp.md](webapp.md), [server.md](server.md)); the audio rules are the
player's ([playback.md](playback.md)); remote control and the listen party are
[party-remote.md](party-remote.md); the native player it parallels is [android.md](android.md).

**Status (2026-11-16): D0 delivered (workspace, pins, CI); D1 onwards planned.** This note is the
plan the coding lots follow, in order. Each lot, when it lands, replaces its plan paragraph by what
was learned (rule, why, number) — a delivered lot is not a changelog.

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
| Transcription | [whisper-rs](https://github.com/tazz4843/whisper-rs) (moved to Codeberg), `whisper-rs-sys` 0.15 `build.rs` (read), [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (Vulkan `GGML_VULKAN`, built-in Silero VAD `--vad`), [vad-speech-segments](https://gitea.mueller.network/extern/whisper.cpp/src/branch/master/examples/vad-speech-segments), the server's own `neezer/transcribe/` and `docker/transcribe/worker.py` | `whisper-rs = "0.16.0"`, feature `vulkan`; VAD through `FullParams::enable_vad` + `set_vad_model_path` (Silero GGML); same model ladder and segment rules as the server. |
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
    debug build under Xvfb/lavapipe: **2 782 ms** — that number says the window paints on a machine
    with no GPU, it is not a performance target (D7's 300 ms is a release number).
  - release binaries: `neezer-desktop` **25 545 128 B**, `neezer-transcribe` **39 844 208 B**.
  - fresh release build 3 min 39 s here; CI **8 min 30 s cold, 6 min 3 s warm** (cold: clippy 5 min
    10 s, release 8 min 28 s, test 5 min 35 s; warm: 1 min 3 s, 2 min 27 s, 5 min 59 s).
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

### D1 — `nz-model` and contract fixtures

- **Goal**: Rust types for every payload the app reads, proven against the REAL server's JSON.
- **Files**: `desktop/crates/nz-model/src/{lib,ids,track,album,artist,playlist,library,search,
  podcast,transcript,player,discovery,settings,remote,party,admin}.rs`,
  `desktop/crates/nz-model/tests/fixtures.rs`, `desktop/tools/dump_fixtures.py`, the desktop job
  step that runs it.
- **Contract**: `dump_fixtures.py` starts `python tools/perf_api.py --tracks 300 --playlists 5
  --favorites 120 --big-playlist 200 --podcasts --serve --port 0`, reads the `READY <port> <ids-json>`
  line, logs in as `bench` / `Bench1`, GETs every read route of `webapp/src/lib/api.js` (one per
  `api.*` read method, with the seeded ids) and writes each body to `$NZ_FIXTURES_DIR/<name>.json`.
  The Rust test reads every file of `NZ_FIXTURES_DIR` (default
  `desktop/crates/nz-model/tests/fixtures/`, a committed copy) and deserializes it with the type
  named in a table in `fixtures.rs`. CI regenerates the fixtures from the current server and runs
  the test on them: a server payload change that breaks the desktop fails the desktop job.
- **Types**: ids are `TrackId(String)` etc. (the `deezer_id` field, a number string or a UUID —
  never parse to an integer). Every struct `#[serde(default)]` on optional fields and ignores
  unknown fields (an older or newer server must still parse, rule 7). The track JSON is
  `neezer/webui/__init__.py#_db_track` / `_local_track` (`id`, `deezer_id`, `title`, `duration`,
  `artist{deezer_id,name}`, `artists[{deezer_id,name,role}]`, `display_artist`,
  `album{deezer_id,title,cover}`, `gain`, `unavailable`, `local`, `source`, `explicit`, …).
- **Acceptance**: ≥ 40 fixture files, all deserialize; a test feeds each fixture with an extra
  unknown field and with every optional field removed; `cargo test -p nz-model` < 2 s.
- **Pitfalls**: perf_api's data is random: compare TYPES, never values. Cover URLs are relative
  (`/api/cover/<id>`): keep them relative in the model.

### D2 — `nz-api`

- **Goal**: the only HTTP client: session, lanes, errors, every endpoint as a typed function.
- **Files**: `desktop/crates/nz-api/src/{lib,client,lanes,error,session,cache_trait,endpoints/*}.rs`,
  `tests/` against `perf_api.py --serve` (a `#[ignore]`-free integration test that starts it, like
  D1's script; CI has Python).
- **API**:
  ```rust
  pub enum Lane { User, Prefetch, Bulk }
  pub struct Api { /* base url, reqwest client with cookie jar, gates, cache: Arc<dyn ResponseCache> */ }
  pub struct Fresh<T> { pub value: T, pub from_cache: bool }
  pub enum ApiError { Offline, Unauthorized, RemoteEnded { restored: Option<serde_json::Value> },
      Provider { status: u16, code: String, message: String, retry_in: u32 },
      NotReady { retry_after: u32 }, Busy { retry_after: u32 },
      Http { status: u16, message: String, code: Option<String> }, Decode(String) }
  pub trait ResponseCache: Send + Sync {          // implemented by nz-cache (D3)
      fn get(&self, owner: &str, path: &str) -> Option<String>;
      fn put(&self, owner: &str, path: &str, body: &str);
      fn delete(&self, owner: &str, path: &str);
  }
  impl Api {
      pub async fn get<T: DeserializeOwned>(&self, lane: Lane, path: &str) -> Result<Fresh<T>, ApiError>;
      pub async fn send<B: Serialize, T: DeserializeOwned>(&self, lane: Lane, method: Method, path: &str, body: Option<&B>) -> Result<Option<T>, ApiError>;
      pub async fn stream(&self, lane: Lane, id: &TrackId, q: Quality, start: Option<f64>, from_byte: Option<u64>) -> Result<StreamResponse, ApiError>;
      pub async fn ready(&self, id: &TrackId, q: Quality) -> Result<StreamReady, ApiError>;
      pub fn set_bulk_cap(&self, bytes_per_s: Option<u64>);
      pub fn online(&self) -> watch::Receiver<bool>;
  }
  ```
  plus one function per SPA `api.*` method in `endpoints/` (same names in snake_case: `login`,
  `me`, `search`, `artist`, `album`, `playlist`, `report_listen`, `save_podcast_progress`, …).
- **Acceptance** (tests, counted not timed): with 8 Bulk requests queued and one User request
  issued, the User request is sent first (assert order on a local test server); with a Prefetch
  queued while a User request is in flight, the Prefetch's start is after the User's end + 250 ms;
  a background `503 Retry-After: 2` is retried exactly once after ≥ 2 s; a GET while offline
  returns the cache with 0 network attempts; a POST is never retried; `X-NS-Error` gives
  `Provider` after exactly 1 attempt; login + me + 401 round trip against perf_api.
- **Pitfalls**: never set `Origin`/`Sec-Fetch-Site`; `Accept-Encoding: identity` on streams (a
  gzip-encoded body has no usable byte offsets); the cookie jar is per server URL.

### D3 — `nz-cache`

- **Goal**: the SQLite file, its migrations, and the stores of *Caches*.
- **Files**: `desktop/crates/nz-cache/src/{lib,schema,api_cache,covers,play_cache,downloads,kv,
  queue}.rs`, `src/migrations/v1.sql`.
- **API**: `Cache::open(dir) -> Result<Cache>`; `impl nz_api::ResponseCache for Cache`; `covers()`,
  `play_cache()` (`reserve(track, q) -> PathBuf`, `complete(track, q, bytes)`, `lookup(track, q)`,
  `evict_to(cap_bytes)`), `downloads()` (`add_set`, `remove_set`, `pending(limit)`, `mark(track,
  state, error)`, `local(track) -> Option<(PathBuf, Quality)>`), `kv()` (typed get/set), `queue()`
  (save/load the player snapshot, ≤ 4 000 entries).
- **Acceptance**: migrations idempotent (open twice, run v1 twice); opening a v0 empty file and a
  v1 file both work; `evict_to` keeps the most recently used and never touches `downloads`; saving a
  4 000-entry queue < 30 ms and one statement batch (count it); concurrent read while a write runs
  (WAL) does not block.
- **Pitfalls**: SQLite access happens on `nz-net`/`nz-bulk`, never the main thread.

### D4 — `nz-player`

- **Goal**: the queue and transport as a pure state machine, with the web player's rules.
- **Files**: `desktop/crates/nz-player/src/{lib,state,command,effect,shuffle,listen}.rs`, tests.
- **API**: `PlayerState::apply(&mut self, cmd: Command, now_ms: u64) -> Vec<Effect>`. `Command`
  carries exactly the player-store methods `webapp/src/lib/remote/commands.js` names (`playQueue`,
  `playNext`, `addToQueue`, `next`, `prev`, `jump`, `remove`, `removeAt`, `move`, `clearUpcoming`,
  `toggleShuffle`, `cycleRepeat`, `toggle`, `play`, `pause`, `setVolume`, `toggleMute`, seek),
  serde-tagged with those names so remote control maps 1:1. `Effect`: `Load { entry, autoplay,
  start_s }`, `PreloadNext(Option<Entry>)`, `ReportListen { id, listened_s, next_id, context,
  shuffle }`, `NeedMore { context }` (radio/flow top-up), `Persist`, `Changed`.
- **Rules to port** (read `lib/stores.js` `player`, `Player.svelte#flushListen`, `lib/flow.js`):
  shuffle keeps the current track first and is reversible; repeat off/all/one; `prev` restarts the
  track after 3 s; listen time counts playing time only; radio/flow contexts ask for more before
  the queue runs dry.
- **Acceptance**: each rule a test; on a 4 000-entry queue `toggleShuffle`, `move`, `removeAt` each
  < 1 ms (criterion or a loop of 1 000 with `Instant`, numbers in the PR); a serde round trip of
  every `Command` matches the JSON `commands.js#commandFor` produces (fixture copied from the JS).

### D5 — `nz-audio` I: source, decode, output, gapless, seek

- **Goal**: a track plays through PipeWire from the network or a file, gaplessly into the next.
- **Files**: `desktop/crates/nz-audio/src/{lib,engine,source,decode,voice,mixer,output,clock,
  events}.rs`, tests with real audio made by ffmpeg in a build-time test helper.
- **API**: as in *Architecture*: `Engine::new(cfg, opener: Arc<dyn StreamOpener>)`, `load`,
  `preload_next`, `play`, `pause`, `seek`, `set_volume`, `set_mute`, `set_output_device`,
  `position()`, `events() -> Receiver<EngineEvent>`; `trait StreamOpener { fn open(&self, id, q,
  start: Option<f64>, from_byte: Option<u64>, lane) -> Result<Box<dyn ByteStream>>; fn ready(&self,
  id, q) -> Result<StreamReady>; fn local(&self, id, q) -> Option<PathBuf>; }` (the app implements
  it on `nz-api` + `nz-cache`; tests implement it on files and a local HTTP server).
- **Design**: the source writes the HTTP body to a play-cache file and exposes a blocking
  `Read + Seek` over the bytes received so far (symphonia's `MediaSource`, `is_seekable` only when
  the length is known); decode threads push f32 frames at the device rate into an `rtrb` ring of
  0.5 s per voice; the cpal callback (period 10 ms) pops from the current voice and, at its
  end-of-stream, from the next one in the same callback: no gap, no click. Seek follows *Streams*.
  The output reports underruns and the device's latency.
- **Acceptance**: gapless: two generated sine files back to back, the sample at the seam
  continuous (|Δ| below one sine step) and the output length = the sum of both ± 1 frame; click to
  first sample from a local file < 50 ms, from `perf_api --audio` over loopback < 250 ms (median of
  10, in the PR); 0 underruns over 10 min with the UI busy (stress test: a thread spinning on
  allocations); RSS of the engine < 40 MB while playing; seek in a finished file lands within
  20 ms of the target; seek in a generated stream uses `?start=` (assert the request).
- **Pitfalls**: never allocate or lock in the callback (`rtrb`, preallocated buffers); Opus is
  always 48 kHz but FLAC may be 44.1: resample with rubato in the decode thread, never in the
  callback; Ogg Opus pre-skip must be honoured for gapless.

### D6 — `nz-audio` II: DSP, crossfade, trim, speed, tap

- **Goal**: the web player's sound: normalisation, EQ, bass, mono/balance, crossfade + silence
  trim, podcast speed, and the analysis tap.
- **Files**: `desktop/crates/nz-audio/src/{dsp,crossfade,trim,stretch,tap}.rs` (+ the lines in
  `engine.rs` that call them).
- **Contract**: DSP is `nscore::dsp::Chain` (gain ramp, 10-band EQ, bass, fades — the Android
  chain, whose `Compressor` is the bass/normalisation stage); track gain from `/api/gain/<id>` /
  `POST /api/gains` applied as `lib/gaincache.js` does, and `normalization` = `off|low|medium|high`
  (compressor + make-up gain, `stores.js` `fx.normalize`), the first track of a session normalised
  like the next ([playback.md](playback.md));
  crossfade (`fade.enabled`, `fade.seconds` 6, `fade.onSkip`, `fade.skipSeconds` 1.5) and trim
  (`fade.trim`, `fade.trimDb` −45, bounds from `/api/audio/edges/<id>?db=`) follow
  `Player.svelte#maybeCrossfade`/`maybeTrimEnding` exactly ([playback.md](playback.md)); speed
  0.5–3× through `signalsmith-stretch`, pitch kept, podcasts only; the tap copies post-DSP stereo
  frames into an `rtrb` ring the analysis thread reads (drop when full, never block).
- **Acceptance**: equal-power crossfade measured on two generated tones (sum of powers within
  ±0.5 dB across the fade); trim skips a generated 3 s silence at the end; a skip during a
  crossfade wins at once (`playback.md` *A transport action always wins*); EQ magnitude at the band
  centres equals `Biquad::magnitude` ±0.1 dB; output callback time p99 < 25 % of the period with
  every effect on (measured, in the PR).

### M1 — `nz-mpris`

- **Files**: `desktop/crates/nz-mpris/src/lib.rs`, an example `examples/fake_player.rs`.
- **API**: `Mpris::start(name: &str, tx: Sender<MprisCommand>) -> MprisHandle`;
  `MprisHandle::{set_track(Metadata), set_status(Playing|Paused|Stopped), set_position_s(f64)
  (emits Seeked only on discontinuity), set_volume, set_shuffle, set_loop}`; commands `PlayPause`,
  `Play`, `Pause`, `Next`, `Previous`, `Seek(i64 µs)`, `SetPosition(id, µs)`, `Volume(f64)`,
  `Shuffle(bool)`, `Loop(..)`, `Raise`, `Quit`. Bus name `org.mpris.MediaPlayer2.neezer`;
  `mpris:artUrl` = `file://` path of the cached cover; `mpris:trackid` an object path derived from
  the track id.
- **Acceptance**: `playerctl -p neezer metadata` shows title/artist/album; `playerctl -p neezer
  play-pause` reaches the sender (test with `dbus-run-session` in CI); no property signal per
  position tick (count D-Bus messages over 10 s of playback: ≤ 2).
- **Pitfalls**: run the server on its own `LocalSet` thread (see *Pitfalls*).

### V1 — `nz-viz` (bars and oscilloscope, pure)

- **Files**: `desktop/crates/nz-viz/src/{lib,bars,scope,palette}.rs`.
- **Contract**: `VizState::new(sr)`, `push_frames(&[f32] stereo)`, `bars(n, dt) -> &[f32]`
  (levels then peaks, from `rhythm::viz_bars::Bars` fed by the analyser's bands, the same input the
  web canvas fallback uses), `scope(cols, dt) -> (&[f32], &[f32])` (per channel `[lo, hi]` per
  column, `rhythm::viz_scope::Scope`), palette from the cover (port `lib/coverpalette.js`) or the
  Omarchy theme.
- **Acceptance**: frame-for-frame equality with the web build on a fixture recording (reuse the
  comparison style of `android/nscore/test/compare.mjs`); `bars(64)` + `scope(512)` < 0.3 ms per
  frame on this machine.

### D7 — App shell and wiring

- **Goal**: the window, navigation, player bar, login, and every subsystem wired, with stubs for
  every screen so later lots never edit shared files.
- **Files**: `desktop/app/src/{main,app,state,theme,keymap,actions,nav,toast,load_failed,
  net_indicator,player_bar,login,wiring/*}.rs`, `desktop/app/src/screens/mod.rs` registering EVERY
  route of the inventory, and one stub file per screen (`screens/{home,search,artist,album,playlist,
  library,podcasts,show,releases,recap,history,genres,credits,import,party,remote,now_playing,
  queue,viz,onboarding,settings/{mod,playback,appearance,viz,downloads,discovery,sources,account,
  users,server,transcribe,help,shortcuts}}.rs`) that renders its title and "Bientôt". D7 also
  creates, as empty stubs declared in their `mod.rs`, EVERY file a later lot lists under
  `app/src/widgets/` and `app/src/wiring/`: after D7 no lot edits a `mod.rs`, `keymap.rs`,
  `actions.rs` or `screens/mod.rs`, which is what lets wave 5 run in parallel.
- **Contract**: `AppState` (GPUI `Entity`) owns `Api`, `Cache`, `PlayerState`, `Engine` handle,
  `Mpris`; screens get `&AppState` and emit `Command`s; `nav` is a stack with back/forward
  history and scroll positions kept per entry; the keymap of *Keyboard AND mouse* is complete here
  (actions a later lot implements dispatch to its screen); single instance through the D-Bus name
  `fr.neezer.Desktop` (a second launch raises the window and exits).
- **Acceptance**: cold start to first painted frame < 300 ms (release, `--self-test` prints it);
  login → home → play a track → next, with keyboard only and with mouse only (a scripted run of
  GPUI's test harness for the keyboard path; the mouse path looked at and noted); toasts and
  `LoadFailed` with the server stopped; RSS idle < 120 MB; the window follows an Omarchy theme
  change live (`omarchy-theme-set`).

### D8a — Browse: home, search, artist, album, credits

- **Files**: `screens/{home,search,artist,album,credits}.rs` and new files under
  `app/src/widgets/` it creates (`track_list.rs` — the shared windowed list — `cover.rs`,
  `context_menu.rs`, `card.rs`). D8b uses the widgets once merged; until then it may only read them.
- **Acceptance**: search answers at each keystroke from cache then network (debounce 150 ms), never
  more than 1 request in flight per field; the artist page paints from cache in one frame when
  cached; requests per screen equal to the SPA's (`webapp/test/requests/run.mjs` counts) or fewer;
  scrolling a 4 000-row list at ≥ 230 fps on this machine (frame times logged with
  `NZ_FRAME_LOG=1`, p95 < 4.2 ms); context menu by `Menu`/`Shift+F10` and right click.

### D8b — Library and playlists

- **Files**: `screens/{library,playlist,import}.rs`, `app/src/widgets/{folder_tree,playlist_picker,
  sheets}.rs`.
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

### D13 — Offline

- **Files**: `app/src/wiring/offline.rs`, `screens/settings/downloads.rs`, `app/src/widgets/
  download_badge.rs`.
- **Acceptance**: download a 200-track playlist through `Lane::Bulk` while a network track plays:
  0 underruns and the playing stream's throughput drops by < 30 % (measure both); progress, pause,
  resume after restart, and each failure's cause listed; with the network cut, every downloaded
  track plays and every cached screen opens; play-cache cap respected.

### D14a — Réglages (user sections)

- **Files**: `screens/settings/{mod,playback,appearance,discovery,sources,account,help,shortcuts}.rs`.
- **Acceptance**: every option the SPA's Réglages offers to a non-admin is here, saved to the
  server where the SPA saves it (`/settings`, `/sources`, `/me/accounts*`, `/discover/prefs`,
  `/releases/settings`) and to `kv` where the SPA keeps it locally; output device chooser lists
  PipeWire sinks and switches without stopping the music.

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

### T1 — `nz-transcribe` core (segments, sidecar)

- **Files**: `desktop/crates/nz-transcribe/Cargo.toml` (all deps for T2/T3 declared now),
  `src/{lib,segments,sidecar,error}.rs`, empty `src/{engine,audio,worker,protocol,supervisor,
  control,models}.rs` with `todo!()`-free stubs, `tests/golden.rs`,
  `desktop/tools/transcribe_golden.py` (calls `neezer.transcribe.engine.normalize_segments`,
  `clean_text`, `sidecar.build/dumps/srt_bytes` on a set of raw whisper outputs and writes the
  expected files).
- **Acceptance**: the Rust output byte-equal to the Python golden files (≥ 12 cases: overlap
  cutoff, clamp, tag-only, control chars, 2 001 segments, empty, unicode); CI regenerates the golden
  files from the current Python and re-runs the test.

### T2 — Engine (whisper Vulkan, models, VAD, audio, fallback)

- **Files**: `src/{engine,audio,models}.rs`, the `engine` and `file` subcommands in `src/main.rs`.
- **Contract**: `neezer-transcribe file <audio> [--lang auto] [--model <path>] [--cpu] --out
  <x.json>`; `neezer-transcribe engine --pcm <f32le 16k mono file> --model … --lang … --out …`
  (the child); `neezer-transcribe probe` prints the `engine` event JSON. Fallback causes as in
  *Transcription*.
- **Acceptance**: `jfk.wav` transcribed identically with `--cpu` and Vulkan; speed on this machine
  with `ggml-base` ≥ 40× real time Vulkan and ≥ 12× CPU (the prototype's numbers); with
  `VK_ICD_FILENAMES=/nonexistent` the probe reports `no_vulkan_device` and the file still
  transcribes on the CPU; CI runs the Vulkan path on lavapipe (`mesa-vulkan-drivers`) with
  `ggml-tiny` (cached `actions/cache`, SHA-256 checked); an MP3 and an M4A episode decode to the
  right duration ± 50 ms.
- **Pitfalls**: Vulkan headers (see above); the first GPU call compiles pipelines (seconds): warm
  once per process, not per chunk; VAD model path must be set before `enable_vad(true)` or the crate
  panics.

### T3 — Worker protocol and supervisor

- **Files**: `src/{worker,protocol,supervisor,control}.rs`, the `worker` and `local` subcommands.
- **Contract**: *Transcription* (protocol, JSON lines, SIGSTOP of the engine group, resume of a
  local job at its last chunk, backoff like `worker.py#Client._backoff`, no token in any log).
- **Acceptance**: against `perf_api.py --podcasts` (runner external, a client worker enrolled by the
  test as `bench`): claim → audio → chunks → complete, the server's transcript row `ready` with the
  segments; `cancel` from the server stops within one heartbeat; a `SIGSTOP`ped engine keeps its
  lease (heartbeats continue, assert ≥ 3 during a 15 s pause); 429/5xx back off without a storm
  (count requests).

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
