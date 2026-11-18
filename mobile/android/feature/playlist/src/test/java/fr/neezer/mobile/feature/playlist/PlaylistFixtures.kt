package fr.neezer.mobile.feature.playlist

import fr.neezer.mobile.designsystem.NzRowSource
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.PlaylistExportFormat
import uniffi.nz_ffi.PlaylistExportFormats
import uniffi.nz_ffi.PlaylistExportJob
import uniffi.nz_ffi.PlaylistImportCount
import uniffi.nz_ffi.PlaylistImportItem
import uniffi.nz_ffi.PlaylistImportLine
import uniffi.nz_ffi.PlaylistImportStatus
import uniffi.nz_ffi.PlaylistMember
import uniffi.nz_ffi.PlaylistMembersWrite
import uniffi.nz_ffi.PlaylistMeta
import uniffi.nz_ffi.PlaylistRole
import uniffi.nz_ffi.PlaylistService as PlaylistServiceRow
import uniffi.nz_ffi.PlaylistServices
import uniffi.nz_ffi.PlaylistShareLink
import uniffi.nz_ffi.PlaylistSharedData
import uniffi.nz_ffi.PlaylistSharedHeader
import uniffi.nz_ffi.PlaylistSharedRow
import uniffi.nz_ffi.TrackRow

/**
 * The screen's fixtures (lot MB9): a 4 000-row playlist, the acceptance's worst
 * realistic input, and the small answers around it.
 *
 * No FFI object is built here — `ListHandle` is native — so rows travel through
 * [FakeRows], which is exactly the design system's two-method window.
 */
object PlaylistFixtures {
    const val TRACKS = 4_000

    fun track(number: Int): TrackRow = TrackRow(
        key = "deezer:t$number",
        id = "t$number",
        title = "Titre $number",
        artists = "Artiste ${number % 40}",
        album = "Album ${number % 120}",
        albumId = "album-${number % 120}",
        durationS = (170 + number % 90).toUInt(),
        cover = null,
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = number.toUInt(),
    )

    fun tracks(count: Int = TRACKS): List<TrackRow> = (0 until count).map(::track)

    fun meta(
        id: String = "pl-1",
        title: String = "Playlist de 4 000 titres",
        version: UInt = 1u,
        membersVersion: UInt = 1u,
        owner: String = "bench",
    ): PlaylistMeta = PlaylistMeta(
        id = id,
        deezerId = null,
        title = title,
        description = "Une description",
        cover = null,
        owner = owner,
        nbTracks = TRACKS.toUInt(),
        privateTracks = 0u,
        isFavorite = false,
        version = version,
        membersVersion = membersVersion,
        lastEditedBy = "bench",
        lastEditedAt = "2026-11-18T10:00:00",
    )

    fun owner(): PlaylistMember = PlaylistMember(
        id = "u1",
        name = "bench",
        role = PlaylistRole.OWNER,
        added = null,
    )

    fun page(
        rows: PlaylistRows = FakeRows(tracks()),
        meta: PlaylistMeta = meta(),
        role: PlaylistRole = PlaylistRole.OWNER,
        editable: Boolean = true,
        canManage: Boolean = true,
        members: List<PlaylistMember> = listOf(owner()),
    ): PlaylistPage = PlaylistPage(
        meta = meta,
        role = role,
        editable = editable,
        canManage = canManage,
        admin = false,
        members = members,
        rows = rows,
        recommendations = uniffi.nz_ffi.PlaylistRecommendations(
            state = uniffi.nz_ffi.RecommendationsState.IDLE,
            tracks = emptyList(),
        ),
        collaborative = members.size > 1,
    )

    fun membersWrite(
        members: List<PlaylistMember> = listOf(owner()),
        membersVersion: UInt = 2u,
        role: PlaylistRole = PlaylistRole.OWNER,
        canEdit: Boolean = true,
        canManage: Boolean = true,
        conflict: uniffi.nz_ffi.PlaylistConflict? = null,
        denied: Boolean = false,
    ): PlaylistMembersWrite = PlaylistMembersWrite(
        members = members,
        membersVersion = membersVersion,
        role = role,
        canEdit = canEdit,
        canManage = canManage,
        admin = false,
        conflict = conflict,
        denied = denied,
    )

