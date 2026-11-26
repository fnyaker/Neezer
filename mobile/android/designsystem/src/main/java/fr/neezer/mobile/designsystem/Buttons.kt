package fr.neezer.mobile.designsystem

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * A button's weight. Sillon has three:
 *
 *  * [Primary] — the action a screen is for (« Lire », « Lancer mon Flow »): the
 *    `--btn` surface with `--onbtn` text, i.e. lemon on Encre and INK on Papier
 *    (the lemon is a highlighter there, never a button);
 *  * [Secondary] — a normal action beside it: `--s2` with the theme's ink;
 *  * [Ghost] — a tertiary action that must not compete: no surface at all.
 */
enum class NzButtonKind { Primary, Secondary, Ghost }

@Composable
private fun nzButtonColors(kind: NzButtonKind): Pair<Color, Color> {
    val c = Sillon.colors
    return when (kind) {
        NzButtonKind.Primary -> c.btn to c.onbtn
        NzButtonKind.Secondary -> c.s2 to c.tx
        NzButtonKind.Ghost -> Color.Transparent to c.tx
    }
}

/**
 * The app's button: a pill, 48 dp tall (the acceptance's touch target), 18 dp of
 * side padding, a 13.5 px 600 label and — optionally — one glyph.
 *
 * It answers the press in the SAME frame (M2): the scale and the tint animate
 * on top of the state change, and under eco or reduced motion the scale is 0, so
 * nothing moves and the colour still says "pressed".
 */
@Composable
fun NzButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: NzButtonKind = NzButtonKind.Primary,
    glyph: NzGlyph? = null,
    enabled: Boolean = true,
) {
    NzButton(onClick = onClick, modifier = modifier, kind = kind, glyph = glyph, enabled = enabled) {
        // One line, always: a wrapped label made a three-button row look broken
        // at 360 dp (measured), and a pill that grows to two lines is a bug.
        Text(text, style = Sillon.type.label, maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis)
    }
}

/** The same button around whatever the caller composes (a glyph and a count). */
@Composable
fun NzButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: NzButtonKind = NzButtonKind.Primary,
    glyph: NzGlyph? = null,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) {
    val motion = Sillon.motion
    val (surface, ink) = nzButtonColors(kind)
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val scale by animateFloatAsState(
        targetValue = if (pressed && enabled) 0.98f else 1f,
        animationSpec = motion.displace(),
        label = "nz-button-press",
    )
    // A disabled control is a WELL, not a faded surface: half-transparent lemon
    // on cream read as a bug (seen in the first screenshots), and `--s2`/`--dim`
    // is the pair the SPA uses for "there, but not now".
    val disabledFill = Sillon.colors.s2
    val disabledInk = Sillon.colors.dim
    val fill = when {
        !enabled -> disabledFill
        pressed && kind != NzButtonKind.Primary -> nzMix(ink.copy(alpha = 0.10f), surface, 1f)
        else -> surface
    }
    val labelInk = if (enabled) ink else disabledInk
    Row(
        modifier = modifier
            .scale(scale)
            .sizeIn(minHeight = NzMinTouchTarget, minWidth = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.pill))
            .background(fill)
            .then(
                if (kind == NzButtonKind.Secondary) Modifier.border(1.dp, Sillon.colors.ln, RoundedCornerShape(NzRadius.pill))
                else Modifier,
            )
            .clickable(
                interactionSource = interaction,
                indication = null,
                enabled = enabled,
                role = Role.Button,
                onClick = onClick,
            )
            .padding(horizontal = NzSpacing.l),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s, Alignment.CenterHorizontally),
    ) {
        if (glyph != null) {
            Icon(glyph = glyph, size = 18.dp, tint = labelInk)
        }
        // The label takes the button's own pair. MEASURED on the screenshots:
        // without this, a Text inherits Material's `onSurface` (the theme's ink),
        // and on Papier that painted ink on ink — an invisible « Lire » — while
        // on Encre it painted cream on the lemon (1.16:1).
        CompositionLocalProvider(LocalContentColor provides labelInk) {
            content()
        }
    }
}

/**
 * An icon-only control: 48 × 48 dp of target, one glyph of 20, and a
 * `contentDescription` that is NOT optional — an unlabelled icon control is a
 * bug (`webapp.md` §10).
 */
@Composable
fun NzIconButton(
    glyph: NzGlyph,
    contentDescription: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 20.dp,
    tint: Color? = null,
    enabled: Boolean = true,
    selected: Boolean = false,
) {
    val motion = Sillon.motion
    val colors = Sillon.colors
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val scale by animateFloatAsState(
        targetValue = if (pressed && enabled) 0.92f else 1f,
        animationSpec = motion.displace(),
        label = "nz-icon-button-press",
    )
    val ink = when {
        !enabled -> colors.dim
        selected -> colors.acText
        else -> tint ?: colors.tx
    }
    androidx.compose.foundation.layout.Box(
        modifier = modifier
            .scale(scale)
            .sizeIn(minWidth = NzMinTouchTarget, minHeight = NzMinTouchTarget)
            .clip(CircleShape)
            .clickable(
                interactionSource = interaction,
                indication = null,
                enabled = enabled,
                role = Role.Button,
                onClick = onClick,
            ),
        contentAlignment = Alignment.Center,
    ) {
        Icon(glyph = glyph, size = size, tint = ink, contentDescription = contentDescription)
    }
}

/**
 * A chip: a filter or a choice. **Selected is `--ac`/`--onac`** — a surface, not
 * an accent foreground, which is what keeps Papier's selection a lemon pill with
 * ink on it instead of a lemon label on cream (webapp.md §10).
 */
