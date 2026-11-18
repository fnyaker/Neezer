package fr.neezer.mobile.feature.nowplaying

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.PlayerCommand

/**
 * « À propos »: two sections, two requests, each failing on its own — and the
 * radio's seed filtered out of what is DRAWN (the server sends it first).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class AboutModelTest {

    private fun model(service: FakeService, scope: TestScope): AboutModel = AboutModel(
        service = service,
        scope = scope,
        io = StandardTestDispatcher(scope.testScheduler),
    )

    /** Opening the panel is ONE credits request and ONE radio request. */
    @Test
    fun the_panel_is_two_requests() = runTest {
        val service = FakeService()
        val model = model(service, this)
        model.load("9001", "9001")
        advanceUntilIdle()
        assertEquals(1, service.creditsCalls)
        assertEquals(1, service.similarCalls)
        val state = model.state.value
        assertTrue(state.hasCredits)
        assertEquals(1, state.credits?.credits?.size)
        assertEquals("Compositeur", state.credits?.credits?.first()?.label)
        // The seed the server put first is not shown twice (the SPA's filter).
        assertEquals(listOf("9002", "9003"), state.similar.map { it.key })
    }

    /** A similar list that cannot load says so, and the retry asks again. */
    @Test
    fun the_similar_list_fails_on_its_own() = runTest {
        val service = FakeService(failSimilar = true)
        val model = model(service, this)
        model.load("9001", "9001")
        advanceUntilIdle()
        assertTrue(model.state.value.similarFailed)
        assertTrue(model.state.value.similar.isEmpty())
        // The credits still arrived: one section's failure is not the panel's.
        assertTrue(model.state.value.hasCredits)

        service.failSimilar = false
        model.retry("9001", "9001")
        advanceUntilIdle()
        assertFalse(model.state.value.similarFailed)
        assertEquals(2, service.similarCalls)
    }

    /** « Tout lire » starts the list it shows, as a radio of its own. */
    @Test
    fun play_all_replaces_the_queue_with_the_similar_titles() = runTest {
        val service = FakeService()
        val player = FakePlayerPort()
        val model = model(service, this)
        model.load("9001", "9001")
        advanceUntilIdle()

        model.playAll(player, "9001")
        val command = player.commands.single() as PlayerCommand.PlayQueue
        assertEquals(2, command.tracks.size)
        assertEquals("9002", command.tracks.first().key)
        assertEquals("similar", command.context?.kind)
        advanceUntilIdle()
    }

    /** « Ajouter à la file » appends the same rows and replaces nothing. */
    @Test
    fun queue_next_appends() = runTest {
        val service = FakeService()
        val player = FakePlayerPort()
        val model = model(service, this)
        model.load("9001", "9001")
        advanceUntilIdle()

        model.queueNext(player)
        val command = player.commands.single() as PlayerCommand.AddToQueue
        assertEquals(listOf("9002", "9003"), command.tracks.map { it.key })
    }

    /** A panel with nothing to say says so, rather than an empty sheet. */
    @Test
    fun an_empty_panel_knows_it() = runTest {
        val service = FakeService(similar = listOf(FakeService.row("9001", "Seed")))
        val empty = FakeService(
            similar = listOf(FakeService.row("9001", "Seed")),
            credits = uniffi.nz_ffi.TrackCreditsData(
                credits = emptyList(),
                info = uniffi.nz_ffi.CreditsInfoData(null, null, null, null, null),
                empty = true,
            ),
        )
        val model = model(empty, this)
        model.load("9001", "9001")
        advanceUntilIdle()
        assertFalse(model.state.value.hasCredits)
        assertTrue(model.state.value.similar.isEmpty())
        assertTrue(model.state.value.empty)
        assertEquals("the other fake was never asked", 0, service.creditsCalls)
    }
}
