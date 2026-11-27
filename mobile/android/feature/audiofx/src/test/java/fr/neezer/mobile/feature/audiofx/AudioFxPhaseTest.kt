package fr.neezer.mobile.feature.audiofx

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
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

    /**
     * `AnimatedContent` (which [NzSwap] is) lays its children in a BOX: five
     * cards handed to it directly are all painted at the same bounds, which is
     * what the emulator's `06-lecture-et-son` showed (five siblings at
     * `[85,860][995,1903]`, `precheck overlap=1 targets=1`). The panel stacks
     * them in its own `Column`, and the titles' tops prove it: one card's title
     * is at least a card's own content below the one before, never beside it.
     */
    // A taller viewport than the phone's: all five cards must be PLACED for
    // their bounds to mean anything (an unplaced node reads `boundsInRoot` 0),
    // and the defect is about how they are laid out, not about what fits.
    @Config(sdk = [34], qualifiers = "w360dp-h3000dp-xhdpi")
    @Test
    fun theFiveCardsStackInsteadOfPilingUpInTheSameBox() {
        val model = AudioFxModel(FakeAudioFx())
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    AudioFxPanel(model)
                }
            }
        }
        model.start()
        rule.waitForIdle()
        val titles = listOf(
            FxCopy.OUTPUT_TITLE,
            FxCopy.PAN_TITLE,
            FxCopy.AUDIO_TITLE,
            FxCopy.EQ_TITLE,
            FxCopy.TRANSITIONS_TITLE,
        )
        val tops = titles.map { rule.onNodeWithText(it).fetchSemanticsNode().boundsInRoot.top }
        println("MEASURE audiofx card tops: ${tops.joinToString(" ") { it.toInt().toString() }} px")
        val gap = with(rule.density) { 24.dp.toPx() }
        tops.zipWithNext().forEachIndexed { index, (above, below) ->
            assertTrue(
                "« ${titles[index]} » (haut ${above.toInt()} px) et « ${titles[index + 1]} » " +
                    "(haut ${below.toInt()} px) se chevauchent : les cartes ne sont pas empilées",
                below >= above + gap,
            )
        }
    }
}
