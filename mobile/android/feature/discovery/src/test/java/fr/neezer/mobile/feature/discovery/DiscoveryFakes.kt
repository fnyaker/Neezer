package fr.neezer.mobile.feature.discovery

import fr.neezer.mobile.bridge.ScreenState
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import uniffi.nz_ffi.ExplorerLevelData
import uniffi.nz_ffi.ExplorerSettingsData
import uniffi.nz_ffi.ExplorerStylesData
import uniffi.nz_ffi.MoodRowData
import uniffi.nz_ffi.RecapBarData
import uniffi.nz_ffi.RecapChipData
import uniffi.nz_ffi.RecapRowData
import uniffi.nz_ffi.RecapSlideData
import uniffi.nz_ffi.RecapYearsData
import uniffi.nz_ffi.ReleaseGroupData
import uniffi.nz_ffi.ReleaseRowData
import uniffi.nz_ffi.ReleaseShelfData
import uniffi.nz_ffi.ReleasesData
import uniffi.nz_ffi.StationRowData
import uniffi.nz_ffi.StyleTileData
import uniffi.nz_ffi.TrackRow

/**
 * The fixture the model tests and the screenshot renders share: answers a real
 * server would send, in the real records — so a screenshot proves the SCREEN,
 * not a mock shape that drifted from the boundary.
 *
 * No handle is built here: `ShelfPage`, `MoodMix`, `PlaylistTracks` and
 * `Station` carry a [DiscoveryRows], and a fixture hands a [PlainRows]. That is
 * the whole reason the ports wrap the FFI records.
 */
object DiscoveryFixtures {
    fun row(title: String, position: Int = 0, album: String = "Le matin"): TrackRow = TrackRow(
        key = "row-$position",
        id = "id-$position",
        title = title,
        artists = "Ascenseur",
        album = album,
        albumId = "501",
        durationS = (180 + position).toUInt(),
        cover = "/api/cover/501",
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = position.toUInt(),
    )

    fun rows(): List<TrackRow> = listOf(
        row("Ouverture", 0),
        row("Suite", 1),
        row("Voisin", 2, album = "Voisin"),
    )

    private fun release(
        key: String,
        title: String,
        artist: String,
        kind: String,
        label: String,
        date: String,
        closeTo: String? = null,
        new: Boolean = false,
    ) = ReleaseRowData(
        key = key,
        title = title,
        cover = "/api/cover/$key",
        artist = artist,
        artistId = "42",
        kind = kind,
        kindLabel = label,
        dateLabel = date,
        nbTracks = 2u,
        closeTo = closeTo,
        subtitle = if (closeTo != null) "$artist · proche de $closeTo · $label · $date" else "$artist · $label · $date",
        new = new,
        trackCount = 2u,
    )

    fun releases(): ReleasesData {
        val morning = release("501", "Le matin", "Ascenseur", "album", "Album", "12 nov.", new = true)
        val neighbour = release("502", "Voisin", "June", "single", "Single", "10 nov.", closeTo = "Ascenseur", new = true)
        return ReleasesData(
            count = 3u,
            unseen = 2u,
            pending = 1u,
            freshLabel = "2 nouvelles sorties",
            pendingLabel = "1 sortie n'a pas encore son titre : le serveur le lit en arrière-plan.",
            shelves = listOf(
                ReleaseShelfData(
                    key = "artists",
                    title = "Tes artistes",
                    subtitle = "Les sorties de tes favoris et des artistes que tu écoutes.",
                    trackCount = 2u,
                    releaseCount = 1u,
                    durationS = 390u,
                    mosaic = listOf("/api/cover/501"),
                    closeNames = emptyList(),
                    preview = rows().take(2),
                ),
                ReleaseShelfData(
                    key = "discovery",
                    title = "Découverte",
                    subtitle = "Des artistes proches de tes favoris, et d'autres à découvrir.",
                    trackCount = 1u,
                    releaseCount = 1u,
                    durationS = 200u,
                    mosaic = listOf("/api/cover/502"),
                    closeNames = listOf("Ascenseur"),
                    preview = rows().drop(2),
                ),
            ),
            groups = listOf(
                ReleaseGroupData(
                    release = morning,
                    artist = "Ascenseur",
                    label = "Album · 2 titres · Nouveau",
                    trackCount = 2u,
                ),
                ReleaseGroupData(
                    release = neighbour,
                    artist = "June",
                    label = "Single · 1 titre · Nouveau",
                    trackCount = 1u,
                ),
            ),
            others = listOf(release("503", "Pas encore lue", "Nina", "ep", "EP", "1 nov.")),
            releaseTotal = 3u,
            trackCount = 3u,
            empty = false,
        )
    }

