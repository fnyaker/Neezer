package fr.neezer.mobile.feature.discovery

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.ExplorerDiscoverData
import uniffi.nz_ffi.ExplorerSettingsData
import uniffi.nz_ffi.ExplorerStylesData
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.MoodRowData
import uniffi.nz_ffi.PlayContext
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.RecapData
import uniffi.nz_ffi.RecapYearsData
import uniffi.nz_ffi.ReleaseSettingsData
import uniffi.nz_ffi.ReleasesData
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.StationRowData
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow

/**
 * A windowed list, as the four screens of this lot read one (lot MB14).
 *
 * It is an interface of its own, not `ListHandle`, for the reason MB6 wrote
 * down: the JVM tests and the screenshot renders hold a fake over a plain list,
 * and the paging rule is then an assertion instead of a hope. [handle] is what
 * the real screen windows with `rememberListWindow`; a fake has none.
 */
interface DiscoveryRows {
    /** The real handle, or `null` for a fake. */
    val handle: ListHandle?

    fun len(): Int

    fun window(start: Int, count: Int): List<TrackRow>

    /** `ListHandle.filter`: `appcore`'s fold, answered as the new length. */
    fun filter(query: String): Int = len()

    fun sort(by: uniffi.nz_ffi.SortKey, descending: Boolean): Int = len()
}

/** The real handle, unchanged: the rows never cross the boundary whole. */
class HandleRows(override val handle: ListHandle) : DiscoveryRows {
    override fun len(): Int = handle.len().toInt()

    override fun window(start: Int, count: Int): List<TrackRow> =
        handle.window(start.coerceAtLeast(0).toUInt(), count.coerceAtLeast(0).toUInt())

    override fun filter(query: String): Int = handle.filter(query).toInt()

    override fun sort(by: uniffi.nz_ffi.SortKey, descending: Boolean): Int =
        handle.sort(by, descending).toInt()
}

/** A shelf's page, with its rows windowed like any long list. */
data class ShelfPage(
    val key: String,
    val title: String,
    val subtitle: String,
    val description: String,
    val facts: String,
    val totalDurationS: ULong,
    val mosaic: List<String>,
    val releases: List<uniffi.nz_ffi.ReleaseRowData>,
    val trackCount: Int,
    val rows: DiscoveryRows,
)

/** One mix, when its card is tapped. */
data class MoodMix(
    val id: String,
    val name: String,
    val subtitle: String,
    val ready: Boolean,
    val reason: String,
    val rows: DiscoveryRows,
)

/** One playlist of a finished discovery, with its checked titles windowed. */
data class PlaylistTracks(
    val id: String,
    val title: String,
    val cover: String?,
    val countLabel: String,
    val trackCount: Int,
    val rows: DiscoveryRows,
)

/** A started station: its context and the rows to play now. */
data class Station(
    val kind: String,
    val id: String,
    val title: String,
    val trackCount: Int,
    val rows: DiscoveryRows,
)

/** Where one style's discovery stands, and what it found. */
data class DiscoverResult(
    val phase: String,
    val style: String,
    val level: Int,
    val playlists: List<PlaylistTracks>,
    val busyStatus: Int,
    val retryAfterS: Int,
    val busyCause: String,
)

/** One year in review, with its top titles windowed. */
data class RecapDeck(
    val year: Int,
    val partial: Boolean,
    val periodLabel: String,
    val minutes: UInt,
    val listens: UInt,
    /** "8 540" / "2 180": the glance panel prints these, never the raw count. */
    val minutesLabel: String,
    val listensLabel: String,
    val slides: List<uniffi.nz_ffi.RecapSlideData>,
    val posterLines: List<String>,
    val shareText: String,
    val mosaic: List<String>,
    val topArtist: String,
    val topGenre: String,
    val biggestDay: String,
    val emptyReason: String,
    val emptyTitle: String,
    val emptyText: String,
    val nearbyYear: Int?,
    val empty: Boolean,
)

