package fr.neezer.mobile.feature.playlist

import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.NzRowSource
import fr.neezer.mobile.designsystem.asRowSource
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.PlayContext
import uniffi.nz_ffi.PlaylistConflict
import uniffi.nz_ffi.PlaylistData
import uniffi.nz_ffi.PlaylistExportFormats
import uniffi.nz_ffi.PlaylistExportJob
import uniffi.nz_ffi.PlaylistImportStatus
import uniffi.nz_ffi.PlaylistMember
import uniffi.nz_ffi.PlaylistMembersWrite
import uniffi.nz_ffi.PlaylistMeta
import uniffi.nz_ffi.PlaylistOrder
import uniffi.nz_ffi.PlaylistRecommendations
import uniffi.nz_ffi.PlaylistRole
import uniffi.nz_ffi.PlaylistServices
import uniffi.nz_ffi.PlaylistShareLink
import uniffi.nz_ffi.PlaylistSharedData
import uniffi.nz_ffi.PlaylistWrite
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.SearchScope
import uniffi.nz_ffi.SortKey as FfiSortKey
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow

/** The most rows « Ajouter des titres » lists for one search. */
const val ADD_SEARCH_MAX = 200

/**
 * The rows of one playlist, as the screen draws them (lot MB9).
 *
 * The page travels as a `ListHandle` and this port keeps the design system's own
 * two-method window ([NzRowSource]) together with the three operations the lists
 * have always had — `filter`, `sort`, `version` — so the model is plain Kotlin
 * over a fake in a JVM test while the real screen windowed the real handle
 * (rule 4). The source is [asRowSource], never a local adapter: the day that
 * function was written as `object : NzRowSource { override fun len() = len() }`
 * it called itself and the first windowed screen died of a `StackOverflowError`
 * (MB6's note, `RowSourceTest` holds it).
 */
interface PlaylistRows {
    /** The design system's own window source, from `asRowSource()`. */
    val source: NzRowSource

    /**
     * The real handle, for `rememberListWindow`; `null` for a fake source (a
     * screenshot, a JVM test), which the screen windows with the same `ListWindow`.
     */
    val handle: ListHandle?

    fun len(): Int

    /** Changes on every filter, sort or patch: the UI re-windows when it moves. */
    fun version(): Long

    /** `appcore`'s fold (accents, case, credited names), answered as the new length. */
    fun filter(query: String): Int

    fun sort(by: PlaylistSort, descending: Boolean): Int
}

/** The five orders the SPA offers, plus the playlist's own (`lib/tracklist.js#SORTS`). */
enum class PlaylistSort(val label: String) {
    Position("Ordre d'origine"),
    Title("Titre"),
    Artist("Artiste"),
    Album("Album"),
    Duration("Durée"),
    Added("Date d'ajout"),
}

private fun PlaylistSort.toFfi(): FfiSortKey = when (this) {
    PlaylistSort.Position -> FfiSortKey.POSITION
    PlaylistSort.Title -> FfiSortKey.TITLE
    PlaylistSort.Artist -> FfiSortKey.ARTIST
    PlaylistSort.Album -> FfiSortKey.ALBUM
    PlaylistSort.Duration -> FfiSortKey.DURATION
    PlaylistSort.Added -> FfiSortKey.ADDED
}

/** The real handle, read through the design system's adapter. */
class HandleRows(private val list: ListHandle) : PlaylistRows {
    override val handle: ListHandle = list

    override val source: NzRowSource = list.asRowSource()

    override fun len(): Int = list.len().toInt()

    override fun version(): Long = list.version().toLong()

    override fun filter(query: String): Int = list.filter(query).toInt()

    override fun sort(by: PlaylistSort, descending: Boolean): Int =
        list.sort(by.toFfi(), descending).toInt()
}

/**
 * One page, as the screen draws it: the header, who may do what, the people, and
 * the rows behind [PlaylistRows].
 *
 * Not the FFI's `PlaylistData` record: that one carries the track list as a
 * native `ListHandle`, which a JVM test cannot build. [NZPlaylistService] converts
 * it, so `PlaylistModelTest` and the screenshots run on a fake 4 000-row source —
 * no JNA, no `.so`, no device.
 */
