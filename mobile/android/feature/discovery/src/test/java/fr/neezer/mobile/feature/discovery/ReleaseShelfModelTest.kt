package fr.neezer.mobile.feature.discovery

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.SortKey

/**
 * The shelf page's rules: a card click costs no request, a deep link asks the
 * SPA's own two, the search folds accents, and the sort and the arrow reorder the
 * window the person is reading.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReleaseShelfModelTest {
    private fun TestScope.harness(
        kind: String = "artists",
        service: FakeDiscoveryService = FakeDiscoveryService(),
        player: FakePlayPort = FakePlayPort(),
    ): Pair<ReleaseShelfModel, FakeDiscoveryService> {
        val dispatcher = StandardTestDispatcher(testScheduler)
        val model = ReleaseShelfModel(
            kind = kind,
            service = service,
            player = player,
            scope = CoroutineScope(dispatcher),
            io = dispatcher,
        )
        return model to service
    }

    /** A card click: the kept page paints at once, and the refresh is the feed's. */
    @Test
    fun aCardClickPaintsTheKeptPageAndRefreshesWithTheFeed() = runTest {
        val (model, service) = harness()
        model.load()
        advanceUntilIdle()
        assertEquals("the kept page, then the fresh one", 2, service.shelfReads)
        assertEquals("a deep link asks the feed, the SPA's own request", 1, service.feedReads)
        assertEquals("3 titres · 2 sorties", model.state.value.page?.facts)
        assertEquals(3, model.state.value.rowCount)
        assertFalse(model.state.value.stale)
    }

    /** A page that fails with nothing kept is the cause; the retry asks again. */
    @Test
    fun aPageThatFailsWithNothingKeptSaysWhy() = runTest {
        var attempts = 0
        val service = FakeDiscoveryService(
            page = null,
            feed = {
                attempts++
                if (attempts < 2) throw RuntimeException("Le serveur ne répond pas") else DiscoveryFixtures.releases()
            },
        )
        val (model, _) = harness(service = service)
        model.load()
        advanceUntilIdle()
        val failed = model.state.value.screen as fr.neezer.mobile.bridge.ScreenState.Failed
        assertEquals("Le serveur ne répond pas", failed.cause)

        service.page = DiscoveryFixtures.shelfPage()
        model.retry()
        advanceUntilIdle()
        assertTrue(model.state.value.screen is fr.neezer.mobile.bridge.ScreenState.Ready)
    }

    /** The search folds: "voisin" finds "Voisin", and nothing else is left. */
    @Test
    fun theSearchFoldsAccentsAndNarrowsTheWindow() = runTest {
        val (model, _) = harness()
        model.load()
        advanceUntilIdle()

        model.search("voisin")
        advanceUntilIdle()
        assertEquals(1, model.state.value.rowCount)
        assertEquals("Voisin", model.state.value.rows.single().title)
        assertTrue(model.state.value.searching)

        model.search("")
        advanceUntilIdle()
        assertEquals(3, model.state.value.rowCount)
        assertFalse(model.state.value.searching)
    }

    /** The sort and the arrow reorder the same rows; the duration sort is the scale. */
    @Test
    fun theSortAndTheReverseReorderTheRows() = runTest {
        val (model, _) = harness()
        model.load()
        advanceUntilIdle()

        model.sort("title")
        advanceUntilIdle()
        assertEquals(
            listOf("Ouverture", "Suite", "Voisin"),
            model.state.value.rows.map { it.title },
        )

        model.reverse()
        advanceUntilIdle()
        assertEquals(
            listOf("Voisin", "Suite", "Ouverture"),
            model.state.value.rows.map { it.title },
        )
        assertEquals(ReleaseShelfModel.sortKey("duration"), SortKey.DURATION)
        assertEquals(ReleaseShelfModel.sortKey("inconnu"), SortKey.POSITION)
    }

    /** « Lire » queues what is on screen, from the first; a row starts at its own index. */
    @Test
    fun playingQueuesTheWindowAtTheRightRow() = runTest {
        val player = FakePlayPort()
        val (model, _) = harness(player = player)
        model.load()
        advanceUntilIdle()

        model.playAll()
        assertEquals(3, player.played[0].rows.size)
        assertEquals("artists", player.played[0].id)

        model.playRow(model.state.value.rows[1])
        assertEquals(1, player.played[1].start)
        assertEquals(3, player.played[1].rows.size)

        model.queueNext(model.state.value.rows[0])
        assertEquals(1, player.queued.size)
    }

    /** An unknown shelf is the first one, never a blank page (`ReleaseShelf.svelte#key`). */
    @Test
    fun anUnknownShelfFallsBackToTheFirst() {
        assertEquals("artists", shelfKindOrFirst("favorites"))
        assertEquals("artists", shelfKindOrFirst("artists"))
        assertEquals("discovery", shelfKindOrFirst("discovery"))
    }
}
