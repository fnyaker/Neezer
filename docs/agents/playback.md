# Playback: crossfade, output path, app core, list performance

Reference notes for agents working on this area. Read [CLAUDE.md](../../CLAUDE.md) first: its process and rules apply here. Other areas: [docs/agents/](./).

**A seek is answered by the server, or explicitly deferred** (`lib/audio/seekplan.js`, `Player.svelte`,
`neezer/webui/__init__.py`). A finished file (archived FLAC, cached Opus, upload, episode) answers a
byte `Range` with 206; the player seeks it directly. A source that is on disk but has to be encoded
answers `?start=<seconds>` with a 200 (`Accept-Ranges: none`, no length, `X-NS-Start`): the element
starts at 0 = that offset, so the page must NOT set `currentTime` or use `el.duration`; the bar reads
`offset + currentTime` and the track's own duration. `GET /api/stream/<id>/ready?q=` says which of the
three it is — `{ready, ranges, bytes, cached, offset, streaming}` — and the page chooses: reload the
bare URL and seek, reload with `start`, or keep the sound and poll (1 s, 1.5 s while `streaming`).
`seekableAt` only accepts a range whose END is finite: Chromium reports `[0, Infinity]` for a generated
stream, and treating that as a real range made every chase "land" without ever asking the server, so
the seek restarted the stream from 0. A direct seek that comes back to the start is chased after 4 s.
Measured: FLAC 80 % seek = 1 request, 930 836 B, TTFB 2.9 ms, Chromium lands at 240.24 s / 300;
Opus `?start=240` = 1 116 423 B, TTFB 98 ms, exact sample (silence 1.000 s at `-ss 119`, none at
`-ss 121`). A seek is watched only until it **lands** (`seekVerdict`: landed / lost / waiting, then
`watchSeek` disarms): leaving the watch on `timeupdate` made a position 4 s later look like a missed
seek and chase the OLD target — the "it jumps back to where I clicked 10–20 s ago" report. One chase
maximum if it never lands, then the bar is the real position; a new seek and `onEnded` cancel the
watch. `test/native/run.mjs` B3: seek 0.92 s, playback reaches 6.16 s, **0 `/ready` after landing**.
The server contract is in [server.md](server.md); the native player's seek uses the same plan, and its
own held path is in [android.md](android.md).

**A transport action the person takes always wins.** Play, pause, next, previous and seek are applied
at once, even while the player is recovering from a refused resume, switching source or chasing a
stream. Those internal states may gate the *automatic* transport, never the person's command, and each
recovery/switch/chase handler is cancelled when a newer action lands — `recoverPlayback` used to stay
true forever after a source that never answered, leaving the icon on “playing” and every pause/play
press with no effect. The first load gets a grace (a slow start is not a failure), no skip waits on a
timer, and a pause is effective in under 500 ms in every state, measured by
`docs/repro/e2e_transport_repro.py`. A visible `playbackStatus` says “reprise”, “changement de
qualité” or “buffering”, so silence never looks like playing.

