package fr.neezer.mobile.baselineprofile

import androidx.benchmark.macro.junit4.BaselineProfileRule
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Until
import org.junit.Rule
import org.junit.runners.JUnit4
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Generates the app's Baseline Profile (docs/agents/mobile.md, "Size and cold
 * start": the official sample went 324.8 → 229.0 ms with one).
 *
 * The journey is the one every person walks first: **start → home → the library
 * → its first scroll → play**. A profile covers the classes and methods a run
 * touches, so leaving the scroll out profiles the first screen only, and leaving
 * play out makes the first track tap — the action rule 1 puts above everything —
 * the part that still pays the JIT.
 *
 * Run it against the emulator (`mobile/tools/measure.sh` wraps it):
 *
 * ```sh
 * mobile/tools/emu.sh start --port 5580 && mobile/tools/emu.sh wait --port 5580
 * cd mobile/android && gradle :baselineprofile:generateBaselineProfile
 * ```
 *
 * The plugin writes `app/src/benchmark/generated/baselineProfiles/baseline-prof.txt`
 * on the host; committing that file is MQ2's (it regenerates it over the whole
 * journey and ships it in the release), so this run's output is a generated
 * artifact and is not committed.
 *
 * The "play" step exists only from MB3b (no player, no queue row before it): the
 * generator taps what a person taps — the first row of the library — and MB3b's
 * player makes that tap audible. The step stays in the journey so the profile
 * always follows the app's own shape rather than a hard-coded list.
 */
@RunWith(JUnit4::class)
class BaselineProfileGenerator {

    @get:Rule
    val rule = BaselineProfileRule()

    @Test
    fun generate() = rule.collect(
        packageName = BenchTarget.PACKAGE_NAME,
        includeInStartupProfile = true,
    ) {
        pressHome()
        startActivityAndWait()

        // The shell is up when its first tab paints (the same node the other
        // benches wait for: an event, not a sleep).
        check(device.wait(Until.hasObject(By.text(BenchTarget.TAB_HOME)), BenchTarget.SCREEN_TIMEOUT_MS)) {
            "the shell never showed « ${BenchTarget.TAB_HOME} »"
        }

        BenchTarget.open(this, BenchTarget.TAB_LIBRARY)
        repeat(2) { BenchTarget.fling(this, up = true) }
        repeat(2) { BenchTarget.fling(this, up = false) }

        // Play: the first row of the library, found through the list's own
        // semantics (a scrollable container, then its first clickable child) —
        // never a screenshot coordinate. MB0's placeholder has no such row, so
        // the step taps nothing today; MB8's rows make it the real "first track
        // plays" step the profile needs, without this file changing.
        val list = device.findObject(By.scrollable(true))
        list?.findObject(By.clickable(true))?.click()
    }
}
