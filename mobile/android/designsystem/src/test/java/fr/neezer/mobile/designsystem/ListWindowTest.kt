package fr.neezer.mobile.designsystem

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.TrackRow

/**
 * `rememberListWindow`'s policy, counted.
 *
 * The acceptance is a NUMBER: **at most 3 `window()` calls for a fling of 2 000
 * rows on an 8 000-row list**. So the source below counts its own calls, and the
 * fling is replayed the way `LazyListState` reports it during a real fling: one
 * viewport report per frame, ~33 rows apart, `scrolling = true`, until it lands.
 *
 * Why the cap matters: chasing the viewport's page at 60 fps across 2 000 rows is
 * 31 calls, i.e. 31 × 64 rows of a 4 000-row queue crossing the FFI for rows the
 * person flew past. `ListWindow` answers 2.
 */
class ListWindowTest {

    private class FakeSource(private val rows: Int) : NzRowSource {
        var calls = 0
            private set
        val starts = mutableListOf<Int>()

        override fun len(): Int = rows

        override fun window(start: Int, count: Int): List<TrackRow> {
            calls += 1
            starts += start
            return (start until minOf(start + count, rows)).map { nzDemoRow(it) }
        }
    }

    private val pageSize = 64

    /** 8 000 rows: the SPA's own worst case (`tools/perf_api.py`'s library). */
    private val libraryRows = 8_000

    @Test
    fun aFlingOf2000RowsAsksThreeWindowsAtMost() {
        val source = FakeSource(libraryRows)
        val window = ListWindow(source, pageSize)

        // The screen opens on row 0: the page under the viewport.
        window.onVisible(first = 0, last = 9, scrolling = false)
        assertEquals("an open list loads the page under the viewport", 1, source.calls)

        // The fling: 60 frames of ~33 rows — the shape of a real fling on a
        // 8 000-row list (2 000 rows in about a second).
        var first = 0
        repeat(60) {
            first += 33
            window.onVisible(first = first, last = first + 9, scrolling = true)
        }
        assertEquals("a flying viewport must not chase the pages it crosses", 1, source.calls)

        // It lands at row 1 980 and the finger lifts.
        window.onVisible(first = first, last = first + 9, scrolling = false)
        val forTheFling = source.calls

        // MEASURED: 1 page on open + the 2 the landed viewport covers (it
        // straddles rows 1 980–1 989, pages 30 and 31) = 3, against 31 for a
        // per-frame chase. The acceptance's ceiling is 3.
        assertEquals(3, forTheFling)
        assertTrue("the acceptance's cap", forTheFling <= 3)

        // Only the landing pages are in memory: everything crossed is dropped.
        assertEquals(setOf(30, 31), window.loadedPages.keys.toSet())
        assertNotNull("the landed page is there", window.rowAt(first))
        assertNotNull("and the rest of the landed viewport", window.rowAt(first + 9))
        assertNull("a crossed page was dropped", window.rowAt(10))

        // The next settled report preloads the page ahead, so the following
        // screen paints from memory.
        val before = source.calls
        window.onVisible(first = first, last = first + 9, scrolling = false)
        assertEquals(1, source.calls - before)
        assertTrue(window.loadedPages.keys.contains(32))
    }

    @Test
    fun aSlowScrollLoadsEveryPageOnce() {
        val source = FakeSource(libraryRows)
        val window = ListWindow(source, pageSize)
        window.onVisible(0, 9, scrolling = false)

        // A finger dragging: ~4 rows a frame, the list is being READ.
        var first = 0
        while (first < 1_200) {
            first += 4
            window.onVisible(first, first + 9, scrolling = true)
        }
        // Pages 0 … 18 are behind or under the viewport; each was fetched once.
        val expected = (0..(first + 9) / pageSize).toSet()
        assertTrue(
            "every page under the viewport is loaded: expected $expected, got ${window.loadedPages.keys}",
            window.loadedPages.keys.containsAll(expected),
        )
        assertEquals(
            "no page is fetched twice (${source.starts})",
            source.starts.size,
            source.starts.distinct().size,
        )
        assertNotNull(window.rowAt(first))
    }

    @Test
    fun memoryIsBoundedByTheKeepRadius() {
        val source = FakeSource(libraryRows)
        val window = ListWindow(source, pageSize)
        window.onVisible(0, 9, scrolling = false)
        // Jump 20 pages away (a search result, a "go to album").
        window.onVisible(20 * pageSize, 20 * pageSize + 9, scrolling = false)
        assertTrue(
            "pages beyond ${ListWindow.KeepPages} away are dropped, got ${window.loadedPages.keys}",
            window.loadedPages.keys.all { kotlin.math.abs(it - 20) <= ListWindow.KeepPages + 1 },
        )
        assertTrue("a 8 000-row list keeps a handful of rows", window.rows.size <= 9 * pageSize)
    }

    @Test
    fun anEmptyListAsksNothing() {
        val source = FakeSource(0)
        val window = ListWindow(source, pageSize)
        window.onVisible(0, 0, scrolling = false)
        assertEquals(0, source.calls)
        assertEquals(0, window.len)
    }

    @Test
    fun theLastPageIsShort() {
        val source = FakeSource(130)
        val window = ListWindow(source, pageSize)
        // The viewport straddles the last two pages: 64 rows of page 1 and the
        // 2 rows page 2 still has.
        window.onVisible(120, 129, scrolling = false)
        assertEquals(130, window.len)
        assertEquals(66, window.rows.size)
        assertNotNull(window.rowAt(129))
        assertNull("past the end", window.rowAt(130))
    }
}
