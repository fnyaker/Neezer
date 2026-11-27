package fr.neezer.mobile.feature.entity

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.appearSpecs
import androidx.compose.animation.core.TweenSpec
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * `MutedNote` — the dim sentence every entity screen draws when it has nothing to
 * show — ENTERS instead of popping (lot X4, D3).
 *
 * What is asserted and what is not: the node is in the tree from the frame it is
 * asked for at every level (at eco because `appearSpecs` is `None`, at Full because
 * `AnimatedVisibility(MutableTransitionState)` composes its content on the mount
 * frame and walks its alpha from 0), and it stays ONE node through the enter and
 * after it. The opacity at 50 % is a picture, not a semantics assertion — the same
 * honest limit X0 wrote down. The curve is `appearSpecs`' and `TransitionTest`
 * holds it; here the note names the preset it uses.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class EntityBitsMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val sentence = "Aucun titre pour ce rôle."

    /**
     * One FRAME, composed and applied: `waitForIdle` returns before the
     * recomposition that sees the new state, and the write composes on the second
     * frame on this tree (measured in `TransitionTest`).
     */
    private fun twoFrames() {
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
    }

    private fun host(level: NzMotionLevel) {
        var asked by mutableStateOf(false)
        ask = { asked = true }
        rule.setContent {
            NzTheme(motion = level) {
                Box {
                    if (asked) MutedNote(sentence)
                }
            }
        }
    }

    private var ask: (() -> Unit)? = null

    @Test
    fun theEmptyNoteIsPresentInTheFrameItIsAskedForAtEco() {
        host(NzMotionLevel.Off)
        rule.waitForIdle()
        rule.onAllNodesWithText(sentence).assertCountEquals(0)

        rule.mainClock.autoAdvance = false
        ask!!()
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        // Eco plays nothing: the sentence is whole, not walking from 0.
        rule.onAllNodesWithText(sentence).assertCountEquals(1)
    }

    @Test
    fun theEmptyNoteHoldsOneNodeThroughItsEnterAtFull() {
        host(NzMotionLevel.Full)
        rule.waitForIdle()

        rule.mainClock.autoAdvance = false
        ask!!()
        twoFrames()
        // Composed on the mount frame (its alpha is 0 and rises): ONE node, never
        // two copies of the sentence.
        rule.onAllNodesWithText(sentence).assertCountEquals(1)
        rule.mainClock.advanceTimeBy(200)
        rule.onAllNodesWithText(sentence).assertCountEquals(1)
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()
        rule.onAllNodesWithText(sentence).assertCountEquals(1)
    }

    /**
     * The note hands `appearSpecs` to `NzAppearOnMount`: 140 ms and a 4 dp rise at
     * Full, every spec 0 ms at eco (which is why the eco case above is legitimate).
     */
    @Test
    fun theNoteUsesTheAppearancePreset() {
        val motion = NzMotion(NzMotionLevel.Full)
        val preset = appearSpecs(motion)
        assertEquals(NzDur.Fast, (preset.fade as TweenSpec<*>).durationMillis)
        assertEquals(
            "le ressaut est `moveS` (4 dp hors Full, 0 en éco et en mouvement réduit)",
            4.dp,
            motion.moveS,
        )
        assertNotNull("une apparition monte de 4 dp au standard", preset.move)
        assertTrue("à l'éco il n'y a rien à jouer", appearSpecs(NzMotion(NzMotionLevel.Off)).isNone)
    }
}
