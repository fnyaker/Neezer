package fr.neezer.mobile.feature.podcasts

import uniffi.nz_ffi.ChapterData
import uniffi.nz_ffi.ChaptersData
import uniffi.nz_ffi.EpisodeData
import uniffi.nz_ffi.EpisodeMarkerData
import uniffi.nz_ffi.EpisodePositionData
import uniffi.nz_ffi.EpisodePositionEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.NoHandle
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.PositionInfo
import uniffi.nz_ffi.ShowArchiveData
import uniffi.nz_ffi.ShowData
import uniffi.nz_ffi.ShowPageData
import uniffi.nz_ffi.ShowTranscribeData
import uniffi.nz_ffi.SortKey
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.TranscribeSettingsData
import uniffi.nz_ffi.TranscribeStatusData
import uniffi.nz_ffi.TranscriptInfoData
import uniffi.nz_ffi.TranscriptPageData
import uniffi.nz_ffi.TranscriptSearchData
import uniffi.nz_ffi.TranscriptSegmentData

/**
 * What the podcast tests hold instead of the core (lot MB13).
 *
 * The models are plain Kotlin over [PodcastService] and [PodcastsPlayer], so a
 * fake makes every rule an ASSERTION with a count — how many requests a warm
 * open costs, whether a refusal rolled the switch back, which rows a tap
 * queued — without JNA, a `.so` or a device.
 */
