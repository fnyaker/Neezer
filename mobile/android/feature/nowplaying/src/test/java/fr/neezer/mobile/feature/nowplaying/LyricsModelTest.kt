package fr.neezer.mobile.feature.nowplaying

import fr.neezer.mobile.bridge.ScreenState
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The words' rules: WHEN they are asked (never before the audio), what the device
 * keeps (0 requests on a warm open), what a failure shows, and the card's own
 * limits — all on the JVM with a counting fake.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LyricsModelTest {

    private fun model(
        service: FakeService,
        scope: TestScope,
    ): LyricsModel = LyricsModel(
        service = service,
        scope = scope,
        io = StandardTestDispatcher(scope.testScheduler),
    )

    /** A cold open is ONE request; the copy answers the second time, with none. */
    @Test
    fun the_words_are_one_request_and_the_copy_is_free() = runTest {
        val service = FakeService(cachedLyrics = null)
        val model = model(service, this)

        model.open("9001", "Fauve", "Le chant des sirènes")
        advanceUntilIdle()
        assertEquals(1, service.lyricCalls)
        assertEquals(1, service.cachedCalls)
        val state = model.state.value
        assertEquals(4, state.lyrics!!.synced.size)
        // The ♪ between two verses is not a line: four synced entries, three lines.
        assertEquals(3, state.lines.size)

        // A warm panel: the words are already there, so a second call asks nothing.
        model.open("9001", "Fauve", "Le chant des sirènes")
        advanceUntilIdle()
        assertEquals(1, service.lyricCalls)
    }

    /** A device with a copy paints it FIRST, then the fresh answer replaces it. */
    @Test
    fun a_cached_copy_paints_first() = runTest {
        val service = FakeService(cachedLyrics = FakeService.lyrics())
        val model = model(service, this)
        val seen = mutableListOf<ScreenState<uniffi.nz_ffi.LyricsData>>()
        val job = backgroundScope.launch {
            model.state.collect { seen += it.screen }
        }
        model.open("9001", "Fauve", "Le chant des sirènes")
        advanceUntilIdle()
        val ready = seen.filterIsInstance<ScreenState.Ready<uniffi.nz_ffi.LyricsData>>()
        assertEquals("Loading then two Ready: $seen", 2, ready.size)
        assertTrue("the copy is painted, marked stale", ready[0].stale)
        assertFalse("then the fresh answer replaces it", ready[1].stale)
        assertEquals(1, service.lyricCalls)
        job.cancel()
    }

    /** A failure with nothing to show is a sentence and a retry, never a spinner. */
    @Test
    fun a_failure_names_its_cause_and_offers_a_retry() = runTest {
        val service = FakeService(cachedLyrics = null)
        service.failLyrics = "Hors ligne"
        val model = model(service, this)
        model.open("9001", "Fauve", "Le chant")
        advanceUntilIdle()
        val failed = model.state.value.screen
        assertTrue("$failed", failed is ScreenState.Failed)
        assertEquals("Hors ligne", (failed as ScreenState.Failed).cause)
        // A Failed flow stays alive to serve « Réessayer »: the test closes it.
        model.clear()
    }

    /** An answer without words is a STATE: « Paroles indisponibles ». */
    @Test
    fun an_empty_answer_is_a_state() = runTest {
        val service = FakeService(cachedLyrics = null, fresh = FakeService.emptyLyrics())
        val model = model(service, this)
        model.open("9002", "Fauve", "Sans paroles")
        advanceUntilIdle()
        assertTrue(model.state.value.none)
        assertFalse(model.state.value.canCreate)
    }

    /** The active line follows the position being heard; a marker is skipped. */
    @Test
    fun the_active_line_follows_the_position() = runTest {
        val service = FakeService(cachedLyrics = FakeService.lyrics())
        val model = model(service, this)
        model.open("9001", "Fauve", "Le chant")
        advanceUntilIdle()

        model.onPosition(0.0)
        assertEquals(0, model.state.value.activeLine)
        assertEquals("Il est cinq heures", model.state.value.activeText)

        // Between 12 s and 24 s the synced line is the marker: the overlay shows
        // the line's own text (`lyrics.js`'s `line.text || ""`), and the PANEL
        // is what draws a marker as « • • • ».
        model.onPosition(13.0)
        assertEquals(1, model.state.value.activeLine)
        assertEquals("♪", model.state.value.activeText)

        model.onPosition(25.0)
        assertEquals("Paris s'éveille", model.state.value.activeText)
    }

    /** A track change empties the panel; the next ask waits for the audio. */
    @Test
    fun a_track_change_empties_the_words() = runTest {
        val service = FakeService(cachedLyrics = FakeService.lyrics())
        val model = model(service, this)
        model.onTrack("9001", "Fauve", "Un", playable = true)
        advanceUntilIdle()
        assertEquals(1, service.lyricCalls)

        // The next title, not playable yet: nothing is asked.
        model.onTrack("9002", "Fauve", "Deux", playable = false)
        advanceUntilIdle()
        assertEquals(1, service.lyricCalls)
        assertNull(model.state.value.lyrics)

        // Playable: the one request.
        model.onTrack("9002", "Fauve", "Deux", playable = true)
        advanceUntilIdle()
        assertEquals(2, service.lyricCalls)
    }

    /**
     * The card's limits are the SPA's: one line minimum, three maximum, lyric
     * order, and the refusal is a sentence the person reads.
     */
    @Test
    fun the_card_keeps_one_to_three_lines() = runTest {
        val service = FakeService(cachedLyrics = FakeService.lyrics())
        val model = model(service, this)
        model.open("9001", "Fauve", "Le chant")
        advanceUntilIdle()

        model.openCard()
        // It opens on the line being heard, else the first.
        assertEquals(listOf(0), model.state.value.selected)

        model.toggleLine(1) // the second selectable line
        assertEquals(listOf(0, 1), model.state.value.selected)
        assertEquals(listOf("Il est cinq heures", "Paris s'éveille"), model.cardTexts())

        model.toggleLine(2)
        assertEquals(listOf(0, 1, 2), model.state.value.selected)
        model.toggleLine(3) // a fourth: refused, and said
        assertEquals(listOf(0, 1, 2), model.state.value.selected)
        assertEquals("Trois lignes au maximum : retirez-en une.", model.state.value.notice)

        model.toggleLine(0)
        assertEquals(listOf(1, 2), model.state.value.selected)
        model.toggleLine(2)
        assertEquals(listOf(1), model.state.value.selected)
        model.toggleLine(1) // the last one: refused
        assertEquals(listOf(1), model.state.value.selected)
        assertEquals("Gardez au moins une ligne.", model.state.value.notice)
    }

    /** The card's texts come from Rust's order rule, not from the tap order. */
    @Test
    fun the_cards_texts_are_in_lyric_order() = runTest {
        val service = FakeService(cachedLyrics = FakeService.lyrics())
        val model = model(service, this)
        model.open("9001", "Fauve", "Le chant")
        advanceUntilIdle()
        // The panel opens on the line being heard (the first here), so a tap on
        // the last line ADDS it, and a tap on the first REMOVES it.
        assertEquals(listOf(0), model.state.value.selected)
        model.toggleLine(2)
        assertEquals(listOf("Il est cinq heures", "Je me réveille"), model.cardTexts())
        model.toggleLine(0)
        assertEquals(listOf("Je me réveille"), model.cardTexts())
    }

    /** The file's name is Rust's (it is what a share target sees). */
    @Test
    fun the_card_names_its_file() = runTest {
        val service = FakeService(cachedLyrics = FakeService.lyrics())
        val model = model(service, this)
        model.open("9001", "Fauve", "Le chant des sirènes")
        advanceUntilIdle()
        assertEquals("neezer-fauve-le-chant-des-sirènes.png", model.state.value.filename)
    }
}