    fun shareLink(token: String = "tok000000000000.mac000000000000000000000"): PlaylistShareLink = PlaylistShareLink(
        token = token,
        url = "/pl/$token",
        created = "2026-11-18T10:00:00",
        expires = null,
    )

    fun formats(): PlaylistExportFormats = PlaylistExportFormats(
        default = "opus320",
        formats = listOf(
            PlaylistExportFormat("opus320", "Opus 320", "opus"),
            PlaylistExportFormat("flac", "FLAC", "flac"),
        ),
    )

    fun services(): PlaylistServices = PlaylistServices(
        services = listOf(
            PlaylistServiceRow(
                id = "spotify",
                name = "Spotify",
                ready = true,
                linked = true,
                account = "bench",
                fileOnly = false,
            ),
            PlaylistServiceRow(
                id = "apple",
                name = "Apple Music",
                ready = false,
                linked = false,
                account = null,
                fileOnly = true,
            ),
        ),
        formats = listOf(PlaylistExportFormat("csv", "CSV", "csv")),
    )

    fun job(running: Boolean = true, matched: UInt = 0u, missing: UInt = 0u): PlaylistExportJob =
        PlaylistExportJob(
            service = "spotify",
            stage = if (running) "match" else "done",
            running = running,
            total = 4u,
            done = if (running) 0u else 4u,
            matched = matched,
            missingCount = missing,
            missing = if (missing > 0u) {
                listOf(uniffi.nz_ffi.PlaylistMissingTitle("Absent", "Personne"))
            } else {
                emptyList()
            },
            url = if (running) null else "https://open.spotify.com/playlist/xyz",
            error = null,
            relink = false,
            percent = if (running) 0u else 100u,
        )

    fun importDone(): PlaylistImportStatus = PlaylistImportStatus(
        id = "job-import",
        state = "done",
        source = "spotify",
        sourceLabel = "Spotify",
        kind = "playlist",
        url = "https://open.spotify.com/playlist/abc",
        name = "Une playlist de quelqu'un",
        cover = null,
        total = 3u,
        truncated = false,
        done = 3u,
        matched = 2u,
        error = null,
        playlistId = null,
        counts = listOf(PlaylistImportCount("deezer", 2u)),
        items = listOf(
            PlaylistImportItem(
                line = PlaylistImportLine("Titre 0", listOf("Artiste 0"), null, 180u),
                track = track(0),
                via = "deezer",
                sure = true,
            ),
            PlaylistImportItem(
                line = PlaylistImportLine("Titre 1", listOf("Artiste 1"), null, 181u),
                track = track(1),
                via = "deezer",
                sure = true,
            ),
            PlaylistImportItem(
                line = PlaylistImportLine("Introuvable", listOf("Nul"), null, 0u),
                track = null,
                via = "",
                sure = false,
            ),
        ),
        running = false,
        percent = 100u,
    )

    fun shared(): PlaylistSharedData = PlaylistSharedData(
        header = PlaylistSharedHeader("Partagée", "Par bench", null, 4u, "bench"),
        rows = (0 until 4).map { number ->
            PlaylistSharedRow(
                row = track(number),
                mid = "t$number",
                streamable = number < 3,
                streamPath = "/api/shared/playlist/tok/stream/t$number",
            )
        },
        token = "tok",
        playable = 3u,
    )
}

/**
 * A windowed source over a plain list: the design system's own two methods, and
 * the three operations a list has.
 */
