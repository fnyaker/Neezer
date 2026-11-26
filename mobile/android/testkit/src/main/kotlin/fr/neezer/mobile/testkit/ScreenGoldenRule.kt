package fr.neezer.mobile.testkit

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onRoot
import com.github.takahirom.roborazzi.AccessibilityCheckAfterTestStrategy
import com.github.takahirom.roborazzi.ExperimentalRoborazziApi
import com.github.takahirom.roborazzi.RoborazziATFAccessibilityCheckOptions
import com.github.takahirom.roborazzi.RoborazziATFAccessibilityChecker
import com.github.takahirom.roborazzi.RoborazziOptions
import com.github.takahirom.roborazzi.RoborazziOptions.CompareOptions
import com.github.takahirom.roborazzi.RoborazziRule
import com.github.takahirom.roborazzi.captureRoboImage
import com.dropbox.differ.SimpleImageComparator
import com.google.android.apps.common.testing.accessibility.framework.AccessibilityCheckPreset
import com.google.android.apps.common.testing.accessibility.framework.AccessibilityViewCheckResult
import org.hamcrest.Matcher
import org.hamcrest.Matchers
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runners.model.Statement

/**
 * The ONE rule a verifying golden wraps (plan T3).
 *
 * It hosts the screen, captures the composition's root and compares it to the
 * committed golden, with the plan's exact options:
 *
 *  * `changeThreshold = 0.001` and `SimpleImageComparator(maxDistance = 0.007f,
 *    vShift = 2, hShift = 2)`: a 1 dp change fails, a 2 px anti-aliasing wobble
 *    does not;
 *  * the Android Accessibility Test Framework at `LATEST`, failing on `Error`
 *    (a touch target under 48 dp, an unlabelled clickable, a contrast failure) —
 *    `checkRoboAccessibility` runs after every test that uses this rule;
 *  * every image under `mobile/android/screenshots`, the single committed root.
 *
 * Usage:
 *
 * ```kotlin
 * @get:Rule val rule = ScreenGoldenRule()
 *
 * @Test fun home() {
 *     rule.composeRule.setContent { HomeScreen(…) }
 *     rule.capture("home-accueil-heros-360dp-f1.0.png")
 *     rule.composeRule.onNodeWithTag(HOME_HERO_TAG).assertExists()
 * }
 * ```
 *
 * The rule is a JUnit [`TestRule`], not a `RoborazziRule` subclass: it chains the
 * Compose rule (outer) around Roborazzi's (inner), so the activity exists when the
 * capture and the accessibility pass run, and is torn down after both.
 */
@OptIn(ExperimentalRoborazziApi::class)
class ScreenGoldenRule(
    /** Absolute golden root; defaults to the `nz.goldens` property the build hands the test. */
    private val outputDirectoryPath: String = goldenRoot(),
    /**
     * A suppression for ONE screen's rule, never a default: Roborazzi's default
     * suppresses nothing. A screen that must suppress a real ATF finding names
     * the check and the exact node shape here, with the reason in its own test —
     * see `GenresChipsGoldenTest`.
     */
    private val accessibilitySuppressions: Matcher<in AccessibilityViewCheckResult> =
        Matchers.not(Matchers.anything()),
) : TestRule {

    val composeRule = createAndroidComposeRule<ComponentActivity>()

    private val roborazziRule = RoborazziRule(
        composeRule = composeRule,
        captureRoot = composeRule.onRoot(),
        options = RoborazziRule.Options(
            outputDirectoryPath = outputDirectoryPath,
            roborazziOptions = RoborazziOptions(
                compareOptions = CompareOptions(
                    changeThreshold = 0.001f,
                    imageComparator = SimpleImageComparator(
                        maxDistance = 0.007f,
                        vShift = 2,
                        hShift = 2,
                    ),
                ),
            ),
            roborazziAccessibilityOptions = RoborazziATFAccessibilityCheckOptions(
                checker = RoborazziATFAccessibilityChecker(
                    preset = AccessibilityCheckPreset.LATEST,
                    suppressions = accessibilitySuppressions,
                ),
                failureLevel = RoborazziATFAccessibilityChecker.CheckLevel.Error,
            ),
            // After EVERY test, not only after a capture: the bounds tests of the
            // same class render the same screens and get the same check.
            accessibilityCheckStrategy = AccessibilityCheckAfterTestStrategy(),
        ),
    )

    /**
     * Compose the screen at [fontScale], which is what the golden's `-f<scale>`
     * suffix claims.
     *
     * Robolectric's `@Config(fontScale = …)` sets the resource configuration, but
     * the Compose test rule builds its `LocalDensity` before that value reaches
     * the view (MEASURED: the same text measured 77 px at 1.0 and at 1.3 while the
     * image differed, i.e. the value was not the one Compose reads). The plan's
     * option is applied here instead, on the exact local the components read
     * (`Sillon`'s `sp` values resolve through `LocalDensity.fontScale`).
     */
    fun setContent(fontScale: Float = 1f, content: @androidx.compose.runtime.Composable () -> Unit) {
        composeRule.setContent {
            val density = androidx.compose.ui.platform.LocalDensity.current
            androidx.compose.runtime.CompositionLocalProvider(
                androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(
                    density = density.density,
                    fontScale = fontScale,
                ),
            ) {
                content()
            }
        }
    }

    override fun apply(base: Statement, description: Description): Statement =
        RuleChain.outerRule(composeRule).around(roborazziRule).apply(base, description)

    /**
     * Capture the composition's root as [name], under the committed golden root.
     *
     * The caller names it `<module>-<screen>-<state>-<width>dp-f<scale>.png` (the
     * plan's matrix); the name is the comparison key, so renaming a golden is a
     * deliberate edit.
     */
    fun capture(name: String) {
        composeRule.waitForIdle()
        // An ABSOLUTE file: Roborazzi's default path strategy is
        // "relativePathFromCurrentDirectory", which would write beside the module
        // (and leave the committed root untouched). The rule already knows the one
        // golden root, so it names the file itself.
        composeRule.onRoot().captureRoboImage(java.io.File(outputDirectoryPath, name))
    }

    private companion object {
        fun goldenRoot(): String =
            System.getProperty("nz.goldens")
                ?: error(
                    "nz.goldens is not set: a golden test needs the committed " +
                        "mobile/android/screenshots root (see :testkit's build.gradle.kts)",
                )
    }
}