data class PlaylistPage(
    val meta: PlaylistMeta,
    val role: PlaylistRole,
    val editable: Boolean,
    val canManage: Boolean,
    val admin: Boolean,
    val members: List<PlaylistMember>,
    val rows: PlaylistRows,
    val recommendations: PlaylistRecommendations,
    val collaborative: Boolean,
) {
    /** The count the facts line and the "4 000 titres" pill read. */
    val trackCount: Int get() = rows.len()

    /** The owner's entry, which every members list starts with. */
    val owner: PlaylistMember? get() = members.firstOrNull { it.role == PlaylistRole.OWNER }

    /** Whether this is a public link's read-only view (no members, no history). */
    val readOnly: Boolean
        get() = !editable && (role == PlaylistRole.VIEWER || role == PlaylistRole.PUBLIC)
}

fun PlaylistData.toPage(): PlaylistPage = PlaylistPage(
    meta = meta,
    role = role,
    editable = editable,
    canManage = canManage,
    admin = admin,
    members = members,
    rows = HandleRows(tracks),
    recommendations = recommendations,
    collaborative = collaborative,
)

/** What a write answered: the fresh page, and the race it may have lost. */
data class PlaylistWritten(
    val page: PlaylistPage,
    val conflict: PlaylistConflict?,
)

fun PlaylistWrite.toWritten(): PlaylistWritten =
    PlaylistWritten(data.toPage(), conflict)

/** A reorder's answer: the new order's rows and the version the server holds. */
data class PlaylistReordered(
    val rows: PlaylistRows,
    val version: Long,
    val conflict: PlaylistConflict?,
)

fun PlaylistOrder.toReordered(): PlaylistReordered =
    PlaylistReordered(HandleRows(tracks), version = version.toLong(), conflict = conflict)

/**
 * What the playlist screen asks of the core (lot MB9).
 *
 * An interface, not the generated `Screens`, so the model is plain Kotlin that
 * `PlaylistModelTest` runs on the JVM with a fake. [NZPlaylistService] is the one
 * implementation, a one-to-one delegation to `nz-ffi/src/playlist.rs`.
 */
interface PlaylistService {
    /** Disk only, no request: what a warm visit paints before a socket opens. */
    suspend fun cachedPlaylist(id: String): PlaylistPage?

    /** ONE request for the whole page, however long the playlist. */
    suspend fun playlist(id: String): PlaylistPage

    /** The bottom of the page, asked once the list is on screen. */
    suspend fun recommendations(id: String, limit: Int): PlaylistRecommendations

    /** In memory, no request: the drag's answer in the frame. */
    suspend fun move(id: String, from: Int, to: Int): PlaylistReordered

    /** One coalesced `PUT`; a `409` comes back as the server's order. */
    suspend fun saveOrder(id: String): PlaylistReordered

    /** The rows of an open playlist as they stand in memory: 0 requests. */
    suspend fun rows(id: String): PlaylistRows?

    /** Appends in memory (0 requests); the write follows with [saveAppend]. */
    suspend fun append(id: String, tracks: List<TrackRow>): PlaylistReordered

    /** `POST /api/playlist/<id>/tracks` for what [append] added. */
    suspend fun saveAppend(id: String): PlaylistWritten

    /** Removes in memory (0 requests); the write follows with [saveCut]. */
    suspend fun cut(id: String, indexes: List<Int>): PlaylistReordered

    /** `DELETE /api/playlist/<id>/tracks` for what [cut] removed. */
    suspend fun saveCut(id: String, indexes: List<Int>): PlaylistWritten

    suspend fun edit(id: String, title: String?, description: String?): PlaylistWritten

    suspend fun delete(id: String)

    suspend fun leave(id: String)

    suspend fun members(id: String): PlaylistMembersWrite

    suspend fun invite(id: String, username: String, role: PlaylistRole): PlaylistMembersWrite

    suspend fun setRole(id: String, userId: String, role: PlaylistRole): PlaylistMembersWrite

    suspend fun removeMember(id: String, userId: String): PlaylistMembersWrite

    suspend fun shareLink(id: String): PlaylistShareLink?

    suspend fun createShare(id: String): PlaylistShareLink

    suspend fun revokeShare(id: String)

    suspend fun exportFormats(): PlaylistExportFormats

