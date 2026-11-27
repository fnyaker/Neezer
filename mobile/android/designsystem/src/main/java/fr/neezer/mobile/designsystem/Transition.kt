package fr.neezer.mobile.designsystem

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.ContentTransform
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.AnimationVector1D
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.TweenSpec
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp

/**
 * The three presets the screens use. A screen names the STATE, never a duration
 * (M15); every spec here comes from [NzMotion].
 *
 * The levels are declared at the site that knows what changed (a phase, an
 * element in flow, a glyph), never by one wrapper around the app: a global
 * "animate everything" cannot tell a windowed row's mount (M4) from a
 * replace-in-place (M7). `docs/agents/transitions.md`, *Le téléphone*, holds the
 * mapping and the limits of what a Compose test can assert.
 *
 * **The honest limit.** Compose exposes no API for a RUNNING animation's spec,
 * alpha or transform: `getUnclippedBoundsInRoot` is a layout bound and a
 * `graphicsLayer` translation is invisible to it, so `document.getAnimations()`
 * has no native counterpart. The curve is therefore held by the exact specs
 * [inPlaceSpecs] / [appearSpecs] / [glyphSpecs] return (a unit test reads them),
 * and the wiring by the semantics tree of the composables below. No test here
 * claims to "see" an animation; the emulator pictures at 50 % do that.
 */

/** The one literal this file is allowed: `motion.js#swap`'s glyph pop. */
private const val GLYPH_SCALE = 0.85f

/**
 * The specs one preset plays, public so a test can assert the three levels.
 *
 * [move] is `null` when the level has no displacement (reduced/eco), so a caller
 * passes `slideInVertically(null)`-free code; [exit] is `null` for every preset
 * here because an in-flow replacement has **no** exit (M7) — the field stays so a
 * future overlay preset can fill it.
 */
@Immutable
class NzPreset(
    val fade: FiniteAnimationSpec<Float>,
    val move: FiniteAnimationSpec<IntOffset>?,
    val exit: FiniteAnimationSpec<Float>?,
) {
    /**
     * True at eco: every spec is 0 ms, so the composables pass
     * `EnterTransition.None` instead of an empty fade (M13).
     */
    val isNone: Boolean
        get() = (fade as? TweenSpec<*>)?.durationMillis == 0 && move == null && exit == null
}

/**
 * A screen's phase replaced in place (skeleton → content → LoadFailed → empty):
 * A goes at once, B enters with a fade and 4 dp of rise (M7, M6: "le plus
 * fréquent bouge le moins" — the 140 ms fade, not the 200 ms page enter).
 */
fun inPlaceSpecs(motion: NzMotion): NzPreset = NzPreset(
    fade = motion.fast(),
    move = if (motion.moves) TweenSpec(NzDur.Fast, easing = NzEase.Standard) else null,
    exit = null,
)

/**
 * An element that appears IN FLOW, enter only (M7): the same specs as
 * [inPlaceSpecs] — a fade plus 4 dp — and no exit at all, so whatever it replaces
 * keeps its place in the same frame.
 */
fun appearSpecs(motion: NzMotion): NzPreset = inPlaceSpecs(motion)

/**
 * Play↔pause, heart↔heart-filled: 140 ms opacity + `scale .85 → 1`, in only.
 * The scale lives inside the composable's [EnterTransition] (not a readable
 * spec), which is why the screenshot, not this class, proves the 0.85.
 */
fun glyphSpecs(motion: NzMotion): NzPreset = NzPreset(
    fade = motion.fast(),
    move = null,
    exit = null,
)

/**
 * A screen's phase replaced in place: skeleton → content, LoadFailed → content,
 * empty → list. A disappears in the same frame; B enters with a fade and 4 dp of
 * rise (M7, M5: the exit is `None`, never a layout shift).
 *
 * `sizeTransform = null` always: a size walk is one layout per frame and clips
 * (M10), and without it `AnimatedContent` would animate the height of every
 * phase change — the single most important line in this file.
 *
 * @param targetState the phase; `contentKey = { it }` makes a re-emit of the same
 *   phase play nothing (M6).
 */
@Composable
fun NzSwap(
    targetState: Any,
    modifier: Modifier = Modifier,
    label: String,
    content: @Composable (Any) -> Unit,
) {
    val motion = Sillon.motion
    val preset = inPlaceSpecs(motion)
    // A transition lambda is not composable: read the density first.
    val swapPx = with(LocalDensity.current) { motion.moveS.roundToPx() }
    AnimatedContent(
        targetState = targetState,
        modifier = modifier,
        transitionSpec = {
            // sizeTransform = null always: a size walk is one layout per frame
            // and clips (M10). The public `togetherWith` hard-codes a
            // SizeTransform, so the transform is built directly.
            val move = preset.move
            val enter = when {
                preset.isNone -> EnterTransition.None
                move != null -> fadeIn(preset.fade) + slideInVertically(move) { swapPx }
                else -> fadeIn(preset.fade)
            }
            ContentTransform(enter, ExitTransition.None, 0f, null)
        },
        contentKey = { it },
        label = label,
    ) { state ->
        content(state)
    }
}

/**
 * An element that appears IN FLOW, enter only: a toast's body, a details block,
 * a "rien en lecture" line. Whatever it replaces is laid out in the same frame —
 * an exit on an in-flow element would keep A's height while B enters (M7).
 */
