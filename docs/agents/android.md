# Android app

Reference notes for agents working on this area. Read [CLAUDE.md](../../CLAUDE.md) first: its process and rules apply here. Other areas: [docs/agents/](./).


`android/` is a native Kotlin app: a fullscreen WebView hosts the SPA (`<server>/app/`,
configured at first launch: URL + optional port + SSL-verify toggle for self-signed certs)
while `PlayerService` (foreground service + MediaSessionCompat) keeps the process alive in
the background and owns the media notification / lockscreen / Bluetooth controls. Audio
stays in the WebView — the native side is only a remote control + keep-alive. The bridge is
`android/app/src/main/assets/nsshim.js`, injected at document start: it REPLACES
`navigator.mediaSession` and forwards the metadata/playbackState/positionState/action-handlers
that `Player.svelte` already maintains to `window.NSNative.publish()`, and routes transport
commands back via `window.__nsNativeCmd()`. No webapp-side code is involved, so the app works
against any deployed SPA version. Build: `gradle -p android assembleRelease` (JDK 17, SDK 34 —
CI does this; no wrapper is committed).

**Trust boundaries** (audit NS-19). "Verify SSL" unticked is **trust on first use**, never
trust-all (`Ssl.kt`): the setup screen learns the certificate with a handshake it refuses (nothing is
sent), shows its SHA-256 and pins it (`Prefs.pinFor`, per origin); the WebView pins on its first
`onReceivedSslError` if setup could not; afterwards a chain the system accepts OR that exact leaf
passes, a different one is "le certificat a changé", and re-saving the settings re-pins. Another
host is never relaxed. `Prefs.originOf` is the ONE definition of "our server" (scheme, host, port)
for the navigation guard, the shim, the bridge, the pin and the native fetches. `NSNative` answers
only while the top frame is the server's (`onServerPage`, raised by `loadServer` before the page
exists — the shim calls it at document start, before `onPageStarted` arrives). The manifest must
allow cleartext (a LAN server's host is unknown at build time), so http is narrowed in code to the
configured server: `shouldInterceptRequest` for the page, `Net.get` for native fetches — which also
follows redirects by hand so the session cookie only ever goes to the server. Mixed content is
never allowed; backup and device transfer exclude everything (`data_extraction_rules.xml`).

**Surviving a long background stay.** Everything the app *is* — the audio, the queue, the UI —
lives in the WebView's renderer process, which Android may kill whenever nobody is looking at it;
the app process itself is only safe while the foreground service is up (it isn't, once playback
wound down or the paused notification was swiped away). A killed renderer leaves a WebView that
can only ever show a blank page again, which is why "paused in the background too long" used to
mean "restart the app". So MainActivity **rebuilds** the WebView (`rebuildWebView`, on the screen
`doUpdateVisitedHistory` last saw) instead of restarting the activity, and does it **when the user
comes back**, never in the background — a background reload can fail (off the home LAN) and park
the app on an error screen, and the renderer it builds is just as killable. `onRenderProcessGone`
therefore only flags `pendingRebuild` (and drops the now-dead media notification); `onResume`
rebuilds it, retries a load that failed while we were away, and — after a long absence, and only
while nothing is playing — pings the page and rebuilds it if it doesn't answer, which is the one
thing that catches a renderer wedged rather than killed. The SPA persists its session, so a
rebuilt page comes back on the same track at the same position. Having actually caught Android
doing it is also the only moment the app offers the battery-optimization exemption
(`offerBatteryExemptionOnce`, once ever) — the setup screen keeps the same button.

**Updating itself from the server** (`Updater.kt`, `neezer/webui/androidapk.py`, `lib/appversion.js`). The app
fetches its update from ITS server, never from a page it sends the user to: `GET /api/android/apk` (login) serves
`[webapp] android_apk` (an operator's own build) or the GitHub release asset `neezer.apk` of the declared
`android_version` on the repository `android_url` points at, downloaded once into `<cache_dir>/android/` (only
https to GitHub's hosts, a zip or nothing, 200 MB cap). `/api/version`'s `android.apk` says it can. The native side
checks the APK BEFORE asking Android — same package, higher versionCode, signed with the installed key (lineage
included) — so the page can say "signature différente" (a CI build without the signing secrets has a throwaway key)
instead of an installer error; then the "install unknown apps" permission (settings opened, resumed on return) and
the system installer (ACTION_VIEW on a FileProvider uri). `NSNative.bridgeVersion()` (2) is what the page may ask of
an APK: an older one answers nothing and keeps the "Télécharger" link. Progress: `window.__nsUpdate({state,…})`.
This is the delivery channel for the native audio work (study: `/mnt/project-files/natif/etude-rust-natif.md`).

**The native audio core** (`android/nscore`, Rust, `NsCore.kt`; lot N2 of the native-audio plan). The `rhythm`
crate — the analyser the web app runs in its AudioWorklet — compiled for the device, plus `dsp.rs`: the output chain
graph.js builds from Web Audio nodes (ReplayGain ramped, the equal-power fade on its quarter circle, the ten peaking
bands at Q 1.1 and the 120 Hz low shelf, the BiquadFilterNode formulas, and the bass lift's two DynamicsCompressorNodes — its compressor and the limiter after it — on Web Audio's soft-knee curve with its makeup gain, linked across channels; a chain with nothing on multiplies by the gain and nothing else).
Reached over JNI written against the C ABI (`jni.rs`: NewStringUTF and NewDirectByteBuffer at their jni.h slots, every
other argument a primitive); each object is a handle whose buffers are handed to Kotlin ONCE as direct ByteBuffers, and
0 is a no-op everywhere. `node test/compare.mjs` (after `cargo build --release`) drives the host build through those
very entry points from a desktop JVM (`NsCoreCheck.java`) and holds it to rhythm.wasm on six arranged records: no event
on a different frame, every field within 1e-3 of its value (measured: under 1.2e-5 — libm rounds differently in the last
bit, so equality is not the contract). `nscore/build-android.sh` builds arm64-v8a, armeabi-v7a and x86_64 with the
NDK's clang (16 KB pages) into `app/src/main/jniLibs/` (git-ignored; android.yaml runs it before Gradle). Optional:
an APK without it has `NsCore.available` false. `NSNative.nativeCore()` reports it (bridge version 3).

**The native player** (`NativeAudio.kt`, `lib/audio/nativeplayer.js`; lot N3). Réglages → Audio → *Lecteur de
l'application* (`audio.native.choice`, offered from bridge version 4; ON by default from bridge 6, where the app's player carries
the EQ, the bass lift and the animations too; `null` = the app decides, an old `audio.native` true carries over) makes the page's audio elements
Media3/ExoPlayer players in the app's process. Player.svelte is unchanged in logic: `makeEl` gets a
`NativeAudioElement`, the HTMLMediaElement subset it uses (src/load/play/pause/currentTime/volume/playbackRate/
buffered/the events), so the queue, the crossfade on the volumes, the party host and the timekeeper all read it as an
element; the setting changing hands the elements over like leaving Web Audio does (`stale` in Player.svelte), and the
plan is direct with `why: "native"`. The wire is `NSNative.naCreate(pageToken)` / `naCmd(id, json)` and
`window.__nsAudio(id, state)`; every command carries a sequence number the state echoes, and a field is adopted only from
a state that has seen the last command that could change it (a tick in flight must not undo a pause or a seek).
`currentTime` is interpolated between the 250 ms ticks and never steps back by less than 50 ms. Sources are the server's
URLs only, fetched by `ServerDataSource` through `Net` (Range, cookie, pin); one audio focus request for every player (a
crossfade plays two); players belong to the document that made them (a new token releases the old document's,
`pagehide` and the WebView's death release them). A `blob:` source (an IndexedDB download, until N4) plays on a real
`<audio>` inside the element.

**Effects and animations on the native player** (`NativeAnalysis.kt`, lot N5, bridge version 6). Each player's PCM
goes through `NsProcessor`, a Media3 `BaseAudioProcessor` in its `DefaultAudioSink` (16-bit, before Sonic): the Rust
chain with the EQ and the bass lift (`NSNative.naFx`, sent by `nativeplayer.js#sendFx` on every change of the three
stores), then a copy to ONE rhythm analyser — the player told to play last. Its frames go to the page in batches
(≤ 16 or 40 ms, base64) through `window.__nsFrames`, each stamped with the MEDIA time its audio starts at;
`engine.js` (`startNative`, `rh.native`) drives it with the worklet's own messages through `NSNative.naAnalysis` and
holds each frame until the element's `currentTime` (minus the trim) reaches it — ExoPlayer's position already counts
the output path. A frame 3 s ahead of the clock or 1 s behind it is a seek's leftover and dropped. Nothing passes
through the chain while there is no effect and no view (a copy, bit for bit). The normalization still rides on the
element's volume; the oscilloscope has no waveform on this path. `test/nativeanalysis.test.mjs` plays the app with
the shipped rhythm.wasm on an arranged record.

**Behind a locked screen, and seeking a stream with no length** (bridge version 7). With the sound in the app, the
page is SILENT to Chromium, and a hidden silent page is throttled then frozen: a track's `ended` reached it only once
the phone was unlocked, so nothing played after the first track. While the page is hidden, `Player.svelte#updateNativeNext`
names the next ten tracks of the queue (`NativeAudioElement#setNext` → op `next`: each one's URL — the server's, or a
download the app keeps —, its trimmed start, its volume, its key and what the notification shows); the player told to
play last goes on to each by itself at its end (`NativeAudio.kt#advance`, the notification updated from `meta`), and
every state carries `k` until the next source, so a page that heard nothing while frozen still learns it. The element
then dispatches `nsadvance` and nothing else until `onNativeAdvance` jumps the queue to that track and `loadTrack`
ADOPTS the source where it has reached (no reload, no seek). Visible, nothing is named, and repeat one, the sleep timer's
end of track and a page download only it can read end the list. And a first play (or the first at an Opus quality) is
generated as it is sent — no length, no ranges — so ExoPlayer has no seek map and the page waited for a `seekable` that
never came: the bar moved, the sound did not. The player's buffer now holds a whole track (32 MB, so the stream is read
to its end and the server keeps its copy), the state says `sc` (the element is seekable everywhere) and a seek there is
HELD (`chase`: the position reported is the target) until the stream has been read whole, then the same URL is opened
again — a file with ranges now — at the target, muted until it lands; 60 s or four reopenings and it gives up.
`test/nativeplayer.test.mjs` and `node test/native/run.mjs` (the page frozen through two track ends; a seek on a stream
with no seek map) pin both.

**The app owns the track changes while its activity is stopped** (bridge version 8). Bridge 7 still failed on a phone,
because a hidden WebView is not FROZEN, it is THROTTLED: Chromium holds a hidden silent page's timers to once a second,
then once a minute after five minutes (developer.chrome.com, "timer throttling in Chrome 88"), and every event the app
sends it still runs. So the page heard the app's `timeupdate`s, began a crossfade or a trimmed ending itself, cleared
the list while arming it, and stalled in its own throttled steps; and the list was only named at the instant the page
was hidden — an IndexedDB read per track on a page about to stop running. Now: the list is kept up to date while the
page is VISIBLE too, per element (a quality switch or a crossfade hands the track to a player that was told nothing),
and is kept while a crossfade is only being armed; each entry carries where its audio ends (`e`, the silence trim; the
bounds of the next `EDGES_AHEAD` tracks are asked once per session) and the current track's (`end`). `MainActivity`
tells `NativeAudio` when it starts and stops (`setForeground`); only while it is stopped does a player follow the list
(`followsList`) — at its file's end, or at the trim, timed on its own clock (`checkTrim`: the 250 ms tick, then a timed
last stretch; measured in the bench 5.486-5.495 s for a trim at 5.5) — and every state says `bg`. The page reads it
(`appInBackground`) and starts no transition of its own then (`nativeOwnsTransition`: no crossfade — an armed one is
dropped — and no trimmed ending), so behind a locked screen tracks cut at their trim with no overlap, and in front of it
nothing changed. `node test/native/run.mjs` drives it on a hidden page whose timers and frames are held but whose
events run, crossfade and trim on (`--old` runs that scenario as bridge 7: the page crossfaded by itself, the app never
advanced).

**Next / previous from a media button, with the page asleep** (`PlayerService` -> `PlayerService.nativeSkip` -> `NativeAudio.skip`).
The notification, a headset or the lock screen used to send `next` / `prev` to the PAGE (`commandSink` -> `evaluateJavascript`): with the
activity stopped that page is throttled (timers once a minute after five minutes) or frozen, with the activity gone `commandSink` is null
and the command vanished — "after a while in the background I can no longer skip". Media3's model is the opposite: the player that owns
the playlist answers the command, whoever asks. So while the activity is stopped the app's player does it itself, from the list the page
named (bridge 7/8): next = `advance(next.first())`, previous = restart the track past 3 s, else back to the one this player went through
(`trail`, up to 20, the track left returns to the front of the list); each tells the page (`adv`, `onAdvance`) so it adopts the change when
it wakes. Not ours to do (the page is shown, no list, nothing before) -> the command goes to the page as before. The list is only what the
page named (<= 12): past it the app cannot know the next track, and a skip goes to the page. Sources: developer.chrome.com "Timer throttling
in Chrome 88" and the Page Lifecycle API (frozen pages run no timers), Media3 `MediaSession` / `MediaController` docs (commands go to the
session's player). Unit-untestable (ExoPlayer): the build is CI's `android` workflow, the logic is read against `advance`.

**When the app's player fails** (lot N6, `nativeplayer.js#noteNativeFault`, `Player.svelte#fallBackToPage`): a player
error (MEDIA_ERR_DECODE, i.e. ExoPlayer's decoder or audio-track codes, or `naCreate` refusing) is recorded in
`audio.native.fault` BEFORE the element's error fires, so `nativeAudioWanted` is already false and the playing track moves
to a page `<audio>` at the position it reached instead of being retried on the same player. A network error is not a fault.
It holds for a day and for that app version only (an update tries again); Réglages says so under the switch, which still
shows the setting, with *Réessayer maintenant*. `node test/native/run.mjs` drives both (bridge 6 default, a code 3 fault).

**Downloads in the app's storage** (`OfflineStore.kt`, `lib/nativeoffline.js`, `lib/offline.js`; lot N4, bridge
version 5). On an APK that has it, a download's AUDIO is a file in `filesDir/offline`, fetched by the app itself
(`Net`: cookie, pin, `Accept-Encoding: identity`), named by a hash of the page's key — no key names a path — and
published only once the whole body is there (an empty or short body is an error, the `.part` dropped). The page keeps
what it LISTS (the `meta` record, now `native: true`, and the cover) in IndexedDB and plays the file from
`<origin>/__nsoffline/<key>`, which `MainActivity.intercept` answers from disk with byte ranges — for the WebView and
for the requests the service worker passes on (`ServiceWorkerController`) — and which the native player reads straight
off the disk (`NativeAudio`'s `localFile`). The WebView applies a request's `Range` ITSELF to what
`shouldInterceptRequest` returns (it reads `available()` as the whole size and skips the start), so the
body is the file from byte 0, cut at the range's end (`RangeBody`, `RangeBodyTest` models Chromium's
loader): a stream already positioned at the start was skipped twice, and every read past the first
broke a download on the page's own `<audio>`. The Blobs an older APK left in IndexedDB move over ONCE at launch
(`offline.js#moveDownloadsToApp`): 768 KiB pieces as base64 through `offPut` (each must start where the last ended),
`offCommit` checked against the Blob's size, and only then the IndexedDB copy is deleted; a refusal leaves the page's
copy exactly as it was, and nothing moves with less than the Blob + 64 MB free. At launch a file no record lists is
dropped and a record whose file is gone is forgotten. `test/nativeoffline.test.mjs` (the logic) and
`node test/offline/run.mjs` (Chromium: the move byte for byte, the play off `/__nsoffline/`, a new download, the
Réglages list at two widths) pin it.

**Naming the audio output** (`AudioRoute`): a fault measured on the processed path sends ONE
CONNECTION of an output direct (see *Where the sound goes* in [playback.md](playback.md)), and a WebView cannot see what it
plays to, so the activity lists AudioManager's output devices (no permission needed; a Bluetooth
device's product name is its advertised name), ranks them the way Android routes media — Bluetooth
(A2DP, LE, hearing aid), USB, wired, HDMI, speaker — and pushes `{kind, name, conn}` to
`window.__nsAudioRoute` on every change (`registerAudioDeviceCallback`); the page reads the first one
through `NSNative.audioRoute()`. `conn` changes whenever the chosen device does (the radio gone and
back is two changes), prefixed with the process's start time so a stored verdict never matches a
later process's connection by accident. The name is reduced to printable characters before it
reaches a page. An older APK reports nothing: verdicts are then per page's own guess of the output
(the latency its context reports) and live until their retry.

**The car mode's triggers** (`CarTrigger`, `CarMotion`; lot E, native only — no bridge change).
The overlay's visibility is ONE formula, in `CarTrigger`: `(gpsTrigger && a listed navigation app
is in front) || (bluetoothTrigger && an A2DP/BLE output is connected)`. No AccessibilityService:
foreground apps come from `UsageStatsManager.queryEvents(now - 5 s, now)` on a worker every second,
the LAST `ACTIVITY_RESUMED` winning and staying sticky across polls (a navigation app that has been
guiding for ten minutes emits no new event, and a 5 s window alone would call it gone), plus a
one-off 6 h seed at start for the app already in front. `queryEvents` returns null while locked
(the previous state is kept) and needs usage access (without it the trigger stays off, no crash).
The screen being off skips the query; Bluetooth sinks are `AudioManager.getDevices(GET_DEVICES_OUTPUTS)` and
`registerAudioDeviceCallback` (no permission; a headset counts like a car radio, hence the switch).
Speed: fused location 1 Hz high accuracy plus `lastLocation` only when younger than 30 s; after 3 s
without a live fused callback the platform GPS takes over for good. `Location.getSpeed() * 3.6`;
5 km/h and above is moving at once, below it the answer stays "moving" for 5 s (the red light), and
an UNKNOWN speed — no fix, a fix without speed, 5 s of silence, no location permission — is always
"moving" (no crash, keyboard stays restricted). Listener on the main thread, `start`/`stop`
idempotent; `CarMotionTest` (JVM) holds the 5 km/h / 5 s thresholds. Android 10+ requires a
foreground service reading location while its activity is not
visible to declare the `location` type (`developer.android.com/develop/sensors-and-location/location/permissions`):
the car service (lot A) must add `FOREGROUND_SERVICE_LOCATION`, `specialUse|location`
(plus `microphone` once the mic grant exists) and start it with all of them, or the speed stays
unknown while Maps is in front. The `location` type is while-in-use:
on Android 14 the service must be created from a visible app or an exempt start (tile, notification),
so the permission is granted before the service starts. This lot adds no dependency:
`play-services-location` is lot A's.
**The car's sleeves are real covers, not only grooves** (`CarCoverView`, `CarCovers`; no bridge
change). The overlay, the playlist tiles and the settings rows draw the album/playlist image when
one exists and keep the deterministic Sillon grooves as the placeholder (and as the final answer on
failure). The loader is one daemon thread, an in-memory `LruCache` sized to 1/16 of the heap
(3–8 MB), and a `filesDir/car-covers` disk cache whose file name is the SHA-256 of the URL — never a
path built from the URL. A tile asks at its own dp size; the decode is bounded by
`BitmapFactory.inSampleSize` and the encoded body by 4 MB; the disk directory by 24 MB / 300 files,
trimmed oldest-first at most once a minute. MINI, FULL and ULTRA ask for the same URL and share one
transfer (`CoverFanout`; a request from a newer track wins, the older answer is dropped by
generation). The session cookie is read only for the configured origin and `Net` enforces the same
rule again on every redirect; a failed URL is remembered for 60 s so a repaint never retries it.
**A cover URL is attacker-influenced, so it may only make the device fetch the configured origin or
Deezer's image CDN** (`*.dzcdn.net`, HTTPS only): `Net.get` takes an optional `allow` predicate,
asked on the first URL and on every `Location` before a socket is opened, and `CarCovers` passes
`coverAllowed`. A URL with userinfo, a private/loopback/link-local IP literal (`127.0.0.1`, `10/8`,
`172.16/12`, `192.168/16`, `169.254/16`, `[::1]`, `fc00::/7`, `fe80::/10`), another host, or
cleartext off the configured server is refused; the cookie still goes only to the configured origin
and the disk name is still the URL's SHA-256, never a path. A
closed settings screen cancels its queued loads before any of them opens a connection, and the
settings list fetches at most 12 sleeves (the pinned rows first): opening a 500-playlist catalogue
never fans out 500 requests. The request policy is one transfer per distinct URL: a panel page is
at most six cold GETs (zero once the process-wide read or the disk has them), and the fanout holds
ten simultaneous callers on a single transfer (`CarCoverCacheTest`). The disk bound is held by
a test too: ten 3 MB files under a 24 MB budget evict the two oldest; the file name is 64 hex
characters, never a host or a slash. The memory hit is a synchronous `LruCache` lookup that hands the
bitmap to the view's `invalidate()` in the same frame; no device was available for a frame-time
measure in this lot.

**The car overlay is a foreground service, not a second player** (bridge version 9, `CarModeService`,
`CarOverlayRoot`, `CarTileService`). It binds the same `PlayerService` (BIND_AUTO_CREATE), reads
`state()` and registers `onState`, and sends `transport(cmd, value)` — the MediaSession's own routing,
native skip first. The overlay is `TYPE_APPLICATION_OVERLAY`, `FLAG_NOT_FOCUSABLE
| FLAG_NOT_TOUCH_MODAL`. **The window is exactly the painted surface**: FULL/MINI are
`screen − 16 dp` wide at `x=8 dp`, bottom 92 dp; ULTRA is exactly 76 × 76 dp at `x=14 dp`,
bottom 104 dp (`WindowManager` interprets positive x/y under `END`/`BOTTOM` as distance from that
edge). No transparent side padding and no margin inside the window: those bands still receive
touches and used to swallow the GPS app's taps (a 90 × 180 dp window for a 76 dp button). Cross-window
blur is requested only for FULL/MINI and only where the WindowManager reports it enabled (a square
of frost around the round ULTRA disc would be visible). The window alpha is 1: Android 12's
untrusted-touch cap applies to a window that lets a touch PASS THROUGH it, and this one is touchable
and sized to the card, so nothing passes. The person's 40–80 % opacity (default 62) is the card
FILL's alpha, `CarOverlayRoot.setCardOpacity`, as in the prototype's `rgba(14,14,12,glass)`:
titles (26 sp), labels (19 sp) and the lemon play button stay at 100 %. The toast is a separate
`FLAG_NOT_TOUCHABLE` overlay window at top 206 dp, 16 dp sides, alpha 0.8 (the highest value Android
12 lets a pass-through window use), so it never moves or widens the card. The service is
`specialUse|location|microphone`: `specialUse` keeps the overlay above the navigation app,
`location` keeps the speed updates coming once the activity is behind Maps, `microphone` keeps the
search keyboard's voice input alive; each bit is only requested when its grant exists, and a refused
union falls back to `specialUse` rather than dropping the overlay. Auto-start needs a configured
server, a trigger on, overlay access and usable notifications; the tile or the settings screen starts
it by hand. **There is one door to the settings**: the notification's Réglages action is a
`PendingIntent.getService` with `ACTION_OPEN_SETTINGS`, the SPA bridge calls
`CarModeService.openSettings(context)`, and the playlist ghost tile goes through
`CarHost.openSettings()`; all three lead to the same `openSettingsNow()`, which detaches the overlay
before launching the activity (with no service alive the helper launches it directly, since nothing
draws). MainActivity's next `onResume` sends `ACTION_REFRESH` — and a navigation app becoming visible
again clears the suppression — so `CarTrigger.refresh()` re-reads the state and the overlay
reattaches once (guarded by `overlayAttached` and `!dismissed`). `MainActivity` owns the `jsSink` into
the SPA (`__nsCarPlay` / `__nsCarFav`). `PlayerService.evalJs(js, onResult)` carries the page's
boolean answer back (false when no WebView/never our origin): `launch()` changes the screen and toasts
"Lecture : X" (or "Playlist lancée : X" for a playlist) only on true, and otherwise shows "Ouvre Neezer pour lancer la musique" on the current
screen — no bridge change, the callbacks are the existing hooks' return values.

**Linking the Deezer account from the app** (`DeezerLoginActivity`): there is no email/password
endpoint left that a server could call — Deezer's web one (`ajax/action.php` + reCAPTCHA) answers
403 since 2024, and the mobile gateway (`mobile_userAuth`, which *does* return an ARL) needs keys
extracted from Deezer's own binaries. So the only remaining path to an ARL is a real browser
session, and the app provides one: a throwaway WebView loads **Deezer's own login page**, the user
signs in there, and the `arl` cookie Deezer sets is read out of the jar and handed back through
`window.__nsDeezerArl` to `Réglages → Compte Deezer`, which saves it via `/api/settings` (already
validating + probing it). **The password never reaches the app or the server**; only the ARL
crosses back, and only to a page served by the configured server (`MainActivity.deliverArl` checks
the origin). That WebView gets **no `NSNative` bridge**. The Deezer jar is wiped on entry (so the
ARL is always the account just typed) and again on success (no second copy left on the device) —
scoped to `deezer.com`, never `removeAllCookies()`, which would log the user out of their own
server. Cookie arrival is polled (~600 ms, only while the screen is up): Deezer's login is an XHR,
so no navigation fires and there is no cookie-change callback to hook. Google/Apple SSO is refused
inside any WebView — email + password is the supported route. SPA side: `lib/nativeDeezer.js`.

**Car mode, the page side and the pinned playlists** (bridge 9, lots A–E; SPA: `lib/car.js`,
`CarPlaylistsView.kt`, `CarSettingsActivity.kt`). The overlay is native and the WebView still owns
the queue, so the two sides meet at exactly two hooks, installed once by `App.svelte` (never on a
party/remote page): `window.__nsCarPlay(kind, id, title, payload)` launches a Flow (`startFlow`), a
track from its JSON (`player.playQueue`, context `track`), or an entity (`playEntity(kind, id)`) —
never a second playback path — and returns a boolean the service can report. `window.__nsCarFav()`
exists only while a track is current: it toggles that track's favourite and returns the optimistic
new state synchronously. `NSNative.carAvailable()` gates the SPA's settings card and the playlist
page's “Épingler pour la voiture” action (`NSNative.carPin`); an older APK has neither method and the
entries simply do not exist, while a newer APK facing an older SPA finds no hook and must say so.
`carPin` accepts two arities (the WebView bridge resolves an overload by argument count): the old
`(id, title)` and `(id, title, nbTracks)`. `lib/car.js` passes the count when the playlist page has
it (`data.playlist.nb_tracks`), so the tile says "42 titres" before the first `/api/me/playlists`
read; a page without a count still calls the two-argument form. `openCarSettings()` answers a real
boolean (`true` only when the launch was accepted); `lib/car.js` requires `=== true`, so an APK
from before the return value is reported as "Réglages indisponibles" by the new SPA instead of a
silent success. Older SPA + newer APK: the extra return value is ignored, and the two-argument
`carPin` overload is still there — no bridge version bump.
The playlist panel shows Flow + `CarPrefs.pins`, six 84 dp tiles to a page; its catalogue is
`/api/me/playlists` read natively by `CarPlaylistSource` through `Net` with the WebView's cookie
(401 = session absent, not “server down”), and a failed read falls back to the pinned titles the
device remembers. The endpoint is built from the **server root**: `Prefs.baseUrl()` may carry the
SPA's `/app` path (a person pastes the player's address) and the route never lives under it, while a
reverse-proxied prefix is kept (`https://host/musique/app/` → `https://host/musique/api/me/playlists`);
`Prefs.isServerUrl` still holds it to the one origin definition. If the jar has no cookie yet (the
tile can open the settings before the page ever loaded), one `CookieManager.flush()` and a second
read happen off the UI, then the request is still made without a cookie: its 401 or its login-page
HTML is more informative than "server down". The states are kept apart — session absente (401),
hors ligne, délai dépassé, certificat refusé, serveur en erreur (403/5xx), réponse inattendue (HTML)
— and every failure is retryable; the pinned rows are always announced as local ("épinglées").
A successful read is cached 60 s process-wide (`CarPlaylistSource.fresh`, covers included): the
overlay is destroyed and recreated on every drop/reattach, and a fresh cache means returning from
the settings does not cost a second request; a 401 forgets it. The panel labels a cache-served list
"catalogue mémorisé", so a stale list never passes for a live one. `CarSettingsActivity` seeds
its rows from that same cache on open (instant covers, no request), then re-reads on `onResume`
when the last attempt failed or the last success is older than 60 s, at most once every 2 s; a 401
there also forgets the process cache. Tapping a tile calls
`CarHost.launch` only — the service changes to the player and shows the single "Lecture : X" / "Playlist lancée : X" toast
when `__nsCarPlay` returned true, so the panel no longer adds its own "Playlist lancée".
The native search (`CarSearchView`) waits **700 ms** after the last key and needs **2 characters**
before calling `/api/search` (one call fans out to five provider searches), cancels the previous
`HttpURLConnection` on a new keypress or detach, and lets a late answer from an older generation be
dropped. Its suggestion strip is `WRAP_CONTENT` (GONE when empty), so a 640 dp screen fits the ABC +
passenger keyboard: 532 dp panel + 92 dp bottom margin with three results, 16 dp of headroom.
`CarSettingsActivity` is the full-screen Réglages → Mode voiture (manifest
`android:exported="false"`; reached only through `CarModeService.openSettings`, which hides the
overlay first), asks overlay, usage-access, location, Bluetooth and microphone through
`CarPermissions`, and never shows a confirmation dialog. `webapp/test/car.test.mjs` drives the hooks
with injected player functions
(launch kinds, no session, remote, the favourite hook appearing/disappearing); `npm test` 439 and
`npm run build` pass. The Kotlin side is built by CI's `android` workflow; `assembleDebug`,
`lintDebug` and `testDebugUnitTest` also pass locally with JDK 17/SDK 34.
