package fr.neezer.mobile.feature.search

import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.NzRowSource
import fr.neezer.mobile.designsystem.asRowSource
import uniffi.nz_ffi.ImportOffer
import uniffi.nz_ffi.PlayContext
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.RadioPlan
import uniffi.nz_ffi.SearchEntity
import uniffi.nz_ffi.SearchFallback
import uniffi.nz_ffi.SearchResults
import uniffi.nz_ffi.SearchScope
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.TopResult
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.TranscriptHit
import uniffi.nz_ffi.nzSearchMinTerm
import uniffi.nz_ffi.nzSearchTopTracks

/**
 * One answer of the search screen, as the screen draws it (lot MB6).
 *
 * Not the FFI's `SearchResults` record: that one carries the track list as a
 * `ListHandle`, a native object a JVM test cannot build. The port converts it to
 * [NzRowSource] (the design system's own two-method window interface), so
 * `SearchModelTest` and the screenshot fixtures run on a plain `List<TrackRow>`
 * — no JNA, no `.so`, no device — while the real screen keeps the windowed
 * handle the lists use everywhere (rule 4).
 */
data class SearchAnswer(
    val term: String,
    val scope: SearchScope,
    val top: TopResult?,
    /** The list the screen draws, windowed (`ListWindow`), the fallback woven in. */
    val tracks: NzRowSource,
    /** How many rows the list holds — the "Tout afficher 42" count. */
    val trackCount: Int,
    val artists: List<SearchEntity>,
    /** The albums with the shows mixed in after the first two. */
    val albums: List<SearchEntity>,
    val playlists: List<SearchEntity>,
    val podcasts: List<SearchEntity>,
    val transcripts: List<TranscriptHit>,
    val transcriptTotal: Int,
    val transcriptMore: Boolean,
    val transcriptEpisodes: Int,
    val transcriptSegments: Int?,
    val transcriptTruncated: Boolean,
    /** How many of the tracks came from the fallback ("YT" rows). */
    val youtube: Int,
    val youtubeAnswered: Boolean,
    val youtubeAvailable: Boolean,
    val youtubeFailure: SearchFallback?,
    val radioTotal: Int,
    val radioOffered: Boolean,
) {
    /** Anything to draw at all (`Search.svelte#hasResults`). */
    val hasResults: Boolean
        get() = when (scope) {
            SearchScope.LIBRARY ->
                artists.isNotEmpty() || albums.isNotEmpty() || trackCount > 0 ||
                    playlists.isNotEmpty() || podcasts.isNotEmpty()
            SearchScope.PODCASTS -> transcripts.isNotEmpty()
            SearchScope.SHOWS -> podcasts.isNotEmpty()
        }
}

/**
 * What the search asks of the core (lot MB6).
 *
 * An interface, not the generated `Screens`, so the screen's logic is plain
 * Kotlin that `SearchModelTest` runs on the JVM with a fake. [NzSearchService] is
 * the one implementation, a one-to-one delegation to `nz-ffi/src/search.rs`.
 */
interface SearchService {
    /**
     * The answer this device kept for [term]: disk only, no request.
     *
     * [fallback] is the screen's YouTube chip: with it off the kept fallback rows
     * are not woven in, which is what makes turning the chip cost nothing.
     */
    suspend fun cachedSearch(term: String, scope: SearchScope, fallback: Boolean): SearchAnswer?

    /** One request: `GET /api/search`, the transcripts, or the shows. */
    suspend fun search(term: String, scope: SearchScope, fallback: Boolean): SearchAnswer

    /** `GET /api/youtube/search` — the fallback, its rows woven into the list. */
    suspend fun youtubeHits(term: String): SearchAnswer

    /** The radio's opening titles and the context the player keeps. */
    suspend fun searchAsRadio(term: String): RadioPlan

    fun recentSearches(): List<String>

    fun noteSearch(term: String)

    fun clearRecentSearches()

    /** Whether this account can ask the fallback at all (the chip is offered). */
    fun youtubeAvailable(): Boolean

    /** Whether the shows tab exists for this account (an admin's). */
    fun isAdmin(): Boolean

    /** The playlist link in [text], if the box holds one (an import, not a query). */
    fun importOffer(text: String): ImportOffer?

    /** The shortest term the core will search (two characters). */
    val minTerm: Int

    /** How many titles the "Tout" tab shows before "Tout afficher" takes over. */
    val topTracks: Int
}