    /** The same feed with no shelf at all: the list opens by itself. */
    fun releasesWithoutShelves(): ReleasesData = releases().copy(shelves = emptyList(), unseen = 0u)

    fun emptyReleases(): ReleasesData = ReleasesData(
        count = 0u,
        unseen = 0u,
        pending = 0u,
        freshLabel = "",
        pendingLabel = "",
        shelves = emptyList(),
        groups = emptyList(),
        others = emptyList(),
        releaseTotal = 0u,
        trackCount = 0u,
        empty = true,
    )

    fun shelfPage(kind: String = "artists"): ShelfPage = ShelfPage(
        key = kind,
        title = if (kind == "discovery") "Découverte" else "Tes artistes",
        subtitle = "Les sorties de tes favoris et des artistes que tu écoutes.",
        description = if (kind == "discovery") {
            "Des artistes proches de Ascenseur."
        } else {
            "Les sorties récentes des artistes que tu suis et de ceux que tu écoutes le plus."
        },
        facts = "3 titres · 2 sorties",
        totalDurationS = 590u,
        mosaic = listOf("/api/cover/501", "/api/cover/502"),
        releases = emptyList(),
        trackCount = 3,
        rows = PlainRows(rows()),
    )

    fun recapYears(): RecapYearsData = RecapYearsData(
        chips = listOf(
            RecapChipData(year = 2026, label = "2026 · en cours"),
            RecapChipData(year = 2025, label = "2025"),
        ),
        latest = 2026,
        nowYear = 2026,
        minYear = 2024,
        hasAny = true,
    )

    fun recapDeck(empty: Boolean = false, partial: Boolean = true): RecapDeck {
        val slides = if (empty) {
            emptyList()
        } else {
            listOf(
                RecapSlideData(
                    id = "year",
                    kicker = "2026 · en cours",
                    title = "Ton année 2026",
                    glyph = "sparkles",
                    lede = "2 180 écoutes · 8 540 minutes",
                    text = "",
                    footnote = "du 1ᵉʳ janvier au 13 novembre 2026",
                    rows = emptyList(),
                    bars = emptyList(),
                    mosaic = listOf("/api/cover/501", "/api/cover/502"),
                    poster = emptyList(),
                ),
                RecapSlideData(
                    id = "minutes",
                    kicker = "2 sur 9",
                    title = "8 540 minutes",
                    glyph = "timer",
                    lede = "soit 5 j 22 h de musique",
                    text = "écoutes par mois",
                    footnote = "Ton mois le plus fort : juillet",
                    rows = emptyList(),
                    bars = (1..12).map { month ->
                        RecapBarData(
                            month = month.toUByte(),
                            label = listOf("janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.")[month - 1],
                            listens = if (month == 7) 300u else 120u,
                            pct = if (month == 7) 100u else 40u,
                        )
                    },
                    mosaic = emptyList(),
                    poster = emptyList(),
                ),
                RecapSlideData(
                    id = "artists",
                    kicker = "3 sur 9",
                    title = "Tes 5 artistes",
                    glyph = "user",
                    lede = "",
                    text = "",
                    footnote = "",
                    rows = listOf(
                        RecapRowData("42", 1u, "Ascenseur", "210 écoutes", "", "/api/cover/501", 0.0),
                        RecapRowData("43", 2u, "June", "190 écoutes", "", null, 0.0),
                    ),
                    bars = emptyList(),
                    mosaic = emptyList(),
                    poster = emptyList(),
                ),
                RecapSlideData(
                    id = "share",
                    kicker = "9 sur 9",
                    title = "Partage ton année",
                    glyph = "share",
                    lede = "",
                    text = "",
                    footnote = "",
                    rows = emptyList(),
                    bars = emptyList(),
                    mosaic = emptyList(),
                    poster = listOf(
                        "Ton année 2026, en cours",
                        "8 540 minutes",
                        "2 180 écoutes",
                        "Artiste : Ascenseur",
                        "Titre : Ouverture",
                        "Ton mois le plus fort : juillet",
                    ),
                ),
            )
        }
        return RecapDeck(
            year = 2026,
            partial = partial,
            periodLabel = "du 1ᵉʳ janvier au 13 novembre 2026",
            minutes = 8_540u,
            listens = 2_180u,
            minutesLabel = "8 540",
            listensLabel = "2 180",
            slides = slides,
            posterLines = listOf(
                "Ton année 2026, en cours",
                "8 540 minutes",
                "2 180 écoutes",
                "Artiste : Ascenseur",
                "Titre : Ouverture",
                "Ton mois le plus fort : juillet",
            ),
            shareText = "Mon année 2026 (en cours) en musique : 2 180 écoutes, 8 540 minutes, avec Ascenseur en tête.",
            mosaic = listOf("/api/cover/501", "/api/cover/502"),
            topArtist = "Ascenseur",
            topGenre = "Techno",
            biggestDay = "61 écoutes",
            emptyReason = if (empty) "thin" else "",
            emptyTitle = if (empty) "Trop peu d'écoutes en 2026" else "",
            emptyText = if (empty) "5 écoutes, 12 minutes — pas assez pour raconter une année." else "",
            nearbyYear = if (empty) 2025 else null,
            empty = empty,
        )
    }