class FakePodcastService(
    var shows: List<ShowData> = listOf(show("show-1", "Les Histoires", episodes = 3)),
    var pages: MutableList<ShowPageData> = mutableListOf(),
    var cardsByIndex: MutableMap<Int, EpisodeData> = mutableMapOf(),
    var chapters: MutableMap<String, ChaptersData> = mutableMapOf(),
    var transcripts: MutableMap<String, TranscriptPageData> = mutableMapOf(),
) : PodcastService {
    /** Every call the screen made, in order — the request COUNT of the tests. */
    val calls = mutableListOf<String>()

    var cachedShowsAnswer: List<ShowData>? = null
    var cachedShowAnswer: ShowPageData? = null
    var failShows: Throwable? = null
    var failSubscribe: Throwable? = null
    var failArchive: Throwable? = null
    var failSearch: Throwable? = null
    var failChapters: Throwable? = null
    var failTranscribeToggle: Throwable? = null
    var failAddMarker: Throwable? = null
    var failRemoveMarker: Throwable? = null
    var failTranscript: Throwable? = null
    var searchAnswer: List<ShowData> = listOf(show("deezer-9", "Trouvé", deezer = "9400009"))
    var searchDelayAnswer: (() -> List<ShowData>)? = null
    var transcribeSettingsAnswer: TranscribeSettingsData? = settings()
    var failTranscribeSettings: Throwable? = null
    var transcribeSupportedAnswer: Boolean? = true
    var positionsAnswer: List<EpisodePositionEntry> = emptyList()
    var markersByEpisode: MutableMap<String, MutableList<EpisodeMarkerData>> = mutableMapOf()
    var localSearchAnswer: List<TranscriptSegmentData> = emptyList()

    override suspend fun cachedShows(): List<ShowData>? {
        calls += "cachedShows"
        return cachedShowsAnswer
    }

    override suspend fun shows(): List<ShowData> {
        calls += "shows"
        failShows?.let { throw it }
        return shows
    }

    override suspend fun search(query: String): List<ShowData> {
        calls += "search:$query"
        failSearch?.let { throw it }
        return searchDelayAnswer?.invoke() ?: searchAnswer
    }

    override suspend fun subscribe(url: String): ShowData {
        calls += "subscribe:$url"
        failSubscribe?.let { throw it }
        return shows.first()
    }

    override suspend fun unsubscribe(id: String) {
        calls += "unsubscribe:$id"
    }

    override suspend fun cachedShow(id: String): ShowPageData? {
        calls += "cachedShow:$id"
        return cachedShowAnswer
    }

    override suspend fun show(id: String): ShowPageData {
        calls += "show:$id"
        return pages.firstOrNull() ?: page(id, loaded = 0, next = null)
    }

    override suspend fun showMore(id: String): ShowPageData {
        calls += "showMore:$id"
        return pages.getOrElse(1) { page(id, loaded = 50, next = null) }
    }

    override suspend fun episodeCards(id: String, start: Int, count: Int): List<EpisodeData> {
        calls += "cards:$start:$count"
        return (start until start + count).mapNotNull { cardsByIndex[it] }
    }

    override suspend fun autoArchive(id: String, on: Boolean): ShowArchiveData {
        calls += "autoArchive:$on"
        failArchive?.let { throw it }
        return ShowArchiveData(show = shows.first().copy(autoArchive = on, archivedCount = 9u), queued = 17u)
    }

    override suspend fun setShowTranscribe(
        id: String,
        auto: Boolean?,
        backlog: String?,
        language: String?,
    ): ShowTranscribeData {
        calls += "setShowTranscribe:auto=$auto,backlog=$backlog,language=$language"
        failTranscribeToggle?.let { throw it }
        val show = shows.first().copy(
            autoTranscribe = auto ?: shows.first().autoTranscribe,
            transcribeLanguage = language ?: shows.first().transcribeLanguage,
        )
        return ShowTranscribeData(show = show, queued = 12u, skipped = 1u, backlog = backlog ?: "all")
    }

    override suspend fun progress(): List<EpisodePositionEntry> {
        calls += "progress"
        return positionsAnswer
    }

    override suspend fun positions(): List<EpisodePositionEntry> {
        calls += "positions"
        return positionsAnswer
    }

    override suspend fun episodePosition(episodeId: String): EpisodePositionData? {
        calls += "episodePosition:$episodeId"
        return positionsAnswer.firstOrNull { it.episodeId == episodeId }?.position
    }

    override suspend fun loadShowMarkers(id: String) {
        calls += "loadShowMarkers:$id"
    }

    override suspend fun loadEpisodeMarkers(episodeId: String): List<EpisodeMarkerData> {
        calls += "loadEpisodeMarkers:$episodeId"
        return markersByEpisode[episodeId].orEmpty()
    }

    override suspend fun episodeMarkers(episodeId: String): List<EpisodeMarkerData> {
        calls += "episodeMarkers:$episodeId"
        return markersByEpisode[episodeId].orEmpty()
    }

    override suspend fun addMarker(
        episodeId: String,
        positionMs: Int,
        label: String?,
    ): EpisodeMarkerData {
        calls += "addMarker:$episodeId:$positionMs"
        failAddMarker?.let { throw it }
        val marker = marker("m-new", episodeId, positionMs, label.orEmpty())
        markersByEpisode.getOrPut(episodeId) { mutableListOf() } += marker
        return marker
    }

    override suspend fun removeMarker(markerId: String) {
        calls += "removeMarker:$markerId"
        failRemoveMarker?.let { throw it }
        markersByEpisode.values.forEach { it.removeAll { marker -> marker.id == markerId } }
    }

    override suspend fun cachedChapters(episodeId: String): ChaptersData? {
        calls += "cachedChapters:$episodeId"
        return chapters[episodeId]
    }

    override suspend fun chapters(episodeId: String): ChaptersData {
        calls += "chapters:$episodeId"
        failChapters?.let { throw it }
        return chapters[episodeId] ?: chaptersOf(episodeId)
    }

    override suspend fun cachedTranscript(
        episodeId: String,
        limit: Int,
        offset: Int,
        at: Double?,
    ): TranscriptPageData? {
        calls += "cachedTranscript:$episodeId:$offset"
        return null
    }

    override suspend fun transcript(
        episodeId: String,
        limit: Int,
        offset: Int?,
        at: Double?,
        q: String?,
    ): TranscriptPageData {
        calls += "transcript:$episodeId:${offset ?: -1}:${q ?: ""}"
        failTranscript?.let { throw it }
        return transcripts[episodeId] ?: transcriptPage(episodeId, offset ?: 0, limit)
    }

    override suspend fun localSearch(
        episodeId: String,
        q: String,
        limit: Int,
    ): List<TranscriptSegmentData> {
        calls += "localSearch:$episodeId:$q"
        return localSearchAnswer
    }

    override suspend fun showSearch(id: String, q: String, limit: Int): TranscriptSearchData {
        calls += "showSearch:$id:$q"
        failSearch?.let { throw it }
        return TranscriptSearchData(
            q = q,
            hits = emptyList(),
            total = 0u,
            more = false,
            episodes = 24u,
            truncated = false,
        )
    }

    override suspend fun transcriptionSupported(): Boolean? {
        calls += "transcriptionSupported"
        return transcribeSupportedAnswer
    }

    override suspend fun transcribeSettings(probe: Boolean): TranscribeSettingsData {
        calls += "transcribeSettings:$probe"
        failTranscribeSettings?.let { throw it }
        return transcribeSettingsAnswer ?: settings()
    }

    override suspend fun transcribeStatus(): TranscribeStatusData {
        calls += "transcribeStatus"
        return TranscribeStatusData(
            available = true,
            enabled = true,
            reason = null,
            queued = 1u,
            running = 1u,
            ready = 8u,
            error = 0u,
            budgetLimitMs = 7_200_000uL,
            budgetUsedMs = 0uL,
        )
    }

    override suspend fun startTranscript(episodeId: String, language: String?, force: Boolean) {
        calls += "startTranscript:$episodeId:$force"
    }

    override suspend fun stopTranscript(episodeId: String) {
        calls += "stopTranscript:$episodeId"
    }

    override suspend fun deleteTranscript(episodeId: String) {
        calls += "deleteTranscript:$episodeId"
    }

    /** The calls whose name starts with [prefix] — a test's own count. */
    fun count(prefix: String): Int = calls.count { it.startsWith(prefix) }
}

