package fr.neezer.mobile.feature.remote

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
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
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.RemoteDuration
import uniffi.nz_ffi.RemoteLevel

/**
 * Contrôle à distance, the owner's sheet (lot MB19): make a link that lets
 * somebody drive THIS player, hand it over, see who is on which link, cut any of
 * them — and, while this phone drives another player, the banner and the way out.
 *
 * `webapp/src/components/RemoteSheet.svelte` and `RemoteHostChip.svelte`, ported:
 * the same four levels with the same words (the core's, from `nz-remote`), the
 * same durations, the same two-step « Tout couper », the same live line per link.
 * Two differences, both deliberate and named in the note:
 *
 * - **no QR code**: the phone shares the link through the system sheet (the SPA's
 *   other half of the same button), and a QR is a library this lot may not add;
 * - **the level blurb sits under its name in ONE column at 360 dp** and in two at
 *   600 dp — the SPA's two-column grid squeezed the blurbs to three words on a
 *   phone (measured in its own screenshot).
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun RemoteSheet(
    visible: Boolean,
    model: RemoteModel,
    onDismiss: () -> Unit,
    onShare: (String) -> Unit,
    onCopy: (String) -> Unit,
    modifier: Modifier = Modifier,
    selfName: String = "Ce téléphone",
) {
    val state by model.state.collectAsState()
    NzSheet(visible = visible, onDismiss = onDismiss, modifier = modifier) {
        // The SPA's sheet is `min(600px, 100%)`: on a tablet the pane keeps its
        // reading column instead of stretching a 600 dp row across the screen.
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
        Column(
            modifier = Modifier
                .widthIn(max = 600.dp)
                .fillMaxWidth()
                .heightIn(max = 560.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            SheetHeader(live = state.status?.controllers?.size ?: 0, onClose = onDismiss)
            if (state.status?.driven == true) {
                HostChip(status = state.status!!, onCut = { model.cutAll() })
            }
            val invite = state.invite
            if (invite != null) {
                InviteCard(
                    row = invite,
                    onShare = { onShare(model.inviteText(invite)) },
                    onCopy = { onCopy(invite.url) },
                    onDone = { model.dismissInvite() },
                )
            } else {
                NewLinkSection(model = model, state = state)
            }
            LinksSection(model = model, state = state)
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Icon(NzGlyph.Monitor, size = 14.dp, tint = Sillon.colors.dim)
                Text(
                    text = selfName,
                    style = Sillon.type.bodySmall,
                    color = Sillon.colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                if (state.links.isNotEmpty()) {
                    NzButton(
                        text = if (state.armedCut) "Confirmer : tout couper" else "Tout couper",
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.LogOut,
                        onClick = { model.cutAll() },
                    )
                }
            }
        }
        }
    }
}

@Composable
private fun SheetHeader(live: Int, onClose: () -> Unit) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Box(
            modifier = Modifier
                .size(44.dp)
                .background(
                    if (live > 0) colors.ac else colors.s2,
                    RoundedCornerShape(NzRadius.card),
                ),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                NzGlyph.Remote,
                size = 20.dp,
                tint = if (live > 0) colors.onac else colors.tx,
            )
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text("Contrôle à distance", style = Sillon.type.title, color = colors.tx)
            Text(
                text = when {
                    live == 1 -> "1 appareil pilote ce lecteur"
                    live > 1 -> "$live appareils pilotent ce lecteur"
                    else -> "Confiez ce lecteur à quelqu'un : il le pilote depuis son téléphone, avec la même app."
                },
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
        NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = onClose)
    }
}

/**
 * « Contrôlé par Léa · Couper » — the owner's chip, always within reach: one tap
 * takes the player back. `RemoteHostChip.svelte`, without the fixed overlay (on
 * the phone the sheet lives in a dialog and the player bar is MB11's).
 */
