package fr.neezer.mobile.designsystem

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.TweenSpec
import androidx.compose.runtime.Immutable
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp

/**
 * Sillon's motion, with the eco/reduced guard (docs/agents/transitions.md, the
 * rules M1–M15 and `webapp/src/lib/motion.js`).
 *
 * The three levels, read ONCE per animation — exactly as `motion.js` reads
 * `level()`:
 *
 *  * [NzMotionLevel.Full] — the table as written;
 *  * [NzMotionLevel.Reduced] — the OS `prefers-reduced-motion`. Every
 *    translate/scale goes to 0, every duration is capped at 140 ms: reduce, keep
 *    the meaning (M14);
 *  * [NzMotionLevel.Off] — eco mode. `duration = 0`, no displacement, and a
 *    status indicator is the ONE thing that keeps moving (M13: a frozen spinner
 *    reads as a hang, and CLAUDE.md rule 2 wants a running job to say so).
 *
 * A component NEVER writes a duration: it asks the tokens. That is rule M15 on
 * the phone.
 */
enum class NzMotionLevel {
    Full,
    Reduced,
    Off,
    ;

    companion object {
        /**
         * What the device asks for: Android's animator duration scale is 0 when
         * « Supprimer les animations » is on, which is the platform's own
         * `prefers-reduced-motion`.
         */
        fun fromSystem(scale: Float): NzMotionLevel = if (scale <= 0f) Reduced else Full
    }
}

/** Millisecond durations — `motion.js#DUR`, the same table. */
object NzDur {
    const val Instant = 90
    const val Fast = 140
    const val Base = 200
    const val Sheet = 280
    const val SheetOut = 200
    const val Screen = 320
    const val ScreenOut = 240

    /** The cap reduced motion puts on every duration (M14). */
    const val ReducedCap = Fast
}

/** Pixels of travel — `motion.js#MOVE`. */
object NzMove {
    const val S = 4
    const val M = 8
}

/** `motion.js#SCALE`. */
object NzScale {
    const val Pop = 0.96f
    const val Dialog = 0.97f
    const val Screen = 0.98f
}

/**
 * The three curves, Material 3's (`motion.js#EASE_CSS`): enter decelerates,
 * exit accelerates, an in-place move is standard.
 */
object NzEase {
    val Enter: Easing = CubicBezierEasing(0.05f, 0.7f, 0.1f, 1f)
    val Exit: Easing = CubicBezierEasing(0.3f, 0f, 0.8f, 0.15f)
    val Standard: Easing = CubicBezierEasing(0.2f, 0f, 0f, 1f)
}

/**
 * The three specs a list row's insert / remove / reorder plays — what
 * `LazyItemScope.animateItem` takes, with the app's guard already applied.
 *
 * * [fadeIn] is **always `null`**: a windowed row never animates its mount (M4,
 *   `motion.js#shiftRows` — a mount plays nothing);
 * * [placement] is the 200 ms standard move at [NzMotionLevel.Full], `null` at
 *   reduced and eco (a move is a displacement, M14);
 * * [fadeOut] is the 140 ms exit at Full **and** reduced (M14 keeps opacity),
 *   `null` at eco (M13).
 */
@Immutable
data class NzItemSpecs(
    val fadeIn: FiniteAnimationSpec<Float>?,
    val placement: FiniteAnimationSpec<IntOffset>?,
    val fadeOut: FiniteAnimationSpec<Float>?,
)

/**
 * The motion tokens in force. Everything a component animates goes through this
 * object, so eco and reduced motion are one switch, not fifteen.
 */
@Immutable
class NzMotion(val level: NzMotionLevel = NzMotionLevel.Full) {
    /** A duration as the level allows it. */
    fun dur(ms: Int): Int = when (level) {
        NzMotionLevel.Full -> ms
        NzMotionLevel.Reduced -> minOf(ms, NzDur.ReducedCap)
        NzMotionLevel.Off -> 0
    }

    /** A hairline interaction: a hover, a press, a selection, a switch's thumb. */
    fun instant(easing: Easing = NzEase.Standard): FiniteAnimationSpec<Float> = tween(NzDur.Instant, easing)

    /** A small appearance: a glyph swap, a badge, a tab panel fade. */
    fun fast(easing: Easing = NzEase.Enter): FiniteAnimationSpec<Float> = tween(NzDur.Fast, easing)

