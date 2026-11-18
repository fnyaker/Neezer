package fr.neezer.mobile.feature.nowplaying

import fr.neezer.mobile.designsystem.NzRowSource
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.AudioSettings
import uniffi.nz_ffi.NowPlaying
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.PositionInfo
import uniffi.nz_ffi.Quality
import uniffi.nz_ffi.Repeat
import uniffi.nz_ffi.SleepState
import uniffi.nz_ffi.SleepTimer

/**
 * The full-screen player's state machine (lot MB11).
 *
 * Everything the SPA's `MobileNowPlaying.svelte`, `QueueList.svelte`,
 * `SleepButton.svelte`, `QualityMenu.svelte` and `SpeedButton.svelte` decide, in
 * one plain-Kotlin object over [PlayerPort] and [NowPlayingService] — so
 * `NowPlayingModelTest` proves each rule on the JVM with a fake (no JNA, no
 * `.so`, no device) and the real components are thin.
 *
 * What is NOT here, on purpose: the transport's own rules (the queue, the
 * advance, the crossfade, gapless, the listen POST) are MB3b's `nz-core::player`.
 * A screen sends a command and draws the snapshot it was handed; a model that
 * second-guessed the engine is the bug MB3c measured (`PLAYING` over silence).
 *
 * The rules this file owns:
 *
 * - **Every click is optimistic.** [toggle], [cycleRepeat], [setShuffle],
 *   [seek], [move], [remove], [clearUpcoming] update [state] in the same frame
 *   and then send their command; the authoritative snapshot replaces the guess
 *   when it arrives. A played row is never moved or removed (`QueueList.svelte`:
 *   only what is AFTER the playhead can be), and a drag whose target did not
 *   change sends nothing.
 * - **The quality applies at the NEXT track and says so** (desktop's rule, the
 *   same badge): it is a setting, not a live re-open of the playing stream, and
 *   the toast is the sentence that explains the wait.
 * - **The sleep timer is the player's.** The menu sends `set_sleep`; the label
 *   reads `snapshot().sleep`, which the engine keeps against its own fade. A
 *   countdown here would disagree with the audio within a minute.
 * - **The queue is a LIST HANDLE, never a list.** [queue] re-asks the player's
 *   handle when Rust says the queue changed (`onQueueChanged`); the screen
 *   windows it (`rememberListWindow`), so 4 000 rows never cross whole.
 */
