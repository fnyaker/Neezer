package fr.neezer.mobile.designsystem

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

// -- toasts -------------------------------------------------------------------

/** One toast: a sentence, and at most one action (« Réessayer »), as the SPA's `Toasts`. */
@Immutable
data class NzToast(
    val id: Long,
    val text: String,
    val actionLabel: String? = null,
    val onAction: (() -> Unit)? = null,
)

/**
 * The app's ONE toast queue (`webapp.md` §8: a `--tx` pill 2.4 s above the
 * player, `role="status"`).
 *
 * It is an object, not a state a screen owns, because a toast is raised by
 * whatever failed — a repository, a ViewModel, a swipe — and must be readable
 * from anywhere. At most five are kept (the SPA's cap): a burst of failures must
 * not paper over the screen.
 */
object NzToasts {
    /** How long a toast stays (the SPA's 2.4 s). */
    const val LingerMs = 2400L

    /** At most five at once. */
    const val Max = 5

    private var nextId = 0L

    /** The live list. Compose observes it; a screen never mutates it. */
    val toasts = mutableStateListOf<NzToast>()

    /** Raise a toast. [text] is a French sentence with its cause, never a code. */
    fun show(text: String, actionLabel: String? = null, onAction: (() -> Unit)? = null): NzToast {
        val toast = NzToast(++nextId, text, actionLabel, onAction)
        toasts.add(toast)
        while (toasts.size > Max) toasts.removeAt(0)
        return toast
    }

    fun dismiss(id: Long) {
        toasts.removeAll { it.id == id }
    }

    fun clear() {
        toasts.clear()
    }
}

/**
 * Where toasts are drawn: the bottom of the screen, above the mini player, in
 * the flow of whatever screen hosts it. Each one rises 8 dp and fades in over
 * `--dur-base` (140 ms out), and is removed after [NzToasts.LingerMs] by an
 * effect keyed on its id — never by a screen's timer.
 */