    fun explorerStyles(): ExplorerStylesData = ExplorerStylesData(
        tiles = listOf(
            StyleTileData("Techno", "12 titres", 12u, 100u),
            StyleTileData("House", "4 titres", 4u, 33u),
            StyleTileData("Électronique", "16 titres", 16u, 100u),
            StyleTileData("Detroit", "2 titres", 2u, 16u),
        ),
        source = "history",
        tracks = 34u,
        empty = false,
    )

    fun stationRows(): List<StationRowData> = listOf(
        StationRowData("Techno", null, 12u, "12 titres"),
        StationRowData("House", null, 4u, "4 titres"),
        StationRowData("Detroit", "Techno", 2u, "2 titres"),
    )

    fun explorerSettings(): ExplorerSettingsData = ExplorerSettingsData(
        levels = listOf(
            ExplorerLevelData(1u, "Prudent", "Des playlists qui restent dans le style, sans surprise."),
            ExplorerLevelData(2u, "Mesuré", "Le style, avec quelques voisins immédiats."),
            ExplorerLevelData(3u, "Équilibré", "Le style et ce qui l'entoure."),
            ExplorerLevelData(4u, "Curieux", "Le style, ses voisins et des passerelles."),
            ExplorerLevelData(5u, "Large", "Tout ce que Deezer rapproche de ce style."),
        ),
        maxPolls = 60u,
        pollMs = 2_000u,
    )

    fun discover(phase: String, playlists: Int = 2): DiscoverResult = DiscoverResult(
        phase = phase,
        style = "Techno",
        level = 3,
        playlists = (0 until playlists).map { index ->
            PlaylistTracks(
                id = "p$index",
                title = if (index == 0) "Techno profond" else "Detroit",
                cover = "/api/cover/60$index",
                countLabel = if (index == 0) "12 titres vérifiés sur 60" else "Titres vérifiés",
                trackCount = if (index == 0) 1 else 0,
                rows = PlainRows(if (index == 0) listOf(row("Vérifié", 0)) else emptyList()),
            )
        },
        busyStatus = 0,
        retryAfterS = 0,
        busyCause = "",
    )

    fun busy(status: Int, retryAfter: Int): DiscoverResult = DiscoverResult(
        phase = "busy",
        style = "Techno",
        level = 3,
        playlists = emptyList(),
        busyStatus = status,
        retryAfterS = retryAfter,
        busyCause = if (status == 429) "Trop de découvertes demandées pour l'instant." else "Le serveur est occupé.",
    )