/** A list a fake or a render hands the screens (no core, no `.so`). */
class PlainRows(rows: List<TrackRow>) : DiscoveryRows {
    /**
     * A plain list that filters and sorts the way `nz_core::lists` does, but in
     * Kotlin: a test and a render exercise the SCREEN's toolbar without a
     * `.so`. `filter` folds with [foldGenre] and matches title, artists and
     * album, which is what the core's folded search matches too.
     */
    private var current: List<TrackRow> = rows
    private var query: String = ""
    private var by: uniffi.nz_ffi.SortKey? = null
    private var descending: Boolean = false

    override val handle: ListHandle? get() = null

    override fun len(): Int = current.size

    override fun window(start: Int, count: Int): List<TrackRow> =
        current.drop(start.coerceAtLeast(0)).take(count.coerceAtLeast(0))

    override fun filter(query: String): Int {
        this.query = query
        apply()
        return current.size
    }

    override fun sort(by: uniffi.nz_ffi.SortKey, descending: Boolean): Int {
        this.by = by
        this.descending = descending
        apply()
        return current.size
    }

    private fun apply() {
        val needle = foldGenre(query)
        var rows = if (needle.isEmpty()) {
            all
        } else {
            all.filter { row ->
                foldGenre(row.title).contains(needle) ||
                    foldGenre(row.artists).contains(needle) ||
                    foldGenre(row.album).contains(needle)
            }
        }
        val key: uniffi.nz_ffi.SortKey? = by
        if (key != null) {
            val sorted = when (key) {
                uniffi.nz_ffi.SortKey.TITLE -> rows.sortedBy { it.title.lowercase() }
                uniffi.nz_ffi.SortKey.ARTIST -> rows.sortedBy { it.artists.lowercase() }
                uniffi.nz_ffi.SortKey.ALBUM -> rows.sortedBy { it.album.lowercase() }
                uniffi.nz_ffi.SortKey.DURATION -> rows.sortedBy { it.durationS }
                else -> rows
            }
            rows = if (descending) sorted.reversed() else sorted
        }
        current = rows
    }

    private val all: List<TrackRow> = rows
}

/**
 * What Découverte asks of the core: this lot's `nz-ffi` methods, one to one
 * (`nz-ffi/src/discovery.rs`).
 *
 * An interface, not the generated `Screens`, so every rule of this lot is plain
 * Kotlin that the JVM tests run with a fake — no JNA, no `.so`, no device.
 */
interface DiscoveryService {
    // -- Nouveautés ----------------------------------------------------------

    /** The feed the device kept, or `null` the first time. */
    fun cachedReleases(): ReleasesData?

    /** `GET /releases` (+ the unseen badge's `POST /releases/seen`). */
    suspend fun releases(): ReleasesData

    /** `POST /releases/seen` on the person's own action. */
    suspend fun releasesSeen()

    /** One shelf's page, from the kept feed: NO request. */
    fun releaseShelf(kind: String): ShelfPage?

    /** Every new title, windowed: what "Tout lire" queues. */
    fun releasesTracks(): DiscoveryRows

    /** One release's titles, windowed: the group a person expands. */
    fun releaseGroupTracks(releaseId: String): DiscoveryRows

    // -- Récap ---------------------------------------------------------------

    fun cachedRecapYears(): RecapYearsData?

    suspend fun recapYears(): RecapYearsData

    fun cachedRecap(year: Int): RecapDeck?

    suspend fun recap(year: Int): RecapDeck

    // -- moods ---------------------------------------------------------------

    fun cachedMoods(): List<MoodRowData>?

    suspend fun moods(): List<MoodRowData>

    /** `GET /api/moods/<id>` — one request per tap. */
    suspend fun moodTracks(id: String): MoodMix

    /** The year's top titles, windowed: what « Lire ces 5 titres » queues. */
    fun recapTop(year: Int): DiscoveryRows

    // -- Explorer ------------------------------------------------------------

    fun cachedExplorerStyles(): ExplorerStylesData?

    suspend fun explorerStyles(): ExplorerStylesData

    suspend fun explorerDiscover(style: String, level: Int): DiscoverResult

    suspend fun explorerPoll(style: String, level: Int): DiscoverResult

    fun explorerSettings(): ExplorerSettingsData

