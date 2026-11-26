package fr.neezer.mobile.feature.genres

import androidx.compose.animation.core.TweenSpec
import androidx.compose.animation.core.FiniteAnimationSpec
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzEase
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The genre tree's chevron TURNS, and it turns with `displace()` (lot X5).
 *
 * The row draws ONE glyph (`ChevronRight`) and rotates it to 90° when the branch
 * is open — the SPA's 200 ms standard (`ChevronRight → 90°`, T6e), instead of
 * swapping `ChevronRight` for `ChevronDown` in a single frame.
 *
 * **What this holds, and the honest limit.** The ANGLE is a `graphicsLayer`
 * rotation, invisible to `getUnclippedBoundsInRoot`, so its picture is the
 * golden's (T3's `genres-arbre-*`, unchanged at rest because 90° of a chevron is
 * the same shape). What a test CAN hold is the spec the row passes: [chevronSpec]
 * is the function `TreeRow` calls, and it is `displace(NzDur.Base)` at Full, 0 ms
 * under reduced motion and at eco (M14: a turn is a displacement). Asserted on
 * the tokens, exactly as X0's `TransitionTest` holds its presets.
 */
class GenresChevronTest {

    private fun tween(spec: FiniteAnimationSpec<Float>): TweenSpec<Float> {
        require(spec is TweenSpec<Float>) { "expected a tween, got $spec" }
        return spec
    }

    @Test
    fun theChevronTurnsOverTheStandardDisplacementAtFull() {
        val spec = chevronSpec(NzMotion(NzMotionLevel.Full))
        assertEquals("the SPA's 200 ms", NzDur.Base, tween(spec).durationMillis)
        assertEquals("a displacement walks on the standard curve", NzEase.Standard, tween(spec).easing)
        assertEquals(
            "the same spec as the token the design system publishes",
            tween(NzMotion(NzMotionLevel.Full).displace(NzDur.Base, NzEase.Standard)).durationMillis,
            tween(spec).durationMillis,
        )
    }

    @Test
    fun theChevronDoesNotMoveUnderReducedMotionNorAtEco() {
        for (level in listOf(NzMotionLevel.Reduced, NzMotionLevel.Off)) {
            val spec = chevronSpec(NzMotion(level))
            assertEquals("$level: no displacement (M14)", 0, tween(spec).durationMillis)
        }
    }
}