/** The one real implementation: this lot's `nz-ffi` methods, converted. */
class NzSearchService(private val screens: Screens) : SearchService {
    override suspend fun cachedSearch(
        term: String,
        scope: SearchScope,
        fallback: Boolean,
    ): SearchAnswer? = screens.cachedSearch(term, scope, fallback)?.toAnswer()

    override suspend fun search(term: String, scope: SearchScope, fallback: Boolean): SearchAnswer =
        screens.search(term, scope, fallback).toAnswer()

    override suspend fun youtubeHits(term: String): SearchAnswer =
        screens.youtubeHits(term).toAnswer()

    override suspend fun searchAsRadio(term: String): RadioPlan = screens.searchAsRadio(term)

    override fun recentSearches(): List<String> = screens.recentSearches()

    override fun noteSearch(term: String) = screens.noteSearch(term)

    override fun clearRecentSearches() = screens.clearRecentSearches()

    override fun youtubeAvailable(): Boolean = screens.youtubeAvailable()

    override fun isAdmin(): Boolean = screens.searchIsAdmin()

    override fun importOffer(text: String): ImportOffer? = screens.searchImportOffer(text)

    override val minTerm: Int get() = nzSearchMinTerm().toInt()

    override val topTracks: Int get() = nzSearchTopTracks().toInt()
}

/** The FFI's answer, read the way the screen draws it. */
fun SearchResults.toAnswer(): SearchAnswer = SearchAnswer(
    term = term,
    scope = scope,
    top = top,
    // `asRowSource()` is `:designsystem`'s own adapter, fixed in PR #321 (it used
    // to call itself); `RowSourceTest` holds it on the device.
    tracks = tracks.asRowSource(),
    trackCount = tracks.len().toInt(),
    artists = artists,
    albums = albums,
    playlists = playlists,
    podcasts = podcasts,
    transcripts = transcripts,
    transcriptTotal = transcriptTotal.toInt(),
    transcriptMore = transcriptMore,
    transcriptEpisodes = transcriptEpisodes.toInt(),
    transcriptSegments = transcriptSegments?.toInt(),
    transcriptTruncated = transcriptTruncated,
    youtube = youtube.toInt(),
    youtubeAnswered = youtubeAnswered,
    youtubeAvailable = youtubeAvailable,
    youtubeFailure = youtubeFailure,
    radioTotal = radioTotal.toInt(),
    radioOffered = radioOffered,
)

/**
 * What the search asks of the player (MB3b's `PlayerApi`).
 *
 * The screen never holds a `PlayerApi` itself: it hands the rows and the context
 * to this seam, so a JVM test asserts the COMMAND a press sends with a fake, and
 * the real one is one thin file. The queue rules are the player's.
 */
interface SearchPlayer {
    /**
     * Replaces the queue with [tracks] and starts at the first one, under a
     * context the player keeps: `kind` and `id` are what its own refill reads
     * (`radio` + `track:<ref>`/`artist:<id>`), which is why a launch costs no
     * request at all.
     */
    fun play(tracks: List<TrackRow>, contextKind: String, contextId: String)

    /**
     * Plays a whole entity (`artist`, `album`, `playlist`…): what the best
     * result's own control does (`TopResult.svelte#play` → `playEntity`). The
     * player resolves the list and keeps the context.
     */
    fun playEntity(kind: String, id: String)
}

/** A port that does nothing, for a process whose core could not be built. */
object NoSearchPlayer : SearchPlayer {
    override fun play(tracks: List<TrackRow>, contextKind: String, contextId: String) = Unit

    override fun playEntity(kind: String, id: String) = Unit
}

/** The real port: `PlayerCommand.PlayQueue`, on the player's own worker. */
class NzSearchPlayer(private val player: PlayerApi) : SearchPlayer {
    override fun play(tracks: List<TrackRow>, contextKind: String, contextId: String) {
        if (tracks.isEmpty()) return
        player.command(
            PlayerCommand.PlayQueue(
                tracks = tracks.map(::ref),
                start = 0u,
                context = PlayContext(kind = contextKind, id = contextId),
            ),
            0uL,
        )
    }

    override fun playEntity(kind: String, id: String) {
        player.command(PlayerCommand.PlayEntity(kind = kind, id = id, shuffle = false), 0uL)
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
 * The core owns the ONE `PlayerApi` (`NzCore::player()` is a `OnceLock` since
 * PR #313), so asking per press is correct and cheap; a cache here would keep an
 * object the core could replace. `:playback` (MB3c) exposes the app's facade over
 * the same object, and nothing on this screen changes the day it does.
 */
object SearchPlayerHolder {
    fun api(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()
}
