package fr.neezer.mobile.feature.search

import fr.neezer.mobile.designsystem.NzRowSource
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CancellationException
import uniffi.nz_ffi.ImportOffer
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.RadioPlan
import uniffi.nz_ffi.SearchFallback
import uniffi.nz_ffi.SearchScope
import uniffi.nz_ffi.TrackRow

/**
 * The two seams, as a test sees them: every call counted, and a failure or a
 * hold a test can ask for.
 *
 * The counts ARE the acceptance (requests, not durations): a settled query is
 * `searches == 1` and `fallbacks == 1`, « daft punk » typed fast is the same two,
 * a warm read is `cacheReads == 1` and `searches == 0`.
 */
class FakeSearchService(
    /** What `cachedSearch` answers, by term (a `null` is "nothing kept"). */
    val kept: MutableMap<String, SearchAnswer> = mutableMapOf(),
    /** What `search` answers, by term; the default is a library answer. */
    val fresh: MutableMap<String, SearchAnswer> = mutableMapOf(),
    /** What `youtubeHits` answers. */
    var woven: SearchAnswer = SearchFixtures.answer(trackCount = 5, youtubeAt = 4),
    var plan: RadioPlan = SearchFixtures.plan(),
) : SearchService {
    var searches = 0
    var fallbacks = 0
    var cacheReads = 0
    var radioCalls = 0
    var cancelled = 0

    /** Zeroes the counters, so a second measure is not the sum of both. */
    fun forget() {
        searches = 0
        fallbacks = 0
        cacheReads = 0
        radioCalls = 0
        cancelled = 0
    }

    /** The terms searched, in order. */
    val terms = mutableListOf<String>()
    val scopes = mutableListOf<SearchScope>()
    val fallbackFlags = mutableListOf<Boolean>()

    /** A failure the next `search` throws. */
    var searchFailure: Throwable? = null

    /** A failure the next `youtubeHits` throws. */
    var fallbackFailure: Throwable? = null

    /** Held when a test wants a read IN FLIGHT (the cancellation proof). */
    var gate: CompletableDeferred<Unit>? = null

    var recents: MutableList<String> = mutableListOf()
    var admin = false
    var youtube = true
    var offer: ImportOffer? = null

    override suspend fun cachedSearch(
        term: String,
        scope: SearchScope,
        fallback: Boolean,
    ): SearchAnswer? {
        cacheReads += 1
        val answer = kept[term] ?: return null
        return if (fallback) answer else answer.copy(youtube = 0, youtubeAnswered = false)
    }

    override suspend fun search(
        term: String,
        scope: SearchScope,
        fallback: Boolean,
    ): SearchAnswer {
        searches += 1
        terms += term
        scopes += scope
        fallbackFlags += fallback
        try {
            gate?.await()
        } catch (e: CancellationException) {
            cancelled += 1
            throw e
        }
        searchFailure?.let { throw it }
        return fresh[term] ?: when (scope) {
            SearchScope.LIBRARY -> SearchFixtures.answer(term = term)
            SearchScope.PODCASTS -> SearchFixtures.podcastsAnswer(term)
            SearchScope.SHOWS -> SearchFixtures.showsAnswer(term)
        }
    }

    override suspend fun youtubeHits(term: String): SearchAnswer {
        fallbacks += 1
        fallbackFailure?.let { throw it }
        return woven.copy(term = term)
    }

    override suspend fun searchAsRadio(term: String): RadioPlan {
        radioCalls += 1
        return plan
    }

    override fun recentSearches(): List<String> = recents.toList()

    override fun noteSearch(term: String) {
        recents.removeAll { it.equals(term, ignoreCase = true) }
        recents.add(0, term.trim())
        while (recents.size > SearchModel.RECENT_MAX) recents.removeAt(recents.lastIndex)
    }

    override fun clearRecentSearches() = recents.clear()

    override fun youtubeAvailable(): Boolean = youtube

    override fun isAdmin(): Boolean = admin

    override fun importOffer(text: String): ImportOffer? =
        offer?.takeIf { text.contains(it.url) }

    override val minTerm: Int = 2

    override val topTracks: Int = 10

    /** The library answer the fake hands out for [term], for a test's own asserts. */
    fun answerFor(term: String): SearchAnswer =
        fresh[term] ?: SearchFixtures.answer(term = term)
}

/** The player, as a test sees it: what a press would have sent. */
class FakeSearchPlayer : SearchPlayer {
    data class Press(val tracks: List<TrackRow>, val kind: String, val id: String)

    val presses = mutableListOf<Press>()
    val entities = mutableListOf<Pair<String, String>>()

    override fun play(tracks: List<TrackRow>, contextKind: String, contextId: String) {
        presses += Press(tracks, contextKind, contextId)
    }

    override fun playEntity(kind: String, id: String) {
        entities += kind to id
    }
}

/** An answer with nothing in it, for the empty states. */
fun emptyAnswer(term: String, scope: SearchScope = SearchScope.LIBRARY): SearchAnswer =
    SearchFixtures.answer(
        term = term,
        scope = scope,
        trackCount = 0,
        artists = emptyList(),
        albums = emptyList(),
        playlists = emptyList(),
        top = null,
    )

/** `NzException.Unsupported`, the server being older than a route (rule 7). */
fun unsupported(feature: String = "transcription"): NzException =
    NzException.Unsupported(feature)

/** The row source of an answer, for a test that reads what the screen would. */
fun rowsOf(answer: SearchAnswer): NzRowSource = answer.tracks

/** A fallback's own refusal, as the core hands it over. */
fun failedFallback(): SearchAnswer =
    SearchFixtures.answer(youtubeFailure = SearchFallback.UNAVAILABLE)
