package fr.neezer.mobile.feature.library

import fr.neezer.mobile.designsystem.NzRowSource
import uniffi.nz_ffi.FavoriteIdsData
import uniffi.nz_ffi.FolderRow
import uniffi.nz_ffi.HistoryArtistData
import uniffi.nz_ffi.HistoryCursorData
import uniffi.nz_ffi.HistoryEventData
import uniffi.nz_ffi.HistoryPageData
import uniffi.nz_ffi.HistorySummaryData
import uniffi.nz_ffi.HistoryTrackData
import uniffi.nz_ffi.LibraryAlbum
import uniffi.nz_ffi.LibraryArtist
import uniffi.nz_ffi.LibrarySection
import uniffi.nz_ffi.LibrarySectionKind
import uniffi.nz_ffi.LibraryTree
import uniffi.nz_ffi.PlaylistRow
import uniffi.nz_ffi.SortKey
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.UploadUsageData

/**
 * Rows a test draws, as a [LibraryRows] over a plain list.
 *
 * The real one wraps a `ListHandle`; this one is the same three methods plus the
 * two tools the favourites table has, and it COUNTS them — so a test asserts
 * "one filter pass, one sort" instead of hoping.
 */
class FakeRows(initial: List<TrackRow>, private val versionOf: ULong = 1uL) : LibraryRows {
    override val handle: uniffi.nz_ffi.ListHandle? = null

    private val items = initial
    private var query = ""
    private var visible = initial

    var windows = 0
        private set
    var filters = 0
        private set
    var sorts = 0
        private set

    override fun len(): Int = visible.size

    override fun window(start: Int, count: Int): List<TrackRow> {
        windows++
        if (start >= visible.size || count <= 0) return emptyList()
        return visible.subList(start, minOf(start + count, visible.size))
    }

    override fun filter(value: String): Int {
        filters++
        query = value
        visible = if (value.isBlank()) items else matches(value)
        return visible.size
    }

    /** The SPA's own filter order: the query first, then the order (`sort` keeps it). */
    override fun sort(by: SortKey, descending: Boolean): Int {
        sorts++
        val base = if (query.isBlank()) items else matches(query)
        visible = when (by) {
            SortKey.TITLE -> base.sortedBy { it.title.lowercase() }
            SortKey.ARTIST -> base.sortedBy { it.artists.lowercase() }
            SortKey.ALBUM -> base.sortedBy { it.album.lowercase() }
            SortKey.DURATION -> base.sortedBy { it.durationS }
            SortKey.ADDED -> base
            else -> base
        }
        if (descending) visible = visible.reversed()
        return visible.size
    }

    private fun matches(value: String): List<TrackRow> =
        items.filter { it.title.contains(value, ignoreCase = true) }

    override fun version(): ULong = versionOf

    fun keys(): List<String> = visible.map { it.key }
}

/** The rows a page draws, as the designsystem's own source (a fake window). */
fun sourceOf(rows: LibraryRows): NzRowSource = object : NzRowSource {
    override fun len(): Int = rows.len()

    override fun window(start: Int, count: Int): List<TrackRow> = rows.window(start, count)
}

/** The fixtures the model tests and the renders share (the real records). */
object LibraryFixtures {
    const val VOYAGE = "ffffffff-1111-4111-8111-000000000001"
    const val EMPTY_FOLDER = "ffffffff-1111-4111-8111-000000000002"
    const val ROUTE = "aaaaaaaa-1111-4111-8111-000000000001"
    const val MATIN = "aaaaaaaa-1111-4111-8111-000000000002"
    const val SHARED = "aaaaaaaa-1111-4111-8111-000000000003"
    const val IMPORTED = "aaaaaaaa-1111-4111-8111-000000000004"
    const val ARTIST = "bbbbbbbb-1111-4111-8111-000000000001"
    const val ALBUM = "cccccccc-1111-4111-8111-000000000001"

    fun row(title: String, position: Int = 0, artists: String = "Ascenseur"): TrackRow = TrackRow(
        key = "k-$position",
        id = "row-$position",
        title = title,
        artists = artists,
        album = "Le matin",
        albumId = "501",
        durationS = 180u + position.toUInt(),
        cover = "/api/cover/501",
        explicit = false,
        liked = true,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = position.toUInt(),
    )

    fun rows(count: Int): List<TrackRow> = (0 until count).map { index ->
        row(if (index % 2 == 0) "Ouverture $index" else "Sinnerman $index", index)
    }

    fun folder(id: String, title: String, position: Long, count: UInt): FolderRow =
        FolderRow(id = id, title = title, position = position, count = count)