    /** A page, a menu, a toast, a row shifting. */
    fun base(easing: Easing = NzEase.Enter): FiniteAnimationSpec<Float> = tween(NzDur.Base, easing)

    /** A sheet entering (phone). */
    fun sheet(): FiniteAnimationSpec<Float> = tween(NzDur.Sheet, NzEase.Enter)

    /** A sheet leaving: shorter than the entrance (M5). */
    fun sheetOut(): FiniteAnimationSpec<Float> = tween(NzDur.SheetOut, NzEase.Exit)

    /** The full-screen player entering. */
    fun screen(): FiniteAnimationSpec<Float> = tween(NzDur.Screen, NzEase.Enter)

    /** The full-screen player leaving (240 ms): `motion.js#screen`'s exit half. */
    fun screenOut(): FiniteAnimationSpec<Float> = tween(NzDur.ScreenOut, NzEase.Exit)

    /**
     * A DISPLACEMENT — a press scale, a knob's travel, a chevron's rotation: the
     * table at Full, and NOTHING at reduced or eco (M14: no displacement). A
     * colour or an opacity keeps [instant]/[fast], which may still walk in
     * reduced motion.
     */
    fun displace(ms: Int = NzDur.Instant, easing: Easing = NzEase.Standard): FiniteAnimationSpec<Float> =
        if (moves) tween(ms, easing) else tween(0)

    /** A displacement on an integer offset (a reorder, a segmented indicator). */
    fun displaceOffset(ms: Int = NzDur.Base, easing: Easing = NzEase.Standard): FiniteAnimationSpec<IntOffset> =
        if (moves) TweenSpec(durationMillis = dur(ms), easing = easing) else TweenSpec(0)

    /** An in-place move (an indicator, a reorder). */
    fun standard(): FiniteAnimationSpec<Float> = tween(NzDur.Base, NzEase.Standard)

    fun <T> tween(ms: Int, easing: Easing = NzEase.Enter): TweenSpec<T> =
        TweenSpec(durationMillis = dur(ms), easing = easing)

    /**
     * A press: `scale(0.98)` in, and nothing at all under eco or reduced motion
     * (M14 — a press scale is a displacement). Same rule as [displace].
     */
    fun pressScale(): FiniteAnimationSpec<Float> = displace(NzDur.Instant)

    /**
     * A list row's insert / remove / reorder, with the guard applied. Feed it to
     * `Modifier.animateItem(...)` at the call site (X5); this object never
     * animates a windowed row's mount (M4).
     */
    fun itemSpecs(): NzItemSpecs = NzItemSpecs(
        fadeIn = null,
        placement = if (moves) displaceOffset(NzDur.Base) else null,
        fadeOut = if (level == NzMotionLevel.Off) null else tween(NzDur.Fast, NzEase.Exit),
    )

    /** Is there any movement at all? (eco and reduced answer no — M14 removes displacement). */
    val moves: Boolean get() = level == NzMotionLevel.Full

    /**
     * May a STATUS indicator keep moving? Always: M13's one deliberate exception.
     * Eco stops every animation EXCEPT the ones marked as status, because a frozen
     * spinner reads as a hang and CLAUDE.md rule 2 wants a running job to say so.
     */
    val statusMoves: Boolean get() = true

    /** `--move-s`: a glyph or a title swap. */
    val moveS: Dp get() = if (moves) NzMove.S.dp else 0.dp

    /** `--move-m`: a page, a toast, a popover's offset. */
    val moveM: Dp get() = if (moves) NzMove.M.dp else 0.dp

    /** Menus and popovers pop from 0.96. */
    val scalePop: Float get() = if (moves) NzScale.Pop else 1f

    /** A centred dialog from 0.97. */
    val scaleDialog: Float get() = if (moves) NzScale.Dialog else 1f

    /** The full-screen player from 0.98. */
    val scaleScreen: Float get() = if (moves) NzScale.Screen else 1f

    /** A status indicator's loop, which eco keeps (M13) — nothing in eco becomes a hang. */
    fun statusSpec(periodMs: Int = 1000): FiniteAnimationSpec<Float> =
        TweenSpec(durationMillis = periodMs, easing = NzEase.Standard)

    override fun toString(): String = "NzMotion($level)"
}
