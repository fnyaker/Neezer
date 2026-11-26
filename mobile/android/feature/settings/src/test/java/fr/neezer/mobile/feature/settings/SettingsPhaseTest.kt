package fr.neezer.mobile.feature.settings

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Les cartes de Réglages changent de phase en fondu (lot X6) : la ligne
 * « Chargement… » laisse la place au corps **dans la même image** en éco, et un
 * corps qui s'ouvre (le formulaire ARL) entre en fondu sans faire sauter la carte.
 *
 * The curve is `inPlaceSpecs`'s (X0); a Compose test cannot read a running
 * animation, so these assertions hold the WIRING and the spec's numbers.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class SettingsPhaseTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun model(fake: FakeSettingsService): SettingsModel = SettingsModel(
        service = fake,
        scope = CoroutineScope(Dispatchers.Unconfined),
        io = Dispatchers.Unconfined,
        playback = null,
    )

    @Test
    fun theSourcesPhaseReplacesInTheSameFrameAtEco() {
        val fake = FakeSettingsService()
        fake.readGate = CompletableDeferred()
        val model = model(fake)
        model.start()
        model.open("sources")

        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    SettingsSection("sources", model)
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("Chargement…").assertExists()
        rule.onNodeWithText("Ordre des sources").assertExists()
        // The sources' body is not there yet (its own admin-only action).
        rule.onNodeWithText("FLAC · sans perte").assertDoesNotExist()

        fake.readGate?.complete(Unit)
        rule.waitForIdle()
        rule.onAllNodesWithText("Chargement…").assertCountEquals(0)
        rule.onNodeWithText("FLAC · sans perte").assertExists()

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }

    @Test
    fun theLinkFormAppearsInTheSameFrameAtEco() {
        val fake = FakeSettingsService()
        val model = model(fake)
        model.start()
        model.open("sources")
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    SettingsSection("sources", model)
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("Relier un compte").assertExists()

        rule.onNodeWithText("Relier un compte").performClick()
        rule.waitForIdle()
        // The body replaces the button IN FLOW, in the frame it was asked for.
        rule.onNodeWithText("Relier un compte").assertDoesNotExist()
        rule.onNodeWithText("Relier").assertExists()
        rule.onNodeWithText("ARL de votre session Deezer — la clé", substring = true).assertExists()
    }

    @Test
    fun thePhaseFunctionReadsTheThreeStates() {
        assertEquals(
            SETTINGS_PHASE_FAILED,
            settingsPhase(SettingsModel.Section<String>(failure = "Le serveur n'a pas répondu.")),
        )
        assertEquals(SETTINGS_PHASE_LOADING, settingsPhase(SettingsModel.Section<String>()))
        assertEquals(SETTINGS_PHASE_CONTENT, settingsPhase(SettingsModel.Section(data = "corps")))
    }
}
