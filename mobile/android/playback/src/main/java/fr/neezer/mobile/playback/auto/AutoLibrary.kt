// :playback — installing Android Auto's tree in the media session, and playing
// what a head unit taps (docs/agents/mobile.md, "MB25 — Android Auto").
//
// Two jobs, both thin:
//
// 1. **`register`** hands MB3c's `LibraryTrees` seam the real tree once the
//    service has built its session (a `MediaLibrarySession` is what
//    `notifyChildrenChanged` needs, so the registration cannot happen earlier).
// 2. **`play`** answers a tap on a NODE — Favoris, a playlist, Mon Flow,
//    Récents, Téléchargements — with the titles the node showed, or with the
//    entity the PLAYER resolves. A tap on a title never comes here: it keeps
//    MB3c's own path (`onSetMediaItems` → `NzPlayer` → `PlayQueue`), which is
//    what the notification, the lock screen, Bluetooth and MB24's car overlay
//    use.
package fr.neezer.mobile.playback.auto

import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaLibraryService.MediaLibrarySession
import androidx.media3.session.MediaSession
import com.google.common.util.concurrent.ListenableFuture
import com.google.common.util.concurrent.SettableFuture
import fr.neezer.mobile.playback.LibraryTrees
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import uniffi.nz_ffi.AutoPlay
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.nzErrorText

@OptIn(UnstableApi::class)
object AutoLibrary {
    /**
     * The tree's own scope, one per PROCESS: a media service Android recreates
     * adopts the same reader instead of stacking a second one, so a swiped task
     * or a low-memory kill of the service cannot leak a coroutine per
     * recreation.
     */
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val main = Handler(Looper.getMainLooper())

    @Volatile
    private var screens: Screens? = null

    @Volatile
    private var player: PlayerApi? = null

    @Volatile
    private var session: MediaLibrarySession? = null

    /**
     * Installs the tree on the session `NzMediaService` just built. Called ONCE
     * per service creation, after `MediaLibrarySession.build()`: a session is
     * what lets the tree say "this node has more" while the car is looking at
     * it.
     */
    fun register(screens: Screens?, player: PlayerApi?, session: MediaLibrarySession) {
        this.screens = screens
        this.player = player
        this.session = session
        val client = screens ?: return
        val tree = AutoLibraryTree(client, ::notifyChanged, scope)
        LibraryTrees.current = tree
        tree.warmUp()
        Log.i(TAG, "arbre Auto installé")
    }

    /** `true` for a media id that names a NODE rather than a title. */
    fun isContainer(mediaId: String): Boolean = AutoNode.isContainer(mediaId)

    /**
     * `true` for a media id the tree built for a TITLE (`neezer:track:<key>`):
     * such a row carries what the player needs, and Media3's default resolution
     * would refuse it (no `LocalConfiguration`).
     */
    fun isTreeItem(mediaId: String): Boolean = AutoNode.kind(mediaId) == AutoNodeKind.Track

    /**
     * What a tap on a node plays.
     *
     * - **The titles it showed** ([AutoPlay.Queue]): the queue is what the car
     *   displayed, so the tap costs no request at all — the driver hears what
     *   they saw.
     * - **The player's own entity** ([AutoPlay.Entity], Mon Flow): the queue
     *   asks for each next title itself, one request at a time, exactly as it
     *   does everywhere else in the app.
     *
     * The answer is a future Media3 waits for, so nothing blocks the session's
     * thread; a failure carries the French cause (`nzErrorText`) and the car
     * says it instead of staying silent (rule 3).
     */
    fun play(mediaId: String): ListenableFuture<MediaSession.MediaItemsWithStartPosition> {
        val answer = SettableFuture.create<MediaSession.MediaItemsWithStartPosition>()
        val client = screens
        if (client == null) {
            answer.setException(IllegalStateException("Arbre Auto non installé"))
            return answer
        }
        scope.launch {
            try {
                when (val plan = client.autoPlay(mediaId)) {
                    is AutoPlay.Queue ->
                        answer.set(
                            MediaSession.MediaItemsWithStartPosition(
                                plan.tracks.map(AutoItems::mediaItem),
                                0,
                                IDLE_POSITION_MS,
                            ),
                        )
                    is AutoPlay.Entity -> {
                        // The queue is the player's: an EMPTY list here means
                        // "Media3, set nothing" (`NzPlayer.handleSetMediaItems`
                        // ignores an empty list), and `PlayEntity` fills the real
                        // one on the player's own thread.
                        player?.command(
                            PlayerCommand.PlayEntity(plan.kind, plan.id, false),
                            System.nanoTime().toULong(),
                        )
                        answer.set(
                            MediaSession.MediaItemsWithStartPosition(
                                emptyList(),
                                0,
                                IDLE_POSITION_MS,
                            ),
                        )
                    }
                }
            } catch (error: NzException) {
                answer.setException(IllegalStateException(nzErrorText(error)))
            } catch (error: RuntimeException) {
                answer.setException(error)
            }
        }
        return answer
    }

    /** Told on the MAIN thread: Media3's session is not thread-safe. */
    private fun notifyChanged(parent: String, count: Int) {
        main.post {
            val current = session ?: return@post
            current.notifyChildrenChanged(parent, count, null)
        }
    }

    private const val TAG = "NzAutoLibrary"

    /** A queue starts at its first title, never at a saved position. */
    private const val IDLE_POSITION_MS = 0L
}
