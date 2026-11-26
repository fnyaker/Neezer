package fr.neezer.mobile.designsystem

import androidx.compose.animation.core.TweenSpec
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The eco/reduced guard, as numbers (docs/agents/transitions.md, M13/M14/M15).
 *
 * The SPA proves the same table two ways: `test/motion.test.mjs` fails when
 * `lib/motion.js` and `app.css` disagree, and the Chromium bench measures the
 * result. Here the tokens themselves are asserted, because a Compose animation
 * cannot be observed without a clock and worker rule 6 forbids waiting on one.
 */
class MotionTest {

    private fun duration(spec: Any?): Int = (spec as TweenSpec<*>).durationMillis

    @Test
    fun ecoStopsEverything() {
        val eco = NzMotion(NzMotionLevel.Off)
        assertEquals(0, duration(eco.instant()))
        assertEquals(0, duration(eco.fast()))
        assertEquals(0, duration(eco.base()))
        assertEquals(0, duration(eco.sheet()))
        assertEquals(0, duration(eco.sheetOut()))
        assertEquals(0, duration(eco.screen()))
        assertEquals(0, duration(eco.standard()))
        assertEquals(0, eco.moveS.value.toInt())
        assertEquals(0, eco.moveM.value.toInt())
        assertEquals(1f, eco.scalePop)
        assertEquals(1f, eco.scaleDialog)
        assertEquals(1f, eco.scaleScreen)
        assertFalse("eco has no displacement at all", eco.moves)
    }

    @Test
    fun ecoKeepsAStatusIndicatorMoving() {
        // M13's one exception: a frozen spinner reads as a hang, and CLAUDE.md
        // rule 2 wants a running job to say so. Everything else is still.
        val eco = NzMotion(NzMotionLevel.Off)
        assertTrue("a status indicator is the ONE thing eco keeps", eco.statusMoves)
        assertEquals(
            "a status loop keeps its real period in eco (a frozen spinner reads as a hang)",
            1000,
            duration(eco.statusSpec(1000)),
        )
        assertFalse("everything else is still", eco.moves)
    }

    @Test
    fun reducedKeepsFadesAndDropsDisplacement() {
        val reduced = NzMotion(NzMotionLevel.Reduced)
        assertEquals(NzDur.ReducedCap, duration(reduced.fast()))
        assertEquals("a longer walk is capped at 140 ms", NzDur.ReducedCap, duration(reduced.sheet()))
        assertEquals(NzDur.ReducedCap, duration(reduced.screen()))
        assertEquals(NzDur.ReducedCap, duration(reduced.base()))
        assertEquals("an instant interaction keeps its 90 ms", NzDur.Instant, duration(reduced.instant()))
        assertEquals(0, reduced.moveM.value.toInt())
        assertEquals(1f, reduced.scalePop)
        assertFalse(reduced.moves)
    }

    @Test
    fun fullIsTheSpaTable() {
        val full = NzMotion(NzMotionLevel.Full)
        // `webapp/src/lib/motion.js#DUR`, digit for digit.
        assertEquals(90, duration(full.instant()))
        assertEquals(140, duration(full.fast()))
        assertEquals(200, duration(full.base()))
        assertEquals(280, duration(full.sheet()))
        assertEquals(200, duration(full.sheetOut()))
        assertEquals(320, duration(full.screen()))
        assertEquals(4, full.moveS.value.toInt())
        assertEquals(8, full.moveM.value.toInt())
        assertEquals(0.96f, full.scalePop)
        assertEquals(0.97f, full.scaleDialog)
        assertEquals(0.98f, full.scaleScreen)
        assertTrue(full.moves)
    }

    @Test
    fun theSystemScaleIsTheReducedSignal() {
        assertEquals(NzMotionLevel.Reduced, NzMotionLevel.fromSystem(0f))
        assertEquals(NzMotionLevel.Full, NzMotionLevel.fromSystem(1f))
        assertEquals(NzMotionLevel.Full, NzMotionLevel.fromSystem(0.5f))
    }

    @Test
    fun anExitIsShorterThanItsEntrance() {
        // M5: an exit is ~70 % of the entrance.
        assertTrue(NzDur.SheetOut < NzDur.Sheet)
        assertTrue(NzDur.ScreenOut < NzDur.Screen)
        assertTrue(NzDur.Fast < NzDur.Base)
    }

    @Test
    fun displacedThingsStopUnderReducedAndEco() {
        val full = NzMotion(NzMotionLevel.Full)
        assertEquals("a press lives 90 ms at Full", NzDur.Instant, duration(full.displace()))
        assertEquals(
            "M14: a displacement is gone under reduced motion (0 ms, not a short walk)",
            0,
            duration(NzMotion(NzMotionLevel.Reduced).displace()),
        )
        assertEquals(0, duration(NzMotion(NzMotionLevel.Off).displace()))
        assertEquals("a reorder lives 200 ms at Full", NzDur.Base, duration(full.displaceOffset()))
        assertEquals(
            "M14: a reorder is a displacement too",
            0,
            duration(NzMotion(NzMotionLevel.Reduced).displaceOffset()),
        )
        assertEquals(0, duration(NzMotion(NzMotionLevel.Off).displaceOffset()))
    }

    @Test
    fun screenOutIsTheExitHalfOfTheTable() {
        assertEquals(240, duration(NzMotion(NzMotionLevel.Full).screenOut()))
        assertEquals(NzDur.ReducedCap, duration(NzMotion(NzMotionLevel.Reduced).screenOut()))
        assertEquals(0, duration(NzMotion(NzMotionLevel.Off).screenOut()))
    }

    @Test
    fun itemSpecsAreTheTableWithTheGuard() {
        val full = NzMotion(NzMotionLevel.Full).itemSpecs()
        assertEquals("M4: a windowed row never animates its mount", null, full.fadeIn)
        assertEquals("M5: a row moves on the 200 ms standard walk", NzDur.Base, duration(full.placement))
        assertEquals(NzEase.Standard, (full.placement as TweenSpec<*>).easing)
        assertEquals("M5: the explicit removal keeps the 140 ms exit", NzDur.Fast, duration(full.fadeOut))

        val reduced = NzMotion(NzMotionLevel.Reduced).itemSpecs()
        assertEquals(null, reduced.fadeIn)
        assertEquals("M14: a move is a displacement, so it is gone", null, reduced.placement)
        assertEquals("M14 keeps opacity: the removal still fades", NzDur.Fast, duration(reduced.fadeOut))

        val eco = NzMotion(NzMotionLevel.Off).itemSpecs()
        assertEquals(null, eco.fadeIn)
        assertEquals(null, eco.placement)
        assertEquals("M13: eco removes the fade too", null, eco.fadeOut)
    }

    @Test
    fun pressScaleKeepsMeaningWhatItsKdocSays() {
        assertEquals(NzDur.Instant, duration(NzMotion(NzMotionLevel.Full).pressScale()))
        assertEquals(0, duration(NzMotion(NzMotionLevel.Reduced).pressScale()))
        assertEquals(0, duration(NzMotion(NzMotionLevel.Off).pressScale()))
    }
}