**A prefetch of the next track steps aside; it never trickles.** The next track's preload
(`lib/playcache.js#prefetchTrack`, through `lib/offline.js#fetchYielding`) must never take the link
from the track being played. While a watched element wants it (`lib/netyield.js#playerWantsLink` —
the prefetch's own hold does not count, since it holds the link itself), no request is started, and
mid-body the request is ABORTED and resumed with `Range: bytes=<received>-` once the element lets
go; a body the server cannot resume is restarted from zero, never read at 4 KB/s. The trickle was
measured and rejected at 16 KB/s (CDP, 150 ms, 40 s tracks): 4 KB/s is 27 % of the link, and it still
cost the playing track — a next at 1 s took 29.5 s against 20.9 s without the preload, a next at
14 s took 87.7 s against 25.9 s, with four watchdog reloads. With abort + Range (2026-11-26, median
of 3): 16 KB/s cold 20.8 s against 20.9 s (0.995 ×), hot 26.7 s against 25.5 s (1.047 ×), no reload,
blob incomplete at the tap (22 KB of 320 KB in 14 s) so the tap streams as before; 64 KB/s cold
3.9 s against 3.9 s, hot 0.3 s against 4.2 s with the whole blob on the device and **0** stream
request at the tap; LAN 0.3 s, 0 request. The preload starts 3 s after a track change
(`PREFETCH_DELAY`): starting early costs nothing now that it steps aside. The download path keeps
its own trickle for a live transcode (it cannot restart cheaply), and `fetchYielding`'s defaults are
that path byte for byte (`node test/download/run.mjs` unchanged).

**Crossfade + silence trimming** (`components/Player.svelte`, `lib/edges.js`,
`neezer/webui/edges.py`). Two ideas that need each other: masters carry a beat of digital silence
at each end, and a crossfade that ignores them fades one track's silence into another's — the gap it
was meant to remove. The server measures the bounds once per (file, threshold) with ffmpeg's
`silencedetect` and caches them; it **never archives to answer**, so asking about the next track can
never start a download, and a file that is not on disk simply answers `ready:false` and plays
untrimmed. A trailing silence is one that runs to EOF — and current ffmpeg CLOSES it at the flush
(`silence_end` equal to the stream's end, measured on 6.1 across mp3/ogg/opus/m4a/flac), where older
builds left it open; reading only the open form meant the tail was never trimmed on any recent
ffmpeg, so both caches are versioned (`edges3-` server-side, `audio.edges.v3` on the device) and a
fix to the measurement must bump them again. **A trim is for the EDGES and nothing else**: at most
10 s at either end (`MAX_EDGE_TRIM`; it was 30, and a quiet 15-s outro was cut — heard as a skip in
the middle of the song), never past a decode that stopped short of the container's `Duration`, and
the player drops bounds measured on a file whose length is not the track's (`Player.svelte#edgesFor`,
`EDGE_LENGTH_SLACK`). The handover happens at the **start** of the fade, not its end: the incoming element
becomes `audio` and the queue advances there and then, so the seek bar and the lock screen name the
track you are beginning to hear. A manual skip is not an overlap — it gets a 60 ms ramp, long enough
to kill the click of a cut mid-waveform and short enough to be inaudible as a delay. Crossfading
needs the Web Audio graph, so like the effects it is off by default (see the note in `graph.js`
about a suspended AudioContext silencing a backgrounded tab) — except on the direct path (below),
where the same equal-power curve runs on the two elements' volumes.
**No skip waits on a timer.** A backgrounded WebView fires timers as little as once a minute, and a
crossfaded skip used to start its overlap on a `setTimeout(0)` and its softened cut on a 60 ms one:
"next" on a car radio sat on the old track for anything up to that minute (measured with timers on
20 s boundaries: 2-17 s, now 0.15 s — `test/skip/run.mjs`). A skip's overlap starts synchronously,
the short ramp's wait follows the fade's own clock (`graph.js#fadeDone`: an `ended` from the audio
thread for a routed element; a hidden page snaps a direct ramp under 0.3 s, which nothing could step
anyway), and every crossfade deadline (start, give-up, end) is also checked on `timeupdate`
(`xfadeTick`), which the media pipeline fires whatever the page's timers do.

**Behind a locked screen the app's next-list is a LEASE, not the queue** (`Player.svelte#updateNativeNext`,
`nativeplayer.js`, `android/…/NativeAudio.kt`). The page names what follows and the app follows it while its activity is stopped;
bridge 10 names **32 titles visible and up to 200 compact entries when the activity stops**, and asks the page for more
(`window.__nsNeedNext`) once at most every 30 s when four remain. This matters because the native player made the page silent to
Chromium: an audible page is the one the Page Lifecycle API does not freeze, so the page may stop running entirely and cannot
re-name the list — the old ten-title lease then ended in silence until the app was reopened (`test/native/run.mjs` freezes a
40-title queue through 36 ends). The app also retries an IO failure by itself while stopped (2/5/10 s, three times; a 4xx or a
decode error still fails fast), keeps the renderer important while it owns the sound, survives the renderer death without releasing
its player, and marks the session paused when the list really ends instead of leaving a "playing" notification. **A title that cannot
be played costs that title, not the list**: a non-retryable failure while the activity is stopped consumes the entry and starts the
next one (three in a row at most), reported as a `fault` on the advancing state so the page never records a player fault for one bad
file. And the player that streams holds the link for the downloads itself (`YieldGate.holdPlayer`, 15 s to take it / 30 s to let go
on its own buffer), because the page's lease is renewed by timers a locked screen stops. `repeat=one` is
restarted by the app; `repeat=all` wraps to the hint length; the Flow pre-chooses a batch of ten when the activity stops (past it,
a frozen page still stops). Old APKs keep the ten-entry behaviour; old SPAs have no refill hook and are never sent a list they
would drop whole (32 KiB `MAX_AUDIO_CMD`).

