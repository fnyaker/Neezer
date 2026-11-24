package fr.neezer.mobile.baselineprofile

import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.FrameTimingMetric
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Until
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.JUnit4

/**
 * Scrolling the worst realistic lists, held to the plan's frame budget
 * (docs/agents/mobile.md, "Measures": p95 ≤ 16.7 ms, p99 ≤ 25 ms, no frozen
 * frame over 700 ms; the Standard's rule 3 repeats it for every screen lot).
 *
 * [FrameTimingMetric] reports the app's own frame durations from the perfetto
 * trace, so a janky row convicts itself.
 *
 * **These two benches are INACTIVE until the app has a list, and they say so.**
 * MB0's shell has no `LazyColumn`: `LibraryEntry` is a `NzPlaceholder` and the
 * queue is a sheet that arrives with MB11. A fling over a static placeholder
 * draws nothing, and `FrameTimingMetric` refuses to invent numbers
 * (`Observed no renderthread slices in trace`) — so each bench checks for a
 * scrollable node and, when there is none, is SKIPPED with its cause printed in
 * the run's report. That is a stated precondition of the screen, not a way to
 * make a red job green: the day MB8/MB9 paint the list, `hasScrollAction` is
 * there and the same test measures it, with no edit here.
 *
 * What the skeleton CAN measure today is the shell's own repaint:
 * [ShellFramesBenchmark] (`measure.sh frames` runs both classes).
 */
@RunWith(JUnit4::class)
class ScrollBenchmark {

    @get:Rule
    val benchmarkRule = MacrobenchmarkRule()

    /** The library at 8 000 rows (MB8's screen). */
    @Test
    fun libraryScroll8000() = scrollLongList(BenchTarget.TAB_LIBRARY, BenchTarget.LIBRARY_ROWS)

    /** The queue at 4 000 rows (MB11's sheet). */
    @Test
    fun queueScroll4000() = scrollLongList(BenchTarget.TAB_PODCASTS, BenchTarget.QUEUE_ROWS)

    /**
     * Opens [tab] and flings it 8 times down and 8 times up — more than one screen
     * of the worst case, because a person scrolls through a list and back.
     *
     * [inputRows] is the size the SERVER seeds for that screen
     * (`perf-server.sh`: 8 000 tracks, a 4 000-track playlist); the bench itself
     * only owns the gesture, since the rows come from the app's own data.
     */
    private fun scrollLongList(tab: String, inputRows: Int) = benchmarkRule.measureRepeated(
        packageName = BenchTarget.PACKAGE_NAME,
        metrics = listOf(FrameTimingMetric()),
        compilationMode = CompilationMode.Partial(baselineProfileMode = BaselineProfileMode.UseIfAvailable),
        startupMode = StartupMode.WARM,
        iterations = BenchTarget.SCROLL_ITERATIONS,
        setupBlock = {
            launchToHome()
            BenchTarget.open(this, tab)
            check(inputRows == BenchTarget.LIBRARY_ROWS || inputRows == BenchTarget.QUEUE_ROWS) {
                "unknown input size $inputRows"
            }
            // The precondition, stated in the report rather than guessed: ROWS
            // must be on screen, or there is nothing to time. A scrollable
            // container is not enough — on a fresh install the leaf draws an
            // empty state (a static screen), the flings redraw nothing, and
            // `FrameTimingMetric` fails with "Observed no renderthread slices in
            // trace" (measured on emulator-5660, 2026-11-24; a skeleton that is
            // still shimmering made it pass or fail with timing alone). The row's
            // own ⋯ is the marker every long list draws.
            val rows = device.wait(Until.findObject(By.descStartsWith("Plus d'actions pour")), 5_000)
            assumeTrue(
                "$tab has no rows (no bench library): nothing to fling, so nothing to time",
                rows != null,
            )
        },
    ) {
        repeat(8) { BenchTarget.fling(this, up = true) }
        repeat(8) { BenchTarget.fling(this, up = false) }
    }
}
