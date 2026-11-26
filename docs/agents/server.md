# Server architecture: Deezer, archive, resilience, background work

Reference notes for agents working on this area. Read [CLAUDE.md](../../CLAUDE.md) first: its process and rules apply here. Other areas: [docs/agents/](./).


**The frontier (detachment, lots A-I).** Deezer is a supplier of SOUND and of catalogue FACTS, nothing
else. An archived track is a file like an upload: identity (UUID of the row), cover, tempo, genre,
gain, lyrics, neighbours, radio and home are all this server's own, from the database and the disk.
Nothing the user does writes to the Deezer account (no star, playlist, listen or position is
mirrored; there is no option for it), and nothing reads the account except the ONE-OFF import in
Réglages → Compte Deezer (`importer.import_account`: playlists and favourites, added, never removed).
`tests/test_detach_guard.py` enforces it: a spy client, every `/api` route visited, and an `ALLOWED`
list of the calls that fetch sound or catalogue facts — adding a Deezer call means adding it there.
What stays on Deezer on purpose is listed in `docs/detachement-deezer.md` (D18).

Three code layers, from low to high:

1. **`deezerpy/`** — the vendored Deezer client (based on RemixDev's deezer-py). Two transports
   with deliberate reliability roles (learned the hard way; do not "simplify" back to one):
   - `api.py` — **public `api.deezer.com`**. Used for **search**, the **catalogue charts**
     (Explore) and **artist pages**. Stable, typed, returns image URLs.
   - `gw.py` — **private gateway `gw-light.php`**. Used for album/playlist/show pages, the track
     records, track and artist radio seeds, the one-off account import and stream tokens.
   - **Gotcha — full tracklists:** album/playlist *page* calls only return the first ~10–40 tracks.
     Always fetch the full list via `get_album_tracks` (`song.getListByAlbum` nb:-1) /
     `get_playlist_tracks` (`playlist.getSongs` nb:-1).

2. **`neezer/deezer/`** — the proxy logic:
   - `provider.py` — ARL login + stream decryption; the single object the rest of the app talks to.
   - `archive.py` — `ensure_archived` (first-play fetch → Blowfish-CBC decrypt → tag → store under
     `archive_dir`) and import helpers.
   - `library.py` — DB upsert + deterministic archive paths. `ids.py` — deterministic `uuid5` IDs
     from Deezer IDs (plus nullable `deezer_id` columns on Track/Album/Artist/Playlist).
   - `importer.py` — the one-off account import (`import_account`) and the scheduled refresh of
     subscribed podcasts (`sync`); writes straight to the DB and never to Deezer. The playlists the
     old nightly sync wrote ("Deezer · Flow", smart tracklists, "Nouveautés en vrac") are ordinary
     playlists now: kept, never refreshed or purged.
   - `lyrics.py` — archive synced lyrics as a `.lrc` sidecar next to each archived audio file
     (+ embedded plain text in the tags), sourced from the public **LRCLIB** API
     (https://lrclib.net); synced wins over plain. `ensure_lyrics` is called on
     archive and on first `/api/lyrics` view; `backfill_archived_lyrics` (CLI `deezer lyrics`)
     backfills every already-archived track that lacks a sidecar.
   - `prefetch.py` — background preload of upcoming tracks + the `/api/download` pre-archive worker.
   - `scheduler.py` — auto-sync: full sync on startup (after ~20s) then daily at `sync_at` (04:00)
     or every `sync_interval`, whenever a `sync_user` exists.

**Never save a whole row that was read before a long request.** Peewee's `save()` writes
every column the instance loaded, so a `/rest/stream` that holds `request.user` for the
duration of a first FLAC fetch would write back a stale `disabled`/`session_epoch`/
`password`, and a `Track` would write back a stale `play_count`/`gain`/`unavailable`/
`last_modification`. Use a targeted `UPDATE` (`Track.update(play_count=Track.play_count + 1,
…)`) or `save(only=(…))`; the same rule covers `TranscribeWorker` and `PodcastChannel`. A
revoked worker is checked in the `UPDATE` itself, not only on the instance read earlier.

**Foreground first on the provider link.** The shared Deezer window (40 calls / 5 s) has lanes:
`USER` (a tapped page or play), `PREFETCH`, `BULK`; a freed slot goes to the most urgent, and
`workload.renice()` now also declares the thread's Deezer calls `BULK`. A slow provider call never
runs in a request thread when it can be moved: Subsonic `refreshPodcasts` and
`downloadPodcastEpisode` answer immediately and use a single reniced worker (the download queue is
bounded and deduplicated by episode; the first play keeps its synchronous download),
`GET /me/accounts/<id>/library/<kind>` serves the held listing and refreshes behind it (one
refresh per account/kind/5 min, retried 60 s after a failure), and `/api/android/apk` (the
WebView app) and `/api/android/native-apk` (the native app, `?abi=`) answer **202 + Retry-After**
while one worker fetches the APK — two channels, two cache families, never the same file. The archive backfills and nightly sync call
`renice`/`Throttle` between items, so a person's play always wins. A 4xx from `media.deezer.com` is
a transport failure (`DeezerError`), never a verdict that the title is unavailable.

**Resilience rules (learned from a production outage — do not regress).** The governing rule:
**Deezer is optional to the app running.** Everything on disk — playing archived music, browsing,
playlists, favourites, uploads, the SPA itself — must keep working at full speed while Deezer is
unreachable, and the parts that do need Deezer must fail *fast* and *without a verdict*.

- Every upsert in `library.py` is a check-then-insert, which is **not atomic**. Concurrent plays of
  the same album race and the loser gets a unique-constraint violation (and, on Postgres, a poisoned
  transaction). All of them go through `library.create_or_get` (insert inside its own
  `db.atomic()` → savepoint, then read the winner's row back). Keep any new upsert on that path.
- `deezerpy` requests carry a **default timeout** (`deezerpy.DEFAULT_TIMEOUT`, applied by `_Session`)
  — `requests` has none, and one black-holed socket parks a server thread until the gunicorn worker
  is killed. Never build a bare `requests.Session()` for Deezer, and never pass a *scalar* timeout
  (that applies the whole budget to the connect phase alone).
- **Latency, not errors, is what an outage takes the app down with.** Every call blocking for the
  timeout parks a thread; enough of them and the worker runs out of threads and of file descriptors
  (`Errno 24`), and the app is down for everyone — including the users who never needed Deezer. So:
  - `deezerpy/_circuit.py` holds a **per-host circuit breaker** on the shared `_Session`. After a
    few *transport* failures a host is short-circuited: further calls raise `DeezerUnavailable`
    instantly instead of paying the timeout, until a cooldown elapses and one probe is let through.
    Per host on purpose (a rate-limited `api.deezer.com` must not stop playback from the CDN); only
    transport failures count (an HTTP error or a gateway error payload is Deezer *answering*).
  - `api_call` in `api.py`/`gw.py` retries against a **wall-clock budget** (`NET_BUDGET`), not just a
    count, and never retries a short-circuited call. Their non-network replays (CSRF refresh,
    gateway `FALLBACK`) are bounded too — they used to recurse without limit.
  - `DeezerProvider.available()` is the instant, no-network "is it worth calling Deezer?" read.
    `_dz_api()` / `_dz_live()` in the webui are the route-level version: they return `None` instead
    of raising, so a route drops to its database answer at once. **Reading `provider.dz` logs in**,
    so touching it outside a `try` is how an outage became a 500 — go through those helpers.
  - A failed login is not retried on every request (`_login_retry_at` backoff), and a re-login
    `close()`s the old client so its connection pool goes back to the OS.
- **A failure to reach Deezer is never a verdict about the data.** `TrackUnavailable` and
  `ShowUnavailable` condemn a track / retire a subscription, and the user is then offered to replace
  or delete it — so they may only be raised when Deezer *answered*. `errors.is_transport_failure()`
  (which walks the `__cause__` chain) is the test; `provider.resolve` downgrades any unconfirmed
  verdict to a plain `DeezerError`, and `_url_from_info` raises rather than reporting "no source"
  when it could not ask.
- Background work backs off instead of burning through itself: the download queue waits out the
  breaker's cooldown (`prefetch._outage_wait`, and *only* while a circuit is actually open) rather
  than failing every queued id in seconds, and the scheduler postpones a sync while Deezer is
  unreachable (the local scan still runs).
- **Podcast episode URLs come from third-party feeds**, so `check_public_url` refuses anything but
  http(s) to a public address — and then the fetch **connects to the address it just validated**
  (`_PinnedAddressAdapter`, via requests' `build_connection_pool_key_attributes`). Checking the
  *name* alone can never be enough: the guard resolves it and the HTTP client resolves it again, so
  a resolver that answers differently the second time (DNS rebinding) walks straight past a
  name-based check and its answer gets archived and served by `/api/stream`. The pin moves the TCP
  connection only — SNI, certificate validation and the `Host` header still carry the real hostname
  — and it walks the host's addresses itself, since pinning gives up the fail-over
  `create_connection` does for free. It steps aside when a proxy is configured (the proxy does the
  resolving, and it is the operator's boundary).
- `socket.getaddrinfo` takes no timeout, and glibc's defaults make one stalled lookup 20s+.
  `requests` folds DNS into its connect timeout, so the session is covered — but the podcast SSRF
  pre-flight (`check_public_url`) resolves *before* requests is involved, once per redirect hop, on
  a host a third-party feed chose. It goes through `provider.resolve_addresses`: a deadline, a
  short-lived cache (positive *and* negative, so the next caller never re-pays a timeout), and a
  bounded set of **daemon** lookup threads — a `ThreadPoolExecutor` would be joined at interpreter
  exit and one wedged name would hang the shutdown of the worker being recycled because of it.
- The `/api` blueprint has a catch-all error handler: an unforeseen failure becomes a JSON 500 with
  the traceback in the log, never an HTML page the SPA can't parse.
- **Reaching Deezer again is the SERVER's job, never the client's.** A page only Deezer has, asked
  while Deezer does not answer, is a 503 marked `X-NS-Error: deezer-unreachable` (`_deezer_missing` /
  `_deezer_unreachable`, with `retry_in`); the proxy switched off is `deezer-disabled`. `lib/api.js`
  reads the header as final and never retries it (an unmarked 503 — a restart — still is): the
  server's calls already retry inside their budget and the breaker probes once its cooldown is over,
  so three client retries only fetched the same instant refusal ~3 s later. Deezer ANSWERING that a
  thing is not there stays a 404. The screens say "Deezer est injoignable" (`LoadFailed.svelte`),
  never "introuvable", and only the person's "Réessayer" asks again.
- The ARL can die at any moment. `DeezerProvider.check_login()` is the cached health check and
  distinguishes `"arl"` (credential dead — admin action) from `"network"` (says nothing about it);
  `/api/deezer/status` surfaces it, with `retry_in`, and the SPA raises a notice
  (`lib/deezerhealth.js`) — an actionable one for the credential, an explanatory one ("your
  downloaded library is still there") for an outage, re-checked when the server says it will retry.

3. **`neezer/webui/`** — the custom `/api` blueprint (`__init__.py`, all routes `@login_required`,
   numeric-id validation on stream/favorite), `share.py` (waveform peaks + full-file/ffmpeg-clip
   downloads for the SPA's share sheet, all cached), `edges.py` (where a file's audio starts and
   stops, for the crossfade), `analysis.py` (serving the whole-track verdict) and `spa.py`, which serves the built Svelte SPA at
   **`/app/`** (hash-routed). Admin UI stays at `/`, Subsonic at `/rest`.

**A request handler never echoes an exception; log injection is logsafe's job, not each call's.**
CodeQL's `py/log-injection` is already neutralised at runtime: every `logger` record in this package
has its line breaks escaped by `neezer/logsafe.py` (a `LogRecordFactory` installed by `web.py` and
the daemon, pinned by `tests/test_security.py::LogInjectionTestCase`). A client-chosen id/name may
therefore reach `logger` as it is; the seven alerts of the studio/analysis lot were dismissed as
false positives, because the query cannot see a package-wide record factory. A `print` or a
hand-written log file is NOT covered and must sanitize. `py/stack-trace-exposure` has no such guard:
answer a caught exception with this server's own constant — or a whitelist of them, as
`genreshare._parse_error` and `genre._extractor_refusal` do — never with `str(exc)`; the detail
belongs in `exc_info=True`, never in `jsonify`. 15 alerts in these routes were closed that way.

The factory escapes every control character, not just the line breaks: CR/LF (and U+2028/2029) keep
their readable escape, while every other C0, DEL and C1 becomes `\xNN` — ESC included, because an
ANSI sequence clears the screen or rewrites the line without needing a newline, and NUL truncates in
several readers (`tests/test_logsafe.py`). It cleans `record.args` only, so **log through `%s`
arguments, never an f-string**: `logger.info(f"user {name}")` bypasses it.

**`# nosemgrep` closes nothing here.** Semgrep does honour it, but it still writes the result into
its SARIF with `suppressions: [{kind: inSource}]`, and GitHub code scanning ignores that: the alert
is created and the "Semgrep OSS" check stays red. `# nosec Bxxx` is different — Bandit drops the
finding before emitting SARIF, so the Bandit alert does close. So a Semgrep finding is answered by
**restructuring until the rule no longer matches**: name the parse in a helper the tainted value only
reaches as a parameter (`_offset_seconds`, `_seconds`), build generated HTML with a
`string.Template` instead of an f-string/`%`/`+`, or let the value carry what the rule needs. If the
line cannot be restructured, leave it intact and document it — never disable a rule.

**Streaming interception** lives in `neezer/api/media.py` (`_ensure_deezer_archived`): first play
archives the FLAC, then the existing transcode/cache/`send_file` path runs unchanged. The permanent
FLAC archive is separate from neezer's capped `transcode_cache`.

**Seeking a track, and what tells a player it can** (`/api/stream`, `/rest/stream`,
`/api/stream/<id>/ready`; `tests/test_stream_seek.py`). A player can only move inside what the
server can address, so every answer says which of the three things it is, and the player picks:

- **A finished FILE** — an archived FLAC, a cached Opus transcode, an upload, an archived episode —
  answers `Accept-Ranges: bytes`, `Content-Length`, `ETag`/`Last-Modified`, a byte `Range` with
  `206` + `Content-Range`; `If-Range` is honoured (a stale validator gets the whole body, never a
  stitched one) and a range past the end is `416` + `Content-Range: bytes */<size>`, which is how a
  player learns the real length (the `/api` error handler keeps that header). `send_file(...,
  conditional=True)` already did all of it; what was missing was anything that checked it end to
  end. Measured on a real ffmpeg FLAC: a seek to 80 % of a 4.6 MB file costs **930 836 B and 2.9 ms
  to the first byte** instead of 4 630 836 B, and Chromium lands at **240.24 s** of a 300-s track.
- **A stream GENERATED as it is sent** — a live archive, a cold transcode — declares
  `Accept-Ranges: none`, `Cache-Control: no-store` and no length. Chromium reads that as
  `duration: null`, and **a seek outside the buffered part restarts at 0** (measured: asked 200 s,
  the element went to 0.08 s). That is the "the bar does nothing / the track starts again" pair:
  the two answers used to be indistinguishable, so the browser guessed.
- **`GET /api/stream/<id>/ready[?q=OPUS_N]`** says what the next answer would be, and by contract
  starts nothing (no download, no transcode): `ready` (a file, therefore ranges), `ranges`
  (`"bytes"` or null), `bytes` (the archive's size), `cached` (the Opus transcode is in the cache —
  the key is `deezer-…`, `youtube-…` or `episode-…` depending on the row, they disagree at your
  peril), `offset` (`?start=` is honoured right now) and `streaming` (nothing is published yet and
  an archive is being written: a `.part` sibling of the final name, which is how every writer here
  publishes). **2 SELECT**, one of them the session's user, and a **93 B** payload. `ready` is the
  published file, never a `.part`, and the directory is only listed when nothing is published — so
  a sibling `.part` (a second download, or one a killed process left) can never hide a finished
  archive, and one stale file cannot block every seek of that track for ever. A file written in
  place by an outside process (a copy landing in a scanned library) cannot be told apart;
  archives can, and they are what the app fetches. That `streaming` now ends in **download** time,
  not in listening time: the fetch has its own thread (*Archive completeness*, below), so a player
  that stops reading while it chases a seek does not hold the archive back.

**A seek into a source that is on disk but that is not served as a file** (the Opus transcode of an
archived track, or a FLAC one wants from a moment) is a **`start=<seconds>`** parameter on
`/api/stream`, and the OpenSubsonic **`timeOffset`** (the `transcodeOffset` extension) on
`/rest/stream`. ffmpeg gets `-ss` **before** `-i`, the input seek: it starts on the sample asked —
a marked FLAC sliced at 119 s keeps exactly 1.000 s of silence, at 121 s none — and costs
**39-40 ms** to the first byte where encoding from 0 costs 43-71 ms. A FLAC is re-encoded
(lossless: a stream copy keeps the old STREAMINFO and would claim the whole track's length), MP3
and Ogg are copied, and an MP4 is not cut at all (its muxer needs a seekable output and writes
nothing to a pipe) — that answer is 409, because the file itself serves the byte range that seeks
it. The slice is **never** written to `transcode_cache` (a slice is not the track) and bumps no
play count; `X-NS-Start` carries the offset actually applied (clamped to `duration - 0.25`),
because the client's timeline for that answer starts at 0. A source that is **not on disk answers
409 + `Retry-After`**: the live stream must not be restarted from 0, so the client keeps the sound
it has and reloads when `/ready` turns `ready` or `offset`.

Two more rules hold the whole thing up. A `Range` on a generated stream is answered `200` from the
start, never a fake `206`: `bytes=0-` is every browser's first request and must not break, and a
truncated `206` would make the player believe it landed. And **a failed transcode is never
published**: `cache.set_generated` keeps what a generator completed, so
`_opus_generator`/`_opus_live_generator` raise when ffmpeg exited non-zero **or** the live source
was not consumed to the end. A network hiccup used to cache the first half of a long track as the
whole track — a 15 kB "song", served for ever with a length and byte ranges, which no seek could
get past.

**libopus caps its bitrate PER CHANNEL: 256 kbit/s.** `OPUS_320` is therefore a valid request for a
stereo source and a refused one for a mono source: on the image's ffmpeg 9.0.2 `-b:a 320k` on a mono
input exits 234 with *"The bit rate 320000 bps is unsupported. Please choose a value between 500 and
256000."*, where the same request on stereo is accepted (512k is the stereo ceiling). It used to be
passed straight through, so `q=OPUS_320` on a mono track answered an EMPTY body and cached nothing —
and on a cold track it also killed the archive: ffmpeg never starts, the feeder's first write fails,
`source.close()` drops the partial. Every Opus encode goes through `webui._opus_args` (`_opus_kbps`:
`min(requested, 256 x channels)`), never a blanket 256 — a stereo track keeps its 320 (measured: 301
kbit/s in the cached file, against 241 kbit/s for a mono `OPUS_320`, which is exactly the 256k
encode). The channel count is read without decoding and without ffprobe: `mediafile` (mutagen,
header only, 0.2 ms warm) for a finished file, and on the LIVE path — no file yet — the first
block's FLAC STREAMINFO (`_flac_channels`, 21 bytes in); a first block that is not a FLAC (Deezer's
MP3 fallback) forces `-ac 2` rather than guessing, since a stereo output makes every rate we offer
valid and is a no-op on stereo audio. The cache key stays the REQUESTED quality (`…-opus320.ogg` can
hold 256k for a mono track): `/ready` must keep answering `cached` for the `q` the player asked.
The **Subsonic** path cannot clamp per channel — a command template has one `%outratek` and no
channel count — so `docker/default.conf` and both sample configs pin `-ac 2` on the Opus
transcoders: every rate up to 512k is then valid and the file really carries what `%outratek` says
(measured 316.9 kbit/s for a mono source at 320k), which also keeps `estimateContentLength` honest.
A config file written before that line keeps the bug until the operator adds `-ac 2` (the image's
own `/etc/neezer/default.conf` is replaced on upgrade). The **ZIP export** (`webui/export.py`) follows
the same rule: its Opus formats build their arguments per track (`_opus_format` → `_opus_args` +
`_audio_channels`), so "Opus 320" of a mono title is no longer a failure listed in `_erreurs.txt`.

**Decrypting a stream** (`provider.decrypt_blocks` / `iter_decrypted`). Deezer encrypts the 2048-byte
blocks at FILE offsets 0, 6144, 12288… — every third one, each its own CBC chain (IV `00..07`), never
a short last block. The body is re-cut into those blocks by byte count, whatever pieces the network
delivers it in: a chunked response arrives in HTTP chunks of any size (`iter_content(2048)` caps a
piece, it never fills one), and counting pieces — which the code did until a test ran it over a real
connection — decrypted every chunked response into noise. One ECB cipher per stream with the chain
applied by hand, not a CBC cipher per block: re-running Blowfish's key schedule 6800 times was 70% of
the cost (0.44 → 0.16 s of CPU per 40 MB FLAC, same bytes). **Nothing that does not open like audio
is archived**: the first decrypted bytes must be `fLaC`, `ID3` or an MPEG frame sync
(`looks_like_audio`), an empty body is an error, and `download_to` removes its `.part` whatever
stopped it. An archive is forever — a file on disk is what "archived" means — so noise kept there
would never be fetched, or play, again. `/api/stream` reads the first block before answering, so
that refusal is its 502 rather than a response dying half-sent. Podcast episodes get the same
guarantee the other way round: their hosts serve every kind of audio, so `iter_episode` refuses what
is certainly NOT audio — markup or JSON (an error page or a consent wall answered with a 200), or an
empty body (`looks_like_a_page`) — instead of listing formats it would one day be wrong about, and
`download_episode_to` drops its partial too.

**Podcasts** are Deezer *shows*/*episodes*, kept in dedicated `PodcastChannel`/`PodcastEpisode` tables
(not Track rows — episodes have no artist/album and map onto Subsonic's podcast types). The gw methods
(`deezer.pageShow`, `show.add/deleteFavorite`, `episode.bookmarkSet`) and the public `/podcast` shape were
confirmed from a HAR capture (2026-07-03): episodes are **plain MP3 straight from the podcast host** (no FLAC,
no Blowfish — `provider.download_episode_to` just follows redirects), archived under
`archive_dir/Podcasts/<Show>/` on first play. **That archive is not the user's library**:
`local.scan_local` never descends into `archive_dir/Podcasts/` and skips any file a
`PodcastEpisode.path` claims (even one moved elsewhere), so an archived episode never becomes a
second, listing `Track` row; `/api/me/local` and the offline local search leave such rows out too.
A `Track` an older server's scan made from an episode stays in the database — nothing deletes
archived audio — it is simply not listed, and the episode keeps its own route.
Subsonic endpoints live in `neezer/api/podcast.py`
(`getPodcasts`, `getNewestPodcasts`, `createPodcastChannel`, `refreshPodcasts`, `delete*`,
`downloadPodcastEpisode`); `media.py` resolves a stream/download id to a Track **or** a `PodcastEpisode`.
The local channel rows are the source of truth for subscriptions; `importer.sync_podcasts` refreshes them.
Per-user playback state is server-side: `PodcastProgress` (auto-saved position, `finished` flag) and
`PodcastMarker` (manual bookmarks) via `/api/podcast/progress` + marker CRUD; the SPA merges them with
its localStorage copy newest-wins (`webapp/src/lib/podcastProgress.js`); nothing is sent to Deezer.

**Podcast chapters** (`neezer/deezer/chapters.py`, `webui/podcast_chapters.py`, tables
`podcast_chapter*` schema `20261116`): Deezer supplies neither chapters nor a transcript, so the
source precedence is sidecar `<episode>.chapters.json` → MP4 `chpl` → ID3 `CHAP`/`CTOC` → the
publisher URL embedded as a tag (only with `chapter_urls` on, **off by default**) → none. Extraction
is mutagen-only (no ffprobe), **0.25 ms** for a 57.6 MB / 1 h file, at archive finalize, lazily on
the route for an older archive, by a one-shot resumable boot backfill and by
`neezer-cli podcast chapters`. Episode rows store milliseconds, the API answers seconds;
`GET /api/podcast/episode/<eid>/chapters` is 200 even for an unarchived episode (`archived:false`)
or one with no chapters (`scanned:true`, `source:"none"`, never an invented chapter), 404 for an
unknown id, and **2 SELECT warm**. The show payload gains `chapter_count` in one grouped query
(+18.5 B/episode raw, +0.19 B gzip). The publisher URL is fetched only on a BULK worker, validated
on every redirect hop (public address only, pinned, 256 KiB cap, JSON only, no cookies) and a
failure is recorded without failing the episode.

**Podcast transcripts** (`neezer/transcribe/`, tables `podcast_transcript*` schema `20261117`, columns and lease tables schema `20261121`; routes in `webui/transcribe.py`). The text lives on the **data disk**, next to the episode: `<episode>.transcript.json` is canonical (format version, language, engine/model, duration, segments `{start_ms,end_ms,text}`, timestamps) and `<episode>.transcript.srt` is regenerated from it. Both are written temp-file + `os.replace`, only from a runner thread (never a request), and both paths must resolve under `archive_dir/Podcasts/`; audio is never deleted. The DB is the **index**: `podcast_transcript`/`_segment`, plus `podcast_channel.auto_transcribe`/`transcribe_language` and `podcast_transcript.source`/`worker_name`/`transcript_path`. A missing sidecar does not destroy a row and a missing row does not destroy a sidecar; `neezer-cli transcribe rebuild` recreates rows+segments from the sidecars (measured **5 statements per episode**, idempotent).

The engine is whisper.cpp only, CPU (`whisper-cli -ng -oj`), on 600 s chunks with a 2 s overlap; segments become absolute milliseconds, the slice inside the previous overlap is dropped, empty/bracket-only text is dropped and text is capped at 2 000 chars. Language is resolved episode > show > global (`auto` detects once over 120 s, then pins); a low-confidence detection falls back to `-l auto` for the first chunk and pins `result.language`. The model is chosen from the configured model's directory by **free memory** (`MemAvailable`): `large-v3-turbo-q8_0` at ≥ 2.5 GB, otherwise `q5_0`, then `small`, else the configured file; the binary and the model are validated (executable, GGML magic, ≥ 1 MiB) with paths that only ever come from the configuration file. Nothing is downloaded at runtime and `engine.py`/`sidecar.py` import neither the database nor Flask (pinned by a test).

The queue is `queued → leased | running → ready`, with `stopped` and `error` reprenable at `done_ms`. **No cap** on active rows: a row costs a few statements, the runner still takes one job at a time (manual before automatic), so a big queue adds no CPU load; `queue_backlog("all")` writes `FILL_BATCH` = **100** rows per call and says `more`, and `resume_auto` on the runner thread (after each job and on the external scheduler's pass) writes the next batches under `quiet_wait()`, so a 10 000-episode show never blocks the request that started it; the daily budget stays the only throttle on automatic jobs. `queue_status` previews **50** active rows and counts the rest (`active_total`); a **15 min** lease with a heartbeat write at most once a minute and **3** expiries → `error lease_lost`; one server runner at a time, many clients; a boot requeues `running` (inline) and expired leases (external). Everything is `Priority.BULK`, reniced + idle-I/O, and yields completely: before a chunk through `Throttle(streams_active(), foreground_busy(), listening_for() < 15, load > 0.8)`, and **inside** a chunk by `SIGSTOP`ping the process group while `workload.streams_active() > 0` (the counter added for a whole episode: `listening_for` is stale 45 s into a one-hour stream). A user's stop is read from the DB every 2 s, kills the process group, keeps `done_ms` and the segments, and writes the partial sidecar. Auto jobs reserve their duration in the `user_id IS NULL` transcripts of the last 24 h (`daily_minutes`, default 120, `0` = unlimited) — a skipped auto episode creates **no row** (it is not "waiting"); manual `--episode`/`POST` jobs bypass the budget but not the queue. The archive trigger is one hook at the end of `archive._finalize_episode` (local import, `try/except`): a new archived episode of a marked show is queued, never transcribed in the request. Marking a show queues the backlog through `queue_backlog(show, backlog=all|recent|none)` — `recent` = the 10 newest, `none` = new episodes only; the mode is an argument because the per-show storage is the admin route's lot. `neezer.transcribe.start(app)` (called from `web.py`) starts the inline runner and resumes interrupted
work; `runner=external` leaves jobs `queued` and says `waiting` instead of running whisper in the
API, with a light 60 s scheduler (`resume_if_interrupted` then `resume_auto`, BULK/reniced, also
woken by a claim) so the backlog and interrupted jobs keep moving. `GUNICORN_WORKERS > 1` refuses
the inline runner (each worker would requeue what its siblings transcribe): the status says
`multi_worker` and the deployment must use `runner=external`.

Measured on SQLite: enqueue **6 statements** (**7** when there is no local engine and an enrolled PC carries the queue: one `EXISTS` on `transcribe_worker`), claim **8**, heartbeat **3 warm / 5 when it renews**, a chunk **6**, idle `resume_auto` **2** (the `tr.*` read), `job.workers_report` **1**; a 1 300 s episode through the fake engine is 3 chunks and 6 segments. Sidecar: **600 segments/h × 92 chars = 85.7 kB JSON/h (143 B/segment)**, SRT 78.5 kB/h. Config: `[transcribe] enabled|bin|model|runner|daily_minutes|search_segments|max_minutes|language`, paths file-only, `enabled=False` by default, and every missing piece is a named reason (`engine_missing`, `model_missing`, `model_invalid`, `ffmpeg_missing`, `no_audio`, `no_duration`, `too_long`, `too_big`, `exit`, `timeout`, `cancelled`, `lease_lost`, `budget`, `no_worker`) with no boot failure.

**Who can run the queue is not the local engine.** `_readiness` (`webui/transcribe.py`) and the three gates in `job.py` (`enqueue`, `auto_enqueue`, `resume_auto`) accept work when the local whisper.cpp is usable, **or** `runner=external`, **or** at least one row of `transcribe_worker` is `enabled` and not revoked (`job.worker_ready()`). Enrolment, never liveness: a PC that is off at this second claims the queued episode when it comes back, and "cannot reach it" is not a verdict — refusing there was the reported bug (a server with no whisper.cpp refused activation while its Vulkan PC was enrolled). `engine_missing` stays the local engine's own verdict; the gate that says nothing at all can run is **`no_worker`**. The payloads say where a job will run: `destination` (`server|external|workers`) and `workers {enrolled, online}` in `/status` (any account, counts only, **no hostname**) and `/settings` (admin), which also carries `machines [{name,kind,online,last_seen}]`. "Online" is `last_seen` within `job.ONLINE_SECONDS` = **120 s** (24 × the workers' 5 s idle claim poll, so a wifi hiccup never flips it); it is shown, never enforced. Cost of the fields: `/status` **6 statements** (+1) and **+61 B raw** for a non-admin, `/settings` **5**; the extra `EXISTS` runs only when the local engine is not usable.

**A 429 on a queue write is a window, not a verdict.** The three routes that write to the queue carry per-user `WindowLimiter`s: `mark_limiter` **60 / 10 min**, `start_limiter` **60 / 10 min**, `start_hour_limiter` **300 / h**. They predate the unlimited queue, when a full queue could refuse a start; now a row is a few statements and the runner takes one job at a time, so the caps only stop a runaway client. The SPA's batch offers **20** starts in one click (`webapp/src/lib/transcript.js` `batchOptions`), and the old **3 / 10 min** window refused the 4th POST of the app's own batch (measured on `master`: 429 `rate_limited`, `Retry-After` up to 600 s, on start #4 and on the second show marked). The response always carries `Retry-After`; the webapp waits exactly that long and sends the same call **once** more (`withRateLimitRetry`), a second refusal is shown, never a third attempt. Loops are still stopped: the 61st start in 10 min, the 301st in an hour, get 429.

The **read/search API** is `webui/transcribe.py` (blueprint `transcribe_api`): `GET /api/transcribe/status`
for every account (admin gets the cleaned `last_error`), `GET/PUT /api/transcribe/settings` admin-only
(`enabled`, language, limits and search scope in `Meta tr.*`; `bin`/`model`/`runner` are refused as
server-side), `POST /api/podcast/<pid>/transcribe` to mark a show and enqueue its `all|recent|none`
backlog, `POST|DELETE /api/podcast/episode/<eid>/transcribe` to start/stop/delete, and the read
routes `GET .../transcript` (page, `at`, `q`), `GET /api/podcast/<pid>/transcript/search` and
`GET /api/transcripts/search`. A page is **3 statements**, an `at`/`q` page 4, both searches 4 with
exactly **1 `search_text LIKE`**; the show page now costs 6 SELECT (the episode/archived counts
are one grouped query; the transcript state adds one when present),
each `transcript` payload is ~113 B when ready. Search scopes are the show's 100 latest ready
episodes and the most recent **1.2 M segments** (configurable; `0` = unlimited), and the payload
says when it is truncated. Transcript reads are `read` for remote sessions; settings/start/stop/
delete/mark are `NEVER`. No route exposes `transcript_path`. The `Meta` backlog key is
`tr.bl.<24>` because `Meta.key` is 32 characters; a database/column would be the durable
alternative. `settings.load` merges defaults < config file < UI, while `paths()` reads the file
only. `transcribe_settings` being `NEVER` also hides the card from an admin remote controller
(deliberate; change to `{"GET": "admin", "PUT": NEVER}` if that is wanted).

**Exporting the text** is three more read routes in the same blueprint:
`GET /api/podcast/episode/<eid>/transcript/export?format=txt|srt|vtt|md` (one file),
`GET /api/podcast/<pid>/transcripts/seasons` (the derived seasons with `episodes`/`transcribed`
counts, **3 statements** whatever the show's size: channel, episode list, ready ids — never one
query per season) and `GET /api/podcast/<pid>/transcripts/export?season=<key|all>&format=…`
(a streamed ZIP, one file per READY transcript numbered by its reading position, plus
`MANQUANTS.txt` naming each episode that is not ready and its state). The text is rendered one
episode at a time while streaming (`neezer/transcribe/export.py`), the ZIP reuses
`webui/export.py`'s `_Sink`/`_safe`/`_claim_export` (one export per user, **429** when busy), and
nothing is transcribed, queued or fetched from a provider: a pure DB read. **Deezer has no season
field**, so `neezer/transcribe/seasons.py` derives them: a season number read from the title
(`Saison 2`, `S2E5`) when at least half the show's titles carry one, the publication year
otherwise; groups are newest first, `Hors saison` / `Sans date` last, and the episode inside a
group is oldest first.

The **worker protocol** is `webui/transcribe_worker.py` (blueprint `transcribe_worker_api`) and is
how the sidecar transcribes without a database or an archive mount. An admin creates a worker with
`POST /api/transcribe/workers` (`name`, `kind=server|client`); the response shows the token
`<worker-uuid>.<32-byte secret>` **once**, and the database keeps it encrypted with the server's
`password_secret` (never in a GET, never in a log). The worker sends `Authorization: Bearer …` and
the lease in `X-NS-Lease`; the lease itself is stored only as SHA-256. `POST /worker/claim` returns
one job plus `audio_url`, duration, language, chunk size and start; `heartbeat` returns
`cancel|pause|done_ms`; `chunk` appends an ordered piece; `complete`, `fail` and `release` close or
requeue it. `GET /worker/audio/<eid>` serves the archived file with `Range`/206, `no-store`, and
403 without the matching lease; no payload ever contains a filesystem path. The sidecar is written
by a daemon thread, never inside the request, and a late flush only updates `transcript_path` and
`updated` (it can never revert a `ready` row). Worker routes are `NEVER` for remote sessions; the
three admin worker routes are `admin` (a remote admin link can create/revoke a worker — deliberate,
confirm if that is too broad). Measured: workers list 3 statements/184 B, create 4/207 B, claim
11/316 B, heartbeat 4/78 B, chunk 8/49 B, complete 6/51 B, fail 6/31 B, audio 6 statements plus the
bytes. A revoked or disabled worker gets 403 before the queue is read; 32 live workers is the cap.

**Unavailable tracks** (`neezer/webui/availability.py`): Deezer drops tracks from its catalogue,
and `Track.unavailable` (a timestamp, so a verdict expires and is re-tested) records the ones we
have confirmed dead. The verdict is only ever set from `TrackUnavailable` — the dedicated exception
`provider._resolve_once` raises when Deezer *answers* that there is no source — never from a network
error, and never before a re-login has confirmed it (an expired media licence token looks exactly the
same). **Archiving clears it**: once the FLAC is on disk the track plays forever, whatever Deezer
does, which is why the probe checks the file before asking anyone. `/api/track/<id>/probe` is what the
player calls the moment playback errors (the `<audio>` element can't tell a dead track from a dropped
packet), so a dead track is skipped at once instead of after four reloads. `/api/replace` swaps a
track for another one — same position in every playlist, plus favourites — in a worker thread, and
mirrors it to Deezer for the admin's own playlists. It then **retires the source**
(`_retire_replaced`): a row that is flagged unavailable, has no file, and is referenced by no
playlist and no favourite is deleted — otherwise the corpse stayed listed under "Indisponibles" for
ever with nothing left to replace. Narrow on purpose, since `/api/replace` accepts any source. The
SPA mirrors that timing: the work is a worker thread, so the list is dropped optimistically
(`resolvedUnavailable`) and refetched only once the job reports done (`unavailableVersion`) —
refetching on sheet-close re-read the old state and the row flickered back.

**YouTube is a FALLBACK, never a second catalogue** (`neezer/deezer/youtube.py`,
`neezer/webui/youtube.py`, `webapp/src/lib/youtube.js`; optional extra `neezer[youtube]`:
`ytmusicapi` + `yt-dlp` + `yt-dlp-ejs`, with Deno in the image for yt-dlp's challenge solver). It
is reached in exactly two ways, both a person asking: the search screen's **YouTube** box (unticked
by default, asked on Enter or when ticked — never per keystroke) and the replacement sheet's
*Chercher aussi sur YouTube*. Every rule is about calling YouTube as little as possible:
- **Search is ONE request** to YouTube Music's API (`ytmusicapi`, `filter="songs"`: title, artists,
  album, duration, square art; ~0.9 s), kept a day per query, empty answers too. yt-dlp never
  searches.
- **A hit costs nothing until it is played.** It is served under the UUID its row WILL have
  (`ids.youtube_track_uuid(video_id)`) and its metadata is remembered (`remember`/`recall`: memory
  + the disk cache), so starring it, adding it to a playlist or choosing it as a replacement
  creates the row from the search's own answer (`webui/youtube.materialize`, reached from
  `/stream`, `/favorite`, `_resolve_tracks`, `availability._resolve_local`) — no second request.
  An id whose hit is forgotten is simply unknown.
- **The audio is fetched once, on the first PLAY** (`ensure_archived`: one extraction + one
  download, ~2 s for five minutes, serialized per video, two at a time), staged in
  `archive_dir/.youtube-staging` (the local scanner skips dot-directories) and published under
  `archive_dir/YouTube/<artist>/<album>/<title> [<id>].opus` with tags, cover and a `.json`
  sidecar. A prefetch (`X-NS-Background`) never fetches; nothing bulk-archives it; the cleanup
  never deletes it (it only considers `deezer_id` rows). Subsonic's `stream` goes through the same
  call (`api/media._ensure_youtube_archived`).
- **The original stream is kept, never re-encoded**: Opus ~130-160 kbit/s (format 251) remuxed to
  Ogg with `-c copy` (AAC/m4a only when YouTube has no Opus). `/stream` serves the file as is unless
  the client asks for LESS than it holds (`TRANSCODE_MARGIN`), and the player shows the real rate
  (`SourceBadge.svelte`: "YT 133k" beside a FLAC or Opus 320 chip; read once from `/probe`).
- A row is `deezer_id NULL` + `youtube_id` set, so it is NOT an upload: `scan_local` never prunes
  it, `/me/local` and the local search skip it until it is archived, the probe answers "unknown"
  without asking anyone, and `verify_gone` only accepts YouTube's own verdict. Its artist/album
  carry no id in the API (no page to open: empty ids are "not a link" to the SPA).
- **Failing is not a verdict, and a refusal is left alone**: only YouTube answering that a video is
  gone (`YoutubeUnavailable`) marks a track unavailable; a network failure holds YouTube off for a
  minute and a refusal (429, "confirm you're not a bot") for fifteen (`held`), during which nothing
  is asked. The admin switch (Réglages → *Source de secours : YouTube*, `Meta yt.enabled`) turns
  it off for everyone; what is archived keeps playing. `tests/test_youtube.py` counts the requests.

**Importing a shared playlist** (`neezer/linkimport.py` reads the link, `webui/playlist_import.py`
matches and keeps it, `routes/Import.svelte`; reached from a link pasted into the search box and from
the library). Deezer, Spotify (its public embed page), YouTube (`ytmusicapi`) and Apple Music (the
share page's own JSON): no account, no token. **The pasted text is only ever parsed** — every request
goes to a fixed host with a validated id, and a short link's redirect is followed by hand, each hop
required to stay on the same service. Each line is matched library first (on disk: free, and it
never vanishes), then Deezer (the ISRC when given, else a search), then YouTube (a YouTube link's own
video, or one search) — by the replacement sheet's own `match_score`, or `closeness` ≥ `PLAUSIBLE`
with the artist credited (`sure: false`, listed under "À vérifier"); anything less stays unmatched,
since a wrong song is worse than a hole. A worker job (4 searches in flight), polled; the result is
TEMPORARY (process memory, 3 h) and plays as it is. *Enregistrer* (admin) makes a LOCAL playlist —
not created on the Deezer account — and queues its tracks for archiving.

**One rule for "is it the same recording" and one scale for quality** (multi-provider lot C, `neezer/matching.py`,
`neezer/providers/quality.py`). `match_score`, `closeness`, `_core_title`, `PLAUSIBLE` moved out of
`webui/availability.py` unchanged (it and `playlist_import` re-import the same objects) and gained
`same_recording(a, b)` -> `sure|maybe|no` (equal ISRC, else core title + main artist + length within 3 s + same
version words; a cut's label such as "Radio Edit" is `maybe`, never `sure`) and `match_key` (one dict pass to group a
list; confirm a collision with `same_recording`). `quality.py` notes an offer or an archived file's header (mutagen,
nothing decoded) as `Quality(cls, effective_kbps)`: 50 hires, 40 lossless, 30 from 192 effective kbit/s, 20 from 96, 10
below; the effective rate applies the codec factor (Opus x1.5, AAC x1.3, Vorbis x1.2, MP3 x1). Pure functions,
no schema, nothing on a route's path; `tests/test_multi_c.py` pins the table.

**Unavailable means BOTH sources are gone** — not "Deezer dropped it". `DELETE /api/track/<id>` is
the third answer next to replace-and-upload, and `availability.verify_gone` re-checks *at the moment
of deletion*: a file on disk → refuse (and clear the verdict); Deezer still resolves → refuse; an
inconclusive network answer → refuse. Only `TrackUnavailable` with no file, or a local upload whose
file is gone, authorises it. Scoped like a replacement (own playlists/favourites; an admin also drops
the row, `recursive=True`); nothing is mirrored to Deezer.

**The "Indisponibles" list is built for clearing out** (`components/UnavailableList.svelte`,
`lib/replace.js`). Tick several (shift-click for a range) and `POST /api/tracks/delete` (≤ 200 ids)
deletes them in ONE job that runs `verify_gone` per track in the worker (a Deezer call each, never
in the request thread) and reports `deleted` / `kept: [{id, reason}]`: a selection is no way round
the re-check. On a desktop each row also shows up to two ONE-CLICK stand-ins,
`/replace/candidates/<id>?best=2` → `match_score`: same title once brackets and " - …" are set
aside, the same main artist, no version word the dead track did not have (remix, live, edit,
acoustic…), a length within 15 s; never YouTube. Anything less sure stays behind "Autres choix" (the
sheet, where it can be auditioned). Fetched only for rows near the viewport, two at a time. The
candidates also search the local library by TITLE (artist checked): the `"<artist> <title>"` query
is one LIKE per field and never found the copy you already had. The query drops "(Remastered…)" /
" - Radio Edit" (`_core_title`), the sheet has a search box (`q`, ≤ 200 chars), and **YouTube is
asked on its own when Deezer finds nothing** (`youtube_auto`) — "nothing" meaning nothing that LOOKS
like the song (`closeness` ≥ `PLAUSIBLE`: the artist's other songs come back for any query naming
them, and used to block it), and for a typed query nothing at all. Five songs: ranked by
`closeness` to the dead track, or in YouTube's own order for a typed query (what was typed is what
they must be close to). The inline `?best=` falls back to YouTube the same way, through
`match_score` — which tolerates a CUT's label (edit, extended, club, dub) when the length agrees to
3 s: YouTube Music calls Stromae's 3:27 album take "Alors on danse (Radio Edit)". Checked against the
real YouTube Music on five real dead-track shapes; the two it still refuses are other cuts.

**The audio auto-match** (`deezer/audiomatch.py` = the ears, `deezer/automatch.py` = which
candidates and in what order, `webui/automatch.py` = when, and applying it). A dead track has no audio
left anywhere but the 30 s preview Deezer's public API may still serve for it, so that is the ONLY
reference (the user's choice: no ISRC shortcut; no preview → left for a person). Candidates pass a
metadata gate first (same core title, same main artist, ±20 s), then: files on disk; Deezer (its
preview, then — if that did not overlap — its whole stream, decrypted straight into ffmpeg, never
archived); YouTube only when Deezer had nothing, at most 2 downloads, staged and deleted unless the
candidate wins (then `ensure_archived(produced=)` publishes that very file). ≥ `THRESHOLD` (0.99)
replaces everywhere with the admin's scope (`availability.replace_everywhere`, the same code as a
person's replacement) WITHOUT asking — so the panel always lists what it replaced, with the score.
What 0.99 means is measured on real ffmpeg (`tests/test_audiomatch.py`): the same master as FLAC,
Opus 128 or −6 dB reads 0.9991-0.9999, a remaster 0.992 (same performance: accepted), the same
arrangement re-performed with 10 ms of timing drift 0.981, the instrumental 0.74. Two things made
those numbers: aligning to the SAMPLE after the hop-grid search (0.984 → 0.9999 on the same master)
and a 40 dB floor (at 60, Opus's noise fill in near-empty bands read 0.985). No BLAS (band means by
cumulative sum): its threads would escape the worker's `renice`. Runs on the EVENT
(`library.UNAVAILABLE_HOOKS`, first verdict only — a weekly re-confirmation is not news; one worker,
bounded queue; under `app.testing` nothing starts, tests call `drain`) and on the admin's button
(`/api/automatch/run`, progress, causes tallied, stopped — not failed — when Deezer is unreachable).
Switch `Meta dz.automatch`, ON by default; nothing runs without numpy + ffmpeg.

**An archived track carries its whole identity on disk**, which is what makes the above safe:
`_finalize_archive` writes the audio, `cover.jpg`, the `.lrc` lyrics, the file tags, **refreshes the
DB row from the authoritative `song.getData` payload** (`library.refresh_track_metadata` — rows are
often created from a thinner playlist listing, and it only ever upgrades fields), and writes a
`<track>.json` sidecar (`library.save_track_metadata` / `read_track_metadata`) with the Deezer ids,
contributor roles and ISRC that no audio tag can hold. That sidecar is a **whitelist** of gw fields —
the raw payload also carries stream tokens and signed URLs, which must never be written to a file
that outlives the session.

**Archive completeness** (`neezer/deezer/backfill.py`, `neezer/webui/storage.py`) is
**event-driven, never polled**. Archiving happens the moment something becomes yours:

- *playing* it (the FLAC download runs on a thread of its own — `archive.open_live_stream`, one
  `_LiveArchive` per track, shared by every listener — and the response **tails** the growing
  `.part` instead of fetching it: a client that stops reading (a full buffer, a paused tab, a seek,
  a skip) no longer stops the fetch, which is the difference between an archive that lands in
  seconds and one that stayed a `.part` for minutes. Measured on a 6.6 MB FLAC whose listener read
  1 MB then nothing: **complete in 1.2 s** with the producer thread, **never** (30 s cap) when the
  same generator fetched and yielded; and a reload while it downloads **joins** it — **1 Deezer
  connection instead of 2**. The file is published (tagged COPY, atomic rename, row finalised) as
  soon as the download ends; a reader keeps reading the raw `.part` and so never sees a file being
  retagged, and that `.part` goes with its last reader. When the last listener leaves the download
  keeps full speed for **1.0 s** (the player's chase reads `/ready` at 600 ms then every second, so
  a seek reload joins it and clears the clock: measured **0.2 ms** to the first byte on a `.part`
  400 ms old, 1 producer and 1 Deezer resolve), then stops at its next chunk and `on_abort` gives
  the rest to the background queue at **BULK** — nobody is waiting for those bytes. There is no
  paced grace any more: skipping five cold tracks leaves **nothing** behind (measured: all five
  producers gone **1.02 s** after the last skip, no `.part`, five rows queued BULK). An orphan must
  never take the link from the track the person moved on to. A cold Opus request is that same live
  stream piped through ffmpeg as it downloads
  (`webui._opus_live_generator`: a feeder thread owns the source, 100 ms Ogg pages, first audio
  0.26 s at 512 KB/s where it used to wait for the whole FLAC);
  `api/media.py::_ensure_deezer_archived` calls `ensure_archived` outright; podcast episodes archive
  on first play too). An archive is TAGGED BEFORE IT IS PUBLISHED (`archive._tag_archive`): tagging
  rewrites the file in place, and a published file is being served;
- *starring a track* (`/api/favorite`, and Subsonic's `star` via `annotation._archive_starred`);
- *favouriting an album, a playlist or an artist* (`/api/favorite/<kind>` and Subsonic's `star` →
  `backfill.archive_entity`, in a worker thread, always the FULL tracklist — never the ~10-track
  page). An artist is OFF by default (`on_fav_artist`: starring one is how you follow it), and then
  **means the whole discography**: `_archive_discography` walks Deezer's `all` tab
  (official releases + "more", not the guest appearances), fed release by release so the first album
  downloads while the rest is still being listed, and deduplicated across editions;
- *adding tracks to a playlist* or creating one (`/api/playlists`, `/api/playlist/<id>/tracks`, and
  Subsonic's `createPlaylist`/`updatePlaylist`);
- *subscribing to a show* (`/api/podcasts` POST → `backfill.archive_show`, every episode) — **only
  when that show's own switch is on**. `PodcastChannel.auto_archive` ("Archiver automatiquement les
  nouveaux épisodes", on the show's page, admin; `POST /api/podcast/<id>/auto_archive`) is the ONE
  authority over what the archive keeps of a show: `archive_show`, the sweep's `collect` and the
  Subsonic `refreshPodcasts` all read it. The `on_podcast` rule only picks its value for a NEW
  subscription, so the page never shows a switch that something else overrides. Turning it on queues
  what the show is missing (BULK, one at a time); the page then re-reads the show every 6 s while the
  count moves and stops after ~30 s without progress or when hidden. Migration 20261029 set every
  existing show ON: before it, the sweep archived every show whole, and an upgrade must not quietly stop.

Everything goes through the bounded, **priority-ordered** background download queue
(`prefetch.download_ids` / `download_episode_ids`, see the priority section below) and is fail-soft:
archiving is a *consequence* of the action, never a condition for it. Rows already on disk are filtered out first, so re-starring a big library costs
nothing. A discography is bigger than the queue, so `backfill._queue_all` waits for room
(`QUEUE_RETRY_DELAY`) instead of dropping the overflow — it gives up only if `archive_library` is
switched off under it. **Do not add a periodic archive loop** — the app knows the instant it
happens, so re-asking on a timer is work for nothing.

**A person waiting in front of the app outranks every background job**
(`neezer/deezer/workload.py`, `webapp/src/lib/ladder.js`, `webapp/src/lib/coverqueue.js`). That
rule needs a mechanism per kind of contention, and all of them exist because the app once queued a
user's own requests for thirty seconds behind work they never asked for:

- **Request threads.** The server answers from one worker × 16 threads, and a first play of a Deezer
  track holds one of them for as long as the FLAC takes. The client PREFETCHES through the same
  route, so a handful of prefetches took the whole pool. A request nobody is looking at says so
  (`X-NS-Background: 1`, or `?bg=1`) and is held to a quarter of the pool; over that it is
  **refused** (503 + `Retry-After`), never queued — a background request that waits is still holding
  the thread it was meant not to hold, and the client reads the refusal as "not cached yet". A
  background `/api/stream` of a not-yet-archived track never downloads inline at all: it queues the
  archive at prefetch priority and answers at once.
- **Download slots.** `workload.Priority` orders one queue serving three urgencies — `USER` (0),
  `PREFETCH` (5), `BULK` (9, the default). The client says **why** it wants a track (`why` on
  `/api/download`), never how urgent it is, and an unknown reason is bulk, so nothing can claim the
  front by accident. Everything below a person's own tap — `BULK` **and** `PREFETCH` — also
  `quiet_wait` while a foreground archive is in flight (one link, one Deezer session, one disk
  head): the track they are hearing is (a), the next track's preload is only (b). At rest the
  workers still run flat out, because `quiet_wait` returns at once when nobody is waiting.
- **CPU.** `workload.renice` puts the archive workers and every batch job behind everything else,
  and `workload.cpu_workers` sizes a library-wide job from the machine (half its cores) rather than
  asking the operator.
- **The client's own ladder** (`lib/ladder.js`): a track change used to fire seven requests at once.
  Now the audio goes first; the artwork, the genre verdict, the lyrics, the silence bounds and the
  play count wait until the element can play and then run **one at a time**; the prefetch of the
  next track waits for the same signal. Caching the artwork before telling the media session about
  it is also what stopped the same cover being fetched twice per track change.
- **Bandwidth on the device** (`lib/netyield.js`, `YieldGate.kt`). A playlist downloading on a
  phone took the link from the track the person had just pressed (four FLACs at full speed against
  one stream: measured on a shared 256 kbit/s link, the track took 30 s to start against 7 s idle).
  The playing element holds the link while it plays from the network with under 15 s in hand
  (released at 30 s, or once the file is all in; every `makeEl` element is watched, so crossfades
  and fresh elements count), and so does the preload of the NEXT track; while held, a download does
  not start, and one in progress steps aside — a resumable body (length known, `Accept-Ranges`) is
  aborted and picked up with a `Range` at the same byte, a live transcode is read at a 4 KB/s
  trickle (the receive window holds the server to it, and the connection never idles into a
  proxy's 60 s send timeout). A dropped connection resumes too (3 times). The app's own downloads
  hear it as a LEASE (`NSNative.offYield(ms)`, 45 s renewed on the element's events, which keep
  coming in a hidden page; capped at 60 s natively, so a dead page cannot stop them). Same idea as
  Media3's `PriorityTaskManager` and Chromium's `ResourceScheduler`. `node test/download/run.mjs`:
  6.1 s to start with eight downloading, 0 stalls, every download finished once paused.
- **Artwork** (`lib/coverqueue.js`): six covers in flight (three at most for covers not on screen
  yet), each freed slot going to whichever waiting cover is closest to the viewport — sideways too,
  for a shelf — with the scroll direction counted as closer. A cover more than two screens away is
  not fetched until a scroll brings it near, and a turn that has not settled in 4 s gives its slot
  back. It used to admit anything that had waited 8 s, and `Cover.svelte` asked for every card's
  1000px copy on mount, outside the queue: opening *Nouveautés* (300 cards) put 311 requests in
  flight and the covers scrolled to waited minutes on a phone's link. The hi-res copy is now asked
  only once the base art is drawn, and only where the tile is drawn larger than 500 device pixels.
  Art that costs no request (above the fold, or already on the device) never queues.

Worker recycling (`GUNICORN_MAX_REQUESTS`) is **off by default**: the hours-long library jobs run on
threads inside the worker, and recycling killed them mid-run.

The nightly sync then runs `sweep_for` as the *safety net* for what events can't see (a Deezer-side
change we only learn about at sync time, a download that failed while the server was down), and
Réglages → Compte has the same sweep as a button with live progress; `_sweep_lock` keeps them from
running twice at once. All of it is gated by `[deezer] archive_library` (default on, checked in
`backfill.archiving_enabled` so the switch silences the events too) and **only ever adds**.
`/api/storage` reports archive size, free disk and the two derived caches; `/api/cache/flush` empties
those caches (expiring the protection first, or the button would silently do nothing) and cannot
touch `archive_dir`.

**Archive rules** (`neezer/deezer/rules.py`, `/api/archive/rules`, Réglages → Archive) make all of
the above configurable at runtime: one boolean per event (`rules.EVENTS`), the artist scope
(`all` / `releases` / `top` + `artist_limit`), and the cleanup policy. Values live one-per-row in
`Meta` under the `dz.` prefix (`Meta.key` is `CharField(32)` — keep names short) and override the
config file, exactly like the ARL; `rules.load` caches for a second because it is read on every star
and every play. `archive_library` stays the master switch above all of them. There is deliberately no
`on_play` switch — playing a Deezer track *must* archive it, since the Opus transcode reads the
archived FLAC; what is optional is `on_play_context` (playing one track pulls its whole album or
playlist, opt-in, deduplicated per container per hour by `backfill._first_time_seen`).

**Response time is measured, on a library the size real ones reach** (`tools/perf_api.py`: 8 000
tracks, a 4 000-track playlist, 4 000 favourites, sixty playlists, Deezer off so the answer is this
server's own work). Four rules came out of it, each with its number:

- **A big list is read as TUPLES** (`webui._db_track_rows`, reached through `_db_tracks` whenever it
  is handed a query). Three quarters of a 4 000-track list's time was Peewee building a Track, an
  Album and an Artist per row — three UUIDs and a datetime parsed, a setattr per column — for
  `_db_track` to flatten straight back into a dict. Selecting exactly the columns the JSON carries,
  with the UUIDs read as text (`_raw`: a CAST, spelled CHAR on MySQL) and the credits read through a
  subquery on the list's own selection instead of ten 400-parameter IN lists: favourites **405 → 99
  ms**, a 4 000-track playlist **447 → ~110 ms**, byte for byte the same answer
  (`tests/test_webui_lists.py` compares the two paths on every shape a row takes). A query with a
  LIMIT keeps the id lists — MySQL refuses LIMIT inside IN.
- **A track payload says what its FILE is** (`webui._quality_fields`): `quality` (`FLAC`,
  `OPUS 320`, `MP3 320`), `bitrate` (kbit/s) and `suffix`, on `_db_track`, `_db_track_rows` (big
  lists) and `_local_track` (uploads). Only once the row is archived (`last_modification > 0`, or a
  local upload): before that `Track.bitrate` is the NOMINAL rate of the quality that was asked for,
  and Deezer may yet answer with a lower format — so an unarchived row carries none of the three
  rather than claim what nobody hears. YouTube keeps `_youtube_fields`'s own `bitrate` label and is
  never given a generic `quality` (it would replace "OPUS 133" with a bare "OPUS"). The columns
  were already in the row (the path and `last_modification` join the same SELECT), so the statement
  count does not move (3 for 4 000 favourites, 2 for `/me/local`, pinned by
  `tests/test_webui_quality.py`); the cost is the label itself, ~5.6 ms of CPU per 4 000 rows, and
  +192 KB of JSON (+3.3 KB gzip, +1.1 %) on that favourites page.
- **Nothing is counted per row.** `/me/playlists` asked two statements per playlist (122 for sixty,
  90 ms) and is four in all (24 ms); `/me/favorite-ids` hydrated every starred row twice to read one
  id (314 ms) and is one two-column query (19 ms); an artist's album grid and the podcast list counted
  per album / per show and are one grouped query each; the mix fallback no longer lazy-loads each
  row's album and artist (`pl.get_tracks()`). The tests pin statement counts, not times.
- **Every JSON GET carries a validator** (`web.py#_validate`: a weak ETag over the body and
  `private, no-cache`). The web app repaints a list from its offline copy and asks again on every
  visit, and the answer is usually identical — 2 MB of JSON and 30 ms of gzip to say nothing changed.
  Now the browser revalidates on its own and an unchanged list is an empty 304.
- **The SPA's own files go out precompressed.** They are sent as files (passthrough), which the
  on-the-fly gzip never touched: the main bundle went out as 460 kB. The build writes a brotli and a
  gzip copy of every text and wasm file that compresses (`vite.config.js#precompress`) and `spa.py`
  serves the smallest one the client accepts (`Vary: Accept-Encoding`, the type still the file's):
  main bundle **460 → 132 kB**, the analyser **219 → 71 kB**, the app core **63 → 19 kB**.

**Gain and lyrics no longer come from Deezer.** `Track.gain` is the measured loudness
(`analysis.apply_measured_gain`, `gain = lufs + GAIN_FROM_LUFS_OFFSET`; `/api/gain`, `/api/gains` read the
database only and take UUIDs or Deezer ids; `regain_from_analyses` re-derives it from stored analyses without
ffmpeg at the start of a backfill). Lyrics are LRCLIB only (`lyrics.ensure_lyrics`, hits more than 3 s off the
track's duration refused, synced replaces plain, `facts.lyrics_tried` in the sidecar stops repeat searches;
the route searches after answering). Shared building blocks: `library.update_track_facts` (sidecar `facts`),
`neezer/libjob.py` (resumable library job), `neezer/filetags.py` (facts read from file tags).

**Local play counts.** `/api/listen` writes `Track.play_count` / `last_play` (≥20 s counts, a skip
doesn't). Before that only Subsonic's `scrobble` did, so a library played entirely through the web
app looked untouched — and the cleanup decides what to drop from exactly this data.

**Recommendations, the radio, the Flow and the releases** (`neezer/listening.py`, `neezer/mix.py`,
`neezer/radio.py`, `neezer/discover.py`, `neezer/releases.py`, `webui/discover.py`). Every screen READS the database: no click asks a provider.
- **Taste is PER PERSON** (`listen_event`, schema `20261026`): every track change the player reports (and Subsonic's
  scrobble) is one row, judged by published rules — a listen is half the track or 240 s (Last.fm), a skip is under
  30 s, a track of 30 s or less is never a listen. A title's weight is ACT-R's base-level activation
  `sum((hours+1)^-0.5)` (Reiter-Haas et al. 2021); a title skipped twice and never heard is rejected. The shared
  `Track.play_count` only fills a new person's profile, faded out over their first 200 events.
- **One assembler** (`mix.assemble`): membership is a Gumbel-top-k draw proportional to the score under the artist
  cap (3 per list, LB Radio), then the ORDER is a windowed multiplicative MMR `rel*(1-(1-lam)*sim)`, lam 0.5, over
  the last 5 with a 0.75 fade (sim: `track_neighbor`, same album 0.8, same style 0.3), no artist within 3 positions.
  Membership and order are separate on purpose: an MMR choosing members pulls in an unrelated cluster for being
  different.
- **The radio is anchored** (`radio.build`, `POST /api/radio/next`): every refill scores from the ORIGINAL seed
  (weight 1) plus the session — positives at most 0.6 in all, fading 0.8 a step, skips -0.5 each (Pereira et al.
  2019) — so it never drifts track by track. Pool: the seed's neighbours, two hops through its top 8 (x0.6, never
  overriding a skip), then same artist / style. A softmax at T 0.04 on the similarity before the draw. The player
  sends what is queued, heard and skipped (`Player.svelte#nextBatch`, also the end-of-queue autoplay).
- **The Flow** keeps the distance model and the slider (`tests/test_discover.py` pins the mean distance MONOTONE in
  the level); known titles come from the top 600 by activation (heard in the last 3 h: x0.25), the genre pool is
  sampled from random keyset starts. It is chosen ONE TITLE AT A TIME (`discover.flow_next`, `POST /api/flow/next`):
  the pool is built once per session (`sid`, kept 15 min), then each pick weighs the session (heard through +0.4,
  skipped under 30 s −0.35, "plus comme ça" +1, "moins comme ça" −0.75, older events fading ×0.85/×0.9) as
  `exp(1.5·affinity)` over the stored neighbours (Rocchio: the negative weaker than the positive). Skips IN A ROW
  count more and widen the distance band (x1.5 per skip after the first, max x3; Spotify's sequential-skip work,
  Meggetto et al. CIKM 2021): without that, five skips in a row left 90-97 % of the same genre, because a near genre
  only falls to x0.19 and a far one is outside the band; measured 97 / 90 / 80 / 27 % after 1 / 2 / 3 / 5. One title
  heard through ends the streak. "Plus comme ça" also adds its own neighbours. Cost on 8 000 titles
  (`tools/perf_api.py --archived`): the first pick of a session 284 ms cold (pool + energy map), then 18-22 ms,
  11 statements, ~400 B gzip; the old `/api/flow` (kept for older apps) was 220 ms and 4.7 KB every refill.
- **The explorer** (`neezer/deezer/explorer.py`, table `discovery` schema `20261102`, `GET|PUT /api/explorer`, the *Exploration
  automatique* card of the studio's Bibliothèque tab): finds music for genres the library lacks, quietly. It only searches genres
  the ENGINE knows (`analysis.known_genres()`, resolved through the tree), weighted by tree closeness to what is played and by how
  little the library holds of them; looks a genre up as Deezer playlists named after it first, then a plain search (all
  `ALLOWED` in the detach guard); fetches the 30 s preview with `automatch.fetch_preview` (Deezer's CDN only, size-capped) and runs
  the SAME analysis the library gets (`analysis.analyze_file`). A find is kept only if the engine hears the target or its direct
  parent/child (tree distance <= 0.1, confidence >= 0.4); everything else is stored `rejected` and never fetched again. Finds are
  `Discovery` rows — NOT tracks, nothing archived: the Flow offers them like the library's unplayed titles (`discover._offer_discoveries`,
  by genre distance, "Une découverte : <genre>", cached 60 s) and playing or liking one archives it through the usual path, unless
  the operator ticked "archiver tout" (bulk priority). It runs only when nobody has listened for 5 min, no foreground download runs and
  load per core <= 0.5, one preview at a time through a `Throttle`, 1.5 s between Deezer calls, stopping at the first outage (never a
  verdict). The queue's next title may be prefetched like any catalogue title, so a find at the head of the queue can be archived just
  before it plays.
  The **Explorer's screens** add two read-only routes in their own blueprint (`neezer/webui/explorer.py`, registered under `/api`
  from `neezer/web.py`; `webui/__init__.py` only supplies the shared login and same-origin guard): `GET /api/explorer/styles` returns the ten PARENT styles of the caller's own taste
  (`discover.parent_styles`: raw verdicts merged into their nearest two-child/family/root parent, confidence and track count included,
  empty with a `source` when the person has no history — never the library's styles as theirs), and `POST|GET /api/explorer/discover`
  (`{style, level 1..5}`): only the POST starts a worker (GET polls and answers `idle` when nothing runs), measuring at most 2 playlists / 8 previews at a level-dependent tree distance
  and confidence (0.10/0.60 at level 1 to 0.25/0.45 at level 5: a sibling at 0.20 may pass at 5, the 0.30 cousin never), on a
  `Throttle` + `renice` worker, cached 24 h in chunked `Meta` rows (60 s for a provider failure) pruned and capped at 512 groups.
  A POST that actually starts a worker spends the person's budget (6 / 10 min, 30 / h → `429` + `Retry-After`); both workers busy is
  `503` + `Retry-After: 5`; a body over 8 KiB is `413`. A Deezer outage is
  `ready` + `playlists: []` + `error: "provider"`, never a 500 and never a verdict about the style. Numbers and thresholds:
  [analysis-genre.md](analysis-genre.md).
- **A Flow around a STYLE** (`discover.style_taste`, `styles` on `POST /api/flow` and `/api/flow/next`, `GET /api/flow/styles`,
  `StylePicker.svelte`, `lib/flow.js#startStyles`): the person's taste is cut down to the chosen genres (a branch means all of
  it: the weighted tracks, artists and albums whose served genre is `within` one of them) and the taste's genres become the
  chosen ones, so everything downstream — the known titles, their sound neighbours, the library's unplayed titles by TREE
  DISTANCE from the style, the discovery band — works unchanged around the style instead of around the whole taste. At
  level 0 it stays inside the branch, higher levels widen it through the tree. A small genre is not left to the random
  sample of the library: its own unplayed titles are drawn too (`taste.focus`, the spellings `TrackAnalysis.style` may
  hold, family ids included). At most 6 styles, names resolved by the tree (an unknown name is a 400, not "everything");
  a style the person never played has no taste and rests on the library's titles of that genre. The choice lives on the
  device (`flow.styles`) and rides in the session body.
- **Energy** (`neezer/energy.py`, `user_taste.energy`): a 0..1 RANK in the library from the analysis (LUFS,
  −LRA, centroid, flux peak, entropy, the tempo only when its confidence >= 0.3 — Spotify's energy, re-built from
  what we measure), cached 10 min and rebuilt on a background thread. The notch (Très calme 0.1 … Intense 0.9, or
  Libre) is a Gaussian fit of width 0.12 with the distance band widened ×2, so it HOLDS (measured 100 % of 40 picks
  in the right cluster) instead of nudging; an unmeasured title weighs 0.3. With Libre, four titles heard through
  set a soft aim.
- **"Je n'aime pas"** (`neezer/dislikes.py`, `track_dislike`, schema `20261031`, `POST /api/dislike`) is about
  RECOMMENDATIONS only (Deezer's "Ne pas recommander", Spotify's taste-profile exclusion): the title, its other rows
  (`match_key`) and the neighbours at >= 0.9 among the first 3 of EITHER list leave the radios, the Flow and the
  mood mixes; its first 20 neighbours are pushed by RANK (x0.3 for the nearest up to x1 at rank 20 — by rank
  because stored scores sit at 0.9 throughout a dense genre, measured in the test graph). A radio seeded on a
  disliked title still plays; albums, playlists and search never filter. In a generated queue the playing title is
  skipped at once. A third title of one artist offers muting it (`artist_hint`). Cached 120 s per person, one
  statement when the list is empty (the profile bound went 10 -> 11).
- **Releases** follow favourites AND each person's most-listened artists (`rel.listened`, 30 by default, Réglages
  -> Compte; the shared play-count fallback is the admin's only), plus close artists of favourites; compilations
  are dropped and a single later released on an album under the same title shows once. Writes are portable
  upserts — PostgreSQL has no REPLACE, which is what the "Nouveautés" error was. `releases.py` wakes every 15 min,
  asks only the artists due, one request a second, stopping at the first outage (never a verdict). The tables are
  keyed by the PROVIDER's ref (`artist_watch`, `artist_release`, `artist_link`, `artist_top`); catalogue
  suggestions are never downloaded to be proposed.

**A genre station** (`webui/genre_radio.py`, `GET /api/radio/genre/<path:style>`,
`POST /api/radio/genre/next`; the Explorer tile's play disc and *Toutes les stations*): the Flow built
around ONE tree node, then **cut to the branch** (`genretree.within(TrackAnalysis.style, node)`)
because the Flow's pool also holds sound neighbours and explorer finds verified only NEAR the style —
a station promises one genre, so catalogue rows (no local analysis) are dropped. `recent` is folded
into `exclude`; caps 40/30/400; `<path:>` because a node may hold a slash ("Disco / funk", sent
`%2F`); an empty branch is `200` + `[]`, an unknown style `404`. It is its own playback context and
writes nothing (`UserTaste`, `flow.styles` and the energy notch untouched). No provider call: on the
8 000-track bench the first answer is **91 ms / 25 statements / 37 titles**, a refill 93 ms / 27.
`remote.POLICY`: `genre_radio` and `genre_radio_next` = `read`.

**Mood mixes** (`neezer/moods.py`, `GET /api/moods`, `GET /api/moods/<id>`): six sets of measured
bands over the archived library — Calme, Concentration, Bonne humeur, Énergie, Soirée, Nuit — built
from the `energy` rank, BPM above `BPM_TRUSTED`, and centroid/flux ranked in the library, never from
a hand-typed tag. A mood with fewer than `READY_MIN` (8) matching titles is not offered. The catalog
is one joined read cached 60 s per (person, day): **208 ms cold / 0.6 ms warm**, 10 statements then 1
on the 8 000-title bench; a 40-title mix is 5.3 ms / 4 statements / 19.1 kB (4.3 kB gzip). The
opening title is drawn among the 12 best fits at `<user>:<date>:<mood>`, so the card's cover is the
page's first track and both are stable for the day. No provider; `mix.assemble` caps an artist at 3
and keeps 3 positions between two of their titles.

**The bottom of a playlist** (`radio.build`, `GET /api/playlist/<id>/recommendations`,
`Playlist.svelte`) seeds the anchored assembler from the playlist's own rows, minus every title it
holds, the unavailable ones and duplicates, and applies the person's skips/dislikes. Library-only,
one request; the draw is seeded `<playlist id>:<date>`. Default 12, max 20; a non-DB/editorial id is
a 404, under two titles an empty answer. A long playlist is represented by an evenly spaced sample
of 80 seeds while ALL its rows stay excluded: on the 8 000-title bench a 4 000-title playlist cost
**2 171 ms with every seed and 139 ms with 80** (12 statements, 7.0 kB / 1.7 kB gzip).

**The listening history** (`webui/history.py`, `GET /api/history`) is the person's own
`listen_event` rows, newest first, with a **keyset cursor** `(at, id)` — not OFFSET, so a play
recorded while somebody pages can neither drop nor repeat an event. 50 per page (200 max),
`outcome=listen|skip`, 30-day summary by default (365 max): listens, skips, minutes (every `listened`
second, skips included) and the top 5 artists/titles by number of LISTENS. The page is one event
query plus one batched `_db_tracks`; **9 statements** for 50 events, 36 kB / 1.4 kB gzip (135 kB /
3.4 kB at the 200 ceiling). `request.webuser` only: an admin never sees another person's history.

**The year in review** (`neezer/recap.py`, `webui/recap.py`, `routes/Recap.svelte`) is the same
history aggregated over a **server-local half-open year**: `GET /api/recap/years` is 1 statement
(one entry per year with at least one event, no `KEEP_DAYS` floor) and `GET /api/recap/<year>` is
**4 statements** (1 for an empty year) — daily `GROUP BY date(at)`, one per-track/style aggregate,
then batched `_db_tracks` payloads. `request.webuser` only, no `?user=`, no provider, no write, no
schema. `enough` means **20 listens or 30 minutes and at least one listen**; tops are by listens;
genres come only from `track_analysis.style` at confidence ≥ 0.4 rolled up through
`discover.lift_style`, with coverage printed and the card absent below 34 %. The process cache keeps
aggregates only (60 s for the current year, 1 h for a past one, 32 entries); ETag 304 is free.
Measured on the plan's 8 000-title bench: years **22.7 ms / 265 B**, a full year **37.8 ms cold**
and **2.0 ms warm / 7.3 kB**, an empty year **1.5 ms / 1 040 B** (351 B gzip — the 12-month shape
cannot fit the plan's 700 B), **2 and 5 statements on the wire** (session included) and **3 warm**
(the aggregates are cached, the title payload is re-read to compute the ETag; a 304 body is empty).
The SPA opens with 2 requests cold, 1 warm, then 0 per card and 0 for the share poster.

**Users, identities and invitations** (`managers/user.py`, schema `20261118`):
`User.name_key` is the folded lookup key (NFKD, diacritics dropped, lowercase) with a unique
nullable index; `User.disabled`, `User.last_login` and `User.onboarding_step`
(`artists|account|done`) are additive. The `invite` table stores only the SHA-256 of a
`<id>.<HMAC>` token (never the clear token), plus `created_by`, `expires`, `used_by`, `used_at`,
`revoked_at` and a note; `used_at` is the single source of truth for a consumed invitation.
`UserManager.add`/`rename` validate the 3–32 `[a-z0-9_.-]` form, compute `name_key`, keep the
spelling the person typed (CLI/Docker and `sync_user` still look up by exact name), and turn an
uniqueness race into `UserNameTaken`. A historical `Alice`/`alice` pair keeps both accounts able
to log in with their own spelling; the loser's `name_key` is NULL (the unique index allows many
NULLs), while `_name_taken` still reserves the folded name for every new account. `UserManager.add`
leaves `onboarding_step='done'`: only an invitation redeem starts at `artists`. Migration
`20261118` is idempotent on all three engines, backfills `name_key` and `onboarding_step`, and
creates zero invitations. Measured: 1 000 existing accounts migrate in 5.3 ms (1.4 ms replay);
`add` is 2 statements, a duplicate 1, `rename` 2.

**Invitations** (`neezer/invites.py`, `webui/invites.py`, table `invite` schema `20261118`):
`POST /api/admin/invites` shows the `<id>.<mac>` token **once** and stores only its SHA-256; the
list is keyset `(created, id)` with `state=pending|used|expired|revoked`, and revoking or renewing
keeps the row (the token is never returned by a GET). 20 creations/h/admin (`WindowLimiter`) and
200 live invitations, both 429 with `Retry-After`. The public `POST /api/invites/inspect|redeem`
take the token in the JSON body (8 KiB, same-origin, 60 inspects/5 min and 5 failures/15 min per
IP and per token hash), answer the **same 404** for every invalid token, and redeem consumes it
with a conditional `UPDATE` in the account transaction (`onboarding_step='artists'`, never admin,
session opened). Measured: creation 4 statements / 206 B, list of 50 3 statements / 3 736 B
(612 B gzip), inspect 1 / 68 B, redeem 5 / 217 B, renew 6 / 194 B. `last_login` is written at most
once an hour; a disabled account gets the generic 401 and its sessions are cut. When
`NEEZER_SECRETS_ENV_ONLY=1`, `NEEZER_SECRET_INVITE_SECRET` holds the token secret; rotating it
invalidates every pending invitation.

**Onboarding artists** (`neezer/onboarding.py`, `neezer/deezer/onboarding_pool.py`,
`webui/onboarding.py`): the feed is built in a background BULK worker, never in a request, and
cached atomically for 24 h. The primary source is **21 public Deezer calls** — the global chart,
the editorial charts and the nine “Top <country>” playlists (FR, GB, US, DE, BR, MX, JP, KR, NG) —
not an undocumented country endpoint; the last good pool is kept when a rebuild fails. If Deezer
is absent or fails and the local library has fewer than 24 artists, **YouTube Music** adds 10
charts (`ZZ` + the nine countries) in 10 calls, cached (132 kB) and proxied through
`/api/onboarding/picture/...` so Google never enters the CSP. A like writes only `Artist` +
`StarredArtist` (idempotent, 5 statements warm, plus `EntityRef` for YouTube), removes an
`ArtistMute`, invalidates the taste profile and **never** creates a `Track`, archives, prefetches or
calls a provider; `Artist.prune` keeps a starred artist alive with no archived track. Related
artists (3–5, the provider's order, filtered for duplicates and diversity) are inserted just after
the liked card; a cold cache answers `pending` and a single worker fills it. Search is one provider
call cold (0 cached); the page is 3 statements / ~5 kB. `GET /api/onboarding/artists?limit=24` is
24 cards, `POST /artists/done` requires one artist unless every source failed, and
`POST /account/done` finishes the step.

**The onboarding account step** (`webui/onboarding_account.py`, blueprint
`onboarding_account_api`): `GET /api/onboarding/account/options` returns what the person can
actually do — Deezer (ARL or native, linked or not), YouTube Music (available/absent/disabled),
and the four public playlist imports — in **2 SELECT / ~931 B**, never a secret and never a
provider call. `POST .../link` reuses `myaccounts.link_deezer`/`link_youtube_headers`; a rejected
credential is a 400 `provider_refused`, an unreachable provider a 503 with `Retry-After`, and no
failure changes `onboarding_step`. `POST .../import` starts the existing `playlist_import` job
(202 `{id, job}`, progress at `/api/import/playlist/<id>`), reniced and behind the foreground; a
non-admin member may save the result but it is **not** bulk-archived. The tokens/ARL never enter a
log. `POST .../account/done` stays in the artists blueprint; every option is skippable. Measured:
options 2 statements / 931 B (391 B gzip), import request 2 statements / 1 980 B, link limited to
10 attempts/15 min/account, bodies capped at 8 KiB.

**The administrative trail** (`neezer/audit.py`, table `audit_log` schema `20261123`) records
account and invitation mutations — `user.create|update|delete`, `invite.create|revoke|renew|redeem` —
with the actor, the target, a short detail, the result code and the timestamp. It has **no foreign
key** on purpose: deleting an account must not erase who deleted it, and no token, password or path
ever lands there. Rows are written in the caller's transaction (a refused action stores its code,
a successful one cannot be lost) and read with `neezer-cli audit list [-n N]`. The table is additive
and idempotent on all three engines.

**The account dashboard** (`webui/admin_users.py`, blueprint `admin_users_api`) exposes
`GET /api/admin/users` (keyset `after`, `q` folded over `name_key`, the raw name and `mail`, so a
historical duplicate whose `name_key` is NULL stays findable, `limit` 1..200),
`GET /api/admin/users/<id>` with upload and playlist counts, `PATCH` (role/state) and `DELETE`,
which requires the exact account name in `{"confirm": "…"}` (400 `confirm_mismatch` otherwise).
Every non-admin gets a bare 403 whatever the id, so the dashboard is not an enumeration oracle.
`UserManager.update_guarded`/`delete_guarded` carry the **at least one active admin** invariant
(checked in the write and re-checked after it, since SQLite has no row locks), refuse
self-disable/self-delete and the holder of the server's Deezer account, bump `session_epoch` on
demotion/disable/re-enable (not on promotion), and reassign `Track.owner` to the acting admin
before deleting — no file is touched and no upload becomes public. The account's podcast shows are
reassigned to the acting admin in the same transaction (their archived episodes stay playable, the
response carries `podcasts_reassigned`), and its provider accounts are deleted with their grants
(Peewee would otherwise null the nullable owner and keep the credential serving). Pending invitations created by
the deleted account are revoked and its `u:<id>` grants forgotten. Measured at 50 accounts: list
**3 statements / 8 147 B (1 500 gzip)**, detail 4, promotion 3, demotion 5. The legacy `/user`
routes and Subsonic `createUser|deleteUser|updateUser` go through the same guards; the CLI keeps
the right to delete.

**Playlist folders** (`webui/playlist_folders.py`, schema `20261114`): a folder belongs to one
person and is private; `playlist_placement` is keyed by `(user, playlist)`, so two people file the
same shared or imported playlist independently. `GET /api/me/playlists` carries `folders` and each
playlist's `folder_id`/`position` in the same answer — **6 statements at 61 playlists**, 8 at 500
(the two chunked pairs double), and a playlist with no placement row is unfiled and sorts by
creation date, so an upgrade needs no backfill. The container order PUT is the complete desired
order but **additive**: ids are filed/moved in that order, a resident the client omitted is
appended, duplicates collapse, unknown ids are ignored, positions renormalise to `0..n-1` in one
transaction; 500 ids cost 6 statements. Deleting a folder returns its playlists to the root and
never deletes one. A container holds 2 000 ids, a person 500 folders; only own or shared playlists
can be filed. `/rest` is untouched. The blueprint installs its own same-origin guard (a Flask
blueprint does not inherit `webapi`'s).

**Collaborative playlists** (`playlist_access.py`, `webui/playlist_members.py`, schema `20261115`):
roles are `owner` / `editor` / `viewer`, plus the admin override. The owner is computed from
`Playlist.user_id` **first**, so a missing member row can never lock them out; `role_for` is the
single authorization authority (never the Subsonic serializer or a component). Every order-sensitive
mutation carries `version` (tracks and metadata) or `members_version` (roles); the compare-and-swap
`UPDATE … WHERE version = expected` runs inside the write transaction and a mismatch returns **409**
`{error:"conflict", scope, version, members_version, last_edited_by, last_edited_at}` — **135 B**,
never the track list — after which the client resyncs with one GET and says who changed it. Editors
add/remove/reorder tracks only; rename, delete, invite, role changes and the public link are
owner/admin. `/api/me/playlists` returns the caller's own lists plus their memberships, never an
admin firehose. Private uploads are filtered per viewer from tracks, counts, cover and recommendation
seeds through `Track.visible_clause`; hidden rows stay in the playlist and are anchored for reorder.
The same visibility now covers Subsonic `getSong`/`getAlbum`/`getArtist`/`getArtists`/`getCoverArt`
and the CSV/list export: a non-owner gets the same 70/404 and never the embedded cover bytes.
`access.snapshot()` is **fail-closed** — it keeps the last valid snapshot for the same database,
and an unreadable first snapshot opens nothing instead of granting the server account to `all`.
The core measures 6 statements for `/me/playlists` at 61 lists, 3/95 B for `/version`, 7 statements
for a 4 000-title page, and two concurrent writers at the same version produce exactly one 200 and
one 409.

The same authority closes the other doors. `/rest` **reads** a playlist for owner/editor/viewer/admin
(and a public playlist for any account) but answers **error 50** to a non-member; `getPlaylists`
lists owned + public + memberships, `songCount`/`duration`/`entry` count only the caller's visible
rows, and `createPlaylist`/`updatePlaylist`/`deletePlaylist` are owner/admin only (error 50
otherwise). `songIndexToRemove` is remapped through the visible list, so a hidden private upload
cannot shift the wrong row. The Deezer importer and `libraryimport` create the `owner` member row in
the playlist's transaction, bump `version` on an overwrite or a merge that adds rows, and a second
account importing the same canonical Deezer id gets a derived id — it never adopts somebody else's
row. `replace_everywhere`/`_remove_everywhere` touch owned **or edited** playlists and stamp
`version` in one UPDATE. `/api/export/playlist/<id>` and `/api/services/list/playlist/<id>` answer
**404** to a non-member and export only visible rows; the share GET/POST/DELETE is 404 to a
non-member and 403 without `can_manage`, while the public token payload stays read-only and carries
no members, versions or `last_edited_*`. `ratelimit.WindowLimiter` is a bounded, thread-safe LRU
(no Flask import): playlist creation is **60/h/account** and member writes **30/10 min**, returning
429 with `Retry-After`. Subsonic `getPlaylists` at 61 playlists is 66 statements (one owner join
saved 61 reads; the per-playlist aggregate remains in `db.py`), a ZIP export 8, a file list 4 and a
public page 6. `/rest/getPlaylist` at 4 000 titles is **9 statements / 0.33 s** (28 016 / 2.65 s
before): one joined `Track+Album+Artist+Folder+root` read, three prefetches and one credits query,
with byte-identical JSON. Creating or filling a playlist uses `Playlist.add_many` (one
`MAX(index)`, INSERTs of 300 in the caller's transaction): **13–14 statements** for 2 000 titles on
`/api`, **19** on Subsonic. Schema `20261122` adds the two real MySQL foreign keys to
`playlist_share` after purging orphans; `GET /api/podcast/<pid>` pages episodes (`limit` 1..200,
default 50, keyset `next`): 1 000 episodes cost **7 statements / 28 kB** per page instead of
553 582 B.

**A public playlist link** (`webui/playlist_share.py`, table `playlist_share` schema `20261113`,
`/pl/<token>` in `spa.py`): the token is `<id>.<hmac>` under the app secret (the remote-link
pattern), so a copy of the table opens nothing; one live link per playlist, revoked or expired means
404 on the very next request. The public page lists the playlist and plays ONLY files already on
disk (`streamable` checks `os.path.isfile`), never archives, transcodes or asks a provider; another
person's private upload is excluded from both the list and the stream. 120 stream requests / 10 min
per (token, IP), `no-store` + `noindex` everywhere. A 4 000-title page costs **5 statements / 50 ms**
and 10.2 kB gzip; Range works. `remote.POLICY`: the owner routes are `NEVER`, the public ones
`queue`.

**A public TRACK link** (`webui/trackshare.py`, table `track_share` schema `20261125`, page
`/t/<token>` in `spa.py`) is the share sheet's default, because a chat is a bad place to send an
audio file. The token is the same `<id>.<hmac>` capability; **one track only** (no route accepts a
media id from the visitor), one live link per track, and revoking it is dead on the next request
(including a disabled owner). The **server** renders the page, not the SPA: a preview bot does not
run JavaScript, and the recipient may be on a slow phone. Measured in `webapp/test/share/run.mjs` on
the built app and a real server: **12 390 B raw / 4 096 B gzip** of HTML, **one cover** (400 px;
640 px for the card) and **zero audio bytes until the play button** (`preload="none"` plus a
`data-src` assigned on click). The page is one public `GET` plus the cover; the Open Graph tags
(`music.song`, `og:audio` on the Opus stream, `og:image`), the oEmbed `rich` answer
(`/api/oembed?url=…`, 480×180 iframe at `?embed=1`) and the Twitter player fields are what the apps
read. The **public stream is Opus only** (128 kbit/s default, `q=OPUS_64..320`), from the archived
file through the normal transcode cache — never the FLAC, never an MP3 straight from Deezer, and no
provider call on any public route (a missing file is a 409). Creating the link archives the track on
the owner's own foreground action, like the file sheet; the default life is 30 days (`ttl: null` =
forever). 120 stream requests / 10 min per (token, IP) and 240 page/cover requests, `no-store` +
`noindex` + `no-referrer`, a per-response nonce CSP (`frame-ancestors 'none'` except the embed page).
Réglages → Compte lists the caller's live links and cuts them (`/api/track-shares`). `remote.POLICY`:
the owner routes are `NEVER`, the public page/stream/cover/oEmbed carry their own capability.

**Cleanup** (`neezer/deezer/cleanup.py`) is the **only** code in the project that deletes archived
audio. It is off by default, needs a free-space floor (`clean_min_free_gb`) to do anything, and is
event-driven like the rest — `prefetch._check_space` looks right after a download, the one moment the
archive can have crossed the floor. Guarantees that are **not** configurable: only rows with a
`deezer_id` are eligible (an upload exists nowhere else — deleting it destroys the only copy), and
the Track row survives, so the title keeps its place in every playlist and re-archives on the next
play (`last_modification = 0` is what marks it "not archived" everywhere). Configurable: what is
protected (favourites / playlist tracks / podcasts, all on), the staleness window, and the deletion
priority (`CLEANUP_ORDERS`). `/api/archive/cleanup/preview` shows exactly what would go, in order,
before anything does.

**Nothing else deletes an archive.** Unsubscribing from a podcast keeps every archived episode: the
channel is flagged `subscribed = False` instead of being deleted (only a show with nothing on disk is
removed), and Subsonic's `deletePodcastEpisode` reports success without touching the file. Same rule
for tracks: the importer keeps archived tracks Deezer stopped returning. And when a WHOLE show leaves
Deezer, `provider.get_show_page` raises `ShowUnavailable` (Deezer answering, never a network error),
the sync sets `PodcastChannel.gone` and the channel becomes a **local podcast**: episodes, art
(`library.save_show_cover` archives a `cover.jpg` beside the audio) and playback all come off disk,
and the sync stops asking about it until the verdict ages out (`importer.GONE_RECHECK`, 7 days).

**A redirect target and a log line are BUILT, never echoed** (`frontend/user.py`,
`tests/test_security.py`). `safe_redirect_target` validates `?returnUrl` and then hands back
`"/" + path + query + fragment` with every leading `/` stripped from the remainder: the client
controls no prefix, so no `//host`, `scheme:` or backslash form can leave the function even if a
later edit of the checks misses one. Its `urlsplit()` is wrapped, because `urlsplit("//[::1")` raises
`ValueError` — `GET /user/login?returnUrl=//[::1` answered **500** instead of the form. The two login
log calls pass the user name and the address through `_log_text` (line breaks escaped, other control
characters replaced): `logsafe`'s record factory already does this for every `neezer.*` logger, but
the blueprint is also imported straight into a WSGI stack that never runs `neezer.web`.

**A suppression must name its rule.** `# nosec B608 - why` silences B608 (bandit reads the ids and
ignores the words after). `# nosemgrep: <rule-id> -- why` must carry the **rule id**: the
`# nosemgrep: a plain-text playlist, not an HTML page` in `frontend/playlist.py` silenced nothing
and its alert stayed open; a template uses `{# nosemgrep: <rule-id> -- why #}`. A false positive gets
the id and the true reason, never a blanket file. Bandit's `nosec` removes its finding from the SARIF
outright; Semgrep still emits it with `suppressions: accepted`, which GitHub code scanning lists as an
**open** alert — so the alert itself is dismissed one by one (`dismissed_reason: false positive`, the
same reason as the inline comment), never in bulk and never by disabling a rule. `db.PathMixin`'s path hash stays
`sha1(..., usedforsecurity=False)` — it is a unique-index KEY, not a signature, and every row of
every existing database already holds those 20 bytes, so a SHA-256 switch would fail to find them
all and duplicate the library. The same scanners call `0o700` on the daemon's socket directory
"widely permissive" (`daemon/server.py`), which is inverted: 0o700 is owner-only.

**This lot's numbers, and the two cases.** The 24 `py/log-injection` alerts of the `neezer/webui/`
routes — `request.path` in the shared error handler of seven blueprints, a Deezer id, an account
name, a party id, a search query, over 16 lines in 11 files — are covered by the hook above and were
closed as false positives, with every one of those lines left exactly as it was. That claim holds
only while the value reaches `logger` as a %-argument: an f-string carries it inside the message,
where the factory has no argument left to escape. `py/stack-trace-exposure` is the other case, with
no guard at all: answer with a constant, never `str(exc)` — **except** a `ServiceError`, whose
message is one of this server's own literal sentences
(`tests/test_service_error_messages.py` walks the AST of `neezer/services/` and fails on any
`raise ServiceError(...)` handed anything but a literal, or an f-string of an integer status code),
which is why the four such answers in `webui/services.py` and `webui/onboarding_account.py` were
closed as false positives too. A line a scanner flagged stays **untouched**: rewriting it moves the
alert onto the new line instead of closing it, so a false positive is dismissed by the API, and only
a real problem changes the code.

**Code scanning in this area** (CodeQL, Semgrep, Bandit): a finding is settled on the line it points
at — `# nosec B603 - fixed argv, the media path is one element, no shell` on every ffmpeg call,
`# nosemgrep: <full rule id>` on the line **before** for Semgrep — never by disabling a rule. A
swallowed exception that hides a real failure gets a `logger.debug(..., exc_info=True)` instead, and
only teardown (a `close()` in a `finally`) keeps a bare `pass`.
**Blowfish and ECB are false positives to DISMISS, not to silence in `decrypt_blocks`.** Deezer's
stream format picks the cipher (BF-CBC), the key and the chaining; the ECB object is the raw block
primitive under that chain and nothing of ours is protected by it (`provider.decrypt_blocks`,
byte-identical to CBC, checked against OpenSSL in `test_deezer_stream`). Putting `# nosec B304` /
`# nosemgrep:` there is tempting but **turns the PR's code-scanning gate red**: CodeQL's alert
fingerprint covers the enclosing function, so any edit inside `decrypt_blocks` re-reports the
pre-existing `py/weak-cryptographic-algorithm` alert as "new in this pull request" (measured: three
pushes, the gate stayed red until the function was byte-identical to master again). Same for
`provider.resolve`, which carries a `py/log-injection` alert: leave those two functions alone, or
expect the gate to complain.
**Log injection needs no per-call escaping:** `neezer/logsafe.py` escapes the line breaks of every
record's ARGUMENTS, so untrusted text is always passed as an argument and never interpolated into
the format string (`tests/test_security.py` pins it; the per-site tests in
`test_deezer_lyrics`/`test_deezer_resilience` fail if one is turned into an f-string).

**Scanner suppressions in `neezer/api/`, `neezer/managers/` and `neezer/lastfm.py`** state the
mechanism, never "safe". `media._popen` receives an argv list built from the operator's TRANSCODING
config, so a title or a path is one argument and never a command (the only client-chosen parts are an
`int` bitrate, a `[a-z0-9]{1,8}` format and a Decimal offset). The Subsonic token `md5(password + salt)`
and Last.fm's `api_sig` are md5 **by wire protocol**, and the SHA1 branch only verifies a pre-argon2
hash that the same successful login rewrites to argon2: those three digest choices are what keeps an
old client and an old database working (rule 7), so their lines are left exactly as they were — an
inline `# nosemgrep` does **not** close an alert here (Semgrep still writes the finding to SARIF with
`suppressions: inSource`, and code scanning raises it), and editing only that line turns an existing
alert into a blocking "new alert in changed code". A rule that must stay is either restructured until
it no longer matches (`timeOffset`/`gain` parse through `Decimal` + `is_finite()` instead of `float`)
or its alert is dismissed with a justification. A failed login goes through the package logger, so
`logsafe.py` keeps a user name on one line (the CodeQL log-injection alerts are false positives for
that reason: never format such a value into the message with an f-string, which bypasses it).
`/getCoverArt` treats `Folder.cover_art` as a file NAME (`basename`), and art answered by the
provider is confined to the archive and cache roots (`_confined_file`: `realpath` then
`startswith(root + os.sep)`), so a hand-edited or restored row cannot read outside the library.

**A transcode target's Content-Type comes from our table, not from `mimetypes`.** `guess_type`
reads the interpreter's table plus the host's `/etc/mime.types`, and the image installs the codec
tools, not Debian's `media-types`: `.ogg` was missing there, so every Opus/ogg transcode was
announced `application/octet-stream` and a player that trusts the header refused a stream whose bytes
were audio. `neezer/media_types.py` lists the closed set of targets; `neezer/api/media.py` asks it
(`tests/api/test_transcode_mimetype.py` empties the host table and still requires `audio/ogg`).
`neezer/db.py`'s `transcodedContentType`, for a song and for a podcast episode, reads the same table:
it announces the same type to the client, and the same test holds it to `audio/ogg`.
