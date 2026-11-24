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
 * The whole journey, in frames (docs/agents/mobile.md, MQ2 and *Measures*:
 * "Smoothness — p95 frame ≤ 16.7 ms, p99 ≤ 25 ms, 0 frozen frames (> 700 ms)").
 *
 * One iteration is one person's first five minutes: home → search « storm » →
 * the album → play → the 8 000 favourites → the 4 000-track playlist → the
 * player and its queue → Réglages › Assistance → the visualiser → home. Every
 * step is proved by a node only its screen draws ([Journey]); a step that never
 * landed fails the run instead of timing the previous screen.
 *
 * The plan's budgets are REAL-PHONE budgets; on the emulator (swiftshader) the
 * same numbers are the floor a screen must not regress against, run to run on
 * the same emulator — the note says so, and [BenchTarget]'s comment repeats why
 * the emulator error is suppressed here and anywhere.
 *
 * The 50-track download and the 30-minute screen-off soak are NOT in this
 * benchmark: they take minutes, and a frame metric over them would be a clock.
 * They live in [JourneyLongTest], and their CPU measurement is
 * `mobile/tools/measure.sh cpu` (the app's own `/proc/<pid>/stat`).
 */
@RunWith(JUnit4::class)
class JourneyBenchmark {

    @get:Rule
    val benchmarkRule = MacrobenchmarkRule()

    /**
     * Three full journeys: enough for a p50/p95 and short enough that a red run
     * is still debuggable. The warm start is the second visit of a person who
     * lives in the app — the worst case is [StartupBenchmark]'s cold one.
     */
    @Test
    fun wholeJourney() = benchmarkRule.measureRepeated(
        packageName = BenchTarget.PACKAGE_NAME,
        metrics = listOf(FrameTimingMetric()),
        compilationMode = CompilationMode.Partial(baselineProfileMode = BaselineProfileMode.UseIfAvailable),
        startupMode = StartupMode.WARM,
        iterations = 3,
        setupBlock = {
            Journey.restart(this)
            // The journey's start is the home of a person who has an account:
            // its cache was written once, and the home paints from it.
            Journey.awaitDesc(device, Journey.HOME)
        },
    ) {
        Journey.searchToAlbum(this)
        val tapToUi = Journey.playAlbum(this)
        println("MEASURE journey tap→« Pause » (UI, upper bound of output_stats): ${tapToUi}ms")

        Journey.flingFavorites(this)
        Journey.flingPlaylist(this)
        Journey.openPlayerAndQueue(this)

        val diagnostic = Journey.readDiagnostic(this)
        println(
            "MEASURE journey mémoire comptée : ${diagnostic.countedBytes}o " +
                "(file ${diagnostic.queueBytes}o, listes ${diagnostic.listBytes}o)",
        )
        println("MEASURE journey rebuffers : ${diagnostic.rebuffers}")
        for ((tag, stats) in diagnostic.tags) {
            println(
                "MEASURE journey réseau [$tag] : ${stats.requests} requêtes, " +
                    "${stats.bytesIn}o reçus, ${stats.bytesOut}o envoyés",
            )
        }

        Journey.openVisualizer(this)
        Journey.tapText(device, BenchTarget.TAB_HOME)
        Journey.awaitDesc(device, Journey.HOME)
    }
}