**WHERE THE SOUND GOES: DIRECT OR PROCESSED** (`lib/audio/output.js`, `lib/audio/timekeeper.js`,
`graph.js`, `Player.svelte#renewElements`, `android/…/AudioRoute.kt`). Played through a Bluetooth car
radio (a Kenwood on AAC), the music "sounded sped up, as if pieces were being cut out", and switching
the rhythm analysis or the automatic latency off changed nothing. Nothing COULD: the full-screen
player's animation had routed the element through `createMediaElementSource`, which is one-way for
the element's whole life, and the analyser's `addModule` had moved the context's rendering onto the
AudioWorklet thread for the context's whole life. On Android that thread hands the device its audio
through a FIFO, and an A2DP sink pulls in bursts large and irregular enough that the FIFO can be
padded with silence and then drop what arrives late (Chromium 40133762 — AudioWorklet + `latencyHint:
"playback"` on Android; and the "Bluetooth + WebAudio + background = stutter" every metering PWA has
filed). The browser's own media pipeline — what every other music app uses — has none of it. **And it
comes and goes**: the same phone on the same radio, reconnected, played cleanly again. So the route
is a policy, `audio.output`, and in its default it is MEASURED rather than assumed:

- **auto** (default): processed everywhere, Bluetooth included — and DIRECT the moment the timekeeper
  measures the processed path losing audio, for as long as the fault can be expected to last: THIS
  connection of this output (`audio.glitchRoutes[routeKey]`, whose `conn` the Android app changes every
  time the output comes back), until a retry. The retry happens at a TRACK CHANGE (`retryIfDue` in
  `loadTrack`: the source is being replaced anyway, so routing the element then is inaudible) 15 min
  later, then 30, 60… up to 4 h while the fault keeps coming back (each retry that finds it costs one
  eight-second window of it); six clean windows in a row on the processed path forget it
  (`noteGoodWindow`), and a new connection starts clean. The context that lost the audio is never
  used again (`requestContextReset`: closed once no routed element plays through it, the engine
  starting over on a fresh one — the state a reconnection cured when it was reported). Réglages →
  Audio says which path, why, and when the next try is, with "Réessayer maintenant".
- **direct**: nothing is ever routed. The normalization and the crossfade ride on `element.volume`
  (the gain can only turn a track DOWN — volume stops at 1 — and the fade is stepped by a 25 ms timer
  AND by every `timeupdate`, since a background WebView may throttle timers: coarser, never stalled);
  the EQ and the bass lift are off; the animations read a COPY (`captureStream()` → a
  MediaStreamAudioSourceNode on `analysisBus`, never toward the destination), rebuilt on every track
  (a MediaStreamAudioSourceNode is bound to one track) and let go when the engine stops, at which
  point the context is SUSPENDED — a phone in a car with its screen off has one stream open, the
  element's. The copy's timing is the context's, not the element's, so the animations are only as
  close as the two output latencies are (no look-ahead: nothing audible goes through the delay).
  Auto uses the same machinery while a fault is being corrected.
- **graph**: always processed, as before; a measured fault is only reported (a toast, the log).

**The first track of a session is normalized like the next one** (`Player.svelte#loadTrackInner`,
`graph.js#setElementGain`). `setTrackGain` used to write through `currentEl`, which is only set at the
first `play()`, so the ReplayGain of track 1 was lost and only track 2 onward was normalized (measured
in the probe: track 1 peak 0 vs +7.3 dBFS on track 2 with normalization high). Every handover now names
its own element (`setElementGain(audio, gainFor(track))`, and the same in `moveSourceToFresh`,
`switchQuality`, `handOverToFresh`, `adoptIncoming` and the late catch-ups); `graph.js` already keeps
`pendingGain` for an element that is not wired yet and applies it before its first audible frame, on the
graph node, on the direct path's volume and for the native element. No ramp is needed on the first
track: the gain lands when the strip is built, at the head of playback. `test/nativegain.test.mjs` pins
the equal volume of the first two elements and the trap that was removed.

