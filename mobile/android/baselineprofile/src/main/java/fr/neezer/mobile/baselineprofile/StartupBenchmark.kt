package fr.neezer.mobile.baselineprofile

import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.StartupTimingMetric
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import org.junit.Rule
import org.junit.runners.JUnit4
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Cold start, the plan's two numbers (docs/agents/mobile.md, "Measures"):
 * TTID ≤ 500 ms and TTFD (`reportFullyDrawn`, the home painted from cache)
 * ≤ 900 ms — measured with the Baseline Profile OFF and ON, 10 iterations each,
 * because a profile's worth is the difference between the two.
 *
 * The "ON" half asks for [BaselineProfileMode.UseIfAvailable], not `Require`:
 * MQ1 ships the harness before any `baseline-prof.txt` exists in the APK (MQ2
 * generates and commits it, after the screens it profiles). `Require` on an APK
 * with no profile fails the run, so it would be a red bench for a file another
 * lot owns. Once MQ2's profile is in the APK, this test measures it unchanged —
 * and the day it is missing again, [startupColdWithProfile] reports the same
 * numbers as [startupColdWithoutProfile], which is the visible symptom.
 *
 * `StartupMode.COLD` means the process is killed before each iteration; the
 * emulator needs no interaction for it (it is the AVD's own state, never a
 * snapshot: `emu.sh` starts it `-no-snapshot`).
 */
@RunWith(JUnit4::class)
class StartupBenchmark {

    @get:Rule
    val benchmarkRule = MacrobenchmarkRule()

    /** The app a person gets today: no compilation hints, a cold process. */
    @Test
    fun startupColdWithoutProfile() = coldStart(CompilationMode.None())

    /**
     * The same start with the Baseline Profile the APK carries (AOT from the
     * profile on first run, then the JIT's own warm-up).
     */
    @Test
    fun startupColdWithProfile() = coldStart(
        CompilationMode.Partial(baselineProfileMode = BaselineProfileMode.UseIfAvailable),
    )

    private fun coldStart(compilation: CompilationMode) = benchmarkRule.measureRepeated(
        packageName = BenchTarget.PACKAGE_NAME,
        metrics = listOf(StartupTimingMetric()),
        compilationMode = compilation,
        startupMode = StartupMode.COLD,
        iterations = BenchTarget.STARTUP_ITERATIONS,
        setupBlock = {
            // Nothing to seed: the start is measured from the launcher intent.
            pressHome()
        },
    ) {
        startActivityAndWait()
        // StartupTimingMetric's first column is TTID (`timeToInitialDisplay`),
        // the frame the system draws for the activity. TTFD
        // (`timeToFullDisplay`, the home painted from cache) appears in the
        // report only once the shell calls `reportFullyDrawn()`; the skeleton
        // does not (MQ1 measured TTID only and says so), and the lot that adds
        // that call gets TTFD here with no change to this file.
        BenchTarget.awaitText(this, BenchTarget.TAB_HOME)
    }
}

/** The whole launch, from the warm-up to the first usable frame, for one iteration. */
internal fun MacrobenchmarkScope.launchToHome() {
    pressHome()
    startActivityAndWait()
    BenchTarget.awaitText(this, BenchTarget.TAB_HOME)
}