@Composable
fun NzAppear(
    visible: Boolean,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    val motion = Sillon.motion
    val preset = appearSpecs(motion)
    val swapPx = with(LocalDensity.current) { motion.moveS.roundToPx() }
    val move = preset.move
    val enter: EnterTransition = when {
        preset.isNone -> EnterTransition.None
        move != null -> fadeIn(preset.fade) + slideInVertically(move) { swapPx }
        else -> fadeIn(preset.fade)
    }
    AnimatedVisibility(
        visible = visible,
        modifier = modifier,
        enter = enter,
        exit = ExitTransition.None,
    ) {
        content()
    }
}

/**
 * A glyph that replaces another (play↔pause, heart↔heart-filled): 140 ms opacity
 * and `scale .85 → 1` in, a 90 ms fade out. The 0.85 is the plan's one allowed
 * literal, because `motion.js#swap`'s default is 1 and its call sites pass 0.85
 * (`PlayerBar.svelte:135`).
 *
 * @param contentDescription the label a reader announces; `null` for a glyph
 *   that only decorates text.
 */
@Composable
fun NzGlyphSwap(
    glyph: NzGlyph,
    contentDescription: String?,
    modifier: Modifier = Modifier,
    size: Dp = 22.dp,
    tint: Color = LocalNzColors.current.tx,
) {
    val motion = Sillon.motion
    val preset = glyphSpecs(motion)
    AnimatedContent(
        targetState = glyph,
        modifier = modifier,
        transitionSpec = {
            val enter: EnterTransition = when {
                preset.isNone -> EnterTransition.None
                motion.moves -> fadeIn(preset.fade) + scaleIn(preset.fade, initialScale = GLYPH_SCALE)
                else -> fadeIn(preset.fade)
            }
            val exit: ExitTransition = if (preset.isNone) ExitTransition.None else fadeOut(motion.instant())
            ContentTransform(enter, exit, 0f, null)
        },
        label = "nz-glyph-swap",
    ) { target ->
        Icon(
            glyph = target,
            size = size,
            tint = tint,
            contentDescription = contentDescription,
        )
    }
}

/**
 * The panel of a `LazyColumn` that replaces another (a library leaf, an artist
 * tab, a phase): [NzSwap] cannot wrap lazy items, so every item of the panel
 * carries [nzPaneEnter] and reads one shared progress. Same specs as
 * [inPlaceSpecs]: 140 ms fade + 4 dp rise at Full, fade only at Reduced, nothing
 * at Off. The first composition does not animate (the screen's own enter, X1,
 * covers it); only a CHANGE of the key does.
 *
 * Why a modifier and not a wrapper: the body of a pane here is a set of
 * `LazyListScope` items, and an `AnimatedContent` around a `LazyColumn` would
 * fade the page's head and its tab strip with it (and re-enter the artist's band
 * on every tab tap). A `graphicsLayer` read in the draw phase also costs no
 * recomposition and no re-layout per frame (M10).
 */
@Stable
class NzPaneEnter internal constructor(
    private val anim: Animatable<Float, AnimationVector1D>,
    internal val risePx: Float,
) {
    /** 0 at the start of a pane change, 1 once it has landed. */
    val progress: Float get() = anim.value
}

/**
 * The shared progress of a pane, keyed on what replaced what (`leaf to phase`,
 * `tab to phase`, `phase`).
 *
 * The new panel's FIRST frame is already at 0, so nothing flashes at full opacity
 * for one frame: `remember(key)` builds the new [Animatable] in the very
 * composition that sees the new key.
 */
@Composable
fun rememberPaneEnter(key: Any): NzPaneEnter {
    val motion = Sillon.motion
    val preset = inPlaceSpecs(motion)
    val risePx = with(LocalDensity.current) { motion.moveS.toPx() }
    val first = remember { PaneKeyHolder(key) }
    if (key != first.initial) first.changed = true
    // remember(key) builds the new Animatable IN the composition that sees the new key, so the
    // new panel's first frame is already at 0 (no one-frame flash at full opacity).
    val anim = remember(key) { Animatable(if (first.changed && !preset.isNone) 0f else 1f) }
    LaunchedEffect(key) { if (anim.value < 1f) anim.animateTo(1f, preset.fade) }
    return remember(anim, risePx) { NzPaneEnter(anim, risePx) }
}

/** What the pane's first key was, and whether it has moved since. */
private class PaneKeyHolder(val initial: Any) {
    var changed: Boolean = false
}

/**
 * The pane's entrance, drawn from [NzPaneEnter]'s progress. Read in the draw
 * layer only: a running enter never recomposes nor re-lays out a row.
 */
fun Modifier.nzPaneEnter(pane: NzPaneEnter): Modifier = graphicsLayer {
    val p = pane.progress
    alpha = p
    translationY = (1f - p) * pane.risePx
}

/**
 * An element that fades in when it is ADDED to the tree (an empty note), enter
 * only. [NzAppear] cannot do it: `AnimatedVisibility(Boolean)` shows its content
 * WITHOUT animation on first composition. At Off the content is present in
 * frame 0.
 */
@Composable
fun NzAppearOnMount(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val motion = Sillon.motion
    val preset = appearSpecs(motion)
    val swapPx = with(LocalDensity.current) { motion.moveS.roundToPx() }
    val move = preset.move
    val enter: EnterTransition = when {
        preset.isNone -> EnterTransition.None
        move != null -> fadeIn(preset.fade) + slideInVertically(move) { swapPx }
        else -> fadeIn(preset.fade)
    }
    // MutableTransitionState, not a Boolean: it is what starts the enter on the
    // frame the composable joins the tree (Android's Animations in Compose guide).
    val state = remember { MutableTransitionState(preset.isNone) }.apply { targetState = true }
    AnimatedVisibility(
        visibleState = state,
        modifier = modifier,
        enter = enter,
        exit = ExitTransition.None,
    ) {
        content()
    }
}