**A level the person changes mid-track moves THAT track; a gain that merely arrives late does not.**
`graph.js` re-applies `normalization` to the node it has, so the SPA has always honoured the first
case; the native engine could not, because the loudness is copied onto the voice when it is built
(`mixer.rs#Playing::loudness`) and nothing heard a later one. `nz-audio` now has
`Engine::set_current_loudness(id, loudness)` (`MixerCmd::SetCurrentLoudness`): it is called ONLY for
a deliberate level change — `Player.svelte#onNormalizationChange`, "the one mid-track change the
[static-gain] rule doesn't apply to: the user asked for it" — and is a no-op when `id` is not the
voice playing, so a call that races a track change never lands on the next title. A gain that arrives
on its own waits for the next voice; that is what `GAIN_LATE_GRACE` (2 s) protects in the web player.
Measured on the phone's queue: **−10.40 dB** at High for a 0 LUFS track (`docs/agents/mobile.md`,
*Tests and emulation*).


Leaving the graph needs a NEW element: the idle one is replaced at once, the playing one hands its
track over to a fresh one at the position it has reached (seek at metadata and again at `canplay`,
measured 3.78 s landed for 3.79 s expected), and where that cannot land (a live first-play stream
that cannot seek) the next track's load does it, since its new source goes onto a fresh element
anyway. A failed mid-song hand-over is not retried on the same track (each try is a stream request).

**Mono and left/right balance are for the EARS, never for the analysers** (`lib/audio/pan.js`,
`graph.js`). The processed chain is `… → limiter → output → [GainNode(channelCount 1, explicit) →
StereoPannerNode] → look-ahead → destination`; the stage is inserted only while `audio.mono` is on or
`audio.balance` ≠ 0, and the default graph connects `output` STRAIGHT to the look-ahead (0 extra
nodes, 0 added latency). `output → analysisBus → analyserLo/Hi / scope / rhythm` is wired BEFORE the
stage, so the rhythm analyser, the stereo scope and the visualizer keep the ORIGINAL stereo signal —
a mono fold would erase the two channels the scope exists to draw. Mono is the W3C speaker down-mix
`0.5·(L+R)`; balance is the W3C stereo-panner law (for a mono source, the equal-power
`cos/sin((b+1)·π/4)`, centre 1/√2 per side, −3.01 dB, deliberately not made up: the stage is after
the limiter, and a √2 make-up could clip a hard pan). A live change ramps `pan` (τ 12 ms) and
enters/leaves the stage through a ~48 ms dry/wet fade whose complementary gains sum to 1; the mono
channel-count change happens while the wet path is silent, so nothing clicks and a throttled timer
never leaves silence. The settings are `audio.mono` / `audio.balance` in localStorage (pan.js, not
stores.js). Direct playback and the Android app's player have no graph to insert the stage into: the
card disables its controls and says « Disponible en lecture traitée. ». A remote control does not
mirror these two keys — they are a property of the output that actually plays — so the card is not
shown there.

The **timekeeper** watches the playing element once a second, in windows of 8 s of uninterrupted
playback (a seek, a stall, a track change or an implausible tick starts a new one), against four
clocks: the element's own position (a routed element is pulled by the context: dropped frames make
it run FAST, padded silence slow — healthy is a few hundred ppm plus a render burst of quantization,
0.25% worst on the model of what Chromium measured; "sped up" is a few percent), the context's clock,
how fast the output timestamp's mapping moves (healthy: one 21.6 ms burst step now and then, 2.4 ms/s
worst; padded-then-dropped gaps: 13-31 ms/s; the bar is 10), and `AudioContext.playbackStats`
underruns where Chrome has them (1% of the time). A window past 4% pace, 3% underrun or 25 ms/s is a
verdict on its own — the simulated fault in `test/output/run.mjs` was convicted 5.2 s after it began;
a milder one needs a second bad window in a row, so one bad moment on a loaded phone is not a
verdict. On the direct path a verdict while the animations copy the sound stops the copy for the
session. Every bad window (and one good one a minute) goes to the diagnostic log;
`window.__nsAudioPath()` says which path the music takes, why, and which context (`serial`).
`test/output.test.mjs` pins the policy (the connection scope, the doubling, the clearing), the
detector on healthy and broken readings, and the volume envelopes; `test/output/run.mjs` drives the
whole thing in Chromium.