    suspend fun services(): PlaylistServices

    suspend fun export(service: String, kind: String, id: String, name: String, isPublic: Boolean): String

    suspend fun exportJob(job: String): PlaylistExportJob

    /** The list file (CSV/XSPF/text) as text, for the share sheet. */
    suspend fun exportListFile(kind: String, id: String, fmt: String): String

    /** The audio ZIP, written to [path] on the Bulk lane; answers the bytes. */
    suspend fun exportArchive(kind: String, id: String, fmt: String, path: String): ULong

    suspend fun importStart(url: String): String

    suspend fun importStatus(job: String): PlaylistImportStatus

    suspend fun importSave(job: String, title: String, keep: List<Int>): PlaylistImportStatus

    /**
     * The library, searched for « Ajouter des titres » — MB6's own `search` with
     * the library scope, windowed into rows here (`ADD_SEARCH_MAX` at most).
     */
    suspend fun searchTracks(term: String): List<TrackRow>

    suspend fun sharedPlaylist(token: String): PlaylistSharedData

    fun cachedSharedPlaylist(token: String): PlaylistSharedData?
}

/** The one real implementation: this lot's `nz-ffi` methods, converted. */
class NZPlaylistService(private val screens: Screens) : PlaylistService {
    override suspend fun cachedPlaylist(id: String): PlaylistPage? =
        screens.cachedPlaylist(id)?.toPage()

    override suspend fun playlist(id: String): PlaylistPage = screens.playlist(id).toPage()

    override suspend fun recommendations(id: String, limit: Int): PlaylistRecommendations =
        screens.playlistRecommendations(id, limit.toUInt())

    override suspend fun move(id: String, from: Int, to: Int): PlaylistReordered =
        screens.playlistMove(id, from.toUInt(), to.toUInt()).toReordered()

    override suspend fun saveOrder(id: String): PlaylistReordered =
        screens.playlistSaveOrder(id).toReordered()

    override suspend fun rows(id: String): PlaylistRows? =
        screens.playlistRows(id)?.let(::HandleRows)

    override suspend fun append(id: String, tracks: List<TrackRow>): PlaylistReordered =
        screens.playlistAppend(id, tracks).toReordered()

    override suspend fun saveAppend(id: String): PlaylistWritten =
        screens.playlistSaveAppend(id).toWritten()

    override suspend fun cut(id: String, indexes: List<Int>): PlaylistReordered =
        screens.playlistCut(id, indexes.map { it.toUInt() }).toReordered()

    override suspend fun saveCut(id: String, indexes: List<Int>): PlaylistWritten =
        screens.playlistSaveCut(id, indexes.map { it.toUInt() }).toWritten()

    override suspend fun edit(id: String, title: String?, description: String?): PlaylistWritten =
        screens.playlistEdit(id, title, description).toWritten()

    override suspend fun delete(id: String) = screens.playlistDelete(id)

    override suspend fun leave(id: String) = screens.playlistLeave(id)

    override suspend fun members(id: String): PlaylistMembersWrite = screens.playlistMembers(id)

    override suspend fun invite(id: String, username: String, role: PlaylistRole): PlaylistMembersWrite =
        screens.playlistInvite(id, username, role)

    override suspend fun setRole(id: String, userId: String, role: PlaylistRole): PlaylistMembersWrite =
        screens.playlistSetMemberRole(id, userId, role)

    override suspend fun removeMember(id: String, userId: String): PlaylistMembersWrite =
        screens.playlistRemoveMember(id, userId)

    override suspend fun shareLink(id: String): PlaylistShareLink? = screens.playlistShareLink(id)

    override suspend fun createShare(id: String): PlaylistShareLink = screens.playlistCreateShare(id, null)

    override suspend fun revokeShare(id: String) = screens.playlistRevokeShare(id)

    override suspend fun exportFormats(): PlaylistExportFormats = screens.playlistExportFormats()

    override suspend fun services(): PlaylistServices = screens.playlistServices()

    override suspend fun export(
        service: String,
        kind: String,
        id: String,
        name: String,
        isPublic: Boolean,
    ): String = screens.playlistExport(service, kind, id, name, isPublic)

    override suspend fun exportJob(job: String): PlaylistExportJob = screens.playlistExportJob(job)

