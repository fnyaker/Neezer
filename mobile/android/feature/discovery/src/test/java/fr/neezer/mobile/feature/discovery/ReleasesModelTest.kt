package fr.neezer.mobile.feature.discovery

import fr.neezer.mobile.bridge.ScreenState
import kotlinx.coroutines.Dispatchers
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
 * Nouveautés' rules, on the JVM: the cache-first read, the failure that keeps
 * what was painted, the badge posted once, the list that opens by itself and the
 * commands a tap sends. No `.so`, no device — and nothing waits on a clock
 * (`runTest`'s virtual time, opened by `advanceUntilIdle`).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReleasesModelTest {
    private fun TestScope.harness(
        service: FakeDiscoveryService = FakeDiscoveryService(),
        player: FakePlayPort = FakePlayPort(),
        toasts: Toasts = Toasts(),
    ): Triple<ReleasesModel, FakeDiscoveryService, FakePlayPort> {
        // UNCONFINED, and that is the point: a model's `launch` (a read, a
        // discovery) runs to completion EAGERLY, so a test never depends on how
        // many dispatch rounds a `channelFlow` needs — measured: the same six
        // `ExplorerModelTest` cases passed on this machine and failed on the CI
        // runner's JDK 17, because `advanceUntilIdle()` returned before a
        // `withContext` continuation had run.
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val model = ReleasesModel(
            service = service,
            player = player,
            scope = CoroutineScope(dispatcher),
            io = dispatcher,
            toast = toasts::record,
        )
        return Triple(model, service, player)
    }

    /** The device's feed paints, then the server's replaces it: one screen open. */
    @Test
    fun theKeptFeedPaintsFirstThenTheServers() = runTest {
        val (model, service, _) = harness()
        model.load()
        advanceUntilIdle()
        assertEquals(1, service.cachedFeedReads)
        assertEquals(1, service.feedReads)
        val state = model.state.value
        assertEquals(3u, state.data?.count)
        assertEquals(2u, state.data?.unseen)
        assertEquals(2, state.shelves.size)
        assertFalse(state.stale)
    }

    /** A dead link keeps what was painted and says it is the kept copy. */
    @Test
    fun aDeadLinkKeepsTheKeptFeedAndFlagsIt() = runTest {
        val service = FakeDiscoveryService(
            failure = { RuntimeException("Hors ligne") },
        )
        val (model, _, _) = harness(service)
        model.load()
        advanceUntilIdle()
        val state = model.state.value
        assertTrue("the kept copy is what the person sees", state.stale)
        assertEquals(3u, state.data?.count)
    }

    /** With nothing kept, the cause and « Réessayer » — and the retry asks again. */
    @Test
    fun aFailureWithNothingKeptIsTheCauseAndARetry() = runTest {
        var attempts = 0
        val service = FakeDiscoveryService(
            cachedFeed = null,
            feed = {
                attempts++
                if (attempts < 2) throw RuntimeException("Le serveur ne répond pas") else DiscoveryFixtures.releases()
            },
        )
        val (model, _, _) = harness(service)
        model.load()
        advanceUntilIdle()
        val failed = model.state.value.screen as ScreenState.Failed
        assertEquals("Le serveur ne répond pas", failed.cause)

        model.retry()
        advanceUntilIdle()
        assertEquals(3u, model.state.value.data?.count)
    }

    /**
     * A feed whose shelves hold nothing opens the per-release list by itself:
     * never a collapsed heading over an empty page (`Releases.svelte`'s own
     * `showAll` rule).
     */
    @Test
    fun aFeedWithNoShelfOpensTheListByItself() = runTest {
        val service = FakeDiscoveryService(
            cachedFeed = DiscoveryFixtures.releasesWithoutShelves(),
            feed = { DiscoveryFixtures.releasesWithoutShelves() },
        )
        val (model, _, _) = harness(service)
        model.load()
        advanceUntilIdle()
        assertTrue("the list is the only thing worth showing", model.state.value.showAll)
    }

    /** A group's titles are read from the kept feed ONCE, then cached in state. */
    @Test
    fun aGroupOpenedTwiceReadsItsRowsOnce() = runTest {
        val (model, service, _) = harness()
        model.load()
        advanceUntilIdle()
        val rowsBefore = service.groupReads

        model.toggleGroup("501")
        advanceUntilIdle()
        assertEquals(rowsBefore + 1, service.groupReads)
        assertEquals(2, model.state.value.groupRows["501"]?.size)

        model.toggleGroup("501")
        advanceUntilIdle()
        model.toggleGroup("501")
        advanceUntilIdle()
        assertEquals("the rows stay in memory", rowsBefore + 1, service.groupReads)
    }

    /** « Tout lire » queues the whole feed, and a group's « Lire » its own titles. */
    @Test
    fun playingQueuesTheFeedOrOneRelease() = runTest {
        val (model, _, player) = harness()
        model.load()
        advanceUntilIdle()

        model.playAll()
        advanceUntilIdle()
        assertEquals(1, player.played.size)
        assertEquals(3, player.played[0].rows.size)
        assertEquals(ReleasesModel.CONTEXT_RELEASES, player.played[0].kind)

        val group = model.state.value.groups.first()
        model.playGroup(group)
        advanceUntilIdle()
        assertEquals(2, player.played.size)
        assertEquals(group.release.key, player.played[1].id)
        assertEquals(ReleasesModel.CONTEXT_ALBUM, player.played[1].kind)

        model.playGroupAt(group, 1)
        advanceUntilIdle()
        assertEquals(1, player.played[2].start)
    }

    /** Opening a shelf posts the badge ONCE, whatever the person does after. */
    @Test
    fun openingAShelfMarksTheFeedSeenOnce() = runTest {
        val (model, service, _) = harness()
        model.load()
        advanceUntilIdle()
        model.seenOnShelf()
        model.seenOnShelf()
        advanceUntilIdle()
        assertEquals(1, service.seenPosts)
    }

    /** An empty feed is the page's own "Rien de neuf", not a failure. */
    @Test
    fun anEmptyFeedIsNotAFailure() = runTest {
        val service = FakeDiscoveryService(
            cachedFeed = DiscoveryFixtures.emptyReleases(),
            feed = { DiscoveryFixtures.emptyReleases() },
        )
        val (model, _, _) = harness(service)
        model.load()
        advanceUntilIdle()
        assertEquals(true, model.state.value.data?.empty)
        assertNull(model.state.value.screen as? ScreenState.Failed)
    }
}
