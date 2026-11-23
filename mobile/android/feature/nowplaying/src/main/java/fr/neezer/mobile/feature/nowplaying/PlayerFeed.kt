package fr.neezer.mobile.feature.nowplaying

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import uniffi.nz_ffi.PlayerListener
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.PositionInfo

/**
 * The player's state, as the views see it: one `StateFlow` fed by Rust's own
 * pushes, and ONE subscription for the whole process.
 *
 * Why a process-wide feed and not a flow per screen: MB3b pushes on CHANGE only
 * (`PlayerListener`), the shell shows the mini player and the sheet at once, and
 * a listener per composable would put a Rust callback list under every
 * recomposition. `install()` is idempotent; the first caller subscribes, the rest
 * share the same flow.
 *
 * **The callback runs on a RUST thread**: it writes a `MutableStateFlow` (no
 * suspension, no lock held across a Compose frame) and returns at once — the
 * rule every listener of this app follows (`docs/agents/mobile.md`, "The FFI
 * boundary").
 */
object PlayerFeed {
    private val _snapshot = MutableStateFlow<PlayerSnapshot?>(null)
    val snapshot: StateFlow<PlayerSnapshot?> = _snapshot.asStateFlow()

    private val installed = java.util.concurrent.atomic.AtomicBoolean(false)

    /** The one port the process talks to, once `install` has seen a player. */
    @Volatile
    var port: PlayerPort = NoPlayerPort
        private set

    /**
     * Subscribes once. `api` is `NzCore::player()` — the ONE player of the process
     * (MB3c's measurement), never a second instance built here.
     */
    fun install(port: PlayerPort, subscribe: ((PlayerListener) -> Unit)?) {
        // The FIRST install owns the feed: the process has one player (MB3c), and
        // a later caller with a different port (a feature's own holder, a test's
        // fake) must not replace what the whole shell is already reading — the
        // bar and the sheet would then disagree on which player they show.
        if (!installed.compareAndSet(false, true)) return
        this.port = port
        // The first snapshot NOW, so a screen opened before any change still has
        // the queue and the title instead of an empty player.
        _snapshot.value = runCatching { port.snapshot() }.getOrNull()
        val sink = object : PlayerListener {
            override fun onSnapshot(snapshot: PlayerSnapshot) {
                // ONLY a state write: this runs on a RUST thread, and calling back
                // into the core from here is the deadlock the boundary's rules
                // name. The reconcile step (a podcast's rate) runs on the
                // collector's own thread, where `NowPlayingHost` drives it.
                _snapshot.value = snapshot
            }

            override fun onQueueChanged(version: ULong) {
                // The queue handle is re-asked by the screen that draws it
                // (`NowPlayingModel.onQueueChanged`), never copied here: the
                // handle is the 4 000-row list, and this callback runs on a Rust
                // thread.
                _queueVersion.value = version.toLong()
            }

            override fun onFault(cause: String) {
                // The snapshot carries the cause; the shell shows it as a toast.
                fr.neezer.mobile.designsystem.NzToasts.show(cause)
            }
        }
        runCatching { subscribe?.invoke(sink) }
    }

    /**
     * A TEST's player replaces the feed's port — never called by the app.
     *
     * The process has ONE player and one feed (the bar and the sheet must not
     * disagree); a test that needs its own player therefore has to take the feed
     * over, and this is that door, named so a search for it finds every caller
     * (one, in `androidTest`).
     */
    internal fun usePortForTest(port: PlayerPort, subscribe: ((PlayerListener) -> Unit)?) {
        installed.set(false)
        install(port, subscribe)
    }

    private val _queueVersion = MutableStateFlow(0L)

    /** Bumps on every queue change, so a screen re-windows its handle. */
    val queueVersion: StateFlow<Long> = _queueVersion.asStateFlow()
}

/**
 * The progress bar's read, counted.
 *
 * **The acceptance's "0 FFI calls while the screen is off" lives here.** A frame
 * calls [frame] with what is DRAWN this instant (a bar on screen AND a lifecycle
 * at least STARTED); with nothing to draw it answers `null` and touches no
 * binding at all. The count is the test's proof, not a hope: a device run flips
 * the screen off and reads [reads] before and after.
 */
class PositionTicker(private val read: () -> PositionInfo) {
    /** How many times the player's `position()` has crossed the boundary. */
    var reads: Int = 0
        private set

    /**
     * One frame: the position when something draws it, `null` when nothing does.
     * The caller keeps the last value, so a paused-but-visible bar still shows
     * where it is.
     */
    fun frame(drawn: Boolean): PositionInfo? {
        if (!drawn) return null
        reads += 1
        return read()
    }
}

/**
 * A podcast's rate, per show — `lib/speed.js`, applied through the player.
 *
 * The player has ONE rate in its audio settings (`AudioSettings.speed`), so the
 * reconcile step is what the SPA's watcher is: on every snapshot, an episode is
 * played at its show's remembered rate, and music is put back at 1. It runs on
 * the same change-only push as everything else, and only sends a command when the
 * rate really differs — so it can never fight MB12's own writes.
 *
 * It is a singleton because the rate belongs to the SHOW, not to a screen: the
 * queue may advance while the sheet is closed, and the next episode must already
 * be at the right speed.
 */
object PodcastSpeed {
    /** The show a rate is kept under: `channel_id`, else the album's provider id. */
    fun keyOf(now: uniffi.nz_ffi.NowPlaying?): String? {
        if (now == null || !now.isEpisode) return null
        val album = now.albumId?.takeIf { it.isNotBlank() }
        return album
    }

    /** The rate the snapshot's rate should be, and whether it differs. */
    fun desired(service: NowPlayingService, snapshot: PlayerSnapshot, current: Float): Float? {
        val now = snapshot.current ?: return if (current != 1f) 1f else null
        val show = keyOf(now) ?: return if (current != 1f) 1f else null
        val want = service.speed(show)
        return if (want != current) want else null
    }

    /** The one service of the process (built on first use, never per snapshot). */
    private val service: NowPlayingService? by lazy { NowPlayingCore.service() }

    /**
     * Puts the engine at the playing show's rate (1 for music). Idempotent: no
     * command is sent when the rate is already right.
     */
    fun reconcile(port: PlayerPort, snapshot: PlayerSnapshot) {
        val service = service ?: return
        val audio = runCatching { port.audio() }.getOrNull() ?: return
        val want = desired(service, snapshot, audio.speed) ?: return
        runCatching { port.setAudio(audio.copy(speed = want)) }
    }
}
