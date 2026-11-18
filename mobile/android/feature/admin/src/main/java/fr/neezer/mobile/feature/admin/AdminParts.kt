package fr.neezer.mobile.feature.admin

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.AdminCause
import uniffi.nz_ffi.AdminJobFailures

/**
 * Les petits blocs des pages d'administration (lot MB17) : une carte, une jauge,
 * un champ, une pastille, un bloc d'échecs.
 *
 * Chacun reprend une forme de la SPA (`Settings.svelte`'s `.card`, `.gauge`,
 * `.stats`, `JobFailures.svelte`) avec les jetons de Sillon et les règles que
 * MB16 a payées : **le libellé d'un commutateur ne vit jamais dans une `Row` à
 * enfant pondéré** (il écrasait une colonne), **une rangée de pastilles est un
 * `FlowRow`** (dix chips ne tiennent pas sur un téléphone à 200 %), and **un bloc
 * dont le texte peut grandir utilise `heightIn`**, jamais `height`.
 */

/** `Settings.svelte`'s `.card`: a title, a subtitle, then whatever it holds. */
@Composable
internal fun AdminCard(
    title: String,
    subtitle: String? = null,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    val colors = Sillon.colors
    NzCard(modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text(text = title, style = Sillon.type.label, color = colors.tx)
            if (subtitle != null) {
                Text(text = subtitle, style = Sillon.type.bodySmall, color = colors.dim)
            }
            content()
        }
    }
}

/** A section title inside a card (`Settings.svelte`'s `h3`). */
@Composable
internal fun AdminSubtitle(text: String) {
    Text(
        text = text,
        style = Sillon.type.labelSmall,
        color = Sillon.colors.tx,
        modifier = Modifier.padding(top = NzSpacing.xs),
    )
}

/** `muted hint` — what a card explains under its controls. */
@Composable
internal fun AdminHint(text: String, warn: Boolean = false) {
    val colors = Sillon.colors
    Row(
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        if (warn) {
            Icon(glyph = NzGlyph.Alert, size = 14.dp, tint = colors.acText)
        }
        Text(
            text = text,
            style = Sillon.type.bodySmall,
            color = if (warn) colors.tx else colors.dim,
            modifier = Modifier.weight(1f, fill = false),
        )
    }
}

/**
 * A gauge (`Settings.svelte`'s `.gauge`): a hairline track and the accent fill,
 * with the two lines that say what it measures. `indeterminate` is the migration
 * bar, which has no total.
 */
@Composable
internal fun AdminGauge(
    fraction: Float,
    label: String,
    detail: String? = null,
    indeterminate: Boolean = false,
) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(8.dp)
                .clip(RoundedCornerShape(4.dp))
                .background(colors.s2)
                .semantics { contentDescription = label },
        ) {
            val share = if (indeterminate) 0.35f else fraction.coerceIn(0f, 1f)
            Box(
                Modifier
                    .fillMaxWidth(share)
                    .height(8.dp)
                    .clip(RoundedCornerShape(4.dp))
                    .background(colors.acText),
            )
        }
        Text(text = label, style = Sillon.type.labelSmall, color = colors.tx)
        if (detail != null) {
            Text(text = detail, style = Sillon.type.bodySmall, color = colors.dim)
        }
    }
}

/** One number and its name (`Settings.svelte`'s `.stat`). */
@Composable
internal fun AdminStat(value: String, label: String) {
    val colors = Sillon.colors
    Column {
        Text(text = value, style = Sillon.type.label, color = colors.tx, maxLines = 1)
        Text(text = label, style = Sillon.type.bodySmall, color = colors.dim)
    }
}

/**
 * A switch with its text ABOVE the row's line — MB16's measured rule: a label
 * inside a `Row` that also holds a weighted child left that child one letter
 * wide.
 */