    fun cachedStationStyles(): List<StationRowData>?

    suspend fun stationStyles(): List<StationRowData>

    suspend fun genreStation(style: String): Station

    // -- what the person can do here -----------------------------------------

    suspend fun muteArtist(provider: String, id: String)

    suspend fun recommendations(level: Double): DiscoveryRows
}

/** The one real implementation: this lot's `nz-ffi` methods. */
class NzDiscoveryService(private val screens: Screens) : DiscoveryService {
    override fun cachedReleases(): ReleasesData? = screens.cachedReleases()

    override suspend fun releases(): ReleasesData = screens.releases()

    override suspend fun releasesSeen() = screens.releasesSeen()

    override fun releaseShelf(kind: String): ShelfPage? =
        screens.releaseShelf(kind)?.let { page ->
            ShelfPage(
                key = page.key,
                title = page.title,
                subtitle = page.subtitle,
                description = page.description,
                facts = page.facts,
                totalDurationS = page.totalDurationS,
                mosaic = page.mosaic,
                releases = page.releases,
                trackCount = page.trackCount.toInt(),
                rows = HandleRows(page.tracks),
            )
        }

    override fun releasesTracks(): DiscoveryRows = HandleRows(screens.releasesTracks())

    override fun releaseGroupTracks(releaseId: String): DiscoveryRows =
        HandleRows(screens.releaseGroupTracks(releaseId))

    override fun cachedRecapYears(): RecapYearsData? = screens.cachedRecapYears()

    override suspend fun recapYears(): RecapYearsData = screens.recapYears()

    override fun cachedRecap(year: Int): RecapDeck? = screens.cachedRecap(year)?.let(::deck)

    override suspend fun recap(year: Int): RecapDeck = deck(screens.recap(year))

    override fun cachedMoods(): List<MoodRowData>? = screens.cachedMoods()

    override suspend fun moods(): List<MoodRowData> = screens.discoveryMoods()

    override suspend fun moodTracks(id: String): MoodMix =
        screens.discoveryMoodTracks(id).let { mix ->
            MoodMix(
                id = mix.id,
                name = mix.name,
                subtitle = mix.subtitle,
                ready = mix.ready,
                reason = mix.reason,
                rows = HandleRows(mix.tracks),
            )
        }

    override fun recapTop(year: Int): DiscoveryRows = HandleRows(screens.recapTopTracks(year))

    override fun cachedExplorerStyles(): ExplorerStylesData? = screens.cachedExplorerStyles()

    override suspend fun explorerStyles(): ExplorerStylesData = screens.explorerStyles()

    override suspend fun explorerDiscover(style: String, level: Int): DiscoverResult =
        screens.explorerDiscover(style, level.toUInt()).toResult()

    override suspend fun explorerPoll(style: String, level: Int): DiscoverResult =
        screens.explorerPoll(style, level.toUInt()).toResult()

    override fun explorerSettings(): ExplorerSettingsData = screens.explorerSettings()

    override fun cachedStationStyles(): List<StationRowData>? = screens.cachedStationStyles()

    override suspend fun stationStyles(): List<StationRowData> = screens.stationStyles()

    override suspend fun genreStation(style: String): Station =
        screens.genreStation(style).let { station ->
            Station(
                kind = station.kind,
                id = station.id,
                title = station.title,
                trackCount = station.trackCount.toInt(),
                rows = HandleRows(station.tracks),
            )
        }

    override suspend fun muteArtist(provider: String, id: String) = screens.muteArtist(provider, id)

    override suspend fun recommendations(level: Double): DiscoveryRows =
        HandleRows(screens.recommendations(level))
}

/**
 * What these screens ask of the player (MB3b's `PlayerApi`).
 *
 * The same seam MB5 uses, declared here rather than shared: a feature module
 * depends on `:bridge` and `:designsystem`, never on another feature module, so
 * a JVM test asserts the COMMAND a tap sends with a fake.
 */
interface PlayPort {
    /** Replaces the queue with the rows of [rows], starting at [start]. */
    fun play(rows: DiscoveryRows, start: Int, kind: String, id: String)