**The equalizer's meter lives only while its card does** (`lib/audio/saturation.js`,
`components/AudioEffects.svelte`, `lib/eqpresets.js`). The engine's tap is refcounted and free when nobody
listens, but the card must also stop paying when it is not on screen: the subscription starts when the card
intersects (`rootMargin: 160px`) in a visible page and stops when it scrolls away, when the tab is hidden,
and on unmount. The bars are painted from the snapshot by direct CSS custom-property writes, not Svelte
state (ten updates per tick would run the component's update pass 250 times a second for ten 6 px lines),
at 13 Hz with a 90 ms CSS glide while the RISK and the warning still see every 25 Hz reading. Measured in
headless Chromium at 390x844 with a real multitone through the chain: a write per reading cost 124 ms of
main thread per second (321 style recalcs in 5 s); half the rate costs 45 ms/s, of which the engine's own
tap is 32 (offscreen, no subscription: 20 ms/s). Layouts stayed at ~7/s in both, so the bars add paint and
style, not layout. The number readout is written straight to its spans at 4 Hz with fixed-width slots for
the same reason. The risk colours are semantic, theme-scoped marks — lemon on Encre, ink on Papier, `--tx`
on Pochette, amber `#f5b041`/`#8a5200`, red `#ff6b6b`/`#b3261e` — measured against the card at 14.4/5.0/5.2:1
minimum (encre/papier/pochette), far above the 3:1 a graphical object needs.
**The warning is about the SETTING, not the recording.** A bypassed, flat or cut-only curve adds no energy,
so it never warns however loud the master or hard the limiter works (`eqRisk`); only a band above 0 dB can
make the equalizer the cause. The bars are less trigger-happy than the first version: warn at +2 dBFS
pre-limiter or 2 dB of limiter reduction, clip at +6/+6 (a finished master at 0 dBFS with 0.38 dB of
reduction is normal). Once warned, the banner is an EPISODE (`createRiskHold`): it stays until the person
changes the setting (band, preset, EQ switch, bass, normalization, volume) while its reduction keeps the
highest value seen, so a peak that falls back 40 ms later no longer makes it flash and vanish.
`test/eq/run.mjs` drives it on screen with a +16 dB curve over a full-scale 1 kHz square: it warns, is still
there one second into the silence with the same 19.8 dB, and « Plat » clears it; the same master with a flat
curve and normalization high never warns, and switching the equalizer off hides it at once.

**The banner's number is a measurement with a baseline, never an estimate.** The raw limiter reduction
includes the master's own level and the normalization: measured on a 60 Hz sine at −0.1 dBFS with
normalization high and a quiet master's ReplayGain, the pre-limiter peak sits at +7.3 dBFS and the limiter
already pulls 6.4 dB with a FLAT curve; raising the 62 Hz band by +1 dB takes it to 7.2 dB
(`test/eq/run.mjs` case 4). Blaming the band for that 7.2 is the "9 dB for +1 dB" report, so each episode
takes the reduction measured just before the change as its baseline and shows the max of the reduction
ABOVE it: 0.9 dB for that +1 dB (below the 2 dB bar, so no banner at all), and 6.0 dB for a +9 dB band whose
raw reduction is 13.2 dB — the button then offers `volumeCutDb(6.0)` = 6.0 dB, not the old peak-derived
margin (`recommendedMargin`, which answered 8.5 dB on this bench and 9 dB on the reported hotter master for
the +1 dB case, is gone and the engine no longer publishes it). A slider drag keeps the baseline of the
gesture's first step (900 ms window), because a reading taken mid-drag already carries the earlier steps;
with no reading yet, the raw reduction is the only honest value. The readout keeps showing the RAW reduction
("Limiteur 7,2 dB"): it is the meter, the banner is the attribution. `test/eqpresets.test.mjs` pins the
baseline, the gesture window and the monotonic max.

**A volume change resets the warning's peak-hold after 160 ms**, not immediately: the analyser's 85 ms
window still holds the pre-change signal, and a hold started inside it kept the old 9.7 dB reduction for the
whole episode while the live peak had already fallen (seen in the bench). The volume change also starts a
fresh episode with a new baseline, so lowering the level to 14 % (the +9 dB case above: 13.2 dB raw → 0)
takes the banner away instead of leaving the old number standing. The player's volume is the media
element's own and applies BEFORE its `MediaElementAudioSourceNode` (measured on Chromium: volume 0.5 halved
the analyser peak), so the warning's "Réduire le volume" really does lower the pre-limiter peak. Presets:
built-ins are code and refuse "Mettre à jour"; a user preset takes the ten bands and the bass lift only (not
its EQ switch or normalization level), and update/delete ride in a toast with "Annuler" rather than a
confirmation dialog (`test/eqpresets.test.mjs`).