class FakeRows(
    initial: List<TrackRow>,
) : PlaylistRows {
    var all: List<TrackRow> = initial
        private set
    private var filter = ""
    private var version = 1L

    override val handle: ListHandle? = null

    override val source: NzRowSource = object : NzRowSource {
        override fun len(): Int = visible().size

        override fun window(start: Int, count: Int): List<TrackRow> {
            val rows = visible()
            if (start >= rows.size) return emptyList()
            return rows.subList(start, minOf(start + count, rows.size))
        }
    }

    /** How many `window()` calls the screen made: the windowing's own count. */
    var windows: Int = 0
        private set

    private fun visible(): List<TrackRow> {
        if (filter.isEmpty()) return all
        return all.filter { row ->
            row.title.contains(filter, ignoreCase = true) ||
                row.artists.contains(filter, ignoreCase = true) ||
                row.album.contains(filter, ignoreCase = true)
        }
    }

    override fun len(): Int = visible().size

    override fun version(): Long = version

    override fun filter(query: String): Int {
        filter = query.trim()
        version += 1
        return len()
    }

    override fun sort(by: PlaylistSort, descending: Boolean): Int {
        val sorted = when (by) {
            PlaylistSort.Position -> all
            PlaylistSort.Title -> all.sortedBy { it.title }
            PlaylistSort.Artist -> all.sortedBy { it.artists }
            PlaylistSort.Album -> all.sortedBy { it.album }
            PlaylistSort.Duration -> all.sortedBy { it.durationS }
            PlaylistSort.Added -> all
        }
        all = if (descending) sorted.reversed() else sorted
        version += 1
        return len()
    }

    fun replace(rows: List<TrackRow>) {
        all = rows
        version += 1
    }

    fun moveRow(from: Int, to: Int) {
        val rows = all.toMutableList()
        val row = rows.removeAt(from)
        rows.add(to, row)
        all = rows.mapIndexed { index, track -> track.copy(position = index.toUInt()) }
        version += 1
    }
}

