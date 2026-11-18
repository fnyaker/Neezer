package fr.neezer.mobile.feature.trackactions

import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.NzGlyph
import uniffi.nz_ffi.MenuActor
import uniffi.nz_ffi.MenuContext
import uniffi.nz_ffi.MenuEntry
import uniffi.nz_ffi.MenuPerson
import uniffi.nz_ffi.MutationReport
import uniffi.nz_ffi.PlaylistChoice
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.ReplaceAnswer
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow

/**
 * What the track actions ask of the core (lot MB10, `nz-ffi/src/actions.rs`).
 *
 * An interface, not the generated `Screens`, so the model is plain Kotlin that
 * `TrackActionsModelTest` runs on the JVM with a fake — no JNA, no `.so`, no
 * device — and every rule of the menu is an assertion over counted calls.
 * [NzTrackActionsService] is the one implementation, a one-to-one delegation.
 */
interface TrackActionsService {
    /** The admin flag and the last playlist used: no request. */
    fun actor(): MenuActor

    /** The menu of [row] — the ONE spec, built in Rust: **0 requests**. */
    fun menu(row: TrackRow, ctx: MenuContext): List<MenuEntry>

    /** The picker's list as the device last saw it: disk only, 0 requests. */
    fun cachedPlaylists(): List<PlaylistChoice>?

    /** `GET /api/me/playlists` — one request. */
    suspend fun playlists(): List<PlaylistChoice>

    /** `POST /api/playlist/<id>/tracks` — one request; answers how many were added. */
    suspend fun addToPlaylist(playlistId: String, title: String, keys: List<String>): Int

    /** `POST /api/playlists` — one request. */
    suspend fun createPlaylist(title: String, keys: List<String>): PlaylistChoice

    suspend fun like(key: String)

    suspend fun unlike(key: String)

    suspend fun dislike(key: String)

    suspend fun undislike(key: String)

    /** `GET /api/replace/candidates/<id>` — one request. */
    suspend fun replaceCandidates(key: String, query: String, fallbacks: Boolean): ReplaceAnswer

    /** `POST /api/replace` — one request; the rewrite is the server's job. */
    suspend fun replace(key: String, candidate: String): MutationReport

    /** `GET /api/replace/status/<job>` — the job's progress. */
    suspend fun replaceStatus(job: String): MutationReport

    /** `POST /api/tracks/delete` — the bulk deletion of dead titles. */
    suspend fun deleteTracks(keys: List<String>): MutationReport

    suspend fun deleteStatus(job: String): MutationReport

    /** `DELETE /api/local/<id>` — one of your own uploads. */
    suspend fun deleteUploads(keys: List<String>): MutationReport

    /** The address to share for `(kind, id)`. */
    suspend fun shareLink(kind: String, id: String): String
}

/** The one real implementation: this lot's `nz-ffi` methods. */
class NzTrackActionsService(private val screens: Screens) : TrackActionsService {
    override fun actor(): MenuActor = screens.menuActor()

    override fun menu(row: TrackRow, ctx: MenuContext): List<MenuEntry> = screens.trackMenu(row, ctx)

    override fun cachedPlaylists(): List<PlaylistChoice>? = screens.cachedMenuPlaylists()

    override suspend fun playlists(): List<PlaylistChoice> = screens.menuPlaylists()

    override suspend fun addToPlaylist(
        playlistId: String,
        title: String,
        keys: List<String>,
    ): Int = screens.addToPlaylist(playlistId, title, keys).toInt()

    override suspend fun createPlaylist(title: String, keys: List<String>): PlaylistChoice =
        screens.createPlaylistFrom(title, keys)

    override suspend fun like(key: String) = screens.like(key)

    override suspend fun unlike(key: String) = screens.unlike(key)

    override suspend fun dislike(key: String) = screens.dislike(key)

    override suspend fun undislike(key: String) = screens.undislike(key)

    override suspend fun replaceCandidates(
        key: String,
        query: String,
        fallbacks: Boolean,
    ): ReplaceAnswer = screens.replaceCandidates(key, query, fallbacks)

    override suspend fun replace(key: String, candidate: String): MutationReport =
        screens.replace(key, candidate)

    override suspend fun replaceStatus(job: String): MutationReport = screens.replaceStatus(job)

    override suspend fun deleteTracks(keys: List<String>): MutationReport = screens.deleteTracks(keys)

    override suspend fun deleteStatus(job: String): MutationReport = screens.deleteStatus(job)

    override suspend fun deleteUploads(keys: List<String>): MutationReport =
        screens.deleteUploads(keys)

    override suspend fun shareLink(kind: String, id: String): String = screens.shareLink(kind, id)
}

/**
 * What a menu entry asks of the PLAYER (MB3b's `PlayerApi`): the verbs that never
 * touch the network themselves, so a JVM test asserts the COMMAND a tap sends.
 * The play rules (the queue, the radio's refill, a removal that follows the
 * playhead) stay the player's — this seam only names them.
 */
interface TrackActionPlayer {
    /** "Lire ensuite" — right after the playing title. */
    fun playNext(row: TrackRow)

    /** "Ajouter à la file". */
    fun addToQueue(row: TrackRow)

    /** "Retirer de la file", by KEY: a playhead that moved cannot remove the wrong row. */
    fun removeFromQueue(key: String, index: Int)

    /** "Lancer la radio": the player fetches the station and owns the queue. */
    fun startRadio(row: TrackRow)

    /** The swipe right on a row (its own gesture in `TrackRowView`). */
    fun swipeQueue(row: TrackRow) = playNext(row)
}

