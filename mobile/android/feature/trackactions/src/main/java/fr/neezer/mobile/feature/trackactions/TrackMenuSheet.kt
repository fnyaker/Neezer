package fr.neezer.mobile.feature.trackactions

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzEase
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.MenuEntry
import uniffi.nz_ffi.MenuItem

/**
 * The ONE track menu, drawn: a bottom sheet (`ContextMenu.svelte`'s phone shape),
 * every entry the core handed over, its accordion, and its hairlines.
 *
 * * **the sheet only LAYS OUT**: which entries exist, in which order, with which
 *   label and glyph, was decided by `nz-core::actions::menu` — the SPA's one spec
 *   (`trackmenu.js`), ported. A surface cannot drift from another;
 * * **a submenu expands INLINE** (an accordion), which is the SPA's phone shape
 *   since the side flyout was unusable on touch;
 * * **every row is a 48 dp target** with its label as its accessible name, and
 *   the sheet is dismissed by the veil, Escape (the keyboard) or a completed
 *   action;
 * * the material, the motion and the colours are Sillon's: nothing here names a
 *   literal colour or a duration.
 *
 * `marker` is the label the device test and the shell look for: it is on the
 * sheet's root in EVERY state, so a screen can be awaited without a clock.
 */
@Composable
fun TrackMenuSheet(
    state: TrackActionsModel.State,
    onRun: (uniffi.nz_ffi.MenuAction) -> Unit,
    onToggleSub: (MenuItem) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    NzSheet(
        visible = !state.closed,
        onDismiss = onDismiss,
        modifier = modifier,
        // The SPA frosts the SLEEVE under a sheet (`NzSheet`'s own seam): the
        // pane's fill is 93 % of the theme's, and on API 31+ the sleeve behind it
        // is blurred, which is what keeps the list's rows from ghosting through
        // the labels. Below 31 nothing blurs and the opaque step carries it — the
        // same platform limit the SPA's own ladder documents.
        backdrop = {
            // Full-pane on purpose: the sleeve covers the whole sheet, so the list
            // behind cannot ghost through the labels at the pane's edges either.
            CoverImage(url = state.row.cover, sizeDp = 640.dp, seed = state.row.id)
        },
    ) {
        Column(
            modifier = Modifier
                // A menu is a reading column, not a wall: past a phone's width the
                // items stay a menu's width and centre in the sheet (the SPA's
                // popover is 220 px at its narrowest, and a 600 dp label row with
                // 350 dp of empty tail is not a menu).
                .widthIn(max = 440.dp)
                .fillMaxWidth()
                .align(Alignment.CenterHorizontally)
                .semantics { contentDescription = MENU_MARKER }
                .heightIn(max = 640.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
        ) {
            Text(
                text = state.row.title,
                style = Sillon.type.titleSmall,
                color = Sillon.colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(
                    start = NzSpacing.s,
                    end = NzSpacing.s,
                    bottom = NzSpacing.s,
                ),
            )
            state.entries.forEach { entry ->
                when (entry) {
                    is MenuEntry.Divider -> MenuDivider()
                    is MenuEntry.Item -> MenuItemRow(
                        item = entry.item,
                        expanded = state.expanded == entry.item.label,
                        onRun = onRun,
                        onToggleSub = onToggleSub,
                    )
                }
            }
        }
    }
}

/** The label a device test and the CI smoke wait for (never a clock). */
const val MENU_MARKER = "Actions du titre"

@Composable
private fun MenuDivider() {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(Sillon.colors.ln),
        )
    }
}

@Composable
private fun MenuItemRow(
    item: MenuItem,
    expanded: Boolean,
    onRun: (uniffi.nz_ffi.MenuAction) -> Unit,
    onToggleSub: (MenuItem) -> Unit,
) {
    val colors = Sillon.colors
    val motion = Sillon.motion
    val chevron by animateFloatAsState(
        targetValue = if (expanded) 90f else 0f,
        animationSpec = if (motion.moves) {
            androidx.compose.animation.core.tween(NzDur.Base, easing = NzEase.Standard)
        } else {
            androidx.compose.animation.core.tween(0)
        },
        label = "menu-chevron",
    )
    val ink: Color = if (item.danger) colors.hrt else colors.tx
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .sizeIn(minHeight = NzMinTouchTarget)
            .background(
                if (expanded) colors.s2 else Color.Transparent,
                RoundedCornerShape(NzRadius.field),
            )
            .selectable(
                selected = expanded,
                role = if (item.sub.isEmpty()) Role.Button else Role.DropdownList,
                onClick = {
                    if (item.sub.isEmpty()) onRun(item.action) else onToggleSub(item)
                },
            )
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Icon(
            glyph = TrackActionsGlyph(item.icon),
            size = 18.dp,
            tint = ink,
            contentDescription = null,
        )
        Text(
            text = item.label,
            style = Sillon.type.body,
            color = ink,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        if (item.sub.isNotEmpty()) {
            Icon(
                glyph = NzGlyph.ChevronRight,
                size = 16.dp,
                tint = colors.dim,
                contentDescription = null,
                modifier = Modifier.rotate(chevron),
            )
        }
    }
    if (expanded && item.sub.isNotEmpty()) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = NzSpacing.xxl)
                .background(colors.s2, RoundedCornerShape(NzRadius.field)),
        ) {
            item.sub.forEach { sub ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .sizeIn(minHeight = NzMinTouchTarget)
                        .selectable(
                            selected = false,
                            role = Role.Button,
                            onClick = { onRun(sub.action) },
                        )
                        .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
                ) {
                    Icon(
                        glyph = TrackActionsGlyph(sub.icon),
                        size = 17.dp,
                        tint = colors.tx,
                        contentDescription = null,
                    )
                    Text(
                        text = sub.label,
                        style = Sillon.type.body,
                        color = colors.tx,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}