@Composable
fun NzToastHost(modifier: Modifier = Modifier, bottomPadding: Dp = 96.dp) {
    val colors = Sillon.colors
    val motion = Sillon.motion
    Box(modifier = modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .padding(horizontal = NzSpacing.l, vertical = bottomPadding),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s, Alignment.Bottom),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            NzToasts.toasts.forEach { toast ->
                key(toast.id) {
                    LaunchedEffect(toast.id) {
                        delay(NzToasts.LingerMs)
                        NzToasts.dismiss(toast.id)
                    }
                    AnimatedVisibility(
                        visible = true,
                        enter = fadeIn(motion.tween(NzDur.Base, NzEase.Enter)) +
                            slideInVertically(motion.tween(NzDur.Base, NzEase.Enter)) { it / 2 },
                        exit = fadeOut(motion.tween(NzDur.Fast, NzEase.Exit)) +
                            slideOutVertically(motion.tween(NzDur.Fast, NzEase.Exit)) { it / 2 },
                    ) {
                        Row(
                            modifier = Modifier
                                .clip(RoundedCornerShape(NzRadius.pill))
                                .background(colors.tx)
                                .padding(horizontal = NzSpacing.l, vertical = NzSpacing.s)
                                .semantics { liveRegion = LiveRegionMode.Polite },
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
                        ) {
                            Text(text = toast.text, style = Sillon.type.bodySmall, color = colors.bg)
                            val actionLabel = toast.actionLabel
                            val onAction = toast.onAction
                            if (actionLabel != null && onAction != null) {
                                Box(
                                    modifier = Modifier
                                        .sizeIn(minWidth = NzMinTouchTarget, minHeight = NzMinTouchTarget)
                                        .clickableNoIndication {
                                            NzToasts.dismiss(toast.id)
                                            onAction()
                                        },
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Text(text = actionLabel, style = Sillon.type.label, color = colors.acText)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// -- skeletons ----------------------------------------------------------------

/** The three skeletons the SPA draws (`Skeleton.svelte`). */
enum class NzSkeletonKind { Shelf, List, Header }

/**
 * A loading placeholder. The shimmer is DECORATIVE (`Skeleton.svelte`, F7): it
 * says "something is coming", it does not claim a job is running. So eco AND
 * reduced motion both get a still `--s2` block — a stopped gradient caught
 * mid-walk reads as a frozen bar.
 */
@Composable
fun Skeleton(
    modifier: Modifier = Modifier,
    kind: NzSkeletonKind = NzSkeletonKind.Shelf,
    count: Int = 6,
) {
    val colors = Sillon.colors
    val motion = Sillon.motion
    val shimmer = motion.level == NzMotionLevel.Full
    val transition = if (shimmer) rememberInfiniteTransition(label = "nz-skeleton") else null
    val progress = transition?.animateFloat(
        initialValue = 1f,
        targetValue = -1f,
        animationSpec = infiniteRepeatable(androidx.compose.animation.core.tween(1400, easing = LinearEasing), RepeatMode.Restart),
        label = "nz-skeleton-walk",
    )?.value ?: 0f
    fun brush(): Brush = if (shimmer) {
        Brush.linearGradient(
            colors = listOf(colors.s2, nzMix(colors.tx, colors.s2, 0.18f), colors.s2),
            start = Offset(progress * 1000f, 0f),
            end = Offset(progress * 1000f + 600f, 0f),
        )
    } else {
        Brush.linearGradient(listOf(colors.s2, colors.s2))
    }
    when (kind) {
        NzSkeletonKind.Header -> Row(
            modifier = modifier.fillMaxWidth().padding(vertical = NzSpacing.l),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xl),
            verticalAlignment = Alignment.Bottom,
        ) {
            Box(Modifier.size(120.dp).clip(RoundedCornerShape(NzRadius.field)).background(brush()))
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                Box(Modifier.width(120.dp).height(12.dp).clip(RoundedCornerShape(NzRadius.field / 2)).background(brush()))
                Box(Modifier.width(200.dp).height(28.dp).clip(RoundedCornerShape(NzRadius.field / 2)).background(brush()))
                Box(Modifier.width(140.dp).height(12.dp).clip(RoundedCornerShape(NzRadius.field / 2)).background(brush()))
            }
        }
        NzSkeletonKind.List -> Column(
            modifier = modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            repeat(count) {
                Row(
                    modifier = Modifier.fillMaxWidth().height(44.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
                ) {
                    Box(Modifier.size(40.dp).clip(RoundedCornerShape(NzRadius.field)).background(brush()))
                    Box(Modifier.weight(1f).height(12.dp).clip(RoundedCornerShape(NzRadius.field / 2)).background(brush()))
                    Box(Modifier.width(40.dp).height(12.dp).clip(RoundedCornerShape(NzRadius.field / 2)).background(brush()))
                }
            }
        }
        NzSkeletonKind.Shelf -> Row(
            modifier = modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            repeat(count) {
                Column(
                    modifier = Modifier.width(120.dp),
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
                ) {
                    Box(Modifier.size(120.dp).clip(RoundedCornerShape(NzRadius.cover)).background(brush()))
                    Box(Modifier.fillMaxWidth(0.6f).height(12.dp).clip(RoundedCornerShape(NzRadius.field / 2)).background(brush()))
                    Box(Modifier.fillMaxWidth(0.4f).height(12.dp).clip(RoundedCornerShape(NzRadius.field / 2)).background(brush()))
                }
            }
        }
    }
}

// -- a failed load ------------------------------------------------------------

/**
 * What a screen shows when its load failed (`LoadFailed.svelte`, rule 3): the
 * CAUSE in a French sentence, and « Réessayer ». Never a spinner, never a count,
 * never colour alone.
 *
 * The block replaces whatever was in the flow at once (M7) and fades in with a
 * 4 dp rise; under eco it is simply there.
 */
@Composable
fun LoadFailed(
    cause: String,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
    title: String = "Chargement impossible",
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.card))
            .background(colors.s1)
            .padding(NzSpacing.xl)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(glyph = NzGlyph.CloudOff, size = 28.dp, tint = colors.dim)
        Text(text = title, style = Sillon.type.titleSmall, color = colors.tx, textAlign = TextAlign.Center)
        Text(
            text = "Cette page n'a pas pu être chargée : $cause.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
            textAlign = TextAlign.Center,
        )
        NzButton(text = "Réessayer", onClick = onRetry, glyph = NzGlyph.Refresh, kind = NzButtonKind.Secondary)
    }
}

// -- connectivity -------------------------------------------------------------

/** What the connectivity notice says (the SPA's two transient states). */
enum class NzNetwork { Offline, Reconnected, Metered }

/**
 * The connectivity pill (`NetworkIndicator.svelte`): transient on purpose — a
 * permanent alarming banner would be wrong when the app keeps working offline
 * (downloads, cached pages). The glyph carries the meaning as much as the words.
 */
@Composable
fun NetworkIndicator(state: NzNetwork, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    val (glyph, text) = when (state) {
        NzNetwork.Offline -> NzGlyph.CloudOff to "Hors ligne — lecture locale disponible"
        NzNetwork.Reconnected -> NzGlyph.Check to "De nouveau en ligne"
        NzNetwork.Metered -> NzGlyph.CloudOff to "Connexion limitée — téléchargements en pause"
    }
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.pill))
            .background(colors.s2)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = glyph, size = 15.dp, tint = colors.tx)
        Text(text = text, style = Sillon.type.labelSmall, color = colors.tx)
    }
}

// -- nothing here -------------------------------------------------------------

/**
 * An empty state: a drawing (the Sillon groove, never a blank page), a sentence
 * and an action — `webapp.md` §6 asks for all three on every empty list.
 */
@Composable
fun EmptyState(
    title: String,
    body: String,
    modifier: Modifier = Modifier,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
    glyph: NzGlyph = NzGlyph.Disc,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(NzSpacing.xl),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            modifier = Modifier
                .size(88.dp)
                .clip(CircleShape)
                .border(1.dp, colors.ln, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Canvas(Modifier.size(88.dp)) {
                val center = Offset(size.width / 2f, size.height / 2f)
                var radius = size.minDimension * 0.12f
                while (radius < size.minDimension / 2f) {
                    drawCircle(color = colors.ln, radius = radius, center = center, style = androidx.compose.ui.graphics.drawscope.Stroke(1f))
                    radius += 6.dp.toPx()
                }
            }
            Icon(glyph = glyph, size = 28.dp, tint = colors.dim)
        }
        Text(text = title, style = Sillon.type.titleSmall, color = colors.tx, textAlign = TextAlign.Center)
        Text(text = body, style = Sillon.type.bodySmall, color = colors.dim, textAlign = TextAlign.Center)
        if (actionLabel != null && onAction != null) {
            Spacer(Modifier.height(NzSpacing.xs))
            NzButton(text = actionLabel, onClick = onAction)
        }
    }
}

/** A click with no Material ripple: Sillon answers with colour and scale, not a grey wash. */
internal fun Modifier.clickableNoIndication(onClick: () -> Unit): Modifier =
    this.clickable(
        interactionSource = MutableInteractionSource(),
        indication = null,
        onClick = onClick,
    )
