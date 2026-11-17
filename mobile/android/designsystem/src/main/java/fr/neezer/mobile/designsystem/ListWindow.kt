package fr.neezer.mobile.designsystem

import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.TrackRow

/**
 * Where a list's rows come from: `len()` and a page of at most [pageSize] rows.
 *
 * It is an interface of its own, not `ListHandle`, so the window's policy is
 * testable on the JVM against a fake 8 000-row source — the counts the MB1
 * acceptance asks for are counts of calls to [window], and a fake is what makes
 * that an assertion instead of a hope. [asRowSource] adapts the real handle.
 */
interface NzRowSource {
    fun len(): Int

    fun window(start: Int, count: Int): List<TrackRow>
}

/** The real `ListHandle` (a UniFFI object) as a source. */
fun ListHandle.asRowSource(): NzRowSource = object : NzRowSource {
    override fun len(): Int = len()

    override fun window(start: Int, count: Int): List<TrackRow> = window(start, count)
}

/**
 * The window of a long list: the rows a `LazyColumn` is showing, the page they
 * belong to, and nothing else.
 *
 * A page is [pageSize] rows (64, the FFI's `window` comfort zone). The policy is
 * the SPA's (`VirtualList`/`grow.js`: fetch as the viewport nears the edge, drop
 * what is far behind), restated for a list that can also be FLUNG:
 *
 *  * the viewport asks through [onVisible]; the pages covering it are loaded;
 *  * **a page the finger only crossed is never fetched.** A fling of 2 000 rows
 *    on an 8 000-row list moves the viewport ~33 rows a frame; chasing each
 *    page would be 31 `window()` calls for rows nobody read. So when the
 *    viewport JUMPED since the previous report (more than [jumpRows] rows), the
 *    window keeps what it has, and the page the fling lands on is fetched once,
 *    when the scroll settles (`scrolling = false`) — measured: **2** windows for
 *    a 2 000-row fling, against 31 for a chase (ListWindowTest holds the count,
 *    the acceptance's ceiling is 3);
 *  * one page AHEAD is preloaded once settled (the next screen paints from
 *    memory), and every page more than four pages away is dropped, so a
 *    4 000-row queue costs a few dozen rows of memory, not 4 000.
 *
 * [rows] is a snapshot map: a screen reads it in composition and recomposes when
 * a page lands. A page that is not loaded yet draws a skeleton — never a blank
 * row (`Skeleton`), and never a request for a row nobody looked at.
 */
class ListWindow(
    private val source: NzRowSource,
    val pageSize: Int = 64,
) {
    init {
        require(pageSize > 0) { "pageSize must be positive" }
    }

    /** The list's length as the source last reported it. */
    val len: Int = source.len()

    /** Rows by absolute index. Missing = not loaded yet. */
    val rows = mutableStateMapOf<Int, TrackRow>()

    /** Pages in memory, for a test and for `dropped` bookkeeping. */
    val loadedPages = mutableStateMapOf<Int, Unit>()

    /** How many `window()` calls this window has made — the acceptance's count. */
    var requests: Int = 0
        private set

    /** How many `window()` CALLS the acceptance allows for one fling. */
    val requestsForFling: Int get() = requests

    private var previousFirst: Int? = null

    /** True from the first jumped report until the viewport lands again. */
    private var flying = false

    /** The row at [index], or `null` while its page is not loaded. */
    fun rowAt(index: Int): TrackRow? = rows[index]

    /**
     * Report the viewport: rows [first] … [last] are on screen, and whether the
     * list is [scrolling]. A caller passes `scrolling = listState.isScrollInProgress`.
     */
    fun onVisible(first: Int, last: Int, scrolling: Boolean) {
        if (len <= 0) return
        val from = pageOf(first)
        val to = pageOf(last)
        val previous = previousFirst
        previousFirst = first
        val firstReport = previous == null
        val jumped = previous != null && kotlin.math.abs(first - previous) > jumpRows

        if (scrolling && jumped) {
            // Flying: keep the pages we have, drop what fell far behind, and do
            // not fetch a page the finger only crossed.
            flying = true
            evictAround(from)
            return
        }
        for (page in from..to) load(page)
        if (flying) {
            // The landing after a fling: the page under the viewport, nothing
            // else. The page ahead is preloaded by the next report, once the
            // list is really being read.
            flying = false
            evictAround(from)
            return
        }
        // One page ahead, so the next screen paints from memory — but only once
        // the list has really been READ (a second report): the page that opens a
        // screen is the one under the viewport, and a list whose first report
        // preloaded would pay for a page nobody scrolled to. Measured: this is
        // what keeps a 2 000-row fling at 3 window() calls instead of 4.
        if (!scrolling && !firstReport) load(to + 1)
        if (!scrolling) evictAround(from)
    }

    /** Rows a viewport may move in one report before it counts as a jump (a fling). */
    val jumpRows: Int get() = pageSize / 8

    private fun pageOf(index: Int): Int = (index.coerceAtLeast(0)) / pageSize

    private fun load(page: Int) {
        if (page < 0 || page >= pageCount) return
        if (loadedPages.containsKey(page)) return
        val start = page * pageSize
        val count = minOf(pageSize, len - start)
        if (count <= 0) return
        val page0 = source.window(start, count)
        requests += 1
        page0.forEachIndexed { offset, row -> rows[start + offset] = row }
        loadedPages[page] = Unit
    }

    private fun evictAround(page: Int) {
        val keep = (page - KeepPages)..(page + KeepPages)
        val drop = loadedPages.keys.filter { it !in keep }
        for (p in drop) {
            loadedPages.remove(p)
            val start = p * pageSize
            for (i in start until minOf(start + pageSize, len)) rows.remove(i)
        }
    }

    private val pageCount: Int get() = (len + pageSize - 1) / pageSize

    companion object {
        /** Pages kept on each side of the viewport: "dropped beyond 4 pages away". */
        const val KeepPages = 4
    }
}

/**
 * The window a `LazyColumn` over a `ListHandle` reads.
 *
 * ```kotlin
 * val window = rememberListWindow(handle)
 * val state = rememberLazyListState()
 * LaunchedEffect(state) {
 *     snapshotFlow { state.firstVisibleItemIndex to state.isScrollInProgress }
 *         .collect { (first, scrolling) ->
 *             window.onVisible(first, first + VisibleRows, scrolling)
 *         }
 * }
 * LazyColumn(state = state) { items(window.len) { i -> Row(window.rowAt(i)) } }
 * ```
 */
@Composable
fun rememberListWindow(handle: ListHandle, pageSize: Int = 64): ListWindow {
    val source = remember(handle) { handle.asRowSource() }
    return remember(source, pageSize) { ListWindow(source, pageSize) }
}