**The ten faders are vertical at every width** (`AudioEffects.svelte`). The track is an 8 px groove inside
a 44 px hit area, the 22 px thumb is drawn in CSS and placed on the value (not a native thumb whose travel
and alignment differ by browser), the setting fills the groove from the bottom to the thumb with a 2 px tick
at 0 dB, and the measured band level is the 30 % tint behind it (its own −60..0 dBFS scale, never read as a
gain). At ≤640 px the ten faders are a 5×2 grid instead of one row (ten 44 px targets do not fit 390 px), so
the phone and the desktop keep the same vertical metaphor. Measured in headless Chromium: thumb axis and
value off by 0.00 px, fader 44×192 px (desktop) / 44×154 (phone), no horizontal overflow at 1280 or 390.
The master volume is the same drawing horizontally: a 44 px row, an 8 px track inset by the 11 px knob
radius, the fill inside it, and the same 0.00 px alignment; the bass, crossfade, trim and balance sliders
share the 6 px track, the 20 px thumb (24 px on touch for the balance) and the 44 px hit area.
`test/eq/run.mjs` measures the geometry at both widths.

**THE APP CORE IS RUST TOO** (`webapp/appcore`, compiled to `src/lib/appcore/appcore.wasm`, reached
through `lib/appcore/core.js`). The interface's own logic — not the DOM, the logic — where the
JavaScript allocated as it went, on the main thread, in the paths that must not stutter:

- **`sync.rs`: every time-sync estimator the app has.** The listen party's NTP clock
  (`party/clock.js#ClockEstimator`), the host's line (`party/anchor.js#AnchorFit`), the guest's
  audio-clock bridge (`party/engine.js#makeClockBridge`) and the output clock the animations, the
  lyric line and the party host share (`audio/latency.js`). Each did its robust statistics the
  obvious way — a median was `[...xs].sort()`, an estimate built and dropped four to six arrays — many
  times a second, in the one subsystem where a collection pause is a late chunk. Now fixed rings,
  nothing allocated once built: an estimate 16.3 → 1.4 µs, the host's add-and-read 1.13 → 0.19 µs.
  What stays in JavaScript is the READING (only the browser can call `getOutputTimestamp()`) and, for
  the output clock, a mirror of its outputs refreshed after each reading, so the dozens of reads a
  second never cross into the module. The timeline's one-line arithmetic (`party/timeline.js`) and the
  engine's Web Audio orchestration stay JavaScript on purpose: a call into wasm costs more than
  `t.p + (S - t.t) / 1000` does, and nodes can only be scheduled from JavaScript.
