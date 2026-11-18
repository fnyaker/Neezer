package fr.neezer.mobile.feature.podcasts

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.ChaptersData
import uniffi.nz_ffi.EpisodeData
import uniffi.nz_ffi.EpisodeMarkerData
import uniffi.nz_ffi.EpisodePositionData
import uniffi.nz_ffi.EpisodePositionEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.PlayContext
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.PositionInfo
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.ShowArchiveData
import uniffi.nz_ffi.ShowData
import uniffi.nz_ffi.ShowPageData
import uniffi.nz_ffi.ShowTranscribeData
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.TranscribeSettingsData
import uniffi.nz_ffi.TranscribeStatusData
import uniffi.nz_ffi.TranscriptPageData
import uniffi.nz_ffi.TranscriptSearchData
import uniffi.nz_ffi.TranscriptSegmentData

/**
 * What the podcast screens ask of the core: this lot's `nz-ffi` methods, one to
 * one (lot MB13).
 *
 * An interface, not the generated `Screens`, so the screens' logic is plain
 * Kotlin that `PodcastsModelTest` and `ShowModelTest` run on the JVM with a
 * fake — no JNA, no `.so`, no device. [NzPodcastService] is the one real
 * implementation.
 */
interface PodcastService {
    /** The shows this device kept, or `null` the first time. Disk only. */
    suspend fun cachedShows(): List<ShowData>?

    /** `GET /api/podcasts` — every show the server holds. */
    suspend fun shows(): List<ShowData>

    /** `GET /api/search/podcasts?q=` — the provider's catalogue. */
    suspend fun search(query: String): List<ShowData>

    /** `POST /api/podcasts {url}` — subscribe and land on the show it answers. */
    suspend fun subscribe(url: String): ShowData

    /** `DELETE /api/podcast/<id>` — unsubscribe; the archive stays. */
    suspend fun unsubscribe(id: String)

    /** The show this device kept, or `null` the first time. Disk only. */
    suspend fun cachedShow(id: String): ShowPageData?

    /** `GET /api/podcast/<id>?limit=50` — the first page and the markers. */
    suspend fun show(id: String): ShowPageData

    /** The next cursor's page, appended to what the device holds. */
    suspend fun showMore(id: String): ShowPageData

    /** The episodes `start..start+count` of the loaded page. No request. */
    suspend fun episodeCards(id: String, start: Int, count: Int): List<EpisodeData>

    suspend fun autoArchive(id: String, on: Boolean): ShowArchiveData

    suspend fun setShowTranscribe(
        id: String,
        auto: Boolean?,
        backlog: String?,
        language: String?,
    ): ShowTranscribeData

    /** `GET /api/podcast/progress` — the merge of both copies. */
    suspend fun progress(): List<EpisodePositionEntry>

    /** The merged positions, disk only. */
    suspend fun positions(): List<EpisodePositionEntry>

    suspend fun episodePosition(episodeId: String): EpisodePositionData?

    /** The show's markers, kept for the rows; fail-soft in the app. */
    suspend fun loadShowMarkers(id: String)

    suspend fun loadEpisodeMarkers(episodeId: String): List<EpisodeMarkerData>

    suspend fun episodeMarkers(episodeId: String): List<EpisodeMarkerData>

    suspend fun addMarker(
        episodeId: String,
        positionMs: Int,
        label: String?,
    ): EpisodeMarkerData

    suspend fun removeMarker(markerId: String)

    suspend fun cachedChapters(episodeId: String): ChaptersData?

    suspend fun chapters(episodeId: String): ChaptersData

    suspend fun cachedTranscript(
        episodeId: String,
        limit: Int,
        offset: Int,
        at: Double?,
    ): TranscriptPageData?

    suspend fun transcript(
        episodeId: String,
        limit: Int,
        offset: Int?,
        at: Double?,
        q: String?,
    ): TranscriptPageData

    suspend fun localSearch(episodeId: String, q: String, limit: Int): List<TranscriptSegmentData>

    suspend fun showSearch(id: String, q: String, limit: Int): TranscriptSearchData

    /** Whether the server has the transcription routes at all (rule 7). */
    suspend fun transcriptionSupported(): Boolean?

    suspend fun transcribeSettings(probe: Boolean): TranscribeSettingsData