    override suspend fun exportListFile(kind: String, id: String, fmt: String): String =
        screens.playlistExportListFile(kind, id, fmt)

    override suspend fun exportArchive(kind: String, id: String, fmt: String, path: String): ULong =
        screens.playlistExportArchive(kind, id, fmt, path)

    override suspend fun importStart(url: String): String = screens.playlistImportStart(url)

    override suspend fun importStatus(job: String): PlaylistImportStatus =
        screens.playlistImportStatus(job)

    override suspend fun importSave(job: String, title: String, keep: List<Int>): PlaylistImportStatus =
        screens.playlistImportSave(job, title, keep.map { it.toUInt() })

    override suspend fun searchTracks(term: String): List<TrackRow> {
        val answer = screens.search(term, SearchScope.LIBRARY, false)
        val handle = answer.tracks
        val out = ArrayList<TrackRow>()
        var start = 0
        val len = handle.len().toInt()
        while (start < len && out.size < ADD_SEARCH_MAX) {
            val page = handle.window(start.toUInt(), 64u)
            if (page.isEmpty()) break
            out += page
            start += page.size
        }
        return out
    }

    override suspend fun sharedPlaylist(token: String): PlaylistSharedData =
        screens.sharedPlaylist(token)

    override fun cachedSharedPlaylist(token: String): PlaylistSharedData? =
        screens.cachedSharedPlaylist(token)
}

/**
 * What the playlist screen asks of the player (MB3b's `PlayerApi`).
 *
 * The screen never holds a `PlayerApi`: it hands the rows and the context to this
 * seam, so a JVM test asserts the COMMAND a press sends with a fake, and the real
 * one is this one thin file.
 */
interface PlaylistPlayer {
    /** Replaces the queue with [tracks] under a context the player keeps. */
    fun play(tracks: List<TrackRow>, contextKind: String, contextId: String, start: Int = 0)

    /** Queues [tracks] shuffled, same context. */
    fun shuffle(tracks: List<TrackRow>, contextKind: String, contextId: String)
}

/** A port that does nothing, for a process whose core could not be built. */
object NoPlaylistPlayer : PlaylistPlayer {
    override fun play(tracks: List<TrackRow>, contextKind: String, contextId: String, start: Int) = Unit

    override fun shuffle(tracks: List<TrackRow>, contextKind: String, contextId: String) = Unit
}

/** The real port: `PlayerCommand.PlayQueue`, on the player's own worker. */
class NZPlaylistPlayer(private val player: PlayerApi) : PlaylistPlayer {
    override fun play(tracks: List<TrackRow>, contextKind: String, contextId: String, start: Int) {
        if (tracks.isEmpty()) return
        player.command(
            PlayerCommand.PlayQueue(
                tracks = tracks.map(::ref),
                start = start.coerceAtLeast(0).toUInt(),
                context = PlayContext(kind = contextKind, id = contextId),
            ),
            0uL,
        )
    }

    override fun shuffle(tracks: List<TrackRow>, contextKind: String, contextId: String) {
        if (tracks.isEmpty()) return
        // The queue is loaded as it is (`playQueue`, `start = 0`) and the player's
        // OWN shuffle state does the rest: the order the person sees stays the
        // playlist's, and the server-side radio rules are the player's.
        player.command(
            PlayerCommand.PlayQueue(
                tracks = tracks.map(::ref),
                start = 0u,
                context = PlayContext(kind = contextKind, id = contextId),
            ),
            0uL,
        )
        player.command(PlayerCommand.SetShuffle(on = true), 0uL)
    }

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

/** The app's player, asked of the core for every command. */
object PlaylistPlayerHolder {
    fun api(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()
}

/**
 * Where a text the person asked to share goes (the public link, a list file).
 *
 * A seam because the two platforms differ (`ACTION_SEND` here, `UIActivity` on
 * iOS) and because a JVM test asserts WHAT was shared with a fake.
 */
interface PlaylistSharer {
    fun share(text: String, title: String)
    fun copy(text: String, label: String)
}

/** A share port that does nothing: the screenshots and a core-less process. */
object NoPlaylistSharer : PlaylistSharer {
    override fun share(text: String, title: String) = Unit
    override fun copy(text: String, label: String) = Unit
}