/** A service that answers what a test told it to, and counts every call. */
class FakePlaylistService(
    var page: PlaylistPage? = PlaylistFixtures.page(),
) : PlaylistService {
    var cachedPage: PlaylistPage? = null
    var failure: Exception? = null
    var writeFailure: Exception? = null
    var writeConflict: uniffi.nz_ffi.PlaylistConflict? = null
    var recs: List<TrackRow> = listOf(
        PlaylistFixtures.track(9_000),
        PlaylistFixtures.track(9_001),
    )
    var recsFailure: Exception? = null
    var membersWrite: uniffi.nz_ffi.PlaylistMembersWrite = PlaylistFixtures.membersWrite()
    var link: PlaylistShareLink? = null
    var job: PlaylistExportJob = PlaylistFixtures.job()
    var importStatus: PlaylistImportStatus = PlaylistFixtures.importDone()
    var searchHits: List<TrackRow> = listOf(PlaylistFixtures.track(500))

    val calls = mutableListOf<String>()
    var orderSaves = 0
        private set
    var appends = 0
        private set
    var cuts = 0
        private set

    private fun note(call: String) {
        calls += call
    }

    override suspend fun cachedPlaylist(id: String): PlaylistPage? {
        note("cached")
        return cachedPage
    }

    override suspend fun playlist(id: String): PlaylistPage {
        note("playlist")
        failure?.let { throw it }
        return page ?: error("no page")
    }

    override suspend fun recommendations(id: String, limit: Int): uniffi.nz_ffi.PlaylistRecommendations {
        note("recommendations")
        recsFailure?.let { throw it }
        return uniffi.nz_ffi.PlaylistRecommendations(
            state = uniffi.nz_ffi.RecommendationsState.READY,
            tracks = recs,
        )
    }

    override suspend fun move(id: String, from: Int, to: Int): PlaylistReordered {
        note("move")
        val rows = page?.rows as? FakeRows
        rows?.moveRow(from, to)
        return PlaylistReordered(rows ?: FakeRows(emptyList()), version = 1L, conflict = null)
    }

    override suspend fun saveOrder(id: String): PlaylistReordered {
        note("saveOrder")
        orderSaves += 1
        writeFailure?.let { throw it }
        val rows = page?.rows ?: FakeRows(emptyList())
        return PlaylistReordered(rows, version = 2L, conflict = writeConflict)
    }

    override suspend fun rows(id: String): PlaylistRows? {
        note("rows")
        return page?.rows
    }

    override suspend fun append(id: String, tracks: List<TrackRow>): PlaylistReordered {
        note("append")
        appends += 1
        val rows = page?.rows as? FakeRows ?: FakeRows(emptyList())
        rows.replace(rows.all + tracks)
        return PlaylistReordered(rows, version = 1L, conflict = null)
    }

    override suspend fun saveAppend(id: String): PlaylistWritten {
        note("saveAppend")
        writeFailure?.let { throw it }
        return PlaylistWritten(
            page = page?.copy(meta = PlaylistFixtures.meta(version = 3u)) ?: error("no page"),
            conflict = writeConflict,
        )
    }

    override suspend fun cut(id: String, indexes: List<Int>): PlaylistReordered {
        note("cut")
        cuts += 1
        val rows = page?.rows as? FakeRows ?: FakeRows(emptyList())
        val kept = rows.all.filterIndexed { index, _ -> index !in indexes }
        rows.replace(kept)
        return PlaylistReordered(rows, version = 1L, conflict = null)
    }

    override suspend fun saveCut(id: String, indexes: List<Int>): PlaylistWritten {
        note("saveCut")
        writeFailure?.let { throw it }
        return PlaylistWritten(page ?: error("no page"), conflict = writeConflict)
    }

    override suspend fun edit(id: String, title: String?, description: String?): PlaylistWritten {
        note("edit")
        writeFailure?.let { throw it }
        return PlaylistWritten(page ?: error("no page"), conflict = writeConflict)
    }

    override suspend fun delete(id: String) {
        note("delete")
        writeFailure?.let { throw it }
    }

    override suspend fun leave(id: String) {
        note("leave")
        writeFailure?.let { throw it }
    }

    override suspend fun members(id: String): PlaylistMembersWrite {
        note("members")
        return membersWrite
    }

    override suspend fun invite(id: String, username: String, role: PlaylistRole): PlaylistMembersWrite {
        note("invite:$username")
        return membersWrite
    }

    override suspend fun setRole(id: String, userId: String, role: PlaylistRole): PlaylistMembersWrite {
        note("setRole:$userId:${role.name}")
        return membersWrite
    }

    override suspend fun removeMember(id: String, userId: String): PlaylistMembersWrite {
        note("removeMember:$userId")
        return membersWrite
    }

    override suspend fun shareLink(id: String): PlaylistShareLink? {
        note("shareLink")
        return link
    }

    override suspend fun createShare(id: String): PlaylistShareLink {
        note("createShare")
        return link ?: PlaylistFixtures.shareLink().also { link = it }
    }

    override suspend fun revokeShare(id: String) {
        note("revokeShare")
        link = null
    }

    override suspend fun exportFormats(): PlaylistExportFormats {
        note("formats")
        return PlaylistFixtures.formats()
    }

    override suspend fun services(): PlaylistServices {
        note("services")
        return PlaylistFixtures.services()
    }

    override suspend fun export(
        service: String,
        kind: String,
        id: String,
        name: String,
        isPublic: Boolean,
    ): String {
        note("export")
        return "job-1"
    }

    override suspend fun exportJob(job: String): PlaylistExportJob {
        note("exportJob")
        return this.job
    }

    override suspend fun exportListFile(kind: String, id: String, fmt: String): String {
        note("listFile")
        return "Artiste - Titre"
    }

    override suspend fun exportArchive(kind: String, id: String, fmt: String, path: String): ULong {
        note("archive")
        return 0uL
    }

    override suspend fun searchTracks(term: String): List<TrackRow> {
        note("search:$term")
        return searchHits
    }

    override suspend fun importStart(url: String): String {
        note("importStart")
        return "job-import"
    }

    override suspend fun importStatus(job: String): PlaylistImportStatus {
        note("importStatus")
        return importStatus
    }

    override suspend fun importSave(job: String, title: String, keep: List<Int>): PlaylistImportStatus {
        note("importSave")
        return importStatus.copy(state = "saved", playlistId = "pl-importee")
    }

    override suspend fun sharedPlaylist(token: String): PlaylistSharedData {
        note("shared")
        return PlaylistFixtures.shared()
    }

    override fun cachedSharedPlaylist(token: String): PlaylistSharedData? {
        note("cachedShared")
        return null
    }
}

/** A player that records what it was asked to play. */
class FakePlayer : PlaylistPlayer {
    val played = mutableListOf<Pair<Int, String>>()

    override fun play(tracks: List<TrackRow>, contextKind: String, contextId: String, start: Int) {
        played += tracks.size to contextId
    }

    override fun shuffle(tracks: List<TrackRow>, contextKind: String, contextId: String) {
        played += tracks.size to "shuffle:$contextId"
    }
}

/** A sharer that records what left the device. */
class FakeSharer : PlaylistSharer {
    val shared = mutableListOf<String>()
    val copied = mutableListOf<String>()

    override fun share(text: String, title: String) {
        shared += text
    }

    override fun copy(text: String, label: String) {
        copied += text
    }
}