    fun station(): Station = Station(
        kind = "genre-radio",
        id = "Techno",
        title = "Station Techno",
        trackCount = 1,
        rows = PlainRows(listOf(row("Station 1", 0))),
    )

    fun moods(): List<MoodRowData> = listOf(
        MoodRowData("calme", "Calme", "Doux et posé", 40u, true, "/api/cover/501"),
        MoodRowData("fete", "Fête", "Ça bouge", 2u, false, null),
    )
}

/**
 * The core a model test talks to: every call counted, and every answer a test
 * can fail on purpose. No `.so`, no JNA, no device.
 */
class FakeDiscoveryService(
    var cachedFeed: ReleasesData? = DiscoveryFixtures.releases(),
    var feed: () -> ReleasesData = { DiscoveryFixtures.releases() },
    var page: ShelfPage? = DiscoveryFixtures.shelfPage(),
    var cachedPage: ShelfPage? = DiscoveryFixtures.shelfPage(),
    var cachedYears: RecapYearsData? = null,
    var years: () -> RecapYearsData = { DiscoveryFixtures.recapYears() },
    var cachedDeck: Map<Int, RecapDeck> = emptyMap(),
    var deck: (Int) -> RecapDeck = { year -> DiscoveryFixtures.recapDeck().copy(year = year) },
    var styles: ExplorerStylesData? = DiscoveryFixtures.explorerStyles(),
    var cachedStyles: ExplorerStylesData? = DiscoveryFixtures.explorerStyles(),
    var discoverAnswers: MutableList<DiscoverResult> = mutableListOf(DiscoveryFixtures.discover("pending")),
    var pollAnswers: MutableList<DiscoverResult> = mutableListOf(DiscoveryFixtures.discover("ready")),
    var stationRows: List<StationRowData>? = DiscoveryFixtures.stationRows(),
    var cachedStationRows: List<StationRowData>? = DiscoveryFixtures.stationRows(),
    var station: Station = DiscoveryFixtures.station(),
    var failure: (() -> Exception)? = null,
) : DiscoveryService {
    var feedReads = 0
        private set
    var cachedFeedReads = 0
        private set
    var seenPosts = 0
        private set
    var shelfReads = 0
        private set
    var groupReads = 0
        private set
    var yearsReads = 0
        private set
    var deckReads = 0
        private set
    var stylesReads = 0
        private set
    var discoverPosts = 0
        private set
    var pollReads = 0
        private set
    var stationStyleReads = 0
        private set
    var stationPosts = 0
        private set
    var topReads = 0
        private set
    val muted = mutableListOf<Pair<String, String>>()

    override fun cachedReleases(): ReleasesData? {
        cachedFeedReads++
        return cachedFeed
    }

    override suspend fun releases(): ReleasesData {
        feedReads++
        failure?.let { throw it() }
        return feed()
    }

    override suspend fun releasesSeen() {
        seenPosts++
    }

    override fun releaseShelf(kind: String): ShelfPage? {
        shelfReads++
        return page
    }

    override fun releasesTracks(): DiscoveryRows {
        groupReads++
        return PlainRows(DiscoveryFixtures.rows())
    }

    override fun releaseGroupTracks(releaseId: String): DiscoveryRows {
        groupReads++
        return PlainRows(DiscoveryFixtures.rows().take(2))
    }

    override fun cachedRecapYears(): RecapYearsData? = cachedYears

    override suspend fun recapYears(): RecapYearsData {
        yearsReads++
        failure?.let { throw it() }
        return years()
    }

    override fun cachedRecap(year: Int): RecapDeck? = cachedDeck[year]

    override suspend fun recap(year: Int): RecapDeck {
        deckReads++
        failure?.let { throw it() }
        return deck(year)
    }

    override fun cachedMoods(): List<MoodRowData>? = DiscoveryFixtures.moods()

    override suspend fun moods(): List<MoodRowData> = DiscoveryFixtures.moods()

    override suspend fun moodTracks(id: String): MoodMix = MoodMix(
        id = id,
        name = "Calme",
        subtitle = "Doux",
        ready = true,
        reason = "",
        rows = PlainRows(listOf(DiscoveryFixtures.row("Mix 1", 0))),
    )

    override fun recapTop(year: Int): DiscoveryRows {
        topReads++
        return PlainRows(DiscoveryFixtures.rows().take(2))
    }

    override fun cachedExplorerStyles(): ExplorerStylesData? = cachedStyles

    override suspend fun explorerStyles(): ExplorerStylesData {
        stylesReads++
        failure?.let { throw it() }
        return styles ?: throw IllegalStateException("no styles")
    }

    override suspend fun explorerDiscover(style: String, level: Int): DiscoverResult {
        discoverPosts++
        failure?.let { throw it() }
        return discoverAnswers.removeFirst()
    }

    override suspend fun explorerPoll(style: String, level: Int): DiscoverResult {
        pollReads++
        failure?.let { throw it() }
        return pollAnswers.removeFirst()
    }

    override fun explorerSettings(): ExplorerSettingsData = DiscoveryFixtures.explorerSettings()

    override fun cachedStationStyles(): List<StationRowData>? = cachedStationRows

    override suspend fun stationStyles(): List<StationRowData> {
        stationStyleReads++
        failure?.let { throw it() }
        return stationRows ?: emptyList()
    }

    override suspend fun genreStation(style: String): Station {
        stationPosts++
        failure?.let { throw it() }
        return station
    }

    override suspend fun muteArtist(provider: String, id: String) {
        muted += provider to id
    }

    override suspend fun recommendations(level: Double): DiscoveryRows =
        PlainRows(DiscoveryFixtures.rows())
}

