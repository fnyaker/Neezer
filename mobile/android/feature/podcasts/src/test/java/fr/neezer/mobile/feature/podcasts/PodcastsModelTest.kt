package fr.neezer.mobile.feature.podcasts

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * « Podcasts » — the subscriptions and the search, as the SPA's
 * `routes/Podcasts.svelte` behaves, with the numbers the acceptance asks for:
 * a warm open paints from the copy with **0 requests**, a dead link keeps it,
 * and a search costs ONE request 300 ms after the last keystroke.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PodcastsModelTest {
    @Test
    fun `a warm open paints the copy before any request`() = runTest {
        val service = FakePodcastService()
        service.cachedShowsAnswer = listOf(show("show-1", "Les Histoires"))
        val model = PodcastsModel(service, this, debounceMs = 300)

        model.start()
        runCurrent()

        val state = model.state.value
        assertFalse(state.loading)
        assertEquals(1, state.shows.size)
        assertEquals(1, service.count("cachedShows"))
        assertEquals("the copy, then the fresh answer: 2 calls", 1, service.count("shows"))
        // The stale flag went true then false: what the screen's note reads.
        assertFalse(state.stale)
    }

    @Test
    fun `a dead link keeps the copy and says it is remembered`() = runTest {
        val service = FakePodcastService()
        service.cachedShowsAnswer = listOf(show("show-1", "Les Histoires"))
        service.failShows = java.io.IOException("le téléphone est hors ligne")
        val model = PodcastsModel(service, this)

        model.start()
        runCurrent()

        val state = model.state.value
        assertTrue("the copy stays on screen", state.shows.isNotEmpty())
        assertTrue("and it is marked as remembered", state.stale)
        assertNull("a kept copy is not a failure", state.failure)
    }

    @Test
    fun `no link and no copy is a cause with a retry, never a spinner`() = runTest {
        val service = FakePodcastService()
        service.failShows = java.io.IOException("le téléphone est hors ligne")
        val model = PodcastsModel(service, this)

        model.start()
        runCurrent()
        val failed = model.state.value
        assertFalse(failed.loading)
        assertNotNull("the cause is named", failed.failure)

        service.failShows = null
        model.retry()
        runCurrent()
        assertNull(model.state.value.failure)
        assertEquals("one click, one attempt", 2, service.count("shows"))
    }

    @Test
    fun `the search waits three hundred milliseconds and asks once`() = runTest {
        val service = FakePodcastService()
        val model = PodcastsModel(service, this, debounceMs = 300)
        model.start()
        runCurrent()
        service.calls.clear()

        model.setQuery("h")
        runCurrent()
        assertEquals("one character asks nothing", 0, service.count("search"))

        model.setQuery("his")
        model.setQuery("hist")
        runCurrent()
        assertEquals("still inside the debounce", 0, service.count("search"))

        advanceTimeBy(301)
        runCurrent()
        assertEquals("one request for the last term", 1, service.count("search"))
        assertEquals(1, service.count("search:hist"))
        assertEquals(1, model.state.value.results.size)
    }

    @Test
    fun `an answer for a term the person typed past is dropped`() = runTest {
        val service = FakePodcastService()
        service.searchDelayAnswer = { listOf(show("deezer-old", "Vieux")) }
        val model = PodcastsModel(service, this, debounceMs = 0)
        model.setQuery("histoire")
        runCurrent()
        // The person types on: the answer that lands describes the old term.
        service.searchDelayAnswer = { listOf(show("deezer-new", "Nouveau")) }
        model.setQuery("histoires")
        runCurrent()

        assertEquals(1, model.state.value.results.size)
        assertEquals("Nouveau", model.state.value.results.first().title)
    }

    @Test
    fun `a catalogue card subscribes by its provider id and opens the show`() = runTest {
        val service = FakePodcastService()
        val model = PodcastsModel(service, this)
        model.start()
        runCurrent()

        model.subscribeCard(show("", "Les Histoires", deezer = "9400001"))
        runCurrent()

        assertEquals(1, service.count("subscribe:https://www.deezer.com/show/9400001"))
        assertEquals("Podcast ajouté", model.state.value.message)
        assertEquals("show-1", model.state.value.openShow)
        assertEquals("", model.state.value.query)
    }

    @Test
    fun `a refused subscribe names the cause and opens nothing`() = runTest {
        val service = FakePodcastService()
        service.failSubscribe = java.io.IOException("refus du serveur")
        val model = PodcastsModel(service, this)
        model.start()
        runCurrent()

        model.subscribeCard(show("", "Les Histoires", deezer = "9400001"))
        runCurrent()

        assertNull(model.state.value.openShow)
        assertNotNull("the cause is what the person reads", model.state.value.message)
        assertNull("nothing is left in flight", model.state.value.subscribing)
    }

    @Test
    fun `a pasted deezer link is submitted and a plain word is not`() = runTest {
        val service = FakePodcastService()
        val model = PodcastsModel(service, this)
        model.start()
        runCurrent()

        model.setQuery("histoires")
        model.submitLink()
        runCurrent()
        assertEquals("a name is searched, not subscribed", 0, service.count("subscribe"))

        model.setQuery("https://www.deezer.com/show/9400001")
        model.submitLink()
        runCurrent()
        assertEquals(1, service.count("subscribe:https://www.deezer.com/show/9400001"))
    }

    @Test
    fun `the search's failure names its cause and can be retried`() = runTest {
        val service = FakePodcastService()
        service.failSearch = java.io.IOException("délai dépassé")
        val model = PodcastsModel(service, this, debounceMs = 0)
        model.setQuery("histoires")
        runCurrent()

        assertNotNull(model.state.value.searchFailure)
        service.failSearch = null
        model.retrySearch()
        runCurrent()
        assertNull(model.state.value.searchFailure)
        assertEquals(1, model.state.value.results.size)
    }
}
