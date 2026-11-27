package fr.neezer.mobile.feature.onboarding

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.nz_ffi.OnboardingStep

/**
 * La mise en route change d'étape en fondu (lot X6) : le squelette, l'échec, les
 * artistes et le compte se remplacent **dans la même image** en éco, et une
 * ré-émission de la même étape n'ajoute aucun nœud (M6).
 *
 * The header — « Étape 1 sur 2 », « Terminer » and the rail — stays OUTSIDE the
 * swap (the plan says so): only the pane changes. The curve is `inPlaceSpecs`'s;
 * a Compose test cannot read a running animation, so the wiring is what is
 * asserted here (X0's note).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class OnboardingScreenMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun aStepChangeReplacesInTheSameFrameAtEco() {
        var state by mutableStateOf(OnboardingModel.State(loading = true))
        var tick by mutableIntStateOf(0)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    tick
                    OnboardingScreen(state = state, actions = OnboardingActions())
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithTag(ONBOARDING_LOADING_TAG).assertExists()
        rule.onNodeWithText("Reliez ce que vous écoutez ailleurs").assertDoesNotExist()

        state = OnboardingModel.State(loading = false, step = OnboardingStep.ACCOUNT)
        rule.waitForIdle()
        rule.onNodeWithText("Reliez ce que vous écoutez ailleurs").assertExists()
        rule.onNodeWithTag(ONBOARDING_LOADING_TAG).assertDoesNotExist()

        // M6: the same step re-emitted plays nothing and adds no node.
        tick++
        rule.waitForIdle()
        rule.onAllNodesWithText("Reliez ce que vous écoutez ailleurs").assertCountEquals(1)
        rule.onAllNodesWithTag(ONBOARDING_LOADING_TAG).assertCountEquals(0)

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }

    @Test
    fun theFourPhasesAreTheScreensOwnStates() {
        assertEquals(ONBOARDING_LOADING, onboardingPhase(OnboardingModel.State(loading = true)))
        assertEquals(
            ONBOARDING_FAILED,
            onboardingPhase(OnboardingModel.State(loading = false, failure = "Le serveur n'a pas répondu.")),
        )
        assertEquals(
            ONBOARDING_ACCOUNT,
            onboardingPhase(OnboardingModel.State(loading = false, step = OnboardingStep.ACCOUNT)),
        )
        assertEquals(
            ONBOARDING_ARTISTS,
            onboardingPhase(OnboardingModel.State(loading = false, step = OnboardingStep.ARTISTS)),
        )
    }
}
