package org.nsupysonic.app

import android.content.ComponentName
import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.core.content.ContextCompat
import androidx.media3.common.C
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import com.google.common.util.concurrent.ListenableFuture
import fr.neezer.mobile.playback.CoverArt
import fr.neezer.mobile.playback.NzMediaService

/**
 * The car's ONE door to the app's player (MB24's first seam).
 *
 * The old app's service bound `PlayerService` with `BIND_AUTO_CREATE`, read its
 * `state()` and `onState {}` and sent it `transport(cmd, value)`. The new app's
 * player is `NzMediaService` over `NzPlayer` over the Rust engine (MB3c), so the
 * car is a Media3 **`MediaController`** on that session: the same commands the
 * notification, the lock screen and Android Auto send, and exactly one player
 * (CLAUDE.md's rule — never a second reader).
 *
 * What is kept from the old binding, because each was a bug once:
 *
 * - **The last state is kept**, so a dropped controller does not leave the card
 *   drawing a track as if it were still playable;
 * - **a lost session is re-built**, a second apart, while the car is up (the
 *   media service is recreated by Android at will);
 * - **every call is on the application thread**: a `MediaController` throws
 *   `IllegalStateException: Method is called from a wrong thread` otherwise
 *   (MB3c's own measurement), so the controller is built with the MAIN looper
 *   and the car's service already renders from it.
 *
 * It holds no rule: the queue, the next title and the sound are the player's.
 * A command whose player is not connected is refused with `false`, and the
 * service turns that into "Ouvre Neezer pour lancer la musique" — the old
 * `car_no_player`.
 */
internal class CarPlayer(
    context: Context,
    private val onChange: () -> Unit,
) {
    private val app = context.applicationContext
    private val main = Handler(Looper.getMainLooper())

    private var controller: MediaController? = null
    private var future: ListenableFuture<MediaController>? = null
    private var last: CarNow? = null
    private var destroyed = false

    /** Reconnects are posted one at a time; a dead session must not storm. */
    private var rebind: Runnable? = null

    val connected: Boolean
        get() = controller != null

    private val listener = object : Player.Listener {
        override fun onEvents(player: Player, events: Player.Events) {
            main.post { onChange() }
        }
    }

    private val controllerListener = object : MediaController.Listener {
        override fun onDisconnected(controller: MediaController) {
            this@CarPlayer.controller = null
            last = null
            onChange()
            // Android recreates the media service when it likes (a swiped task,
            // a low-memory kill of the service only): come back, one second
            // apart, exactly like the old `onBindingDied`.
            scheduleRebind()
        }
    }

    /** Binds the session, once. A second call while connected is a no-op. */
    fun bind() {
        if (destroyed || controller != null || future != null) return
        val token = try {
            SessionToken(app, ComponentName(app, NzMediaService::class.java))
        } catch (_: Exception) {
            return
        }
        val pending = try {
            MediaController.Builder(app, token)
                .setApplicationLooper(Looper.getMainLooper())
                .setListener(controllerListener)
                .buildAsync()
        } catch (_: Exception) {
            scheduleRebind()
            return
        }
        future = pending
        pending.addListener(
            {
                // On the main executor: this is the application thread the
                // controller must be used from.
                try {
                    if (destroyed) {
                        MediaController.releaseFuture(pending)
                        return@addListener
                    }
                    controller = pending.get().also { it.addListener(listener) }
                    onChange()
                } catch (_: Exception) {
                    controller = null
                    scheduleRebind()
                } finally {
                    if (future === pending) future = null
                }
            },
            ContextCompat.getMainExecutor(app),
        )
    }

    private fun scheduleRebind() {
        if (destroyed) return
        rebind?.let { main.removeCallbacks(it) }
        val run = Runnable {
            rebind = null
            bind()
        }
        rebind = run
        main.postDelayed(run, REBIND_MS)
    }

    /**
     * What the card draws right now: the player's own metadata, its position and
     * its transport state. `null` when nothing is loaded (the card then shows
     * its idle face, as it did when `PlayerService.State.active` was false).
     */
    fun now(): CarNow? {
        val controller = controller ?: return last
        val metadata = controller.currentMediaItem?.mediaMetadata ?: return null
        if (controller.mediaItemCount == 0) return null
        val position = controller.currentPosition.coerceAtLeast(0L) / 1000.0
        val durationMs = controller.duration
        val duration = if (durationMs == C.TIME_UNSET || durationMs <= 0L) 0.0 else durationMs / 1000.0
        val now = CarNow(
            title = metadata.title?.toString().orEmpty(),
            artist = metadata.artist?.toString().orEmpty(),
            album = metadata.albumTitle?.toString().orEmpty(),
            cover = CoverArt.urlOf(metadata.artworkUri).orEmpty(),
            playing = controller.isPlaying,
            position = position,
            duration = duration,
            // The heart is the CORE's answer (a `MediaMetadata` extra is not a
            // public contract): the service fills it from `NzCore.car_favorite`.
            favorite = null,
        )
        last = now
        return now
    }

    /** Play if paused, pause if playing: the card's one big button. */
    fun toggle(): Boolean {
        val controller = controller ?: return false
        if (controller.isPlaying) controller.pause() else controller.play()
        return true
    }

    /**
     * The next title, through the player itself — Media3's own `seekToNext`,
     * which is what the notification and Android Auto send too.
     */
    fun next(): Boolean {
        val controller = controller ?: return false
        controller.seekToNext()
        return true
    }

    /**
     * Previous, with the player's own rule: past three seconds it restarts the
     * current title, before that it goes back (Media3's `seekToPrevious`, the
     * same behaviour the old `PlayerService.nativeSkip` had).
     */
    fun previous(): Boolean {
        val controller = controller ?: return false
        controller.seekToPrevious()
        return true
    }

    fun seekTo(seconds: Double): Boolean {
        if (!seconds.isFinite() || seconds < 0.0) return false
        val controller = controller ?: return false
        val duration = controller.duration
        val target = if (duration > 0L) {
            (seconds * 1000.0).toLong().coerceIn(0L, duration)
        } else {
            (seconds * 1000.0).toLong()
        }
        controller.seekTo(target)
        return true
    }

    /** Release everything. The service's own exit path; never called by a view. */
    fun release() {
        destroyed = true
        rebind?.let { main.removeCallbacks(it) }
        rebind = null
        val controller = controller
        this.controller = null
        val pending = future
        future = null
        try {
            controller?.removeListener(listener)
        } catch (_: Exception) {
        }
        try {
            if (pending != null) MediaController.releaseFuture(pending)
            else if (controller != null) controller.release()
        } catch (_: Exception) {
        }
        last = null
    }

    /**
     * The metadata a state carries, for the notification's own line: the old
     * `PlayerService.State` had it, and the car's notification prints
     * "titre · artiste". Public for the service's `buildNotification`.
     */
    fun metadata(): MediaMetadata? = controller?.currentMediaItem?.mediaMetadata

    private companion object {
        /** The old `onBindingDied` retry, unchanged: one second. */
        const val REBIND_MS = 1_000L
    }
}