/** What the screens asked the player: the COMMAND, never a sound. */
class FakePlayPort : PlayPort {
    data class Played(
        val rows: List<TrackRow>,
        val start: Int,
        val kind: String,
        val id: String,
    )

    val played = mutableListOf<Played>()
    val queued = mutableListOf<TrackRow>()
    val radios = mutableListOf<String>()

    override fun play(rows: DiscoveryRows, start: Int, kind: String, id: String) {
        played += Played(rows.all(), start, kind, id)
    }

    override fun playList(rows: List<TrackRow>, start: Int, kind: String, id: String) {
        played += Played(rows, start, kind, id)
    }

    override fun playRow(row: TrackRow, id: String) {
        radios += id
    }

    override fun playNext(row: TrackRow) {
        queued += row
    }
}

/** A coroutine gate a test opens by hand: waits on an EVENT, never on a clock. */
suspend fun <T> await(block: suspend () -> T): T = block()

/** A deferred the test completes once the model has published its state. */
fun latch(): CompletableDeferred<Unit> = CompletableDeferred()

/** What the toasts said, so a failure's sentence is asserted. */
class Toasts {
    val said = mutableListOf<String>()

    fun record(message: String) {
        said += message
    }

    operator fun invoke(message: String) = record(message)
}

/**
 * Waits for a model's STATE, on the test scheduler: it advances the queue until
 * [condition] holds, then gives up with the state in the message.
 *
 * Why a helper and not a bare `advanceUntilIdle()`: a `screenFlow` emission and
 * a `withContext` continuation can take a dispatch round each, and the runner's
 * JVM (JDK 17) settled them differently from this machine's — the same six
 * cases passed here and failed there. A test that waits on the STATE is a test
 * of the model; one that waits on a queue length is a test of the scheduler.
 * Nothing here waits on the clock: `advanceUntilIdle` and `runCurrent` run what
 * is pending, and the loop is bounded.
 */
fun TestScope.settle(
    what: String = "state",
    condition: () -> Boolean,
): Boolean {
    repeat(64) {
        if (condition()) return true
        advanceUntilIdle()
        runCurrent()
    }
    return condition()
}

/** A `ScreenState`'s data, or a test failure. */
fun <T> ScreenState<T>.data(): T = (this as? ScreenState.Ready<T>)?.data
    ?: throw AssertionError("expected Ready, was $this")
