// :playback's window arithmetic (docs/agents/mobile.md, "MB3c": the playlist is
// "previous 1 + current + next 100"). JVM, no device: a wrong `+ start` is a skip
// that plays the wrong title.
package fr.neezer.mobile.playback

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PlaybackWindowTest {

    /** The size the note pins: 1 before + the current one + 100 after. */
    @Test
    fun the_window_is_one_before_the_current_title_and_a_hundred_after() {
        assertEquals(102, PlaybackWindow.SIZE)
        val slice = PlaybackWindow.slice(len = 4_000, index = 500)
        assertEquals(102, slice.count)
        assertEquals(499, slice.start)
        // The current title sits where Media3 expects it: after exactly one
        // previous item.
        assertEquals(1, slice.windowOf(500))
        assertEquals(499, slice.realOf(0))
        assertEquals(600, slice.realOf(101))
    }

    /** A 4 000-row queue crosses 102 rows at a time, never whole. */
    @Test
    fun a_long_queue_is_never_handed_over_whole() {
        for (index in listOf(0, 1, 2, 500, 3_998, 3_999)) {
            val slice = PlaybackWindow.slice(len = 4_000, index = index)
            assertTrue("index $index", slice.count <= PlaybackWindow.SIZE)
            assertTrue("index $index", slice.contains(index))
        }
    }

    /** The first title has no hole before it, and the last no hole after it. */
    @Test
    fun the_edges_are_clamped_not_padded() {
        val first = PlaybackWindow.slice(len = 4_000, index = 0)
        assertEquals(0, first.start)
        assertEquals(PlaybackWindow.SIZE, first.count)
        assertEquals(0, first.windowOf(0))

        val last = PlaybackWindow.slice(len = 4_000, index = 3_999)
        assertEquals(3_998, last.start)
        assertEquals(2, last.count)
        assertEquals(1, last.windowOf(3_999))
    }

    /** A short queue, an empty one, and no current title at all. */
    @Test
    fun a_queue_shorter_than_the_window_is_the_window() {
        val short = PlaybackWindow.slice(len = 3, index = 1)
        assertEquals(0, short.start)
        assertEquals(3, short.count)
        assertEquals(PlaybackWindow.Slice.EMPTY, PlaybackWindow.slice(len = 0, index = 0))
        assertEquals(PlaybackWindow.Slice.EMPTY, PlaybackWindow.slice(len = 10, index = null))
    }

    /** A state is rebuilt only when the window no longer shows the title. */
    @Test
    fun a_window_is_kept_until_the_title_leaves_it() {
        val slice = PlaybackWindow.slice(len = 4_000, index = 500)
        assertTrue(PlaybackWindow.stillShows(slice, 4_000, 500))
        assertTrue(PlaybackWindow.stillShows(slice, 4_000, 599))
        assertFalse("one past the window", PlaybackWindow.stillShows(slice, 4_000, 601))
        assertFalse("one before the window", PlaybackWindow.stillShows(slice, 4_000, 498))
        assertFalse("the queue shrank", PlaybackWindow.stillShows(slice, 300, 500))
    }
}