    /** « Lire ces 5 titres »: the same, from a plain list (a recap's top). */
    fun playList(rows: List<TrackRow>, start: Int, kind: String, id: String)

    /** One row, then its radio — `playThenRadio`, what a reco row does. */
    fun playRow(row: TrackRow, id: String)

    fun playNext(row: TrackRow)
}

/** A port that does nothing, for a process whose core could not be built. */
object NoPlay : PlayPort {
    override fun play(rows: DiscoveryRows, start: Int, kind: String, id: String) = Unit

    override fun playList(rows: List<TrackRow>, start: Int, kind: String, id: String) = Unit

    override fun playRow(row: TrackRow, id: String) = Unit

    override fun playNext(row: TrackRow) = Unit
}

/** The real port: `PlayerCommand`, on the player's own worker. */
class NzPlayPort(private val player: PlayerApi) : PlayPort {
    override fun play(rows: DiscoveryRows, start: Int, kind: String, id: String) {
        playList(rows.all(), start, kind, id)
    }

    override fun playList(rows: List<TrackRow>, start: Int, kind: String, id: String) {
        if (rows.isEmpty()) return
        player.command(
            PlayerCommand.PlayQueue(
                tracks = rows.map(::ref),
                start = start.coerceIn(0, rows.lastIndex).toUInt(),
                // MB3b's context is a kind and an id: the SPA's "Récap 2026"
                // title has no field to travel in, and inventing one would be a
                // contract change another lot owns.
                context = PlayContext(kind = kind, id = id),
            ),
            0uL,
        )
    }

    override fun playRow(row: TrackRow, id: String) {
        playList(listOf(row), 0, "radio", id)
    }

    override fun playNext(row: TrackRow) {
        player.command(PlayerCommand.PlayNext(tracks = listOf(ref(row))), 0uL)
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

/**
 * The app's player, asked of the core (one instance, a `OnceLock` the core
 * owns — MB3c's measurement).
 */
object DiscoveryPlayer {
    fun api(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()

    fun port(): PlayPort = api()?.let(::NzPlayPort) ?: NoPlay
}

/** The year's record, with its top titles windowed by this side. */
private fun deck(data: RecapData): RecapDeck = RecapDeck(
    year = data.year,
    partial = data.partial,
    periodLabel = data.periodLabel,
    minutes = data.minutes,
    listens = data.listens,
    minutesLabel = data.minutesLabel,
    listensLabel = data.listensLabel,
    slides = data.slides,
    posterLines = data.posterLines,
    shareText = data.shareText,
    mosaic = data.mosaic,
    topArtist = data.topArtist,
    topGenre = data.topGenre,
    biggestDay = data.biggestDay,
    emptyReason = data.emptyReason,
    emptyTitle = data.emptyTitle,
    emptyText = data.emptyText,
    nearbyYear = data.nearbyYear,
    empty = data.empty,
)

/** The FFI record's handles, replaced by this side's windowed rows. */
private fun ExplorerDiscoverData.toResult(): DiscoverResult = DiscoverResult(
    phase = phase,
    style = style,
    level = level.toInt(),
    playlists = playlists.map { playlist ->
        PlaylistTracks(
            id = playlist.id,
            title = playlist.title,
            cover = playlist.cover,
            countLabel = playlist.countLabel,
            trackCount = playlist.trackCount.toInt(),
            rows = HandleRows(playlist.tracks),
        )
    },
    busyStatus = busyStatus.toInt(),
    retryAfterS = retryAfterS.toInt(),
    busyCause = busyCause,
)

/**
 * Every row of a windowed list, page by page: what a queue needs before its
 * first note. Bounded by the server's own feed (400 titles), and a page is 128
 * rows — four calls at the very most, never one 4 000-record copy.
 */
fun DiscoveryRows.all(page: Int = 128, ceiling: Int = 4_000): List<TrackRow> {
    val total = len().coerceAtMost(ceiling)
    val out = ArrayList<TrackRow>(total)
    var start = 0
    while (start < total) {
        out += window(start, page)
        start += page
    }
    return out
}
