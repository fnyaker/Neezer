// :playback — Android Auto's browse tree, over MB3c's `LibraryTree` seam
// (docs/agents/mobile.md, "MB25 — Android Auto").
//
// What this class does, and what it must not do:
//
// - **It paints from the DEVICE.** `children()` is called on the session's own
//   thread and must answer in the same frame, so it reads what Rust already
//   copied (`Screens.autoCachedChildren`, disk-only, 0 requests) and slices one
//   page of it. It never waits for the network (rule 1).
// - **It refreshes behind the paint.** A background read (`autoChildren`, the
//   `Prefetch` lane inside Rust) replaces the copy when it differs and tells the
//   browser (`notifyChildrenChanged`) — the pattern MB3c's `LibraryTree` was
//   written for: "answer from its own cache and let the tree notify".
// - **It decides nothing else.** The five nodes, their ids, their order and what
//   a tap plays are Rust's (`nz-core/src/auto/`); this file is Media3 glue.
package fr.neezer.mobile.playback.auto

import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaLibraryService.MediaLibrarySession
import fr.neezer.mobile.playback.LibraryTree
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.nzErrorText

@OptIn(UnstableApi::class)
class AutoLibraryTree(
    private val screens: Screens,
    private val onChanged: (parent: String, count: Int) -> Unit,
    private val scope: CoroutineScope,
) : LibraryTree {
    /**
     * What the device holds, per node, already as Media3 objects: a browse of
     * the same node twice costs neither a request nor a parse. At most one page
     * of a hundred rows per node (Rust caps a node at `MAX_CHILDREN`), so its
     * counted size stays around a hundred kilobytes for the whole tree.
     */
    private val warm = java.util.concurrent.ConcurrentHashMap<String, List<MediaItem>>()

    /** The nodes a read is already in flight for: one request, never a storm. */
    private val loading = java.util.Collections.newSetFromMap(
        java.util.concurrent.ConcurrentHashMap<String, Boolean>(),
    )

    override fun root(): MediaItem = AutoItems.root()

    override fun children(mediaId: String, page: Int, pageSize: Int): List<MediaItem> =
        AutoPage.slice(painted(mediaId), page, pageSize)

    /**
     * One item by its id. A NODE's own row lives in its PARENT's list (the root
     * for a section, the playlist folder for a playlist), so that is where it is
     * read from; a title is searched among the nodes already painted. An id this
     * process never saw answers `null`, which Media3 turns into "not ours"
     * rather than a made-up row.
     */
    override fun item(mediaId: String): MediaItem? {
        if (mediaId == AutoNode.ROOT) return root()
        val parent =
            when (AutoNode.kind(mediaId)) {
                AutoNodeKind.Favorites,
                AutoNodeKind.Playlists,
                AutoNodeKind.Flow,
                AutoNodeKind.Recent,
                AutoNodeKind.Downloads,
                -> AutoNode.ROOT
                AutoNodeKind.Playlist -> AutoNode.PLAYLISTS
                AutoNodeKind.Track -> null
                AutoNodeKind.Root, AutoNodeKind.Unknown -> return null
            }
        if (parent != null) {
            painted(parent).firstOrNull { it.mediaId == mediaId }?.let { return it }
        }
        for (rows in warm.values) {
            rows.firstOrNull { it.mediaId == mediaId }?.let { return it }
        }
        return null
    }

    /** Warms the root and its sections without a request: the tree is device-first. */
    fun warmUp() {
        painted(AutoNode.ROOT)
        refresh(AutoNode.ROOT)
    }

    /**
     * The rows of [parent] as the device has them NOW: the memory copy, else the
     * disk copy, and in both cases one background read that replaces it when the
     * server has more.
     */
    private fun painted(parent: String): List<MediaItem> {
        warm[parent]?.let { return it }
        val copy =
            try {
                screens.autoCachedChildren(parent).map(AutoItems::mediaItem)
            } catch (error: NzException) {
                // A copy that cannot be read is not a failed browse: the read
                // below fetches the node, and the car shows it when it lands.
                Log.i(TAG, "copie $parent : ${nzErrorText(error)}")
                emptyList()
            }
        if (copy.isNotEmpty()) {
            warm[parent] = copy
        } else {
            // Paint nothing, but do not lose the read either: a cold node is one
            // `Prefetch` request away (rule 3's "never a spinner" becomes "never
            // an empty list without a cause").
            refresh(parent)
        }
        return copy
    }

    /**
     * One background read per node. Rust answers from its copy when it is fresh
     * (0 requests) and asks once when it is not, so a warm browse of the tree
     * costs nothing at all — MB25's measured acceptance.
     */
    private fun refresh(parent: String) {
        if (!loading.add(parent)) return
        scope.launch {
            try {
                val fresh = screens.autoChildren(parent).map(AutoItems::mediaItem)
                val before = warm[parent]
                if (fresh == before) return@launch
                if (fresh.isEmpty()) warm.remove(parent) else warm[parent] = fresh
                // The browser paints what it has; this is how it learns that the
                // node has more (MB3c's `LibraryTree` contract).
                onChanged(parent, fresh.size)
            } catch (error: NzException) {
                // The device's copy stays on screen: a node that cannot be
                // reached is not an empty node (CLAUDE.md: a failure to reach a
                // provider is never a verdict).
                Log.i(TAG, "nœud $parent : ${nzErrorText(error)}")
            } finally {
                loading.remove(parent)
            }
        }
    }

    private companion object {
        const val TAG = "NzAutoTree"
    }
}