class NowPlayingModel(
    private val player: PlayerPort,
    private val service: NowPlayingService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
) {
    data class State(
        /** What Rust last pushed (or asked once, for the first frame). */
        val snapshot: PlayerSnapshot? = null,
        /** The hot read's last value: only while a bar is on screen. */
        val position: PositionInfo = PositionInfo(0.0, 0.0, 0.0, false),
        /** The queue's windowed source, re-asked on every change. */
        val queue: NzRowSource? = null,
        /** Bumps when the handle is stale, so a `remember` re-creates the window. */
        val queueVersion: Long = 0,
        val audio: AudioSettings? = null,
        /** The episode's rate chip, when a show has one. */
        val speedRate: Float? = null,
    ) {
        val current: NowPlaying? get() = snapshot?.current
        val playing: Boolean get() = snapshot?.state == PlayState.PLAYING
        val busy: Boolean
            get() = snapshot?.state == PlayState.BUFFERING ||
                (snapshot?.state == PlayState.PLAYING && position.bufferedS <= 0.0 && position.durationS <= 0.0)
        val shuffle: Boolean get() = snapshot?.shuffle == true
        val repeat: Repeat get() = snapshot?.repeat ?: Repeat.OFF
        val sleep: SleepState? get() = snapshot?.sleep
        val len: Int get() = snapshot?.len?.toInt() ?: 0
        val index: Int get() = snapshot?.index?.toInt() ?: -1
        val upcoming: Int get() = (len - index - 1).coerceAtLeast(0)
        val durationS: Double
            get() = position.durationS.takeIf { it > 0.0 }
                ?: (current?.durationS ?: 0.0)
        val quality: Quality get() = audio?.quality ?: Quality.AUTO
    }

    private val _state = MutableStateFlow(State(snapshot = runCatching { player.snapshot() }.getOrNull()))
    val state: StateFlow<State> = _state.asStateFlow()

    /** The counted position reads — the acceptance's "0 FFI calls screen off". */
    val ticker: PositionTicker = PositionTicker { player.position() }

    /** Reads the whole picture: the snapshot, the queue handle and the audio set. */
    fun refresh() {
        val snapshot = runCatching { player.snapshot() }.getOrNull()
        val audio = runCatching { player.audio() }.getOrNull()
        val queue = runCatching { player.queueSource() }.getOrNull()
        _state.update {
            it.copy(
                snapshot = snapshot ?: it.snapshot,
                audio = audio ?: it.audio,
                queue = queue ?: it.queue,
                speedRate = speedFor(snapshot, audio) ?: it.speedRate,
            )
        }
    }

    /** Rust pushed a new picture: replace what the guess said. */
    fun onSnapshot(snapshot: PlayerSnapshot) {
        val audio = runCatching { player.audio() }.getOrNull()
        _state.update {
            it.copy(
                snapshot = snapshot,
                audio = audio ?: it.audio,
                speedRate = speedFor(snapshot, audio),
            )
        }
    }

    /** The queue changed: the handle it drew is stale, so a new one is asked. */
    fun onQueueChanged(version: Long) {
        val queue = runCatching { player.queueSource() }.getOrNull()
        _state.update {
            it.copy(queue = queue ?: it.queue, queueVersion = version)
        }
    }

    /** One visible frame's hot read (the bar calls this, the ticker counts it). */
    fun framePosition(drawn: Boolean) {
        val position = ticker.frame(drawn) ?: return
        _state.update { it.copy(position = position) }
    }

    // -- the transport --------------------------------------------------------

    fun toggle(tapNs: Long = System.nanoTime()) {
        val playing = _state.value.playing
        _state.update {
            it.copy(snapshot = it.snapshot?.copy(state = if (playing) PlayState.PAUSED else PlayState.PLAYING))
        }
        player.command(if (playing) PlayerCommand.Pause else PlayerCommand.Play, tapNs)
    }

    fun next(tapNs: Long = System.nanoTime()) {
        player.command(PlayerCommand.Next, tapNs)
    }

    fun prev(tapNs: Long = System.nanoTime()) {
        player.command(PlayerCommand.Prev, tapNs)
    }

    /** A tap on the rail: the position moves NOW, the engine chases it. */
    fun seek(seconds: Double) {
        _state.update { it.copy(position = it.position.copy(positionS = seconds.coerceAtLeast(0.0))) }
        player.command(PlayerCommand.Seek(seconds.coerceAtLeast(0.0)))
    }

    /** A row tapped (or a cover swiped to): play THAT entry of the queue. */
    fun jump(index: Int, key: String, tapNs: Long = System.nanoTime()) {
        if (index < 0) return
        player.command(PlayerCommand.Jump(index.toUInt(), key), tapNs)
    }

    fun cycleRepeat() {
        val next = when (_state.value.repeat) {
            Repeat.OFF -> Repeat.ALL
            Repeat.ALL -> Repeat.ONE
            Repeat.ONE -> Repeat.OFF
        }
        _state.update { it.copy(snapshot = it.snapshot?.copy(repeat = next)) }
        player.command(PlayerCommand.CycleRepeat)
    }

    fun setShuffle(on: Boolean) {
        _state.update { it.copy(snapshot = it.snapshot?.copy(shuffle = on)) }
        player.command(PlayerCommand.SetShuffle(on))
    }

    fun setVolume(value: Float) {
        val volume = value.coerceIn(0f, 1f)
        _state.update { it.copy(snapshot = it.snapshot?.copy(volume = volume, muted = false)) }
        player.command(PlayerCommand.SetVolume(volume))
    }

    fun toggleMute() {
        val muted = _state.value.snapshot?.muted == true
        _state.update { it.copy(snapshot = it.snapshot?.copy(muted = !muted)) }
        player.command(PlayerCommand.ToggleMute)
    }

    // -- the queue ------------------------------------------------------------

    /**
     * A drag's answer. **Only what is AFTER the playing row can move**
     * (`QueueList.svelte`): a target on or before the playhead is refused here,
     * because the queue's own rules are not something a gesture may break.
     */
    fun move(from: Int, to: Int) {
        if (from == to) return
        val index = _state.value.index
        if (from <= index || to <= index) return
        player.command(PlayerCommand.Move(from.toUInt(), to.toUInt()))
    }

    /** The ✕ on a row: only an upcoming one, and only once. */
    fun remove(index: Int, key: String) {
        val playing = _state.value.index
        if (index <= playing || index < 0) return
        player.command(PlayerCommand.Remove(index.toUInt(), key))
    }

    /** « Vider la file »: everything after the playhead, and the person is told. */
    fun clearUpcoming() {
        if (_state.value.upcoming <= 0) return
        player.command(PlayerCommand.ClearUpcoming)
        toast("File vidée")
    }

    // -- the room -------------------------------------------------------------

    /**
     * The quality, applied at the NEXT track (the badge's rule, desktop D14a's
     * own): the setting is saved now, and the toast says when it takes effect —
     * a menu that offered a live switch would re-open a playing stream under the
     * person's ears.
     */
    fun setQuality(quality: Quality) {
        val audio = _state.value.audio ?: return
        if (audio.quality == quality) return
        val next = audio.copy(quality = quality)
        _state.update { it.copy(audio = next) }
        player.setAudio(next)
        toast("Qualité : ${qualityLabel(quality)} · au prochain titre")
    }

    /** A sleep timer: minutes, the end of the track, or `null` to cancel it. */
    fun setSleep(timer: SleepTimer?) {
        player.setSleep(timer)
        val sentence = when (timer) {
            null -> "Minuteur désactivé"
            is SleepTimer.EndOfTrack -> "Le lecteur s'arrêtera à la fin de ce titre"
            is SleepTimer.Minutes -> "Le lecteur s'arrêtera dans ${minutesName(timer.minutes)}"
        }
        toast(sentence)
    }

    // -- a podcast's rate -----------------------------------------------------

    /** The show a rate belongs to: `channel` first, else the album's provider id. */
    fun showKey(now: NowPlaying?): String? = PodcastSpeed.keyOf(now)

    /** The rate the chip draws (`null` for anything that is not an episode). */
    fun speedRate(): Float? = _state.value.speedRate

    /**
     * A rate chosen for the playing show. It is remembered per show — the SPA's
     * rule — and applied through the player, which owns the engine's speed.
     */
    fun chooseSpeed(rate: Float) {
        val show = showKey(_state.value.current) ?: return
        service.setSpeed(show, rate)
        _state.update { it.copy(speedRate = rate) }
        val audio = _state.value.audio ?: return
        player.setAudio(audio.copy(speed = rate))
    }

    private fun speedFor(snapshot: PlayerSnapshot?, audio: AudioSettings?): Float? {
        val show = PodcastSpeed.keyOf(snapshot?.current) ?: return null
        val fromStore = runCatching { service.speed(show) }.getOrDefault(audio?.speed ?: 1f)
        return if (fromStore > 0f) fromStore else (audio?.speed ?: 1f)
    }

    /**
     * The lyrics of the playing title, asked when the audio is under way — the
     * SPA's `ladder.js` rule (nobody reads along before the song starts, and a
     * request that races its own audio spends the first second of a mobile link
     * on words). Returns the key to ask for, or `null`.
     */
    fun lyricsKeyWhenPlayable(): String? {
        val state = _state.value
        if (state.snapshot?.state != PlayState.PLAYING) return null
        return state.current?.key?.takeIf { it.isNotBlank() }
    }

    /** One save of the queue's order is the player's: nothing is written here. */
    suspend fun warmAudio() = withContext(io) { refresh() }
}