/** A player that records what it was asked, and holds a fake state. */
class FakePodcastsPlayer : PodcastsPlayer {
    var snapshotAnswer: PlayerSnapshot? = null
    var positionAnswer: PositionInfo? = PositionInfo(
        positionS = 0.0,
        durationS = 0.0,
        bufferedS = 0.0,
        playing = false,
    )
    val commands = mutableListOf<String>()
    var played: List<TrackRow> = emptyList()
    var playedStart: Int = -1
    var playedContext: String = ""
    var seeks: MutableList<Double> = mutableListOf()

    override fun snapshot(): PlayerSnapshot? = snapshotAnswer

    override fun position(): PositionInfo? = positionAnswer

    override fun play(episodes: List<TrackRow>, start: Int, contextId: String) {
        played = episodes
        playedStart = start
        playedContext = contextId
        commands += "play:$start:${episodes.size}"
    }

    override fun toggle() {
        commands += "toggle"
    }

    override fun seek(seconds: Double) {
        seeks += seconds
        commands += "seek:$seconds"
    }
}

/** A `ListHandle` a JVM test can hand a model: the model only reads its length. */
fun fakeHandle(rows: Int): ListHandle = EmptyHandle(rows)

/** An empty list handle: `NoHandle` is UniFFI's own "fake object" door. */
private class EmptyHandle(private val size: Int) : ListHandle(NoHandle) {
    override fun len(): UInt = size.toUInt()

    override fun window(start: UInt, count: UInt): List<TrackRow> = emptyList()

    override fun indexOf(key: String): UInt? = null

    override fun filter(query: String): UInt = size.toUInt()

    override fun sort(by: SortKey, descending: Boolean): UInt = size.toUInt()

    override fun version(): ULong = 0uL
}

// -- the fixtures ------------------------------------------------------------

fun show(
    id: String,
    title: String,
    deezer: String? = null,
    episodes: Int = 0,
    autoArchive: Boolean = false,
    autoTranscribe: Boolean = false,
    language: String = "auto",
): ShowData = ShowData(
    id = id,
    deezerId = deezer,
    title = title,
    description = "Une heure de récits",
    cover = null,
    subscribed = true,
    local = false,
    status = "ok",
    episodeCount = episodes.toUInt(),
    archivedCount = 2u,
    autoArchive = autoArchive,
    autoTranscribe = autoTranscribe,
    transcribeLanguage = language,
    transcribedCount = null,
    archiveLine = "2 sur $episodes épisodes archivés sur le serveur",
    subscribeUrl = deezer?.let { "https://www.deezer.com/show/$it" },
)

