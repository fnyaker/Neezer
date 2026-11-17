package fr.neezer.mobile.designsystem

import android.os.Build
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * A blurred copy of what sits BEHIND a pane, where the platform can afford it.
 *
 * Compose has no `backdrop-filter`: it cannot sample the pixels under an
 * element. What it can do, from API 31 on, is blur a composable the caller gives
 * it — the sleeve, exactly what the SPA frosts — with `RenderEffect`. Below API
 * 31 nothing blurs and the pane wears its opaque fill instead (the app.css
 * ladder's own fallback), which is why [NzSheet] takes the picture as an
 * argument instead of pretending.
 */
@Composable
fun NzBlurredBackdrop(
    modifier: Modifier = Modifier,
    radius: Dp = Sillon.colors.glassBlur3,
    backdrop: @Composable () -> Unit,
) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        Box(modifier = modifier.blur(radius)) { backdrop() }
    } else {
        Box(modifier = modifier) { backdrop() }
    }
}

/**
 * A phone bottom sheet: the app's tier-3 surface.
 *
 * The scrim is the 60 % veil of `app.css` (no blur of its own — a blurred scrim
 * under a blurred pane was the SPA's ten copies of the same work), and the pane
 * wears [NzColors.glassBgStrong]: 93 % is the step that keeps an 11 px `--dim`
 * label above 4.8:1 even over a white-ish sleeve.
 *
 * Motion is `sheet` in / `sheetOut` out (280/200 ms), a translation with NO
 * opacity (M8: fading an ancestor of the glass kills its blur). Under eco and
 * reduced motion the pane appears at once: removing the slide would leave
 * nothing to play.
 *
 * @param backdrop the picture the pane frosts (a sleeve): drawn blurred under
 *   the fill on API ≥ 31 and left out below it.
 */
@Composable
fun NzSheet(
    visible: Boolean,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    backdrop: (@Composable () -> Unit)? = null,
    shape: Shape = RoundedCornerShape(topStart = NzRadius.card, topEnd = NzRadius.card),
    content: @Composable ColumnScope.() -> Unit,
) {
    val colors = Sillon.colors
    val motion = Sillon.motion
    val enter: EnterTransition = if (motion.moves) {
        slideInVertically(motion.tween(NzDur.Sheet, NzEase.Enter)) { height -> height }
    } else {
        EnterTransition.None
    }
    val exit: ExitTransition = if (motion.moves) {
        slideOutVertically(motion.tween(NzDur.SheetOut, NzEase.Exit)) { height -> height }
    } else {
        ExitTransition.None
    }
    Box(modifier = modifier.fillMaxSize()) {
        AnimatedVisibility(
            visible = visible,
            enter = fadeIn(motion.tween(NzDur.Fast)),
            exit = fadeOut(motion.tween(NzDur.Fast)),
        ) {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .background(colors.scrim)
                    // The veil takes the tap; it carries no semantics of its own.
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                        onClick = onDismiss,
                    )
                    .clearAndSetSemantics {},
            )
        }
        AnimatedVisibility(
            visible = visible,
            enter = enter,
            exit = exit,
            modifier = Modifier.align(Alignment.BottomCenter),
        ) {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 120.dp, max = 640.dp)
                    .clip(shape),
            ) {
                if (backdrop != null) {
                    NzBlurredBackdrop(modifier = Modifier.matchParentSize()) { backdrop() }
                }
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(colors.glassBgStrong)
                        .navigationBarsPadding()
                        .padding(NzSpacing.l),
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
                ) {
                    content()
                }
            }
        }
    }
}

/**
 * The sticky top bar: the ONE surface a row scrolls under, so it takes the thick
 * glass step (93 %, no blur — a static surface never blurs its own backdrop) and
 * a hairline of `--glass-border` under it.
 */
@Composable
fun NzTopBar(
    title: String,
    modifier: Modifier = Modifier,
    onBack: (() -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit = {},
) {
    val colors = Sillon.colors
    Column(modifier = modifier.fillMaxWidth().background(colors.glassBgStrong)) {
        Row(
            modifier = Modifier
                .statusBarsPadding()
                .height(56.dp)
                .fillMaxWidth()
                .padding(horizontal = NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            if (onBack != null) {
                NzIconButton(glyph = NzGlyph.ChevronLeft, contentDescription = "Retour", onClick = onBack)
            }
            Text(
                text = title,
                style = Sillon.type.titleSmall,
                color = colors.tx,
                modifier = Modifier.padding(start = if (onBack == null) NzSpacing.m else 0.dp),
            )
            androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))
            actions()
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(colors.glassBorder))
    }
}

/**
 * A card: the tier-1 glass surface (`--glass-bg`, 64 %, NO blur — it is static),
 * a 20 dp corner and a hairline. This is the block Réglages, Podcasts and every
 * grid are built from.
 */
@Composable
fun NzCard(
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.card)
    Box(
        modifier = modifier
            .clip(shape)
            .background(colors.glassBg)
            .then(
                if (onClick != null) {
                    Modifier.clickable(role = androidx.compose.ui.semantics.Role.Button, onClick = onClick)
                } else {
                    Modifier
                },
            )
            .padding(NzSpacing.m),
    ) {
        content()
    }
}