- **`tracks.rs` + `fold.rs`: the track lists' search and sort** (`lib/tracklist.js`, used by
  `TrackBrowser.svelte` and `Playlist.svelte`). The lists lowercased every title, credit and album of
  every row on EVERY keystroke (12 000+ strings on a 4 000-track favourites page) and again inside the
  sort comparator — and the answer was wrong for a French library: a code-unit comparison put "Éric"
  after "Zazie", and "beyonce" did not find "Beyoncé". Now a list is indexed once when it changes
  (reconcile.js keeps its identity when it did not): every field FOLDED — case and accents dropped,
  ligatures spelled out, typographic quotes made plain — into one buffer, each sort order computed on
  first use and kept, a keystroke one pass over folded bytes. Measured on 4 000 tracks: a keystroke
  0.46 → 0.11 ms, a keystroke under a sort 0.62 → 0.20 ms, for a 2.3 ms build paid once. The
  unfiltered, unsorted view never touches the core — it is the list itself, the same array — and a
  page without WebAssembly still filters and sorts the old way (`tracklist.js#fallback`).

How it is loaded and shipped: **a separate module from the analyser**, on purpose — that one is
200 KB and loads when an animation needs it, this one is 63 KB (23 KB gzipped) and a party guest with
every animation off, or anybody searching their favourites, needs it at once. `main.js` fetches it
once the first screen has painted; hosting and joining a party await it (`routes/Party.svelte` loads
it on the landing screen, since the tap that joins must create the AudioContext synchronously, and a
session started without it starts its clocks a moment later); the output clock simply takes no
reading until it is in, which is what "no mapping yet" always meant. Every object is a HANDLE freed
by `free()`, with a FinalizationRegistry behind it for a handle whose owner was collected unfreed.
No dependencies, `build.rs` stamps a hash of `src/*.rs`, the binary is committed and
`test/appcore.test.mjs` fails when it is stale — the rhythm crate's rules exactly. That test holds
each estimator to its JavaScript ORACLE (`test/reference/sync.js`) number for number on the traces
it exists for (heavy-tailed asymmetric queueing between clocks 60 ppm apart, a device that slept, an
element read from a jittery timer through seeks and stalls, a context rendering in bursts whose
mapping steps and drifts), the index to a model of what folding means (Unicode's own decomposition,
marks stripped) on every sort, direction and query, and both to being faster. `party.test.mjs` and
`latency.test.mjs` run on the Rust; `test/party/run.mjs` and `test/rhythm/run.mjs` were re-run on it
(guest heard −1.1 / +0.1 / +0.1 / −0.3 ms from the host steady / after a seek / a pause / a skip;
frames −0.7..+0.6 ms against the output clock, 100% on time).

**What a big list costs the main thread is measured in a browser too** (`webapp/test/ui/run.mjs`,
long-task time under a 4x CPU throttle, median of five rounds). Two causes came out of it that no
language change would have touched:

- **The window's rows are keyed by POSITION** (`TrackList.svelte`), not by track. Every keystroke of a
  search (and every sort) handed the window a different track at each position, so all ~30 visible
  rows were destroyed and rebuilt — DOM, five store subscriptions, a cover and an artist line each —
  and that, not the filtering, was the keystroke. Recycled, a row just gets a new `track`: `Cover`
  already resets on a new source, and `TrackRow` drops a gesture begun on the previous track so a
  swipe can never commit on the wrong one. Ten letters typed into 4 000 favourites: **470 → 55 ms**.
- **The offline cache stores the response's TEXT** (`apicache.js`). IndexedDB structured-clones what
  it is given, synchronously, and a 4 000-track list is 4 000 nested objects — cloned on every visit.
  The text comes from a `clone()` of the response, read after the caller has its data: `res.json()`
  parses straight from the bytes, and putting `text()` + `JSON.parse` on the critical path measured
  WORSE than the clone it replaced. Records an older build wrote as objects still read. Opening the
  favourites: **315 → 154 ms**; the playlist 304 → 216; a sort 97 → 52.

**Why the DOM stays Svelte.** "As much Rust as possible" stops at the DOM, deliberately: a Rust UI
framework reaches the DOM through JavaScript glue on every node it touches, so it cannot render a list
or a player faster than Svelte's compiled DOM calls — measured frameworks land level with it at best —
and it would put a few hundred kilobytes of wasm in front of the first paint that the code-splitting
work above took out. Rust goes where it wins: arithmetic over many values, state that must not
allocate, work that must be identical everywhere.
