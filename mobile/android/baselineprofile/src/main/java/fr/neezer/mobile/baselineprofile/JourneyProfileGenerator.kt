package fr.neezer.mobile.baselineprofile

import androidx.benchmark.macro.junit4.BaselineProfileRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.JUnit4

/**
 * The Baseline Profile, generated from the JOURNEY (docs/agents/mobile.md, MQ2:
 * "a Baseline Profile generated from the journey"; *Size and cold start*: the
 * official sample went 324.8 → 229.0 ms with one).
 *
 * MQ1's [BaselineProfileGenerator] walked what existed then — home, the library,
 * a fling, a tap. MQ2's profile must cover what a person does: the search, an
 * album, the playing screen, the two long lists, the player sheet and its queue,
 * Réglages and the visualiser. The classes behind THOSE are what the release's
 * `baseline-prof.txt` lists, and the file this run writes is what
 * `:app/src/main/baseline-prof.txt` ships.
 *
 * The profile is generated on a device with the app's own account already
 * configured (a warm cache is what the profile is for); on this machine that is
 * the headless emulator, and the note says which parts of the profile a real
 * phone would still have to confirm (this rule refuses an emulator by default;
 * `:baselineprofile`'s `defaultConfig` lifts the guard deliberately, with the
 * reason written there).
 *
 * Run it (`mobile/tools/measure.sh profile` wraps the gradle call):
 *
 * ```sh
 * cd mobile/android && gradle :baselineprofile:connectedBenchmarkReleaseAndroidTest \
 *   -Pandroid.testInstrumentationRunnerArguments.class=fr.neezer.mobile.baselineprofile.JourneyProfileGenerator
 * ```
 *
 * The generated `.txt` lands under `:app/src/benchmark/generated/baselineProfiles/`
 * (the AGP plugin's own path). Copying it to `:app/src/main/baseline-prof.txt` is
 * the ONE line this lot ships by hand, and the copy is what the release build
 * merges.
 */
@RunWith(JUnit4::class)
class JourneyProfileGenerator {

    @get:Rule
    val rule = BaselineProfileRule()

    @Test
    fun generate() = rule.collect(
        packageName = BenchTarget.PACKAGE_NAME,
        includeInStartupProfile = true,
    ) {
        Journey.launchToHome(this)

        // The journey, in the order a person walks it. The download and the
        // 30-minute soak are left out on purpose: a profile is about the code a
        // run touches, and the same classes serve them.
        Journey.searchToAlbum(this)
        Journey.playAlbum(this)
        Journey.flingFavorites(this)
        Journey.flingPlaylist(this)
        Journey.openPlayerAndQueue(this)
        Journey.readDiagnostic(this)
        Journey.openVisualizer(this)
        Journey.tapText(device, BenchTarget.TAB_HOME)
        Journey.awaitDesc(device, Journey.HOME)
    }
}
