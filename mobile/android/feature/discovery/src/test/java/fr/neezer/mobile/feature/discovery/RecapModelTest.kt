package fr.neezer.mobile.feature.discovery

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Récap's rules: one `/recap/<year>` for the whole deck, a year that is not
 * four digits never reaching the server, the offline sentence instead of a red
 * error, the card the person keeps, and a tap that clamps at both ends.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class RecapModelTest {
    private fun TestScope.harness(
        service: FakeDiscoveryService = FakeDiscoveryService(),
        player: FakePlayPort = FakePlayPort(),
        toasts: Toasts = Toasts(),
    ): Triple<RecapModel, FakeDiscoveryService, FakePlayPort> {
        // UNCONFINED, and that is the point: a model's `launch` (a read, a
        // discovery) runs to completion EAGERLY, so a test never depends on how
        // many dispatch rounds a `channelFlow` needs — measured: the same six
        // `ExplorerModelTest` cases passed on this machine and failed on the CI
        // runner's JDK 17, because `advanceUntilIdle()` returned before a
        // `withContext` continuation had run.
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val model = RecapModel(
            service = service,
            player = player,
            scope = CoroutineScope(dispatcher),
            io = dispatcher,
            toast = toasts::record,
        )
        return Triple(model, service, player)
    }

    /** `#/recap`: the years once, then the year they name — two requests, no more. */
    @Test
    fun theLatestYearIsResolvedThenOpened() = runTest {
        val (model, service, _) = harness()
        model.open(null)
        advanceUntilIdle()
        assertEquals(1, service.yearsReads)
        assertEquals(1, service.deckReads)
        assertEquals(2026, model.state.value.data?.year)
        assertEquals(2, model.state.value.chips.size)
    }

    /** `#/recap/2026`: the kept deck paints, the request replaces it. */
    @Test
    fun aYearOpensOnItsKeptDeckThenTheServers() = runTest {
        val service = FakeDiscoveryService(cachedDeck = mapOf(2026 to DiscoveryFixtures.recapDeck()))
        val (model, _, _) = harness(service)
        model.open(2026)
        advanceUntilIdle()
        assertEquals(2026, model.state.value.data?.year)
        assertEquals("2 sur 9", model.state.value.slides[1].kicker)
        assertFalse(model.state.value.stale)
        assertNull(model.state.value.failure)
    }

    /** A year that is not four digits costs ZERO requests (`YEAR_RE`). */
    @Test
    fun aMalformedYearNeverReachesTheServer() = runTest {
        val (model, service, _) = harness()
        model.open(26)
        advanceUntilIdle()
        assertEquals(0, service.deckReads)
        assertEquals(0, service.yearsReads)
        assertEquals(RecapModel.Failure.Invalid, model.state.value.failure)
    }

    /** A known offline is a state with its own panel, not a cause in red. */
    @Test
    fun aKnownOfflineIsItsOwnPanel() = runTest {
        val service = FakeDiscoveryService(failure = { RuntimeException("Hors ligne") })
        val (model, _, _) = harness(service)
        model.open(2026)
        advanceUntilIdle()
        assertEquals(RecapModel.Failure.Offline, model.state.value.failure)

        service.failure = null
        model.retry()
        advanceUntilIdle()
        assertEquals(2026, model.state.value.data?.year)
    }

    /** A year too thin to tell a story is the empty panel, with its own words. */
    @Test
    fun aThinYearIsTheEmptyPanel() = runTest {
        val service = FakeDiscoveryService(deck = { year -> DiscoveryFixtures.recapDeck(empty = true).copy(year = year) })
        val (model, _, _) = harness(service)
        model.open(2026)
        advanceUntilIdle()
        assertTrue(model.state.value.isEmpty)
        assertEquals("Trop peu d'écoutes en 2026", model.state.value.data?.emptyTitle)
        assertEquals(2025, model.state.value.data?.nearbyYear)

        model.openNearby()
        advanceUntilIdle()
        assertEquals(2025, model.state.value.activeYear)
    }

    /** A tap turns the story and clamps at both ends; a hold pauses it. */
    @Test
    fun aTapTurnsTheCardAndClampsAtBothEnds() = runTest {
        val (model, _, _) = harness()
        model.open(2026)
        advanceUntilIdle()
        assertEquals(0, model.state.value.index)

        model.tap(true)
        assertEquals(1, model.state.value.index)
        assertTrue(model.state.value.slide?.id == "minutes")

        model.tap(false)
        model.tap(false)
        assertEquals("the first card is a wall", 0, model.state.value.index)

        repeat(10) { model.tap(true) }
        assertEquals("the last card is a wall too", 3, model.state.value.index)

        model.hold(true)
        assertTrue(model.state.value.holding)
        model.hold(false)
        assertFalse(model.state.value.holding)

        model.restart()
        assertEquals(0, model.state.value.index)
    }

    /** « Lire ces 5 titres » queues the year's top, and nothing when it has none. */
    @Test
    fun playingTheTopQueuesTheYearsTitles() = runTest {
        val player = FakePlayPort()
        val (model, service, _) = harness(player = player)
        model.open(2026)
        advanceUntilIdle()

        model.playTop()
        advanceUntilIdle()
        assertEquals(1, service.topReads)
        assertEquals(2, player.played.single().rows.size)
        assertEquals(RecapModel.CONTEXT_RECAP, player.played.single().kind)
        assertEquals("2026", player.played.single().id)
    }

    /** Choosing the year already painted answers at once; another one is opened. */
    @Test
    fun choosingThePaintedYearCostsNothing() = runTest {
        val service = FakeDiscoveryService()
        val (model, _, _) = harness(service)
        model.open(2026)
        advanceUntilIdle()
        val reads = service.deckReads

        model.chooseYear(2026)
        advanceUntilIdle()
        assertEquals(reads, service.deckReads)
        assertEquals(2026, model.state.value.activeYear)

        model.chooseYear(2025)
        advanceUntilIdle()
        assertEquals(reads + 1, service.deckReads)
        assertEquals(2025, model.state.value.activeYear)
    }

    /** The share overlay opens and closes; the poster lines are the core's own. */
    @Test
    fun theShareOverlayCarriesTheCoresPoster() = runTest {
        val (model, _, _) = harness()
        model.open(2026)
        advanceUntilIdle()
        model.share(true)
        assertTrue(model.state.value.sharing)
        assertEquals(6, model.state.value.data?.posterLines?.size)
        assertEquals("Ton année 2026, en cours", model.state.value.data?.posterLines?.first())
        model.share(false)
        assertFalse(model.state.value.sharing)
    }
}
