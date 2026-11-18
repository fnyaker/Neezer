package fr.neezer.mobile.feature.library

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.FavoriteIdsData
import uniffi.nz_ffi.HistoryPageData
import uniffi.nz_ffi.LibraryAlbum
import uniffi.nz_ffi.LibraryArtist
import uniffi.nz_ffi.LibraryTree
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.PlayContext
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.SortKey
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.UploadUsageData

/**
 * One of the library's lists, as this module reads it (lot MB8).
 *
 * The real one wraps a `ListHandle`, so 8 000 favourites stay in Rust and the
 * screen asks the page it is about to paint. It is an interface of its own for
 * one reason: the JVM tests and the screenshot renders hold a fake over a plain
 * list, and the window's own policy is then asserted instead of hoped for.
 *
 * [handle] is what the screen windows with (`rememberListWindow`, the designsystem
 * helper MB6 fixed in #321); a fake has none and windows its own source.
 */
interface LibraryRows {
    /** The real handle, or `null` for a fake (a test, a render). */
    val handle: ListHandle?

    fun len(): Int

    fun window(start: Int, count: Int): List<TrackRow>

    /** `ListHandle.filter`: `appcore`'s fold, answered as the new length. */
    fun filter(query: String): Int

    fun sort(by: SortKey, descending: Boolean): Int

    fun version(): ULong
}

/** The real handle, unchanged — the rows never cross the boundary whole. */
class HandleRows(override val handle: ListHandle) : LibraryRows {
    override fun len(): Int = handle.len().toInt()

    override fun window(start: Int, count: Int): List<TrackRow> =
        handle.window(start.coerceAtLeast(0).toUInt(), count.coerceAtLeast(0).toUInt())

    override fun filter(query: String): Int = handle.filter(query).toInt()

    override fun sort(by: SortKey, descending: Boolean): Int = handle.sort(by, descending).toInt()

    override fun version(): ULong = handle.version()
}

/**
 * What the library asks of the core: this lot's `nz-ffi` methods, one to one.
 *
 * An interface, not the generated `Screens`, so the screen's logic is plain
 * Kotlin that `LibraryModelTest` runs on the JVM with a fake — no JNA, no `.so`,
 * no device. [NzLibraryService] is the one real implementation.
 */
interface LibraryService {
    /** The playlist tree the device kept, or `null` the first time. */
    suspend fun cachedTree(): LibraryTree?

    /** `GET /api/me/playlists` — one request for the folders and the playlists. */
    suspend fun tree(): LibraryTree

    /** Whether the server has the folder routes (rule 7); `null` is a doubt. */
    suspend fun foldersSupported(): Boolean?

    suspend fun createFolder(title: String): LibraryTree

    suspend fun renameFolder(id: String, title: String): LibraryTree

    suspend fun deleteFolder(id: String): LibraryTree

    suspend fun reorderFolders(ids: List<String>): LibraryTree

    suspend fun reorderContainer(folder: String?, ids: List<String>): LibraryTree

    suspend fun movePlaylistTo(playlistId: String, folder: String?): LibraryTree

    /** The favourites this device kept, no request. */
    suspend fun cachedFavorites(): LibraryRows?

    /** `GET /api/me/favorites` — the whole list in ONE request. */
    suspend fun favorites(): LibraryRows

    suspend fun cachedArtists(): List<LibraryArtist>?

    suspend fun artists(): List<LibraryArtist>

    suspend fun cachedAlbums(): List<LibraryAlbum>?

    suspend fun albums(): List<LibraryAlbum>

    suspend fun cachedLocal(): LibraryRows?

    suspend fun local(): LibraryRows

    suspend fun cachedUsage(): UploadUsageData?

    suspend fun usage(): UploadUsageData

    suspend fun cachedDislikes(): LibraryRows?

    suspend fun dislikes(): LibraryRows

    suspend fun removeDislike(reference: String)

    suspend fun cachedUnavailable(): LibraryRows?

    suspend fun unavailable(): LibraryRows

    /** The ids only: every heart, and the car's counter. */
    suspend fun favoriteIds(): FavoriteIdsData
}

/** What the history route asks: its own pair, so the two screens load apart. */
interface HistoryService {
    suspend fun cached(outcome: String): HistoryPageData?

    suspend fun page(
        outcome: String?,
        before: String?,
        beforeId: Long?,
        limit: UInt,
    ): HistoryPageData
}

/** The one real implementation of [LibraryService]. */
class NzLibraryService(private val screens: Screens) : LibraryService {
    override suspend fun cachedTree(): LibraryTree? = screens.cachedLibraryPlaylists()

    override suspend fun tree(): LibraryTree = screens.libraryPlaylists()

    override suspend fun foldersSupported(): Boolean? = screens.foldersSupported()

    override suspend fun createFolder(title: String): LibraryTree = screens.createFolder(title)

