package fr.neezer.mobile.baselineprofile

import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.FrameTimingMetric
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.JUnit4

/**
 * The three long-list frame measures of *Measures*, one test each, on the worst
 * realistic input (docs/agents/mobile.md: "library scroll (8 000), queue scroll
 * (4 000), full-screen player open — p95 ≤ 16.7 ms, p99 ≤ 25 ms, 0 frozen
 * frames").
 *
 * [ScrollBenchmark] (MQ1) was written before the screens existed and says so;
 * this file is MQ2's: each test opens the real screen, waits for the node only
 * that screen draws, and flings the list. `FrameTimingMetric` refuses to invent
 * numbers when nothing redraws (`Observed no renderthread slices`), so a screen
 * that fails to paint fails the run — the precondition is the assertion, not an
 * `assumeTrue` that would turn a regression green.
 *
 * The playlist bench's 4 000 rows are the bench server's own `Playlist 0`
 * (`perf_api.py --big-playlist 4000`); the favourites bench's 8 000 are
 * `--favorites 8000`.
 */
@RunWith(JUnit4::class)
class JourneyListsBenchmark {

    @get:Rule
    val benchmarkRule = MacrobenchmarkRule()

    private fun listBenchmark(open: JourneySteps) = benchmarkRule.measureRepeated(
        packageName = BenchTarget.PACKAGE_NAME,
        metrics = listOf(FrameTimingMetric()),
        compilationMode = CompilationMode.Partial(baselineProfileMode = BaselineProfileMode.UseIfAvailable),
        startupMode = StartupMode.WARM,
        iterations = 3,
        setupBlock = {
            Journey.restart(this)
            open(this)
        },
    ) {
        repeat(6) { Journey.fling(device, up = true) }
        repeat(6) { Journey.fling(device, up = false) }
    }

    /** The library's 8 000 favourites. */
    @Test
    fun favoritesScroll8000() = listBenchmark { scope ->
        Journey.tapText(scope.device, BenchTarget.TAB_LIBRARY)
        Journey.awaitDesc(scope.device, Journey.LIBRARY)
        Journey.tapText(scope.device, Journey.LEAF_TITLES)
        Journey.awaitText(scope.device, "Mélanger", 30_000)
        check(scope.device.findObject(androidx.test.uiautomator.By.scrollable(true)) != null) {
            "the favourites leaf painted no scrollable list"
        }
    }

    /** The 4 000-track playlist. */
    @Test
    fun playlistScroll4000() = listBenchmark { scope ->
        Journey.openPlaylist(scope.device, Journey.BIG_PLAYLIST)
    }

    /**
     * The full-screen player opening over the playing screen: the sheet's own
     * animation and layout are what a tap on the mini player pays.
     */
    @Test
    fun playerOpen() = benchmarkRule.measureRepeated(
        packageName = BenchTarget.PACKAGE_NAME,
        metrics = listOf(FrameTimingMetric()),
        compilationMode = CompilationMode.Partial(baselineProfileMode = BaselineProfileMode.UseIfAvailable),
        startupMode = StartupMode.WARM,
        iterations = 3,
        setupBlock = {
            Journey.restart(this)
            Journey.searchToAlbum(this)
            Journey.playAlbum(this)
        },
    ) {
        // The sheet's own marker, not « EN LECTURE »: the label follows the
        // transport and the album's two 4.8 s titles may have ended by now (the
        // bench measures the sheet's first paint, not the sound).
        Journey.openPlayer(this)
    }
}

/** One journey step a list bench uses as its setup: a function of the scope. */
internal typealias JourneySteps = (androidx.benchmark.macro.MacrobenchmarkScope) -> Unit