@Composable
fun NzChip(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    selected: Boolean = false,
    glyph: NzGlyph? = null,
    enabled: Boolean = true,
) {
    val colors = Sillon.colors
    val motion = Sillon.motion
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val scale by animateFloatAsState(
        targetValue = if (pressed && enabled) 0.97f else 1f,
        animationSpec = motion.displace(),
        label = "nz-chip-press",
    )
    val surface by animateColorAsState(
        targetValue = if (selected) colors.ac else colors.s2,
        animationSpec = motion.tween(NzDur.Instant, NzEase.Standard),
        label = "nz-chip-fill",
    )
    val ink by animateColorAsState(
        targetValue = if (selected) colors.onac else colors.tx,
        animationSpec = motion.tween(NzDur.Instant, NzEase.Standard),
        label = "nz-chip-ink",
    )
    Row(
        modifier = modifier
            .scale(scale)
            .sizeIn(minHeight = NzMinTouchTarget, minWidth = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.chip))
            .background(surface)
            .then(
                if (selected) Modifier else Modifier.border(1.dp, colors.ln, RoundedCornerShape(NzRadius.chip)),
            )
            .clickable(
                interactionSource = interaction,
                indication = null,
                enabled = enabled,
                role = Role.Tab,
                onClick = onClick,
            )
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs, Alignment.CenterHorizontally),
    ) {
        if (glyph != null) Icon(glyph = glyph, size = 15.dp, tint = ink)
        Text(text = text, style = Sillon.type.label, color = ink)
    }
}

/**
 * A segmented control (the library's Musique / Podcasts, a sheet's tabs): one
 * `--s2` well, and the selected segment is an `--ac` pill with `--onac` text.
 * Each segment keeps a 48 dp target even though the pill reads 36 dp.
 */
@Composable
fun NzSegmented(
    options: List<String>,
    selectedIndex: Int,
    onSelect: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val motion = Sillon.motion
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.chip))
            .background(colors.s2)
            .padding(NzSpacing.xxs),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        options.forEachIndexed { index, option ->
            val selected = index == selectedIndex
            val fill by animateColorAsState(
                targetValue = if (selected) colors.ac else Color.Transparent,
                animationSpec = motion.tween(NzDur.Instant, NzEase.Standard),
                label = "nz-segment-fill-$index",
            )
            val ink by animateColorAsState(
                targetValue = if (selected) colors.onac else colors.dim,
                animationSpec = motion.tween(NzDur.Instant, NzEase.Standard),
                label = "nz-segment-ink-$index",
            )
            androidx.compose.foundation.layout.Box(
                modifier = Modifier
                    .weight(1f)
                    .sizeIn(minHeight = NzMinTouchTarget)
                    .clip(RoundedCornerShape(NzRadius.chip))
                    .background(fill)
                    .clickable(role = Role.Tab) { onSelect(index) },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = option,
                    style = Sillon.type.label,
                    color = ink,
                    modifier = Modifier.padding(horizontal = NzSpacing.m, vertical = NzSpacing.s),
                )
            }
        }
    }
}

/**
 * Sillon's switch (`webapp.md`: "A switch is a `--btn` track with an `--onbtn`
 * knob when on, and an `--s2` track ringed by `--dim` with a `--tx` knob when
 * off"). The off track used to be 1.2:1 against its card and an `--accent` track
 * put a white knob on the lemon; this is the pair that was measured.
 *
 * The thumb slides in `--dur-instant` and JUMPS under reduced motion or eco (M14:
 * no displacement), while the track's colour still changes.
 */
@Composable
fun NzSwitch(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    label: String? = null,
) {
    val colors = Sillon.colors
    val motion = Sillon.motion
    val interaction = remember { MutableInteractionSource() }
    val trackWidth = 46.dp
    val trackHeight = 26.dp
    val knob = 20.dp
    val inset = 3.dp
    val travel = trackWidth - knob - inset * 2
    val offset by animateFloatAsState(
        targetValue = if (checked) travel.value else 0f,
        animationSpec = motion.displace(),
        label = "nz-switch-thumb",
    )
    val track = if (checked) colors.btn else colors.s2
    val knobColor = if (checked) colors.onbtn else colors.tx
    Row(
        modifier = modifier
            .sizeIn(minHeight = NzMinTouchTarget, minWidth = NzMinTouchTarget)
            .clickable(
                interactionSource = interaction,
                indication = null,
                enabled = enabled,
                role = Role.Switch,
                onClick = { onCheckedChange(!checked) },
            )
            .padding(vertical = NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Canvas(modifier = Modifier.size(trackWidth, trackHeight)) {
            val radius = CornerRadius(size.height / 2f)
            drawRoundRect(color = track, cornerRadius = radius)
            if (!checked) {
                // The off track is --s2, which is 1.2:1 against a card: the dim
                // ring is what makes the control visible at all.
                drawRoundRect(
                    color = colors.dim.copy(alpha = 0.55f),
                    cornerRadius = radius,
                    style = Stroke(width = 1.dp.toPx()),
                )
            }
            drawCircle(
                color = knobColor,
                radius = knob.toPx() / 2f,
                center = Offset(
                    x = inset.toPx() + knob.toPx() / 2f + offset.dp.toPx(),
                    y = size.height / 2f,
                ),
            )
        }
        if (label != null) {
            Text(text = label, style = Sillon.type.body, color = colors.tx)
        }
    }
}