    override suspend fun renameFolder(id: String, title: String): LibraryTree =
        screens.renameFolder(id, title)

    override suspend fun deleteFolder(id: String): LibraryTree = screens.deleteFolder(id)

    override suspend fun reorderFolders(ids: List<String>): LibraryTree =
        screens.reorderFolders(ids)

    override suspend fun reorderContainer(folder: String?, ids: List<String>): LibraryTree =
        screens.reorderContainer(folder, ids)

    override suspend fun movePlaylistTo(playlistId: String, folder: String?): LibraryTree =
        screens.movePlaylistTo(playlistId, folder)

    override suspend fun cachedFavorites(): LibraryRows? =
        screens.cachedFavorites()?.let(::HandleRows)

    override suspend fun favorites(): LibraryRows = HandleRows(screens.favorites())

    override suspend fun cachedArtists(): List<LibraryArtist>? = screens.cachedArtists()

    override suspend fun artists(): List<LibraryArtist> = screens.artists()

    override suspend fun cachedAlbums(): List<LibraryAlbum>? = screens.cachedAlbums()

    override suspend fun albums(): List<LibraryAlbum> = screens.albums()

    override suspend fun cachedLocal(): LibraryRows? = screens.cachedLocal()?.let(::HandleRows)

    override suspend fun local(): LibraryRows = HandleRows(screens.local())

    override suspend fun cachedUsage(): UploadUsageData? = screens.cachedUploadUsage()

    override suspend fun usage(): UploadUsageData = screens.uploadUsage()

    override suspend fun cachedDislikes(): LibraryRows? =
        screens.cachedDislikes()?.let(::HandleRows)

    override suspend fun dislikes(): LibraryRows = HandleRows(screens.dislikes())

    override suspend fun removeDislike(reference: String) = screens.removeDislike(reference)

    override suspend fun cachedUnavailable(): LibraryRows? =
        screens.cachedUnavailable()?.let(::HandleRows)

    override suspend fun unavailable(): LibraryRows = HandleRows(screens.unavailable())

    override suspend fun favoriteIds(): FavoriteIdsData = screens.favoriteIds()
}

/** The one real implementation of [HistoryService]. */
class NzHistoryService(private val screens: Screens) : HistoryService {
    override suspend fun cached(outcome: String): HistoryPageData? = screens.cachedHistory(outcome)

    override suspend fun page(
        outcome: String?,
        before: String?,
        beforeId: Long?,
        limit: UInt,
    ): HistoryPageData = screens.history(outcome, before, beforeId, limit)
}

/**
 * What the library asks of the player (MB3b's `PlayerApi`).
 *
 * The library never holds a `PlayerApi`: it hands the rows to this seam, so a
 * JVM test asserts the COMMAND a tap sends with a fake. The queue rules (the
 * context, the refill, the shuffle) are the player's and are not repeated here.
 */
interface LibraryPlayer {
    /** Replaces the queue with [tracks] and starts at [start]. */
    fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String)

    /** « Mélanger »: the whole list, shuffled (`shufflePlay`). */
    fun playShuffled(tracks: List<TrackRow>, contextKind: String, contextId: String)

    /** Swipe right on a row: « Lire ensuite ». */
    fun playNext(row: TrackRow)
}

/** A port that does nothing, for a process whose core could not be built. */
object NoLibraryPlayer : LibraryPlayer {
    override fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String) =
        Unit

    override fun playShuffled(tracks: List<TrackRow>, contextKind: String, contextId: String) =
        Unit

    override fun playNext(row: TrackRow) = Unit
}

/** The real port: `PlayerCommand`, in the player's own words. */
class NzLibraryPlayer(private val player: PlayerApi) : LibraryPlayer {
    override fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String) {
        player.command(
            PlayerCommand.PlayQueue(
                tracks = tracks.map(::ref),
                start = start.coerceAtLeast(0).toUInt(),
                context = PlayContext(kind = contextKind, id = contextId),
            ),
            0uL,
        )
    }

    override fun playShuffled(tracks: List<TrackRow>, contextKind: String, contextId: String) {
        // The queue first, then shuffle mode: the SPA's `shufflePlay` keeps the
        // list whole and lets the player draw from it.
        play(tracks, 0, contextKind, contextId)
        player.command(PlayerCommand.SetShuffle(on = true), 0uL)
    }

    override fun playNext(row: TrackRow) {
        player.command(PlayerCommand.PlayNext(tracks = listOf(ref(row))), 0uL)
    }

    /** A screen row, as the queue's own reference (what a playlist row hands it). */
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
        episode = false,
        liked = row.liked,
    )
}

/**
 * The app's player, asked of the core for every command.
 *
 * No cache: `NzCore::player()` answers the ONE instance the core owns (a
 * `OnceLock`, PR #313), so asking per command is correct and cheap — and a cache
 * here would keep an object the core could replace (MB5's own note).
 */
object AppLibraryPlayer {
    fun api(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()
}