@Composable
fun HostChip(
    status: RemoteStatusFi,
    onCut: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Row(
        modifier = modifier
            .fillMaxWidth()
            .background(colors.s2, RoundedCornerShape(NzRadius.pill))
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .semantics { contentDescription = "Contrôlé par ${status.who}, Couper" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(NzGlyph.Remote, size = 16.dp, tint = colors.ac)
        Column(Modifier.weight(1f)) {
            Text(
                text = if (status.who.isBlank()) "Contrôlé par 1 appareil" else status.who,
                style = Sillon.type.label,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            val level = status.hostLevel
            if (!level.isNullOrBlank()) {
                Text(level, style = Sillon.type.bodySmall, color = colors.dim)
            }
        }
        NzButton(
            text = "Couper",
            kind = NzButtonKind.Ghost,
            glyph = NzGlyph.Close,
            onClick = onCut,
        )
    }
}

@Composable
private fun InviteCard(
    row: RemoteLinkRow,
    onShare: () -> Unit,
    onCopy: () -> Unit,
    onDone: () -> Unit,
) {
    val colors = Sillon.colors
    NzCard(Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(NzSpacing.m),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                Icon(levelGlyph(row.level), size = 15.dp, tint = colors.ac)
                Text(row.title, style = Sillon.type.label, color = colors.tx)
            }
            Text(
                "Scannez, ou envoyez le lien. ${row.expiry}.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            // The URL is the one thing a person may need to read out loud: it is
            // selectable-looking text, wrapping rather than cut (a token cut in
            // half is a token nobody can use).
            Text(
                text = row.url,
                style = Sillon.type.mono,
                color = colors.tx,
                modifier = Modifier
                    .fillMaxWidth()
                    .background(colors.bg, RoundedCornerShape(NzRadius.chip))
                    .padding(NzSpacing.s)
                    .clickable(role = Role.Button) { onCopy() },
            )
            FlowRow(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                NzButton(text = "Partager", glyph = NzGlyph.Share, onClick = onShare)
                NzButton(
                    text = "Copier",
                    kind = NzButtonKind.Secondary,
                    glyph = NzGlyph.Copy,
                    onClick = onCopy,
                )
                NzButton(text = "Terminé", kind = NzButtonKind.Ghost, glyph = NzGlyph.Check, onClick = onDone)
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun NewLinkSection(model: RemoteModel, state: RemoteModel.State) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Text("Nouveau lien", style = Sillon.type.label, color = colors.dim)
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val wide = maxWidth >= 600.dp
            if (wide) {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    state.levels.chunked(2).forEach { pair ->
                        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                            pair.forEach { level ->
                                LevelCard(
                                    level = level,
                                    selected = state.level == level.id,
                                    modifier = Modifier.weight(1f),
                                ) { model.setLevel(level.id) }
                            }
                            if (pair.size == 1) Spacer(Modifier.weight(1f))
                        }
                    }
                }
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    state.levels.forEach { level ->
                        LevelCard(level = level, selected = state.level == level.id) {
                            model.setLevel(level.id)
                        }
                    }
                }
            }
        }
        Text("Durée", style = Sillon.type.label, color = colors.dim)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            state.durations.forEach { duration: RemoteDuration ->
                NzChip(
                    text = duration.label,
                    onClick = { model.setDuration(duration.id) },
                    selected = state.duration == duration.id,
                )
            }
        }
        Text("Pour", style = Sillon.type.label, color = colors.dim)
        NzLabelField(
            value = state.label,
            onValueChange = model::setLabel,
            placeholder = "Prénom, pour s'y retrouver (facultatif)",
        )
        NzButton(
            text = if (state.creating) "Création…" else "Créer le lien",
            glyph = NzGlyph.Link,
            enabled = !state.creating,
            onClick = model::create,
        )
    }
}

@Composable
private fun LevelCard(
    level: RemoteLevel,
    selected: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.card)
    val surface = if (selected) colors.ac else colors.s1
    val ink = if (selected) colors.onac else colors.tx
    Row(
        modifier = modifier
            .background(surface, shape)
            .clickable(role = Role.RadioButton, onClick = onClick)
            .padding(NzSpacing.m)
            .semantics { contentDescription = "${level.label} : ${level.blurb}" },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(levelGlyph(level.id), size = 18.dp, tint = if (selected) colors.onac else colors.ac)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(level.label, style = Sillon.type.label, color = ink)
            Text(level.blurb, style = Sillon.type.bodySmall, color = if (selected) colors.onac else colors.dim)
        }
        if (selected) {
            Icon(NzGlyph.Check, size = 14.dp, tint = colors.onac)
        }
    }
}

