// :playback — the library a `MediaLibrarySession` browses (docs/agents/mobile.md,
// "MB3c": "callback delegating browse to `LibraryTrees.current` — a minimal root
// here, MB25 registers the full tree").
package fr.neezer.mobile.playback

import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata

/**
 * What the session can browse. MB25 (Android Auto) implements the real tree —
 * Favoris, playlists, Flow, recent — over `Screens`' lists; MB3c only needs the
 * root to exist, because a `MediaLibrarySession` whose `onGetLibraryRoot`
 * answers nothing is a car that shows an empty screen with no cause.
 *
 * Every method is asked on the session's own thread; an implementation that can
 * wait must answer from its own cache and let the tree notify
 * (`MediaLibrarySession.notifyChildrenChanged`) when it has more — rule 1.
 */
interface LibraryTree {
    /** The one root a controller connects to. */
    fun root(): MediaItem

    /** The direct children of [mediaId], a page at a time. */
    fun children(mediaId: String, page: Int, pageSize: Int): List<MediaItem>

    /** One browsable item, or `null` when [mediaId] is not ours. */
    fun item(mediaId: String): MediaItem?
}

/**
 * The seam MB25 replaces. Until then the root is the only node: `Neezer`, not
 * browsable further — honest, because the app itself has no browse tree yet,
 * and MB0's rule is that a lot never invents a contract a later lot owns.
 */
object LibraryTrees {
    /** The root's id, stable: `LibraryTrees.current.root().mediaId`. */
    const val ROOT_ID = "neezer:root"

    @Volatile
    var current: LibraryTree = MinimalTree

    private object MinimalTree : LibraryTree {
        override fun root(): MediaItem =
            MediaItem.Builder()
                .setMediaId(ROOT_ID)
                .setMediaMetadata(
                    MediaMetadata.Builder()
                        .setTitle("Neezer")
                        .setIsBrowsable(true)
                        .setIsPlayable(false)
                        .build(),
                )
                .build()

        override fun children(mediaId: String, page: Int, pageSize: Int): List<MediaItem> = emptyList()

        override fun item(mediaId: String): MediaItem? = if (mediaId == ROOT_ID) root() else null
    }
}