/** A port that does nothing, for a process whose core could not be built. */
object NoTrackActionPlayer : TrackActionPlayer {
    override fun playNext(row: TrackRow) = Unit

    override fun addToQueue(row: TrackRow) = Unit

    override fun removeFromQueue(key: String, index: Int) = Unit

    override fun startRadio(row: TrackRow) = Unit
}

/** The real port: `PlayerCommand`, on the player's own worker (never blocks a click). */
class NzTrackActionPlayer(private val player: PlayerApi) : TrackActionPlayer {
    override fun playNext(row: TrackRow) {
        player.command(PlayerCommand.PlayNext(tracks = listOf(ref(row))), 0uL)
    }

    override fun addToQueue(row: TrackRow) {
        player.command(PlayerCommand.AddToQueue(tracks = listOf(ref(row))), 0UL)
    }

    override fun removeFromQueue(key: String, index: Int) {
        player.command(
            PlayerCommand.Remove(index = index.coerceAtLeast(0).toUInt(), key = key),
            0uL,
        )
    }

    override fun startRadio(row: TrackRow) {
        // `lib/actions.js#startTrackRadio`: a station seeded on this title. One
        // command: the player asks `/api/radio/track/<id>` on the User lane and
        // plays the seed first — the tap never waits on the network.
        player.command(
            PlayerCommand.StartRadio(kind = "track", id = row.id),
            System.nanoTime().toULong(),
        )
    }

    /** A screen row, as the queue's own reference. */
    private fun ref(row: TrackRow): TrackRef = TrackRef(
        key = row.key,
        id = row.id,
        title = row.title,
        artists = row.artists,
        album = row.album,
        albumId = row.albumId,
        artistId = null,
        cover = row.cover,
        durationS = row.durationS,
        episode = row.source == "podcast",
        liked = row.liked,
    )
}

/** Where a share address goes: the platform's own sheet (never a copy to a log). */
interface TrackActionSharer {
    /** Hands `text` to the platform's share sheet, titled [title]. */
    fun share(text: String, title: String)
}

/** A sharer that only records: the JVM tests', and the shell's fallback. */
class RecordingSharer : TrackActionSharer {
    val shared = mutableListOf<Pair<String, String>>()

    override fun share(text: String, title: String) {
        shared += text to title
    }
}

/**
 * The app's player, asked of the core. `NzCore::player()` is ONE per process
 * (MB3c's measurement), so asking it per opening is correct and cheap (the same
 * holder shape MB5's home and MB7's entity use, in this module's own file).
 */
object TrackActionsPlayerHolder {
    fun api(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()
}

/**
 * The Sillon glyph of an entry's icon name.
 *
 * The core carries the SPA's own icon NAMES (`trackmenu.js`'s vocabulary), so the
 * menu's labels and its glyphs come from the same spec; this table is the one
 * place a name becomes a glyph, and an unknown name falls back to a neutral mark
 * rather than inventing a shape.
 */
fun TrackActionsGlyph(name: String): NzGlyph = when (name) {
    "next" -> NzGlyph.Next
    "queue" -> NzGlyph.Queue
    "close" -> NzGlyph.Close
    "plus" -> NzGlyph.Plus
    "music" -> NzGlyph.Music
    "refresh" -> NzGlyph.Refresh
    "tag" -> NzGlyph.Tag
    "download" -> NzGlyph.Download
    "downloaded" -> NzGlyph.Downloaded
    "share" -> NzGlyph.Share
    "radio" -> NzGlyph.Radio
    "heart" -> NzGlyph.Heart
    "heartFilled" -> NzGlyph.HeartFilled
    "thumbsDown" -> NzGlyph.ThumbsDown
    "thumbsDownFilled" -> NzGlyph.ThumbsDownFilled
    "users" -> NzGlyph.Users
    "user" -> NzGlyph.User
    "disc" -> NzGlyph.Disc
    "mic" -> NzGlyph.Mic
    "bookmarkPlus" -> NzGlyph.BookmarkPlus
    "trash" -> NzGlyph.Trash
    else -> NzGlyph.More
}

/** A queue's context, as the menu's own record (the FFI's `MenuContext`). */
internal fun menuContext(
    surface: String = "list",
    index: Int? = null,
    currentIndex: Int? = null,
    canKeep: Boolean = true,
    canBrowse: Boolean = true,
    admin: Boolean = false,
    remote: Boolean = false,
    downloaded: Boolean = false,
    lastPlaylistId: String? = null,
    lastPlaylistTitle: String? = null,
    lastPlaylistEditable: Boolean = false,
    artists: List<MenuPerson> = emptyList(),
    episode: Boolean = false,
    episodePlaying: Boolean = false,
    channelId: String? = null,
    credits: Boolean = true,
    offline: Boolean = false,
    genreTag: Boolean = false,
    markers: Boolean = false,
): MenuContext = MenuContext(
    surface = surface,
    index = index?.coerceAtLeast(0)?.toUInt(),
    currentIndex = currentIndex,
    canKeep = canKeep,
    canBrowse = canBrowse,
    admin = admin,
    remote = remote,
    downloaded = downloaded,
    lastPlaylistId = lastPlaylistId,
    lastPlaylistTitle = lastPlaylistTitle,
    lastPlaylistEditable = lastPlaylistEditable,
    artists = artists,
    episode = episode,
    episodePlaying = episodePlaying,
    channelId = channelId,
    credits = credits,
    offline = offline,
    genreTag = genreTag,
    markers = markers,
)
