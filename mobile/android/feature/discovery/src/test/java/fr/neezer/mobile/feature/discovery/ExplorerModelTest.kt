package fr.neezer.mobile.feature.discovery

import fr.neezer.mobile.bridge.ScreenState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Explorer's rules: the kept tiles paint first, one POST starts a discovery and
 * the polls never start a second, a busy server is a NAMED WAIT with its own
 * retry, a station is one request that plays at once, and the all-stations list
 * is one `/flow/styles` however many tiles the shelf has.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ExplorerModelTest {
    private fun TestScope.harness(
        service: FakeDiscoveryService = FakeDiscoveryService(),
        player: FakePlayPort = FakePlayPort(),
        toasts: Toasts = Toasts(),
    ): Triple<ExplorerModel, FakeDiscoveryService, FakePlayPort> {
        // UNCONFINED, and that is the point: a model's `launch` (a read, a
        // discovery) runs to completion EAGERLY, so a test never depends on how
        // many dispatch rounds a `channelFlow` needs — measured: the same six
        // `ExplorerModelTest` cases passed on this machine and failed on the CI
        // runner's JDK 17, because `advanceUntilIdle()` returned before a
        // `withContext` continuation had run.
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val model = ExplorerModel(
            service = service,
            player = player,
            scope = CoroutineScope(dispatcher),
            // `Dispatchers.Unconfined`, NOT the test dispatcher: the model's
            // `io` seam exists so the app keeps FFI calls off the main thread,
            // and a `withContext` onto a TEST dispatcher is a task only
            // `advanceUntilIdle` can run — measured on the CI runner (JDK 17),
            // where the same six cases queued the continuation on a scheduler
            // the test never advanced and the panel stayed `pending` for five
            // seconds. Unconfined runs the block inline, on every machine.
            io = Dispatchers.Unconfined,
            toast = toasts::record,
        )
        return Triple(model, service, player)
    }

    /** The kept styles paint, then one `/explorer/styles` replaces them. */
    @Test
    fun theKeptStylesPaintThenTheServers() = runTest {
        val (model, service, _) = harness()
        model.load()
        assertTrue("the tiles never painted: ${model.state.value}", settle("the tiles") { model.state.value.tiles.isNotEmpty() })
        assertEquals(1, service.stylesReads)
        assertEquals(4, model.state.value.tiles.size)
        assertEquals(5, model.state.value.levels.size)
        assertEquals(60, model.state.value.maxPolls)
        assertTrue(model.state.value.screen is ScreenState.Ready)
    }

    /** A style with no taste is the empty sentence, never a spinner. */
    @Test
    fun anAccountWithNoHistorySaysSo() = runTest {
        val empty = DiscoveryFixtures.explorerStyles().copy(tiles = emptyList(), source = "none")
        val service = FakeDiscoveryService(styles = empty, cachedStyles = empty)
        val (model, _, _) = harness(service)
        model.load()
        advanceUntilIdle()
        settle("the answer") { model.state.value.screen is ScreenState.Ready }
        assertTrue(model.state.value.tiles.isEmpty())
        assertEquals("none", model.state.value.data?.source)
    }

    /**
     * ONE POST starts a discovery, and each poll is ONE GET: a poll never starts
     * a second sweep, and the panel paints what the server found.
     */
    @Test
    fun aDiscoveryIsOnePostThenOneGetPerPoll() = runTest {
        val (model, service, _) = harness()
        model.load()
        assertTrue("the tiles never painted: ${model.state.value}", settle("the tiles") { model.state.value.tiles.isNotEmpty() })
        model.chooseStyle(model.state.value.tiles.first())
        assertEquals(ExplorerModel.Phase.Idle, model.state.value.phase)

        model.discover()
        assertTrue("the discovery never answered: ${model.state.value}", settle("the discovery") { model.state.value.phase != ExplorerModel.Phase.Pending || service.discoverPosts > 0 })
        assertEquals(1, service.discoverPosts)
        assertEquals(ExplorerModel.Phase.Pending, model.state.value.phase)

        model.poll()
        assertTrue("the poll never ran: ${model.state.value}", settle("the poll") { service.pollReads > 0 })
        settle("the answer") { model.state.value.phase == ExplorerModel.Phase.Ready }
        assertEquals("a poll starts no sweep", 1, service.discoverPosts)
        assertEquals(ExplorerModel.Phase.Ready, model.state.value.phase)
        assertEquals(2, model.state.value.playlists.size)
        assertEquals("12 titres vérifiés sur 60", model.state.value.playlists[0].countLabel)
    }

    /** A busy server: a named wait, a countdown, and the retry the mode names. */
    @Test
    fun aBusyServerIsANamedWait() = runTest {
        val service = FakeDiscoveryService(
            discoverAnswers = mutableListOf(DiscoveryFixtures.busy(503, 5)),
            pollAnswers = mutableListOf(DiscoveryFixtures.discover("ready")),
        )
        val (model, _, _) = harness(service)
        model.load()
        assertTrue("the tiles never painted: ${model.state.value}", settle("the tiles") { model.state.value.tiles.isNotEmpty() })
        model.chooseStyle(model.state.value.tiles.first())
        model.discover()
        assertTrue("no busy answer: ${model.state.value}", settle("busy") { model.state.value.phase == ExplorerModel.Phase.Busy })

        val state = model.state.value
        assertEquals("Le serveur est occupé.", state.cause)
        assertEquals(5, state.retryLeft)
        assertEquals("Réessaie dans 5 s.", state.busyWait)

        model.tick()
        assertEquals(4, model.state.value.retryLeft)

        // A refused POST left nothing running: « Réessayer » POSTs again.
        assertEquals(ExplorerModel.RETRY_POST, model.state.value.retryMode)
        model.retryDiscover()
        advanceUntilIdle()
        assertEquals(2, service.discoverPosts)
    }

    /** A refused poll keeps the job running: its retry polls, and never POSTs. */
    @Test
    fun aRefusedPollRetriesThePollNotTheSweep() = runTest {
        val service = FakeDiscoveryService(
            discoverAnswers = mutableListOf(DiscoveryFixtures.discover("pending")),
        )
        service.failure = null
        val (model, _, _) = harness(service)
        model.load()
        assertTrue("the tiles never painted: ${model.state.value}", settle("the tiles") { model.state.value.tiles.isNotEmpty() })
        model.chooseStyle(model.state.value.tiles.first())
        model.discover()
        assertTrue("no pending sweep: ${model.state.value}", settle("pending") { model.state.value.phase == ExplorerModel.Phase.Pending })

        service.failure = { RuntimeException("Le serveur ne répond pas") }
        model.poll()
        assertTrue("the poll never failed: ${model.state.value}", settle("the refusal") { model.state.value.phase == ExplorerModel.Phase.Error })
        assertEquals(ExplorerModel.RETRY_POLL, model.state.value.retryMode)
        val posts = service.discoverPosts

        service.failure = null
        model.retryDiscover()
        assertTrue("the retry never answered: ${model.state.value}", settle("the retry") { model.state.value.phase == ExplorerModel.Phase.Ready })
        assertEquals("the sweep already runs: no second POST", posts, service.discoverPosts)
    }

    /** « Essayer un niveau plus large » moves one notch, then discovers. */
    @Test
    fun wideningMovesOneNotchAndDiscovers() = runTest {
        val service = FakeDiscoveryService(
            discoverAnswers = mutableListOf(
                DiscoveryFixtures.discover("ready").copy(playlists = emptyList()),
                DiscoveryFixtures.discover("pending"),
            ),
        )
        val (model, _, _) = harness(service)
        model.load()
        assertTrue("the tiles never painted: ${model.state.value}", settle("the tiles") { model.state.value.tiles.isNotEmpty() })
        model.chooseStyle(model.state.value.tiles.first())
        model.discover()
        assertTrue("an empty sweep was expected: ${model.state.value}", settle("the empty answer") { model.state.value.phase == ExplorerModel.Phase.Empty })

        model.widen()
        assertTrue("the wider sweep never started: ${model.state.value}", settle("the wider answer") { model.state.value.level == 4 && service.discoverPosts == 2 })
        assertEquals(4, model.state.value.level)
        assertEquals(2, service.discoverPosts)
        assertEquals("Le style, ses voisins et des passerelles.", model.state.value.aim?.hint)
    }

    /** A tile's disc: ONE request, then the queue plays at once. */
    @Test
    fun aStationIsOneRequestAndPlaysAtOnce() = runTest {
        val player = FakePlayPort()
        val (model, service, _) = harness(player = player)
        model.load()
        advanceUntilIdle()

        model.startStation("Techno")
        advanceUntilIdle()
        assertEquals(1, service.stationPosts)
        assertEquals("genre-radio", player.played.single().kind)
        assertEquals("Techno", player.played.single().id)
        assertEquals(1, player.played.single().rows.size)
        assertNull(model.state.value.stationBusy)
    }

    /** An empty station says so and queues nothing (the SPA's own toast). */
    @Test
    fun anEmptyStationSaysSoAndQueuesNothing() = runTest {
        val toasts = Toasts()
        val player = FakePlayPort()
        val service = FakeDiscoveryService(
            station = DiscoveryFixtures.station().copy(trackCount = 0, rows = PlainRows(emptyList())),
        )
        val (model, _, _) = harness(service, player, toasts)
        model.load()
        advanceUntilIdle()
        model.startStation("Techno")
        advanceUntilIdle()
        assertTrue(player.played.isEmpty())
        assertEquals("Cette station n'a encore rien à jouer.", toasts.said.single())
    }

    /** The all-stations list: the kept copy, then ONE `/flow/styles`, then a fold. */
    @Test
    fun theAllStationsListIsOneRequestAndFoldsItsSearch() = runTest {
        val (model, service, _) = harness()
        model.load()
        advanceUntilIdle()

        model.openStations()
        advanceUntilIdle()
        assertEquals(1, service.stationStyleReads)
        assertEquals(3, model.state.value.shownStations.size)

        model.searchStations("electro")
        assertTrue(model.state.value.shownStations.isEmpty())
        model.searchStations("techno")
        assertEquals("the parent matches too", 2, model.state.value.shownStations.size)
    }

    /** A playlist's « Lire » queues its CHECKED titles: no second request. */
    @Test
    fun playingAPlaylistUsesTheCheckedTitles() = runTest {
        val player = FakePlayPort()
        val (model, service, _) = harness(player = player)
        model.load()
        advanceUntilIdle()
        model.chooseStyle(model.state.value.tiles.first())
        model.discover()
        assertTrue("no pending sweep: ${model.state.value}", settle("pending") { model.state.value.phase == ExplorerModel.Phase.Pending })
        model.poll()
        assertTrue("no playlist: ${model.state.value}", settle("the playlists") { model.state.value.playlists.isNotEmpty() })

        val requests = service.discoverPosts + service.pollReads
        model.playPlaylist(model.state.value.playlists.first())
        advanceUntilIdle()
        assertEquals(requests, service.discoverPosts + service.pollReads)
        assertEquals("playlist", player.played.single().kind)
        assertEquals(1, player.played.single().rows.size)
        assertNull("the sheet closes", model.state.value.picked)
    }

    /** A (style, level) already answered paints itself: reopening costs nothing. */
    @Test
    fun reopeningARememberedStyleAsksNothing() = runTest {
        val (model, service, _) = harness()
        model.load()
        assertTrue("the tiles never painted: ${model.state.value}", settle("the tiles") { model.state.value.tiles.isNotEmpty() })
        val tile = model.state.value.tiles.first()
        model.chooseStyle(tile)
        model.discover()
        assertTrue("no pending sweep: ${model.state.value}", settle("pending") { model.state.value.phase == ExplorerModel.Phase.Pending })
        model.poll()
        assertTrue("no ready answer: ${model.state.value}", settle("ready") { model.state.value.phase == ExplorerModel.Phase.Ready })

        val posts = service.discoverPosts
        model.close()
        model.chooseStyle(tile)
        assertEquals(ExplorerModel.Phase.Ready, model.state.value.phase)
        assertEquals(2, model.state.value.playlists.size)
        assertEquals(posts, service.discoverPosts)
    }

    /** The style search is `flat()`: accents and punctuation set aside. */
    @Test
    fun theFoldIsTheSpasFlat() {
        assertEquals("electro", foldGenre("Électro"))
        assertEquals("discofunk", foldGenre("Disco / funk"))
        assertEquals("techno", foldGenre(" Techno "))
    }
}
