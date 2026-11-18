package fr.neezer.mobile.feature.search

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.SearchFallback
import uniffi.nz_ffi.SearchScope

/**
 * The search's rules, on the JVM: the request counts, the debounce on a VIRTUAL
 * clock, the kept answer, the fallback's beat, the radio and the chip.
 *
 * Every number asserted here is one the PR states: a settled query = 2 requests,
 * « daft punk » typed fast = 2, a warm read = 0 blocking, a tab switch = 0, a
 * launch = 0.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SearchModelTest {

    private class Bench(
        val service: FakeSearchService = FakeSearchService(),
        val player: FakeSearchPlayer = FakeSearchPlayer(),
    ) {
        val toasts = mutableListOf<String>()
        lateinit var model: SearchModel

        fun start(scope: kotlinx.coroutines.CoroutineScope, io: kotlinx.coroutines.CoroutineDispatcher): SearchModel {
            model = SearchModel(
                service = service,
                player = player,
                scope = scope,
                io = io,
                toast = { toasts += it },
            )
            return model
        }
    }

    private fun bench(
        service: FakeSearchService = FakeSearchService(),
        body: suspend kotlinx.coroutines.test.TestScope.(Bench) -> Unit,
    ) = runTest {
        val bench = Bench(service)
        bench.start(this, StandardTestDispatcher(testScheduler))
        body(bench)
    }

    /** « daft punk » typed fast: ONE debounce, so ONE search and ONE fallback. */
    @Test
    fun typingDaftPunkFastSendsTwoRequests() = bench { bench ->
        "daft punk".forEachIndexed { index, _ ->
            bench.model.onQuery("daft punk".take(index + 1))
        }
        // Nothing was asked while the keys were landing.
        assertEquals(0, bench.service.searches)
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        assertEquals("one read for the whole word", 1, bench.service.searches)
        assertEquals("daft punk", bench.service.terms.single())
        assertEquals(0, bench.service.fallbacks)
        // The fallback has its own, longer beat.
        advanceTimeBy(SearchModel.YOUTUBE_MS + 1)
        runCurrent()
        assertEquals(1, bench.service.fallbacks)
        assertEquals("2 requests for the settled query", 2, bench.service.searches + bench.service.fallbacks)
    }

    /** A new key CANCELS the read in flight: one query at a time, never two. */
    @Test
    fun aNewKeyCancelsTheReadInFlight() = bench { bench ->
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        bench.service.gate = gate
        bench.model.onQuery("da")
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        assertEquals(1, bench.service.searches)
        bench.model.onQuery("daf")
        runCurrent()
        assertEquals("the first read was dropped", 1, bench.service.cancelled)
        gate.complete(Unit)
    }

    /** Two characters is the floor: one letter names nothing and asks nothing. */
    @Test
    fun oneLetterSearchesNothing() = bench { bench ->
        bench.model.onQuery("d")
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        assertEquals(0, bench.service.searches)
        bench.model.onQuery("da")
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        assertEquals(1, bench.service.searches)
    }

    /** A pasted playlist link is an IMPORT: no query, no request. */
    @Test
    fun aPastedPlaylistLinkIsAnImportAndCostsNothing() = bench { bench ->
        bench.service.offer = SearchFixtures.linkOffer
        bench.model.onQuery(SearchFixtures.linkOffer.url)
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        assertEquals(0, bench.service.searches)
        assertEquals(SearchFixtures.linkOffer, bench.model.state.value.importOffer)
        assertEquals(
            "the page is the offer and nothing else",
            listOf(SearchBlock.Header),
            searchBlocks(bench.model.state.value, 10),
        )
    }

    /** The kept copy paints first (stale), the network replaces it (fresh). */
    @Test
    fun theKeptAnswerPaintsThenTheNetworkReplacesIt() = bench { bench ->
        bench.service.kept["daft punk"] = emptyAnswer("daft punk")
        bench.model.onQuery("daft punk")
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        assertEquals(1, bench.service.cacheReads)
        assertEquals(1, bench.service.searches)
        val state = bench.model.state.value
        assertNotNull(state.answer)
        assertFalse("the fresh answer is what is drawn", state.stale)
        assertEquals("the fallback has not answered yet", 0, state.answer!!.youtube)
    }

    /** A refresh that fails keeps what was painted, flagged stale (rule 3). */
    @Test
    fun aFailedRefreshKeepsTheCopy() = bench { bench ->
        bench.service.kept["daft punk"] = emptyAnswer("daft punk")
        bench.service.searchFailure = IllegalStateException("Hors ligne")
        bench.model.onQuery("daft punk")
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        val state = bench.model.state.value
        assertTrue("the copy stays", state.stale)
        assertNotNull(state.answer)
        assertNull("and it is not a failure", state.failure)
    }

    /** Nothing kept and a failure: the cause and « Réessayer ». */
    @Test
    fun nothingToShowIsACauseWithRetry() = bench { bench ->
        bench.service.searchFailure = IllegalStateException("Le serveur ne répond pas")
        bench.model.onQuery("daft punk")
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        assertEquals("Le serveur ne répond pas", bench.model.state.value.failure)
        bench.service.searchFailure = null
        bench.model.retry()
        advanceUntilIdle()
        assertNull(bench.model.state.value.failure)
        assertEquals(2, bench.service.searches)
        assertNotNull(bench.model.state.value.answer)
    }

    /** The fallback joins the SAME list after its own beat, never before. */
    @Test
    fun theFallbackJoinsTheSameListAfterItsBeat() = bench { bench ->
        bench.model.onQuery("daft punk")
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        assertEquals(0, bench.service.fallbacks)
        advanceTimeBy(SearchModel.YOUTUBE_MS + 1)
        runCurrent()
        assertEquals(1, bench.service.fallbacks)
        val state = bench.model.state.value
        assertEquals(1, state.answer?.youtube)
        assertEquals(
            SearchModel.YoutubeUi.Added(1),
            state.youtube,
        )
    }

    /** A refused fallback keeps the list and says why, with its own retry. */
    @Test
    fun aRefusedFallbackKeepsTheList() = bench { bench ->
        bench.service.woven = failedFallback()
        bench.model.onQuery("daft punk")
        advanceTimeBy(SearchModel.DEBOUNCE_MS + 1)
        runCurrent()
        advanceTimeBy(SearchModel.YOUTUBE_MS + 1)
        runCurrent()
        val state = bench.model.state.value
        assertNotNull("the catalogue's list is untouched", state.answer)
        assertEquals(
            SearchModel.YoutubeUi.Failed(SearchFallback.UNAVAILABLE),
            state.youtube,
        )
        bench.model.retryFallback()
        advanceUntilIdle()
        assertEquals("Réessayer asks once more", 2, bench.service.fallbacks)
    }

    /** Enter asks at once: the list and the fallback, no beat. */
    @Test
    fun enterAsksTheFallbackAtOnce() = bench { bench ->
        bench.model.onQuery("daft punk")
        bench.model.submit()
        advanceUntilIdle()
        assertEquals(1, bench.service.searches)
        assertEquals("Enter skips the fallback's beat", 1, bench.service.fallbacks)
        assertEquals("and the term is remembered", listOf("daft punk"), bench.service.recents)
    }

    /** Enter on a term the list already answers asks nothing more. */
    @Test
    fun enterOnASettledListAsksNothingMore() = bench { bench ->
        bench.model.onQuery("daft punk")
        advanceUntilIdle()
        bench.service.forget()
        bench.model.submit()
        advanceUntilIdle()
        assertEquals(0, bench.service.searches)
        assertEquals("the fallback had already answered", 0, bench.service.fallbacks)
        assertEquals(listOf("daft punk"), bench.service.recents)
    }

    /** A tab switch costs nothing: the answer already holds every family. */
    @Test
    fun aTabSwitchCostsNothing() = bench { bench ->
        bench.model.onQuery("daft punk")
        advanceUntilIdle()
        val searched = bench.service.searches
        val fell = bench.service.fallbacks
        bench.model.chooseTab(SearchTab.Tracks)
        bench.model.chooseTab(SearchTab.Albums)
        advanceUntilIdle()
        assertEquals(searched, bench.service.searches)
        assertEquals(fell, bench.service.fallbacks)
        assertEquals(SearchTab.Albums, bench.model.state.value.tab)
    }

    /** A source switch is ONE request of its own, on its own route. */
    @Test
    fun aScopeSwitchIsOneRequest() = bench { bench ->
        bench.model.onQuery("memoire")
        advanceUntilIdle()
        bench.service.forget()
        bench.model.chooseScope(SearchScope.PODCASTS)
        advanceUntilIdle()
        assertEquals(1, bench.service.searches)
        assertEquals(SearchScope.PODCASTS, bench.service.scopes.last())
        assertEquals(1, bench.model.state.value.answer?.transcripts?.size)
        assertEquals("no fallback for the podcasts", 0, bench.service.fallbacks)
    }

    /** An older server hides the transcripts instead of failing every visit. */
    @Test
    fun anOlderServerHidesTheTranscriptScope() = bench { bench ->
        // The library works; the transcripts route is the one the server lacks.
        bench.model.onQuery("memoire")
        advanceUntilIdle()
        bench.service.searchFailure = unsupported()
        bench.model.chooseScope(SearchScope.PODCASTS)
        advanceUntilIdle()
        val state = bench.model.state.value
        assertFalse(state.podcastsAvailable)
        assertEquals(SearchScope.LIBRARY, state.scope)
        assertTrue(
            "and it says so: ${bench.toasts}",
            bench.toasts.any { it.contains("transcriptions") },
        )
    }

    /** « Lancer la radio » plays from the screen's answer and asks nothing. */
    @Test
    fun theRadioPlaysFromTheScreenAndAsksNothing() = bench { bench ->
        bench.model.onQuery("patate")
        advanceUntilIdle()
        bench.model.launchRadio()
        advanceUntilIdle()
        val press = bench.player.presses.single()
        assertEquals(6, press.tracks.size)
        assertEquals("radio", press.kind)
        assertEquals("track:id-0", press.id)
        assertEquals("the core call is the plan, not a request", 1, bench.service.radioCalls)
    }

    /** A radio with nothing to play says so, and plays nothing. */
    @Test
    fun aRadioWithNothingToPlaySaysSo() = bench { bench ->
        bench.service.plan = SearchFixtures.emptyPlan()
        bench.model.onQuery("patate")
        advanceUntilIdle()
        bench.model.launchRadio()
        advanceUntilIdle()
        assertTrue(bench.player.presses.isEmpty())
        assertTrue(
            "the SPA's own sentence: ${bench.toasts}",
            bench.toasts.any { it.contains("n'a rien à jouer") },
        )
    }

    /** A second press in the same burst is one launch. */
    @Test
    fun twoPressesOfTheRadioAreOneLaunch() = bench { bench ->
        bench.model.onQuery("patate")
        advanceUntilIdle()
        bench.model.launchRadio()
        bench.model.launchRadio()
        advanceUntilIdle()
        assertEquals(1, bench.service.radioCalls)
        assertEquals(1, bench.player.presses.size)
    }

    /** The YouTube chip takes the video out of the list, and costs nothing. */
    @Test
    fun theYoutubeChipTakesTheVideoOutWithoutARequest() = bench { bench ->
        bench.model.onQuery("daft punk")
        advanceUntilIdle()
        val searches = bench.service.searches
        // What the device kept for the chip's "off" read: the catalogue's list.
        bench.service.kept["daft punk"] = emptyAnswer("daft punk")
        bench.model.toggleYoutube()
        advanceUntilIdle()
        assertFalse(bench.model.state.value.youtubeOn)
        assertEquals("no request", searches, bench.service.searches)
        assertEquals(0, bench.model.state.value.answer?.youtube)
    }

    /** The device's own terms: read at start, used, cleared. */
    @Test
    fun theRecentTermsComeFromTheDevice() = bench { bench ->
        bench.service.recents = mutableListOf("Air", "Daft Punk")
        bench.model.start(null)
        advanceUntilIdle()
        assertEquals(listOf("Air", "Daft Punk"), bench.model.state.value.recent)
        bench.model.useRecent("Air")
        advanceUntilIdle()
        assertEquals(1, bench.service.searches)
        assertEquals("Air", bench.service.terms.single())
        bench.model.clearRecent()
        assertTrue(bench.model.state.value.recent.isEmpty())
        assertTrue(bench.service.recents.isEmpty())
    }

    /** Leaving remembers what was really searched, and only that. */
    @Test
    fun leavingRemembersTheSearchedTerm() = bench { bench ->
        bench.model.onQuery("daft punk")
        advanceUntilIdle()
        bench.model.leave()
        assertEquals(listOf("daft punk"), bench.service.recents)

        // A word the box still holds but nobody searched is not remembered
        // (`Search.svelte#onDestroy`: `searchedFor === q`).
        bench.service.recents.clear()
        bench.model.onQuery("ka")
        bench.model.leave()
        assertTrue(bench.service.recents.isEmpty())
    }

    /** A deep link (or the shell's `q`) searches its term at once. */
    @Test
    fun aDeepLinkSearchesItsTerm() = bench { bench ->
        bench.model.start("daft punk")
        advanceUntilIdle()
        assertEquals("daft punk", bench.service.terms.single())
        assertEquals("daft punk", bench.model.state.value.query)
    }

    /** A refused launch is a toast, never a dead button. */
    @Test
    fun aRefusedLaunchIsAToast() = bench { bench ->
        bench.service.plan = SearchFixtures.emptyPlan()
        bench.model.onQuery("patate")
        advanceUntilIdle()
        bench.model.launchRadio()
        advanceUntilIdle()
        assertFalse(bench.model.state.value.radioBusy)
        assertTrue(bench.toasts.isNotEmpty())
    }
}
