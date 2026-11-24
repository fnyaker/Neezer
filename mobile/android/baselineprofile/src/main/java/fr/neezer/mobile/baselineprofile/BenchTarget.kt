package fr.neezer.mobile.baselineprofile

import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until

/**
 * What every bench in this module shares: the package it drives, the input sizes
 * of the worst realistic case, and the two gestures the shell understands today.
 *
 * The benches drive the APP through its accessibility tree — the same nodes
 * TalkBack reads — never through screen coordinates: a coordinate bench passes on
 * one device and measures a tap on nothing on the next (docs/agents/mobile.md,
 * "Tests and emulation": a Compose UI test uses semantics).
 *
 * **Why `@RunWith(JUnit4::class)` and not `AndroidJUnit4`.** The AndroidX samples
 * use `androidx.test.ext.junit.runners.AndroidJUnit4`, but that artifact reaches
 * this module only at RUNTIME (`benchmark-macro-junit4`'s POM lists it in the
 * runtime scope, so it is in the test APK and not on the compile classpath) and
 * the compile fails with `Unresolved reference 'runners'`. The module's
 * `testInstrumentationRunner` is `AndroidJUnitRunner`, which runs any JUnit4
 * class, so a plain `JUnit4` runner behaves identically here. Asking for the
 * dependency explicitly would mean editing `gradle/libs.versions.toml`, which is
 * the skeleton lot's file (worker rule 3).
 */
object BenchTarget {

    /** This app's `applicationId` (docs/agents/mobile.md, "Coexistence"). */
    const val PACKAGE_NAME = "fr.neezer.mobile"

    /** The worst realistic library: the SPA's own 8 000-track bench. */
    const val LIBRARY_ROWS = 8_000

    /** The worst realistic playlist: the SPA's own 4 000-track bench. */
    const val QUEUE_ROWS = 4_000

    /** Startup iterations: 10 is the plan's number for a cold start (TTID). */
    const val STARTUP_ITERATIONS = 10

    /** Scroll iterations: 5 keeps a full run under a minute on the emulator. */
    const val SCROLL_ITERATIONS = 5

    /** How long a screen gets to paint before a bench calls it missing. */
    const val SCREEN_TIMEOUT_MS = 10_000L

    /** The tab bar's four labels, as the shell paints them (French UI copy). */
    const val TAB_HOME = "Accueil"
    const val TAB_SEARCH = "Rechercher"
    const val TAB_LIBRARY = "Bibliothèque"
    const val TAB_PODCASTS = "Podcasts"

    /** The shell's one permanent action. */
    const val ACTION_SETTINGS = "Réglages"

    /**
     * Waits for the node whose text is exactly [label], or throws naming what it
     * saw was missing. A bench that proceeds without its screen measures the
     * previous one.
     */
    fun awaitText(scope: MacrobenchmarkScope, label: String, timeoutMs: Long = SCREEN_TIMEOUT_MS): UiObject2 {
        val found = scope.device.wait(Until.findObject(By.text(label)), timeoutMs)
        check(found != null) { "the shell never showed « $label » within ${timeoutMs}ms" }
        return found
    }

    /** Taps [label] and waits for the destination it opens to paint. */
    fun open(scope: MacrobenchmarkScope, label: String) {
        awaitText(scope, label).click()
        scope.device.waitForIdle()
    }

    /**
     * One vertical fling over the content area: from 70 % down to 25 % of the
     * height, a third of the screen's width off-centre so a row's swipe action
     * (MB10's right-swipe) is not what the bench measures.
     */
    fun fling(scope: MacrobenchmarkScope, up: Boolean = true) {
        val device = scope.device
        val w = device.displayWidth
        val h = device.displayHeight
        val x = (w * 0.6f).toInt()
        val from = if (up) (h * 0.70f).toInt() else (h * 0.30f).toInt()
        val to = if (up) (h * 0.30f).toInt() else (h * 0.70f).toInt()
        device.swipe(x, from, x, to, 24)
        device.waitForIdle()
    }
}