/** The badge's own text: `Auto`, `FLAC`, `320`… (the SPA's `SHORT` map). */
fun qualityLabel(quality: Quality): String = when (quality) {
    Quality.AUTO -> "Auto"
    Quality.FLAC -> "FLAC"
    Quality.OPUS320 -> "Opus 320"
    Quality.OPUS128 -> "Opus 128"
    Quality.OPUS64 -> "Opus 64"
}

/** The badge's short form, in mono: the same five rungs. */
fun qualityShort(quality: Quality): String = when (quality) {
    Quality.AUTO -> "AUTO"
    Quality.FLAC -> "FLAC"
    Quality.OPUS320 -> "320"
    Quality.OPUS128 -> "128"
    Quality.OPUS64 -> "64"
}

/** One line of French for a rung, the SPA's `HINT` (its five the player has). */
fun qualityHint(quality: Quality): String = when (quality) {
    Quality.AUTO -> "Selon le débit mesuré"
    Quality.FLAC -> "Sans perte"
    Quality.OPUS320 -> "Haute qualité"
    Quality.OPUS128 -> "Standard"
    Quality.OPUS64 -> "Données réduites"
}

/** « 5 minutes », « 1 heure », « 1 h 30 » — the sleep menu's own names. */
fun minutesName(minutes: UInt): String {
    val m = minutes.toInt()
    return when {
        m < 60 -> "$m minutes"
        m == 60 -> "1 heure"
        else -> "${m / 60} h ${(m % 60).toString().padStart(2, '0')}"
    }
}

/** « 12 min », « fin du titre » — what the moon's badge says. */
fun sleepLabel(sleep: SleepState?): String = when {
    sleep == null -> ""
    sleep.atEndOfTrack -> "fin du titre"
    else -> "${sleep.minutes} min"
}

/** « 1,25× » — French decimal comma, the speed chip's own label. */
fun speedLabel(rate: Float): String {
    if (rate == 1f) return "1×"
    val rounded = Math.round(rate * 100f) / 100f
    return rounded.toString().replace('.', ',') + "×"
}
