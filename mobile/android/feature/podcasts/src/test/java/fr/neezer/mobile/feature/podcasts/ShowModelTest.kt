package fr.neezer.mobile.feature.podcasts

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The show page's rules, measured (lot MB13).
 *
 * The acceptance's two rows are here: **a show of 2 000 episodes pages by
 * cursor** (50 rows a page, one request a page, the window costing none) and
 * **an episode resumes at its position** (read from the device's map, which is
 * what makes it work offline), plus the optimistic writes: the archive switch,
 * the transcription switches, the markers.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ShowModelTest {
    @Test
    fun `the cached page paints first and the fresh one replaces it`() = runTest {
        val service = FakePodcastService()
        service.cachedShowAnswer = page("show-1", loaded = 50, next = "offset:50")
        service.pages = mutableListOf(page("show-1", loaded = 50, next = "offset:50"))
        val model = ShowModel(service, FakePodcastsPlayer(), this)

        model.load("show-1")
        runCurrent()

        val state = model.state.value
        assertFalse(state.loading)
        assertEquals(50, state.loaded)
        assertEquals("offset:50", state.next)
        assertEquals("the copy, no request", 1, service.count("cachedShow"))
        assertEquals("then the page (and its markers)", 1, service.count("show:"))
    }

    @Test
    fun `a window asks for its cards and never for the network`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = "offset:50"))
        service.cardsByIndex = (0 until 50).associateWith { index ->
            episode("ep-%05d".format(index), published = 1_794_960_000L - index * 3_600L)
        }.toMutableMap()
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()
        service.calls.clear()

        model.onVisible(0, 11, scrolling = false)
        runCurrent()

        assertEquals("the rows about to be drawn", 12, model.state.value.cards.size)
        assertEquals("one call for the window", 1, service.count("cards:"))
        assertEquals("no request for rows already held", 0, service.count("show:"))
        assertEquals("the end of the list is far away", 0, service.count("showMore"))
    }

    @Test
    fun `reaching the end asks the next cursor, fifty rows at a time`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(
            page("show-1", loaded = 50, next = "offset:50"),
            page("show-1", loaded = 100, next = "offset:100"),
        )
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()
        service.calls.clear()

        model.onVisible(40, 49, scrolling = false)
        runCurrent()

        assertEquals("one request for the next page", 1, service.count("showMore"))
        assertEquals(100, model.state.value.loaded)
        assertEquals("offset:100", model.state.value.next)
    }

    @Test
    fun `a page that fails keeps the list and names its cause`() = runTest {
        val service = ShowModelService(page("show-1", loaded = 50, next = "offset:50"))
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()

        model.loadMore()
        runCurrent()
        assertEquals("the rows already read stay", 50, model.state.value.loaded)
        assertNotNull(model.state.value.moreFailure)
        assertFalse(model.state.value.loadingMore)
    }

    @Test
    fun `the archive switch moves first and comes back on a refusal`() = runTest {
        val service = FakePodcastService()
        service.shows = listOf(show("show-1", "Les Histoires", episodes = 2_000))
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()

        service.failArchive = java.io.IOException("refus du serveur")
        model.toggleAuto()
        runCurrent()
        assertFalse("a refusal puts the switch back", model.state.value.show?.autoArchive ?: true)
        assertNotNull(model.state.value.message)

        service.failArchive = null
        model.toggleAuto()
        runCurrent()
        assertTrue(model.state.value.show?.autoArchive == true)
        assertNotNull("what it queued is said out loud", model.state.value.message)
        assertTrue(model.state.value.message!!.contains("17"))
    }

    @Test
    fun `the transcription switch says what it queued and what it skipped`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()

        model.setBacklog("recent")
        model.toggleShowTranscribe()
        runCurrent()

        assertTrue(
            "the catch-up mode travels with the switch: ${service.calls}",
            service.calls.any { it.startsWith("setShowTranscribe:auto=true,backlog=recent") },
        )
        assertTrue("what it queued", model.state.value.message!!.contains("12"))
        assertTrue(model.state.value.show?.autoTranscribe == true)
    }

    @Test
    fun `a refused language puts the previous one back`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()

        service.failTranscribeToggle = java.io.IOException("refus du serveur")
        model.setLanguage("fr")
        runCurrent()
        assertEquals("auto", model.state.value.show?.transcribeLanguage)
        assertNotNull(model.state.value.message)
    }

    @Test
    fun `chapters are read once and kept when the link dies`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        service.cardsByIndex = mutableMapOf(0 to episode("ep-0", chapters = 3))
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()
        model.onVisible(0, 11, scrolling = false)
        runCurrent()

        service.failChapters = java.io.IOException("le téléphone est hors ligne")
        model.loadChapters("ep-0")
        runCurrent()
        assertNotNull("the failure is named", model.state.value.chapterFailure["ep-0"])
        assertNull("no chapter is invented", model.state.value.chapters["ep-0"])
        assertTrue("nothing left in flight", model.state.value.chapterBusy.isEmpty())
    }

    @Test
    fun `a marker appears at once and disappears if the server refuses`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        service.cardsByIndex = mutableMapOf(0 to episode("ep-0", chapters = 3))
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()
        model.onVisible(0, 11, scrolling = false)
        runCurrent()

        model.addMarker("ep-0", 600.0)
        runCurrent()
        assertEquals(1, model.state.value.markers["ep-0"]?.size ?: 0)
        assertEquals(600_000, model.state.value.markers["ep-0"]!!.first().positionMs.toInt())

        val marker = model.state.value.markers["ep-0"]!!.first()
        service.failRemoveMarker = java.io.IOException("refus du serveur")
        model.removeMarker(marker)
        runCurrent()
        assertEquals(
            "a refused delete puts the chip back",
            1,
            model.state.value.markers["ep-0"]?.size ?: 0,
        )
        assertNotNull(model.state.value.message)
    }

    @Test
    fun `a tap queues the loaded page at the tapped index`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        service.cardsByIndex = (0 until 50).associateWith { episode("ep-%05d".format(it)) }
            .toMutableMap()
        val player = FakePodcastsPlayer()
        val model = ShowModel(service, player, this)
        model.load("show-1")
        runCurrent()
        model.onVisible(0, 11, scrolling = false)
        runCurrent()

        model.play(3)
        runCurrent()

        assertEquals(3, player.playedStart)
        assertEquals("the queue knows the show it came from", "show-1", player.playedContext)
        assertEquals("the loaded page becomes the queue", 50, player.played.size)
        assertEquals("ep-00003", player.played[3].key)
    }

    @Test
    fun `tapping the playing episode toggles instead of restarting`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        service.cardsByIndex = mutableMapOf(0 to episode("ep-0"))
        val player = FakePodcastsPlayer()
        val snapshot = player.snapshotAnswer
        val model = ShowModel(service, player, this)
        model.load("show-1")
        runCurrent()
        model.onVisible(0, 11, scrolling = false)
        runCurrent()

        // The player says it is on ep-0 already.
        player.snapshotAnswer = playingSnapshot("ep-0")
        model.play(0)
        runCurrent()
        assertEquals(listOf("toggle"), player.commands)
        assertNull(snapshot)
    }

    @Test
    fun `the search needs two characters and names its failure`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        val model = ShowModel(service, FakePodcastsPlayer(), this, debounceMs = 0)
        model.load("show-1")
        runCurrent()
        service.calls.clear()

        model.setSearchQuery("l")
        runCurrent()
        assertEquals(0, service.count("showSearch"))

        service.failSearch = java.io.IOException("délai dépassé")
        model.setSearchQuery("maison")
        runCurrent()
        assertNotNull(model.state.value.searchFailure)
        assertFalse(model.state.value.searchBusy)
    }

    @Test
    fun `a resume position is read from the device, not the network`() = runTest {
        val service = FakePodcastService()
        service.pages = mutableListOf(page("show-1", loaded = 50, next = null))
        service.cardsByIndex = mutableMapOf(
            0 to episode("ep-0", position = resumeAt(1_200)),
        )
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()
        service.calls.clear()
        model.onVisible(0, 11, scrolling = false)
        runCurrent()

        val card = model.state.value.cards[0]
        assertEquals(1_200, card?.position?.seconds?.toInt())
        assertEquals("Reprendre · il reste 40:00", PodcastsCopy.episodeLine(card!!))
        assertEquals("one call, zero requests", 1, service.count("cards:"))
        assertEquals(0, service.count("show:"))
    }

    @Test
    fun `the transcription card hides itself when the server has no route`() = runTest {
        val service = FakePodcastService()
        service.transcribeSupportedAnswer = false
        service.failTranscribeSettings = java.io.IOException("route absente")
        val model = ShowModel(service, FakePodcastsPlayer(), this)
        model.load("show-1")
        runCurrent()

        assertNotNull("the cause is named", model.state.value.transcribeFailure)
        assertFalse("an older server hides the card (rule 7)", model.state.value.transcribeSupported)
    }
}

/** A service whose `showMore` fails on purpose, with everything else real. */
private class ShowModelService(private val first: uniffi.nz_ffi.ShowPageData) : PodcastService by FakePodcastService() {
    override suspend fun show(id: String): uniffi.nz_ffi.ShowPageData = first

    override suspend fun showMore(id: String): uniffi.nz_ffi.ShowPageData =
        throw java.io.IOException("le téléphone est hors ligne")
}



private fun playingSnapshot(key: String) = uniffi.nz_ffi.PlayerSnapshot(
    state = uniffi.nz_ffi.PlayState.PLAYING,
    current = uniffi.nz_ffi.NowPlaying(
        key = key,
        id = key,
        title = "Épisode $key",
        artists = "Les Histoires",
        album = "Les Histoires",
        albumId = "show-1",
        artistId = null,
        cover = null,
        durationS = 3_600.0,
        isEpisode = true,
        quality = "320",
        source = "local",
        liked = false,
    ),
    index = 0u,
    len = 50u,
    shuffle = false,
    repeat = uniffi.nz_ffi.Repeat.OFF,
    volume = 1.0f,
    muted = false,
    streaming = false,
    sleep = null,
    error = null,
    fault = null,
    version = 1uL,
)
