// :playback — the media ids of Android Auto's browse tree (docs/agents/mobile.md,
// "MB25 — Android Auto").
//
// The tree itself is Rust's (`nz-core/src/auto/`): the five nodes, their lanes,
// their device copy and what a tap plays all live there and are tested against a
// real HTTP server. What Kotlin must know WITHOUT asking Rust is the shape of a
// media id, for one reason only: `onSetMediaItems` has to tell a CONTAINER (a
// node whose tap queues the titles it showed) from a TITLE (a queue row handed
// back by the notification, the lock screen or Bluetooth, which must keep the
// old path exactly). Asking Rust would mean an FFI call inside a decision that
// must be made in the same frame, and a `NzException` for the ordinary case.
package fr.neezer.mobile.playback.auto

/** What a media id of the tree names. */
enum class AutoNodeKind {
    /** The root a controller connects to. */
    Root,

    /** `neezer:favorites` — browsable and playable. */
    Favorites,

    /** `neezer:playlists` — the folder of the person's lists. */
    Playlists,

    /** `neezer:playlist:<uuid>` — one list. */
    Playlist,

    /** `neezer:flow` — playable only: the queue asks for each next title. */
    Flow,

    /** `neezer:recent` — the listening history. */
    Recent,

    /** `neezer:downloads` — what the device holds. */
    Downloads,

    /** `neezer:track:<key>` — one title. */
    Track,

    /** Anything else, including a hostile string. */
    Unknown,
}

/**
 * The ids of the tree, and the one question the session's callback asks.
 *
 * Everything here is the SAME vocabulary as `nz-core/src/auto/mod.rs`: the
 * strings are that module's constants, and a test holds the two in step by
 * naming them once (`AutoNodeTest`).
 */
object AutoNode {
    const val ROOT = "neezer:root"
    const val FAVORITES = "neezer:favorites"
    const val PLAYLISTS = "neezer:playlists"
    const val FLOW = "neezer:flow"
    const val RECENT = "neezer:recent"
    const val DOWNLOADS = "neezer:downloads"
    const val PLAYLIST_PREFIX = "neezer:playlist:"
    const val TRACK_PREFIX = "neezer:track:"

    /** The root's title: the app's own name, never a node's. */
    const val ROOT_TITLE = "Neezer"

    /** `true` for an id made of the characters the tree's ids use. */
    private fun valid(argument: String): Boolean =
        argument.isNotEmpty() &&
            argument.length <= 160 &&
            argument.all { c ->
                c in 'A'..'Z' || c in 'a'..'z' || c in '0'..'9' ||
                    c == '-' || c == '_' || c == '.' || c == ':'
            }

    /** What [mediaId] names. */
    fun kind(mediaId: String): AutoNodeKind {
        val id = mediaId.trim()
        return when (id) {
            ROOT -> AutoNodeKind.Root
            FAVORITES -> AutoNodeKind.Favorites
            PLAYLISTS -> AutoNodeKind.Playlists
            FLOW -> AutoNodeKind.Flow
            RECENT -> AutoNodeKind.Recent
            DOWNLOADS -> AutoNodeKind.Downloads
            else ->
                when {
                    id.startsWith(PLAYLIST_PREFIX) && valid(id.removePrefix(PLAYLIST_PREFIX)) ->
                        AutoNodeKind.Playlist
                    id.startsWith(TRACK_PREFIX) && valid(id.removePrefix(TRACK_PREFIX)) ->
                        AutoNodeKind.Track
                    else -> AutoNodeKind.Unknown
                }
        }
    }

    /**
     * `true` when a tap on [mediaId] is a tap on a NODE (its titles become the
     * queue), `false` when it is already a title or something we never built.
     */
    fun isContainer(mediaId: String): Boolean =
        when (kind(mediaId)) {
            AutoNodeKind.Favorites,
            AutoNodeKind.Playlist,
            AutoNodeKind.Flow,
            AutoNodeKind.Recent,
            AutoNodeKind.Downloads,
            -> true
            AutoNodeKind.Root,
            AutoNodeKind.Playlists,
            AutoNodeKind.Track,
            AutoNodeKind.Unknown,
            -> false
        }

    /** The argument of a playlist or track id (`""` for anything else). */
    fun argument(mediaId: String): String {
        val id = mediaId.trim()
        return when (kind(id)) {
            AutoNodeKind.Playlist -> id.removePrefix(PLAYLIST_PREFIX)
            AutoNodeKind.Track -> id.removePrefix(TRACK_PREFIX)
            else -> ""
        }
    }
}

/**
 * One page of a node's children.
 *
 * A car list is browsed a dozen rows at a time, and Rust caps a node at a
 * hundred children (`nz_core::auto::MAX_CHILDREN`): this slices what the device
 * has, so a controller that asks for "everything" (`pageSize` is
 * `Int.MAX_VALUE` on the platform path) still gets one bounded page (rule 4).
 */
object AutoPage {
    /** The ceiling of one page, the same hundred a node holds. */
    const val MAX_PAGE = 100

    fun <T> slice(items: List<T>, page: Int, pageSize: Int): List<T> {
        if (items.isEmpty()) return emptyList()
        val size = if (pageSize <= 0 || pageSize > MAX_PAGE) MAX_PAGE else pageSize
        val start = page.coerceAtLeast(0).toLong() * size
        if (start >= items.size) return emptyList()
        val from = start.toInt()
        return items.subList(from, minOf(from + size, items.size))
    }
}