    suspend fun transcribeStatus(): TranscribeStatusData

    suspend fun startTranscript(episodeId: String, language: String?, force: Boolean)

    suspend fun stopTranscript(episodeId: String)

    suspend fun deleteTranscript(episodeId: String)
}

/** The one real implementation of [PodcastService], over the core's `Screens`. */
class NzPodcastService(private val screens: Screens) : PodcastService {
    override suspend fun cachedShows(): List<ShowData>? = screens.cachedPodcasts()

    override suspend fun shows(): List<ShowData> = screens.podcasts()

    override suspend fun search(query: String): List<ShowData> = screens.searchPodcasts(query)

    override suspend fun subscribe(url: String): ShowData = screens.subscribePodcast(url)

    override suspend fun unsubscribe(id: String) = screens.unsubscribePodcast(id)

    override suspend fun cachedShow(id: String): ShowPageData? = screens.cachedPodcastShow(id)

    override suspend fun show(id: String): ShowPageData = screens.podcastShow(id)

    override suspend fun showMore(id: String): ShowPageData = screens.podcastShowMore(id)

    override suspend fun episodeCards(id: String, start: Int, count: Int): List<EpisodeData> =
        screens.podcastEpisodeCards(id, start.coerceAtLeast(0).c, count.coerceAtLeast(0).c)

    override suspend fun autoArchive(id: String, on: Boolean): ShowArchiveData =
        screens.podcastAutoArchive(id, on)

    override suspend fun setShowTranscribe(
        id: String,
        auto: Boolean?,
        backlog: String?,
        language: String?,
    ): ShowTranscribeData = screens.podcastSetShowTranscribe(id, auto, backlog, language)

    override suspend fun progress(): List<EpisodePositionEntry> = screens.podcastProgress()

    override suspend fun positions(): List<EpisodePositionEntry> = screens.podcastPositions()

    override suspend fun episodePosition(episodeId: String): EpisodePositionData? =
        screens.podcastEpisodePosition(episodeId)

    override suspend fun loadShowMarkers(id: String) = screens.podcastLoadShowMarkers(id)

    override suspend fun loadEpisodeMarkers(episodeId: String): List<EpisodeMarkerData> =
        screens.podcastLoadEpisodeMarkers(episodeId)

    override suspend fun episodeMarkers(episodeId: String): List<EpisodeMarkerData> =
        screens.podcastEpisodeMarkers(episodeId)

    override suspend fun addMarker(
        episodeId: String,
        positionMs: Int,
        label: String?,
    ): EpisodeMarkerData = screens.podcastAddMarker(episodeId, positionMs.coerceAtLeast(0).c, label)

    override suspend fun removeMarker(markerId: String) = screens.podcastRemoveMarker(markerId)

    override suspend fun cachedChapters(episodeId: String): ChaptersData? =
        screens.cachedPodcastChapters(episodeId)

    override suspend fun chapters(episodeId: String): ChaptersData = screens.podcastChapters(episodeId)

    override suspend fun cachedTranscript(
        episodeId: String,
        limit: Int,
        offset: Int,
        at: Double?,
    ): TranscriptPageData? =
        screens.cachedPodcastTranscript(episodeId, limit.c, offset.coerceAtLeast(0).c, at)

    override suspend fun transcript(
        episodeId: String,
        limit: Int,
        offset: Int?,
        at: Double?,
        q: String?,
    ): TranscriptPageData =
        screens.podcastTranscript(episodeId, limit.c, offset?.c, at, q)

    override suspend fun localSearch(
        episodeId: String,
        q: String,
        limit: Int,
    ): List<TranscriptSegmentData> = screens.podcastLocalSearch(episodeId, q, limit.c)

    override suspend fun showSearch(id: String, q: String, limit: Int): TranscriptSearchData =
        screens.podcastShowSearch(id, q, limit.c)

    override suspend fun transcriptionSupported(): Boolean? =
        screens.podcastTranscriptionSupported()

    override suspend fun transcribeSettings(probe: Boolean): TranscribeSettingsData =
        screens.podcastTranscribeSettings(probe)

    override suspend fun transcribeStatus(): TranscribeStatusData = screens.podcastTranscribeStatus()