    fun playlist(
        id: String,
        title: String,
        folder: String? = null,
        tracks: UInt = 12u,
        role: String = "owner",
        owner: String = "Nadia",
        deezer: String? = null,
    ): PlaylistRow = PlaylistRow(
        id = id,
        deezerId = deezer,
        title = title,
        cover = "/api/cover/501",
        tracks = tracks,
        shared = role != "owner",
        role = role,
        owner = owner,
        editable = true,
        canManage = true,
        folderId = folder,
        position = null,
    )

    fun section(
        key: String,
        title: String,
        kind: LibrarySectionKind,
        count: Int,
        rows: List<PlaylistRow>,
    ): LibrarySection = LibrarySection(
        kind = kind,
        key = key,
        title = title,
        count = count.toUInt(),
        rows = rows,
    )

    /** The SPA's own grouping of the fixtures below (the core computes it live). */
    fun tree(): LibraryTree {
        val folders = listOf(
            folder(VOYAGE, "Voyage", 0, 1u),
            folder(EMPTY_FOLDER, "Vide", 1, 0u),
        )
        val playlists = listOf(
            playlist(ROUTE, "Route", folder = VOYAGE),
            playlist(MATIN, "Matin", tracks = 3u),
            playlist(SHARED, "Avec June", tracks = 8u, role = "editor", owner = "June"),
            playlist(IMPORTED, "Import Deezer", tracks = 25u, deezer = "9080"),
        )
        return LibraryTree(
            folders = folders,
            playlists = playlists,
            sections = listOf(
                section("folder:$VOYAGE", "Voyage", LibrarySectionKind.FOLDER, 1, listOf(playlists[0])),
                section("folder:$EMPTY_FOLDER", "Vide", LibrarySectionKind.FOLDER, 0, emptyList()),
                section("root", "Sans dossier", LibrarySectionKind.ROOT, 1, listOf(playlists[1])),
                section("shared", "Partagées avec moi", LibrarySectionKind.SHARED, 1, listOf(playlists[2])),
                section("imported", "Depuis Deezer", LibrarySectionKind.IMPORTED, 1, listOf(playlists[3])),
            ),
        )
    }

    fun artist(name: String, plays: UInt, starred: String, tracks: UInt = 40u): LibraryArtist =
        LibraryArtist(
            id = "artist-$name",
            name = name,
            picture = null,
            tracks = tracks,
            plays = plays,
            starred = starred,
        )

    fun artists(): List<LibraryArtist> = listOf(
        artist("Ascenseur", 340u, "2026-11-02T10:00:00"),
        artist("Zazie", 12u, "2026-11-12T21:00:00"),
    )

    fun albums(): List<LibraryAlbum> = listOf(
        LibraryAlbum(
            id = ALBUM,
            title = "Le matin",
            cover = "/api/cover/501",
            artist = "Ascenseur",
            year = "2024",
            tracks = 12u,
        ),
        LibraryAlbum(
            id = "cccccccc-1111-4111-8111-000000000002",
            title = "Zen",
            cover = null,
            artist = "Zazie",
            year = "2019",
            tracks = 4u,
        ),
    )

    fun usage(): UploadUsageData =
        UploadUsageData(used = 1_073_741_824uL, quota = 5_368_709_120uL, unlimited = false, percent = 20.0)

    fun favoriteIds(): FavoriteIdsData = FavoriteIdsData(
        tracks = listOf("9000001", "9000002"),
        albums = listOf("501"),
        artists = listOf("42"),
        playlists = emptyList(),
        disliked = listOf("7001"),
    )

    fun event(id: Long, at: String, outcome: Int, listened: UInt = 187u, title: String = "Ouverture"): HistoryEventData =
        HistoryEventData(
            id = id,
            at = at,
            listened = listened,
            outcome = outcome,
            source = "album",
            track = row(title, id.toInt().coerceAtLeast(0)),
        )

    fun page(
        events: List<HistoryEventData> = listOf(
            event(102, "2026-11-17T21:04:00", 1),
            event(101, "2026-11-17T20:58:00", -1, listened = 12u, title = "Suite"),
        ),
        next: HistoryCursorData? = HistoryCursorData(before = "2026-11-17T20:58:00", beforeId = 101),
    ): HistoryPageData = HistoryPageData(
        events = events,
        next = next,
        summary = HistorySummaryData(
            days = 30u,
            listens = 41u,
            skips = 3u,
            minutes = 220u,
            topArtists = listOf(
                HistoryArtistData(id = "42", name = "Ascenseur", cover = null, count = 9u),
            ),
            topTracks = listOf(HistoryTrackData(track = row("Ouverture", 0), count = 4u)),
        ),
    )

    fun emptyPage(): HistoryPageData = HistoryPageData(
        events = emptyList(),
        next = null,
        summary = HistorySummaryData(
            days = 30u,
            listens = 0u,
            skips = 0u,
            minutes = 0u,
            topArtists = emptyList(),
            topTracks = emptyList(),
        ),
    )
}