@Composable
internal fun AdminSwitchRow(
    title: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    hint: String? = null,
    enabled: Boolean = true,
) {
    val colors = Sillon.colors
    Row(
        modifier = modifier
            .fillMaxWidth()
            .sizeIn(minHeight = NzMinTouchTarget)
            .clickable(enabled = enabled, role = Role.Switch) { onChange(!checked) }
            .padding(vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Column(Modifier.weight(1f)) {
            Text(text = title, style = Sillon.type.body, color = if (enabled) colors.tx else colors.dim)
            if (hint != null) {
                Text(text = hint, style = Sillon.type.bodySmall, color = colors.dim)
            }
        }
        NzSwitch(checked = checked, onCheckedChange = onChange, enabled = enabled)
    }
}

/** A labelled number field (`Settings.svelte`'s `.num` and the transcribe card). */
@Composable
internal fun AdminField(
    label: String,
    value: String,
    onValue: (String) -> Unit,
    modifier: Modifier = Modifier,
    hint: String? = null,
    suffix: String? = null,
    numeric: Boolean = true,
    onDone: (() -> Unit)? = null,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.field)
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Text(text = label, style = Sillon.type.labelSmall, color = colors.dim)
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = NzMinTouchTarget)
                .clip(shape)
                .background(colors.bg)
                .border(1.dp, colors.ln, shape)
                .padding(horizontal = NzSpacing.m),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            BasicTextField(
                value = value,
                onValueChange = onValue,
                singleLine = true,
                textStyle = Sillon.type.body.copy(color = colors.tx),
                cursorBrush = SolidColor(colors.acText),
                keyboardOptions = KeyboardOptions(
                    keyboardType = if (numeric) KeyboardType.Number else KeyboardType.Text,
                    imeAction = if (onDone != null) ImeAction.Done else ImeAction.Default,
                ),
                modifier = Modifier
                    .weight(1f)
                    .semantics { contentDescription = label },
            )
            if (suffix != null) {
                Text(text = suffix, style = Sillon.type.bodySmall, color = colors.dim)
            }
            if (onDone != null) {
                NzButton(
                    text = "Enregistrer",
                    onClick = onDone,
                    kind = NzButtonKind.Secondary,
                )
            }
        }
        if (hint != null) {
            Text(text = hint, style = Sillon.type.bodySmall, color = colors.dim)
        }
    }
}

/** A wrap of pills: a choice of scopes, orders, hours — never a `Row` (`FlowRow`). */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun AdminChips(
    options: List<Pair<String, String>>,
    selected: String,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    androidx.compose.foundation.layout.FlowRow(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        options.forEach { (id, label) ->
            NzChip(
                text = label,
                onClick = { onSelect(id) },
                selected = id == selected,
                enabled = enabled,
            )
        }
    }
}

/**
 * `JobFailures.svelte`, porté : replié par défaut (une exécution saine ne crie
 * pas), un appui l'ouvre sur le classement des CAUSES, l'échantillon des titres,
 * ce que l'échantillon du serveur a laissé de côté, et « Copier le détail ».
 */
@Composable
internal fun JobFailuresBlock(
    failures: AdminJobFailures,
    label: String,
    onCopy: ((String) -> Unit)? = null,
) {
    if (failures.failed == 0u) return
    val colors = Sillon.colors
    var open by remember(failures.failed) { mutableStateOf(false) }
    val shape = RoundedCornerShape(NzRadius.field)
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .border(1.dp, colors.ln, shape)
            .background(colors.s1),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .sizeIn(minHeight = NzMinTouchTarget)
                .clickable(role = Role.Button) { open = !open }
                .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Icon(
                glyph = if (open) NzGlyph.ChevronUp else NzGlyph.ChevronDown,
                size = 15.dp,
                tint = colors.acText,
            )
            Text(
                text = buildString {
                    append(AdminCopy.countLabel(failures.failed.toInt(), "titre"))
                    append(" en échec")
                    if (failures.causes.size > 1) append(", ${failures.causes.size} causes")
                },
                style = Sillon.type.labelSmall,
                color = colors.tx,
                modifier = Modifier.weight(1f),
            )
        }
        if (open) {
            Column(
                modifier = Modifier.padding(horizontal = NzSpacing.m),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                failures.causes.forEach { cause -> CauseLine(cause) }
                failures.rows.take(40).forEach { row ->
                    Column(Modifier.padding(vertical = NzSpacing.xxs)) {
                        Text(text = row.track.ifBlank { "?" }, style = Sillon.type.bodySmall, color = colors.tx)
                        Text(text = row.reason, style = Sillon.type.mono, color = colors.dim)
                    }
                }
                if (failures.hidden > 0u) {
                    Text(
                        text = "… et ${failures.hidden} autre" + (if (failures.hidden > 1u) "s" else "") +
                            " non détaillé" + (if (failures.hidden > 1u) "s" else "") +
                            " (le serveur en garde un échantillon).",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                if (onCopy != null) {
                    NzButton(
                        text = "Copier le détail",
                        onClick = { onCopy(failuresText(failures, label)) },
                        glyph = NzGlyph.Copy,
                        kind = NzButtonKind.Secondary,
                    )
                }
                Spacer(Modifier.height(NzSpacing.xs))
            }
        }
    }
}