    override suspend fun startTranscript(episodeId: String, language: String?, force: Boolean) =
        screens.podcastStartTranscript(episodeId, language, force)

    override suspend fun stopTranscript(episodeId: String) = screens.podcastStopTranscript(episodeId)

    override suspend fun deleteTranscript(episodeId: String) =
        screens.podcastDeleteTranscript(episodeId)
}

/** A `ListHandle` as the windowed episode list reads it (rule 4). */
fun ListHandle.lenAsInt(): Int = len().toInt()

fun ListHandle.windowAsList(start: Int, count: Int): List<TrackRow> =
    window(start.coerceAtLeast(0).c, count.coerceAtLeast(0).c)

/** `Int` → UniFFI's `UInt`, in one place so no call site can forget it. */
val Int.c: UInt get() = coerceAtLeast(0).toUInt()

/**
 * What the podcast screens ask of the PLAYER (MB3b's `PlayerApi`).
 *
 * The library's rule, kept here: a screen never holds a `PlayerApi` directly, so
 * a JVM test asserts the COMMAND a tap sends with a fake — the queue's own rules
 * (gapless, crossfade away on an episode, the podcast speed) stay the player's.
 */
interface PodcastsPlayer {
    /** The state as of now: what plays, and whether. Cheap (`snapshot()`). */
    fun snapshot(): PlayerSnapshot?

    /** The hot position read — only while a chapter card or a bar is visible. */
    fun position(): PositionInfo?

    /** Replaces the queue with an episode list and starts at [start]. */
    fun play(episodes: List<TrackRow>, start: Int, contextId: String)

    /** Play/pause the current entry. */
    fun toggle()

    /** A seek inside the current entry (a chapter, a transcript hit, a marker). */
    fun seek(seconds: Double)
}

/** A port that does nothing, for a process whose core could not be built. */
object NoPodcastsPlayer : PodcastsPlayer {
    override fun snapshot(): PlayerSnapshot? = null

    override fun position(): PositionInfo? = null

    override fun play(episodes: List<TrackRow>, start: Int, contextId: String) = Unit

    override fun toggle() = Unit

    override fun seek(seconds: Double) = Unit
}

/** The real port: `PlayerCommand`, in the player's own words. */
class NzPodcastsPlayer(private val player: PlayerApi) : PodcastsPlayer {
    override fun snapshot(): PlayerSnapshot? = runCatching { player.snapshot() }.getOrNull()

    override fun position(): PositionInfo? = runCatching { player.position() }.getOrNull()

    override fun play(episodes: List<TrackRow>, start: Int, contextId: String) {
        // The whole loaded page becomes the queue: an episode taps into the
        // list it was read from, exactly as `Show.svelte#playFrom` does.
        player.command(
            PlayerCommand.PlayQueue(
                tracks = episodes.map(::ref),
                start = start.coerceAtLeast(0).c,
                context = PlayContext(kind = "podcast", id = contextId),
            ),
            0uL,
        )
    }

    override fun toggle() {
        player.command(PlayerCommand.Toggle, 0uL)
    }

    override fun seek(seconds: Double) {
        player.command(PlayerCommand.Seek(s = seconds.coerceAtLeast(0.0)), 0uL)
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
        // An episode, and the player knows what that changes (no crossfade, the
        // podcast speed applies, its own resume point).
        episode = true,
        liked = row.liked,
    )
}

/**
 * The app's podcast services, asked of the core once.
 *
 * `NzCore` answers the ONE instance of each (a cheap `Arc` clone), so asking per
 * screen is correct; nothing here caches an object the core could replace.
 */
object AppPodcasts {
    fun service(): PodcastService? =
        runCatching { NzBridge.core?.screens() }.getOrNull()?.let(::NzPodcastService)

    fun player(): PodcastsPlayer =
        runCatching { NzBridge.core?.player() }.getOrNull()?.let(::NzPodcastsPlayer) ?: NoPodcastsPlayer

    /**
     * Whether the person may add podcasts — the SPA's `$isAdmin` (its handover
     * flag is `user.admin` from `/api/me`, which the session kept: no request).
     */
    fun isAdmin(): Boolean =
        runCatching { NzBridge.core?.settings()?.identity()?.admin == true }.getOrDefault(false)
}
