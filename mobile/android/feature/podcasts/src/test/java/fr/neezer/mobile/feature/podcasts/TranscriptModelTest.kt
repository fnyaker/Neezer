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
 * The episode panel's rules (lot MB13): the transcript's pages, a search that
 * falls back to the pages held when the link is dead, and the pure chapter
 * helpers the card and the list share.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class TranscriptModelTest {
    @Test
    fun `a page is read once, then the next one is one more request`() = runTest {
        val service = FakePodcastService()
        val model = TranscriptModel(service, this, pageSize = 200)

        model.open("ep-0", "ready")
        runCurrent()

        val state = model.state.value
        assertEquals("ready", state.status)
        assertEquals(200, state.segments.size)
        assertEquals(240, state.total)
        assertEquals(1, state.page)
        assertEquals(2, state.pages)
        assertEquals("the copy was asked first", 1, service.count("cachedTranscript"))
        assertEquals("then the page", 1, service.count("transcript:ep-0:0:"))

        model.next()
        runCurrent()
        assertEquals("the tail of 240 lines", 40, model.state.value.segments.size)
        assertEquals(2, model.state.value.page)
        assertEquals(1, service.count("transcript:ep-0:200:"))
    }

    @Test
    fun `a search offline falls back to the pages held`() = runTest {
        val service = FakePodcastService()
        service.failTranscript = java.io.IOException("le téléphone est hors ligne")
        service.localSearchAnswer = listOf(
            uniffi.nz_ffi.TranscriptSegmentData(start = 180.0, end = 195.0, text = "…la maison…"),
        )
        val model = TranscriptModel(service, this)

        model.search("maison")
        runCurrent()

        val state = model.state.value
        assertEquals(1, state.hits.size)
        assertTrue(state.searchLocal)
        assertNull("no cause to show: the pages held answered", state.searchFailure)
    }

    @Test
    fun `a search the device cannot answer names its cause`() = runTest {
        val service = FakePodcastService()
        service.failTranscript = java.io.IOException("le téléphone est hors ligne")
        val model = TranscriptModel(service, this)

        model.search("maison")
        runCurrent()

        assertNotNull(model.state.value.searchFailure)
        assertTrue(model.state.value.hits.isEmpty())
    }

    @Test
    fun `one character searches nothing`() = runTest {
        val service = FakePodcastService()
        val model = TranscriptModel(service, this)
        service.calls.clear()

        model.search("m")
        runCurrent()

        assertEquals("a search needs two characters", 0, service.count("transcript"))
        assertFalse(model.state.value.searching2.not())
    }
}

/** The pure chapter arithmetic the card, the list and the ticks share. */
class ChapterHelpersTest {
    @Test
    fun `the current chapter is the last one that started, inside its own span`() {
        val cards = chaptersOf("ep-0").chapters
        assertEquals(0, chapterAt(cards, 30.0)?.first)
        assertEquals(1, chapterAt(cards, 900.0)?.first)
        assertEquals(1, chapterAt(cards, 1_799.0)?.first)
        // The third chapter declares no end: it runs to the file's end.
        assertEquals(2, chapterAt(cards, 3_000.0)?.first)
        // A chapter with an end stops being current past it.
        val ended = listOf(
            uniffi.nz_ffi.ChapterData(index = 0u, start = 0u, end = 10u, title = "", label = "Chapitre 1"),
        )
        assertEquals(null, chapterAt(ended, 11.0))
    }

    @Test
    fun `the next chapter is the first one that starts later`() {
        val cards = chaptersOf("ep-0").chapters
        assertEquals(1, nextChapter(cards, 30.0)?.first)
        assertEquals(2, nextChapter(cards, 1_000.0)?.first)
        assertEquals(null, nextChapter(cards, 3_000.0))
    }

    @Test
    fun `the playing transcript line is the last one that started`() {
        val segments = transcriptPage("ep-0", 0, 200).segments
        assertEquals(0, indexAt(segments, 0.0))
        assertEquals(1, indexAt(segments, 15.0))
        assertEquals(10, indexAt(segments, 150.0))
        assertEquals(-1, indexAt(segments, -5.0))
    }

    /**
     * The labels the panel writes with KOTLIN alone. The ones that come from the
     * core (`reasonText`, `batchOptions`, `pageInfo`, a language's name) are
     * proven by `nz-core`'s own tests — they are Rust's ports of
     * `transcript.js` and reach this screen through `nzPodcast*`; a JVM test
     * holds no `.so` to ask.
     */
    @Test
    fun `the labels the panel writes`() {
        assertEquals("12:05", PodcastsCopy.duration(725))
        assertEquals("1:02:05", PodcastsCopy.duration(3_725))
        assertEquals("1:00", PodcastsCopy.duration(60))
        assertEquals("3 épisodes", PodcastsCopy.episodeCount(3))
        assertEquals("1 épisode", PodcastsCopy.episodeCount(1))
        assertEquals("7 passages", PodcastsCopy.hitCount(7))
        assertEquals("1 passage", PodcastsCopy.hitCount(1))
        assertEquals("Aucun passage pour « x » dans cette émission.", PodcastsCopy.noHit("x"))
        assertTrue(PodcastsCopy.archiveQueued(17).contains("17"))
        assertTrue(PodcastsCopy.transcribeSkipped(2, null).startsWith("2 ignorés"))
        assertFalse(PodcastsCopy.STALE.contains("·").not())
    }
}
