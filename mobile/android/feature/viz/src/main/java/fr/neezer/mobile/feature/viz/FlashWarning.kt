package fr.neezer.mobile.feature.viz

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.Sillon

/**
 * The photosensitivity warning in front of « Débridé » —
 * `components/FlashWarning.svelte`, ported whole: the same sentences, the same
 * condition, the same refusal to be clicked through.
 *
 * It is the only flash level that goes past the WCAG general-flash threshold
 * (three a second), so it is the only one that asks first — and it asks for a
 * DELIBERATE act: « Activer » stays disabled until the switch below it is on.
 * Written to be read: what the mode does, who it is dangerous for (including
 * people who do not know it), what to do if it goes wrong, and the room — a
 * projector's audience accepted nothing.
 *
 * On the phone the box is a [NzSwitch] rather than a checkbox: it is the design
 * system's own toggle, it carries a 48 dp target and its semantics, and the
 * sentence stays a separate, readable line (a label inside a row squeezes the
 * row — MB16's measured lesson).
 *
 * **Who calls it.** The flash level itself (`viz.flash`) is consumed by the
 * worlds' renderer (MB22): the bars and the oscilloscope do not strobe, so this
 * lot delivers the component and its rule and no screen of its own opens it.
 */
@Composable
fun FlashWarningSheet(
    visible: Boolean,
    reduced: Boolean,
    onAccept: () -> Unit,
    onDismiss: () -> Unit,
) {
    val colours = Sillon.colors
    var agreed by remember(visible) { mutableStateOf(false) }
    NzSheet(visible = visible, onDismiss = onDismiss) {
        // The SPA's own layout: a header that stays, a BODY that scrolls (the
        // explanation is long, and at 200 % it is longer than a phone), and the
        // two answers that stay put. `NzSheet` bounds its height, so a body with
        // `weight` and its own scroll is what keeps « Activer » reachable
        // instead of clipped below the fold — measured on the first capture.
        Column(modifier = Modifier.fillMaxWidth()) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    modifier = Modifier
                        .size(42.dp)
                        .clip(RoundedCornerShape(NzRadius.card))
                        .background(colours.s2),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(NzGlyph.Zap, contentDescription = null, tint = colours.acMark)
                }
                Spacer(Modifier.width(NzSpacing.m))
                Column(Modifier.weight(1f)) {
                    Text(VizCopy.FLASH_TITLE, style = Sillon.type.titleSmall, color = colours.tx)
                    Text(VizCopy.FLASH_SUBTITLE, style = Sillon.type.bodySmall, color = colours.dim)
                }
                fr.neezer.mobile.designsystem.NzIconButton(
                    glyph = NzGlyph.Close,
                    contentDescription = VizCopy.FLASH_CLOSE,
                    onClick = onDismiss,
                )
            }
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f, fill = false)
                    .verticalScroll(rememberScrollState())
                    .padding(vertical = NzSpacing.m),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.l),
            ) {
                Text(VizCopy.FLASH_BODY, style = Sillon.type.body, color = colours.tx)
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(NzRadius.field))
                        .background(colours.s2)
                        .padding(NzSpacing.m),
                ) {
                    Icon(NzGlyph.Alert, contentDescription = null, tint = colours.tx)
                    Spacer(Modifier.width(NzSpacing.s))
                    Column {
                        Text(VizCopy.FLASH_WARN_TITLE, style = Sillon.type.label, color = colours.tx)
                        Text(VizCopy.FLASH_WARN_BODY, style = Sillon.type.bodySmall, color = colours.tx)
                    }
                }
                Row(verticalAlignment = Alignment.Top) {
                    Icon(NzGlyph.Cast, contentDescription = null, tint = colours.dim)
                    Spacer(Modifier.width(NzSpacing.s))
                    Text(VizCopy.FLASH_ROOM, style = Sillon.type.bodySmall, color = colours.dim)
                }
                Row(verticalAlignment = Alignment.Top) {
                    Icon(NzGlyph.Settings, contentDescription = null, tint = colours.dim)
                    Spacer(Modifier.width(NzSpacing.s))
                    Text(VizCopy.FLASH_REVERSIBLE, style = Sillon.type.bodySmall, color = colours.dim)
                }
                if (reduced) {
                    Row(verticalAlignment = Alignment.Top) {
                        Icon(NzGlyph.Info, contentDescription = null, tint = colours.dim)
                        Spacer(Modifier.width(NzSpacing.s))
                        Text(VizCopy.FLASH_REDUCED, style = Sillon.type.bodySmall, color = colours.tx)
                    }
                }
            }
            // The decision area stays whole: the sentence, its switch, and the two
            // answers. A switch left inside the scrolling body sat below the fold
            // while « Activer » was already visible and disabled — a button that
            // seems to do nothing is a bug (rule 2).
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(1.dp)
                    .background(colours.ln),
            )
            Column(Modifier.fillMaxWidth().padding(top = NzSpacing.m)) {
                Text(VizCopy.FLASH_AGREE, style = Sillon.type.body, color = colours.tx)
                Spacer(Modifier.height(NzSpacing.s))
                // `NzSwitch` carries `Role.Switch` and an onClick but NOT a
                // `toggleableState` (MB1's component), so TalkBack would say
                // "interrupteur" and never on/off. The switch therefore lives in a
                // `toggleable` row: the state and the action sit on the row (a
                // full-width, 48 dp target whose description is the sentence) and
                // the drawn switch contributes no semantics of its own. The design
                // system's own fix is owed and named in this lot's note.
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .toggleable(
                            value = agreed,
                            onValueChange = { agreed = it },
                            role = Role.Switch,
                        )
                        .semantics { contentDescription = VizCopy.FLASH_AGREE },
                ) {
                    NzSwitch(
                        checked = agreed,
                        onCheckedChange = { agreed = it },
                        modifier = Modifier.clearAndSetSemantics { },
                    )
                }
            }
            Row(
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                modifier = Modifier.fillMaxWidth().padding(top = NzSpacing.s),
            ) {
                NzButton(
                    text = VizCopy.FLASH_CANCEL,
                    onClick = onDismiss,
                    kind = NzButtonKind.Secondary,
                    modifier = Modifier.weight(1f),
                )
                NzButton(
                    text = VizCopy.FLASH_ACCEPT,
                    onClick = {
                        agreed = false
                        onAccept()
                    },
                    kind = NzButtonKind.Primary,
                    glyph = NzGlyph.Zap,
                    enabled = agreed,
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }
}