/** `mode.js#LEVEL_INFO[l].icon`, the three glyphs the SPA uses. */
fun levelGlyph(level: String): NzGlyph = when (level) {
    "read" -> NzGlyph.Eye
    "full" -> NzGlyph.Sliders
    "admin" -> NzGlyph.Shield
    else -> NzGlyph.Queue
}

@Composable
private fun LinksSection(model: RemoteModel, state: RemoteModel.State) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Text("Liens actifs", style = Sillon.type.label, color = colors.dim)
        val failure = state.failure
        when {
            failure != null -> LoadFailed(cause = failure.cause, onRetry = failure.retry)
            !state.linksLoaded && state.links.isEmpty() -> Text(
                "Lecture des liens…",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            state.links.isEmpty() -> Text(
                "Aucun lien sur ce lecteur. Celui que vous créez apparaît ici, avec qui s'en sert.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            else -> {
                if (state.linksStale) {
                    Text(
                        "Hors ligne · version mémorisée",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                state.links.forEach { row ->
                    LinkRowItem(row = row, onCut = { model.cut(row.id) }, onShare = { model.dismissInvite() })
                }
            }
        }
    }
}

@Composable
private fun LinkRowItem(row: RemoteLinkRow, onCut: () -> Unit, onShare: () -> Unit) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = NzMinTouchTarget)
            .padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(levelGlyph(row.level), size = 16.dp, tint = if (row.live) colors.ac else colors.dim)
        Column(Modifier.weight(1f)) {
            Text(
                text = row.label.ifBlank { row.levelLabel },
                style = Sillon.type.label,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            val live = when (row.controllers) {
                0 -> ""
                1 -> " · connecté"
                else -> " · ${row.controllers} connectés"
            }
            val second = buildString {
                if (row.label.isNotBlank()) append(row.levelLabel).append(" · ")
                append(row.expiry)
                append(live)
            }
            Text(
                second,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        NzIconButton(glyph = NzGlyph.Share, contentDescription = "Partager le lien", onClick = onShare)
        NzIconButton(glyph = NzGlyph.Close, contentDescription = "Couper ce lien", onClick = onCut)
    }
}

/** A text field the sheet owns, in the theme's own skin. */
@Composable
private fun NzLabelField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
) {
    val colors = Sillon.colors
    BasicTextField(
        value = value,
        onValueChange = { onValueChange(it.take(40)) },
        singleLine = true,
        textStyle = Sillon.type.body.copy(color = colors.tx),
        cursorBrush = SolidColor(colors.ac),
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.bg, RoundedCornerShape(NzRadius.chip))
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .heightIn(min = NzMinTouchTarget)
            // A field whose only label is a placeholder is a field TalkBack reads
            // as "modifiable" and nothing more.
            .semantics { contentDescription = "Pour, le nom du lien (facultatif)" },
        decorationBox = { inner ->
            Box(contentAlignment = Alignment.CenterStart) {
                if (value.isEmpty()) {
                    Text(placeholder, style = Sillon.type.body, color = colors.dim, maxLines = 1)
                }
                inner()
            }
        },
    )
}

/**
 * `LoadFailed.svelte`, ported: the cause and « Réessayer », never a spinner past
 * the first failure (rule 3).
 */
@Composable
fun LoadFailed(cause: String, onRetry: () -> Unit, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Column(
        modifier = modifier.fillMaxWidth().padding(vertical = NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Icon(NzGlyph.Alert, size = 16.dp, tint = colors.dim)
            Text(cause, style = Sillon.type.bodySmall, color = colors.dim)
        }
        NzButton(text = "Réessayer", kind = NzButtonKind.Secondary, glyph = NzGlyph.Refresh, onClick = onRetry)
    }
}
