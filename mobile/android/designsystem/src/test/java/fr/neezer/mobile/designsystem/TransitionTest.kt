package fr.neezer.mobile.designsystem

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The three presets of `Transition.kt` (lot X0), at the three motion levels.
 *
 * **The honest limit, as the plan states it.** Compose exposes no API to read a
 * *running* animation's spec, alpha or transform: `getUnclippedBoundsInRoot` is a
 * layout bound and a `graphicsLayer` translation is invisible to it, so the SPA's
 * `document.getAnimations()` has no counterpart here. So the CURVE is held by the
 * exact specs [inPlaceSpecs] / [appearSpecs] / [glyphSpecs] return (test 1), and
 * the WIRING by what the semantics tree contains at a given time (tests 2–5).
 * Nothing here claims to "see" an animation: a test that pretended to would
 * always pass. The emulator pictures at 50 % prove the look.
 *
 * No clock sleep: `mainClock` is advanced explicitly where the exit's duration is
 * what is being asserted, and `autoAdvance = false` keeps the frame the test asked
 * for (worker rule 6).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class TransitionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** A spec that must be a tween, so its duration can be read. */
    private fun tween(spec: FiniteAnimationSpec<*>?): TweenSpec<*> {
        assertTrue("expected a tween, got $spec", spec is TweenSpec<*>)
        return spec as TweenSpec<*>
    }

    @Test
    fun theMatrixIsTheTableAtTheThreeLevels() {
        val full = NzMotion(NzMotionLevel.Full)
        val inPlace = inPlaceSpecs(full)
        assertEquals("the in-place fade is the SPA's 140 ms", NzDur.Fast, tween(inPlace.fade).durationMillis)
        assertEquals(NzEase.Enter, tween(inPlace.fade).easing)
        assertEquals("the 4 dp rise walks on the 140 ms standard curve", NzDur.Fast, tween(inPlace.move).durationMillis)
        assertEquals(NzEase.Standard, tween(inPlace.move).easing)
        assertNull("M7: an in-flow replacement has no exit", inPlace.exit)

        val appear = appearSpecs(full)
        assertEquals(NzDur.Fast, tween(appear.fade).durationMillis)
        assertEquals(NzDur.Fast, tween(appear.move).durationMillis)
        assertNull(appear.exit)

        val glyph = glyphSpecs(full)
        assertEquals("the glyph swap is the same 140 ms fade", NzDur.Fast, tween(glyph.fade).durationMillis)
        assertNull("a glyph has no placement", glyph.move)
        assertNull(glyph.exit)

        val reducedMotion = NzMotion(NzMotionLevel.Reduced)
        for (preset in listOf(inPlaceSpecs(reducedMotion), appearSpecs(reducedMotion), glyphSpecs(reducedMotion))) {
            assertEquals("M14 keeps opacity: the fade stays 140 ms", NzDur.Fast, tween(preset.fade).durationMillis)
            assertNull("M14: no displacement", preset.move)
            assertNull(preset.exit)
            assertTrue("reduced motion is not eco", !preset.isNone)
        }

        val eco = NzMotion(NzMotionLevel.Off)
        for (preset in listOf(inPlaceSpecs(eco), appearSpecs(eco), glyphSpecs(eco))) {
            assertEquals("M13: every spec is 0 ms", 0, tween(preset.fade).durationMillis)
            assertNull(preset.move)
            assertNull(preset.exit)
            assertTrue("eco is the marker the composables branch on", preset.isNone)
        }
    }

    @Test
    fun nzSwapAtEcoReplacesInTheSameFrame() {
        var target by mutableStateOf("A")
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Off) {
                NzSwap(targetState = target, label = "phase") { phase ->
                    Box(Modifier.testTag("phase-$phase"))
                }
            }
        }
        rule.onNodeWithTag("phase-A").assertExists()
        target = "B"
        rule.waitForIdle()
        rule.onNodeWithTag("phase-A").assertDoesNotExist()
        rule.onNodeWithTag("phase-B").assertExists()
    }

    @Test
    fun nzSwapDoesNotAnimateASamePhaseReEmit() {
        var tick by mutableIntStateOf(0)
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                NzSwap(targetState = "same", label = "phase") { phase ->
                    // Read `tick` so the parent recomposes WITHOUT changing the key.
                    Box(Modifier.testTag("phase-$phase").then(if (tick % 2 == 0) Modifier else Modifier))
                }
            }
        }
        rule.onNodeWithTag("phase-same").assertExists()
        tick++
        rule.waitForIdle()
        // M6: a re-emit never animates a second copy.
        rule.onAllNodesWithTag("phase-same").assertCountEquals(1)
    }

    @Test
    fun nzAppearLeavesInTheSameFrameAtEveryLevel() {
        var level by mutableStateOf(NzMotionLevel.Full)
        var visible by mutableStateOf(true)
        rule.setContent {
            NzTheme(motion = level) {
                NzAppear(visible = visible) { Box(Modifier.testTag("appears")) }
            }
        }
        for (each in NzMotionLevel.entries) {
            level = each
            visible = true
            rule.waitForIdle()
            rule.onNodeWithTag("appears").assertExists()
            visible = false
            rule.waitForIdle()
            rule.onNodeWithTag("appears").assertDoesNotExist()
        }
    }

    @Test
    fun nzGlyphSwapAtEcoShowsOneGlyph() {
        var glyph by mutableStateOf(NzGlyph.Play)
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Off) {
                NzGlyphSwap(glyph = glyph, contentDescription = glyphLabel(glyph))
            }
        }
        rule.onNodeWithContentDescription("Lecture").assertExists()
        glyph = NzGlyph.Pause
        rule.waitForIdle()
        rule.onNodeWithContentDescription("Lecture").assertDoesNotExist()
        rule.onAllNodesWithContentDescription("Pause").assertCountEquals(1)
    }

    @Test
    fun nzGlyphSwapHoldsTheOutgoingGlyphForItsExit() {
        var glyph by mutableStateOf(NzGlyph.Play)
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                NzGlyphSwap(glyph = glyph, contentDescription = glyphLabel(glyph))
            }
        }
        rule.onNodeWithContentDescription("Lecture").assertExists()

        rule.mainClock.autoAdvance = false
        glyph = NzGlyph.Pause
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(30)
        // Two icons in the tree at 30 ms: the outgoing glyph is held for its
        // 90 ms exit (the content lambda is re-invoked with the NEW label, so
        // both read "Pause" during the swap).
        rule.onAllNodesWithContentDescription("Pause").assertCountEquals(2)
        rule.onNodeWithContentDescription("Lecture").assertDoesNotExist()
        rule.mainClock.advanceTimeBy(120)
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()
        rule.onAllNodesWithContentDescription("Pause").assertCountEquals(1)
    }

    private fun glyphLabel(glyph: NzGlyph): String = if (glyph == NzGlyph.Play) "Lecture" else "Pause"
}
