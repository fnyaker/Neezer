package fr.neezer.mobile.feature.audiofx

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
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
import org.robolectric.annotation.GraphicsMode

/**
 * Lecture & son change de phase en fondu (lot X6) : la ligne du premier `kv`
 * laisse les cinq cartes **dans la même image** en éco (M7, M13). Un écran qui ne
 * peut pas lire dit pourquoi au lieu de tourner (rule 3) — this is the same
 * `NzSwap`, so the wiring and the curve are what the test holds.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class AudioFxPhaseTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun thePanelReplacesInTheSameFrameAtEco() {
        val model = AudioFxModel(FakeAudioFx())
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    AudioFxPanel(model)
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("Lecture des réglages…").assertExists()
        rule.onNodeWithText("Sortie audio").assertDoesNotExist()

        model.start()
        rule.waitForIdle()
        rule.onNodeWithText("Sortie audio").assertExists()
        rule.onNodeWithText("Égaliseur").assertExists()
        rule.onNodeWithText("Lecture des réglages…").assertDoesNotExist()

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }
}