fun episode(
    id: String,
    title: String = "Épisode $id",
    archived: Boolean = true,
    chapters: Int = 0,
    published: Long = 1_794_960_000,
    transcript: TranscriptInfoData? = null,
    position: EpisodePositionData? = null,
    markers: List<EpisodeMarkerData> = emptyList(),
): EpisodeData = EpisodeData(
    id = id,
    deezerId = id,
    title = title,
    description = "Un épisode de plus.",
    durationS = 3_600u,
    durationLabel = "1:00:00",
    published = published,
    publishedLabel = "18 nov. 2026",
    archived = archived,
    unavailable = false,
    explicit = false,
    chapterCount = chapters.toUInt(),
    chaptersLabel = if (chapters > 1) "$chapters chapitres" else "$chapters chapitre",
    status = if (archived) "completed" else "",
    cover = null,
    channelId = "show-1",
    channelTitle = "Les Histoires",
    transcript = transcript,
    position = position,
    markers = markers,
    about = true,
    row = TrackRow(
        key = id,
        id = id,
        title = title,
        artists = "Les Histoires",
        album = "Les Histoires",
        albumId = "show-1",
        durationS = 3_600u,
        cover = null,
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = archived,
        source = "podcast",
        position = 0u,
    ),
)

fun page(id: String, loaded: Int, next: String?): ShowPageData = ShowPageData(
    show = show(id, "Les Histoires", episodes = 2_000),
    episodes = fakeHandle(loaded),
    next = next,
    loaded = loaded.toUInt(),
)

fun marker(id: String, episodeId: String, positionMs: Int, label: String = ""): EpisodeMarkerData =
    EpisodeMarkerData(
        id = id,
        episodeId = episodeId,
        positionMs = positionMs.toUInt(),
        seconds = (positionMs / 1_000).toUInt(),
        timeLabel = PodcastsCopy.duration(positionMs / 1_000),
        label = label,
    )

/** An episode heard up to [seconds]: the resume bar's fixture. */
fun resumeAt(seconds: Int) = uniffi.nz_ffi.EpisodePositionData(
    seconds = seconds.toUInt(),
    durationS = 3_600u,
    finished = false,
    fraction = seconds / 3_600.0,
    remainingS = (3_600 - seconds).toUInt(),
    pending = false,
)

fun chaptersOf(episodeId: String): ChaptersData = ChaptersData(
    episodeId = episodeId,
    chapters = listOf(
        ChapterData(index = 0u, start = 0u, end = 900u, title = "Ouverture", label = "Chapitre 1 · Ouverture"),
        ChapterData(index = 1u, start = 900u, end = 1_800u, title = "Le village", label = "Chapitre 2 · Le village"),
        ChapterData(index = 2u, start = 1_800u, end = null, title = "", label = "Chapitre 3"),
    ),
    source = "id3",
    archived = true,
    scanned = true,
    durationS = 3_600u,
)

fun transcriptInfo(status: String, progress: Double = 0.0): TranscriptInfoData = TranscriptInfoData(
    status = status,
    progress = progress,
    percent = (progress * 100).toInt().toUInt(),
    language = "fr",
    source = "server",
    segmentCount = 240u,
    errorCode = null,
    active = status == "queued" || status == "running" || status == "leased",
    ready = status == "ready",
)

fun transcriptPage(episodeId: String, offset: Int, limit: Int): TranscriptPageData {
    val total = 240
    val from = offset.coerceAtLeast(0)
    val to = (from + limit).coerceAtMost(total)
    return TranscriptPageData(
        episodeId = episodeId,
        status = "ready",
        archived = true,
        language = "fr",
        durationMs = 3_600_000uL,
        doneMs = 3_600_000uL,
        progress = 1.0,
        percent = 100u,
        total = total.toUInt(),
        offset = from.toUInt(),
        limit = limit.toUInt(),
        query = null,
        segments = (from until to).map { index ->
            TranscriptSegmentData(
                start = (index * 15).toDouble(),
                end = ((index + 1) * 15).toDouble(),
                text = "Ligne $index",
            )
        },
        next = if (to < total) to.toUInt() else null,
        errorCode = null,
        errorMessage = null,
        cached = false,
        active = false,
        page = (from / limit + 1).toUInt(),
        pages = ((total + limit - 1) / limit).toUInt(),
    )
}

fun settings(
    available: Boolean = true,
    reason: String? = null,
): TranscribeSettingsData = TranscribeSettingsData(
    available = available,
    enabled = true,
    reason = reason,
    language = "fr",
    languages = TRANSCRIBE_LANGUAGES,
    dailyMinutes = 120u,
    maxMinutes = 240u,
    queued = 1u,
    running = 1u,
    ready = 8u,
    error = 0u,
    engine = "whisper.cpp",
    model = "ggml-small.bin",
)
