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
 * The shell's own frames: what a tap on the tab bar costs to paint, five
 * iterations of a full walk over the four doors (Accueil → Explorer →
 * Bibliothèque → Podcasts → back to Accueil).
 *
 * **Why this bench exists next to [ScrollBenchmark].** The first run of the
 * scroll bench on MB0's skeleton failed with `Observed no renderthread slices in
 * trace - verify that your benchmark is redrawing`: a `NzPlaceholder` is static,
 * a fling over it moves nothing, so there is no frame to time — and
 * `FrameTimingMetric` says so instead of reporting zeros, which is exactly the
 * behaviour MQ1 wants. The shell does repaint when a tab is tapped (the
 * destination changes, the bar's ripple runs), so that is the frame measure the
 * skeleton can actually give. The 8 000/4 000-row result arrives with MB8/MB9
 * and no change to this file.
 *
 * The numbers are the emulator's (swiftshader, `-no-audio`, API 35 x86_64), and
 * the plan's p95 ≤ 16.7 ms / p99 ≤ 25 ms are real-phone budgets: what this bench
 * is for today is a FLOOR that a later screen must not regress, measured the same
 * way every run (docs/agents/mobile.md, MQ1).
 */
@RunWith(JUnit4::class)
class ShellFramesBenchmark {

    @get:Rule
    val benchmarkRule = MacrobenchmarkRule()

    @Test
    fun tabSwitchFrames() = benchmarkRule.measureRepeated(
        packageName = BenchTarget.PACKAGE_NAME,
        metrics = listOf(FrameTimingMetric()),
        compilationMode = CompilationMode.Partial(baselineProfileMode = BaselineProfileMode.UseIfAvailable),
        startupMode = StartupMode.WARM,
        iterations = BenchTarget.SCROLL_ITERATIONS,
        setupBlock = { launchToHome() },
    ) {
        // One tap per door, then back to the first: each is a destination change
        // and a ripple, so every step paints. `BenchTarget.open` waits on the
        // accessibility tree, never on a timer.
        for (tab in listOf(
            BenchTarget.TAB_SEARCH,
            BenchTarget.TAB_LIBRARY,
            BenchTarget.TAB_PODCASTS,
            BenchTarget.TAB_HOME,
        )) {
            BenchTarget.open(this, tab)
        }
    }
}