@Composable
private fun CauseLine(cause: AdminCause) {
    val colors = Sillon.colors
    Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Text(text = "${cause.count}×", style = Sillon.type.labelSmall, color = colors.acText)
        Text(text = cause.reason, style = Sillon.type.mono, color = colors.dim)
    }
}

/** `JobFailures.svelte#asText` — what « Copier le détail » puts on the clipboard. */
internal fun failuresText(failures: AdminJobFailures, label: String): String = buildString {
    appendLine("Neezer — $label : ${failures.failed} en échec")
    appendLine()
    failures.causes.forEach { appendLine("  ${it.count}x ${it.reason}") }
    appendLine()
    failures.rows.forEach { appendLine("  ${it.track.ifBlank { "?" }} — ${it.reason}") }
    if (failures.hidden > 0u) appendLine("  … et ${failures.hidden} autre(s)")
}.trimEnd()

/** One action row of a sheet (`AdminUsers.svelte`'s `.action`). */
@Composable
internal fun AdminActionRow(
    title: String,
    description: String,
    glyph: NzGlyph,
    onClick: () -> Unit,
    enabled: Boolean = true,
    danger: Boolean = false,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.field)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 64.dp)
            .clip(shape)
            .background(colors.s2)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(NzSpacing.m),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Box(
            modifier = Modifier
                .size(36.dp)
                .clip(CircleShape)
                .background(colors.bg),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                glyph = glyph,
                size = 18.dp,
                tint = if (enabled) (if (danger) colors.tx else colors.acText) else colors.dim,
            )
        }
        Column(Modifier.weight(1f)) {
            Text(
                text = title,
                style = Sillon.type.body,
                color = if (enabled) colors.tx else colors.dim,
            )
            Text(text = description, style = Sillon.type.bodySmall, color = colors.dim)
        }
        if (enabled) Icon(glyph = NzGlyph.ChevronRight, size = 18.dp, tint = colors.dim)
    }
}

/** A small status pill (`Settings.svelte`'s `.pill`). */
@Composable
internal fun AdminPill(text: String, ok: Boolean?) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.chip)
    Row(
        modifier = Modifier
            .clip(shape)
            .background(if (ok == true) colors.s2 else colors.s1)
            .border(1.dp, colors.ln, shape)
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xxs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Icon(
            glyph = if (ok == true) NzGlyph.Check else NzGlyph.Alert,
            size = 13.dp,
            tint = if (ok == true) colors.acText else colors.dim,
        )
        Text(
            text = text,
            style = Sillon.type.labelSmall,
            color = colors.tx,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** A card's own "Réessayer" line for a cause, under the card it belongs to. */
@Composable
internal fun AdminCardFailure(cause: String, onRetry: () -> Unit) {
    AdminHint(cause, warn = true)
    NzButton(
        text = "Réessayer",
        onClick = onRetry,
        glyph = NzGlyph.Refresh,
        kind = NzButtonKind.Secondary,
    )
}

/** A dismissible inline error (`AdminUsers.svelte`'s `.inline-error`). */
@Composable
internal fun AdminError(text: String) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.field)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .border(1.dp, colors.ln, shape)
            .background(colors.s1)
            .padding(NzSpacing.m)
            .semantics { liveRegion = LiveRegionMode.Assertive },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(glyph = NzGlyph.Alert, size = 15.dp, tint = colors.acText)
        Text(text = text, style = Sillon.type.bodySmall, color = colors.tx, modifier = Modifier.weight(1f))
    }
}

/** « Hors ligne · version mémorisée » — the note a kept copy carries. */
@Composable
internal fun AdminStaleNote() {
    Text(
        text = "Hors ligne · version mémorisée",
        style = Sillon.type.bodySmall,
        color = Sillon.colors.dim,
    )
}

/** A 44 dp icon button, used for the refresh and the sheet's close. */
@Composable
internal fun AdminIconButton(glyph: NzGlyph, label: String, onClick: () -> Unit) {
    NzIconButton(glyph = glyph, contentDescription = label, onClick = onClick)
}
