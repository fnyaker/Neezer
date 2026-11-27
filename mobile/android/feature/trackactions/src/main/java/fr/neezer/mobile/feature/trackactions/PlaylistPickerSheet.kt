package fr.neezer.mobile.feature.trackactions

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.PlaylistChoice

/** The label the device test waits for. */
const val PICKER_MARKER = "Ajouter à une playlist"

/** Rows the sheet mounts at most (the SPA's own `LIST_MAX`): a sheet, not a page. */
private const val LIST_MAX = 200

/**
 * The playlist picker (`PlaylistPicker.svelte`, its "add a title" mode PORTED).
 *
 * * the copy paints first (`cachedMenuPlaylists`), the answer replaces it, and a
 *   dead link keeps it — « Hors ligne · version mémorisée » — instead of an empty
 *   sheet;
 * * **a tick is an action, not a membership**: the box shows what THIS opening
 *   added, and a refusal unticks it (the SPA's own note — a playlist's tracklist
 *   is not on this screen, so nothing is fetched per list);
 * * the filter only appears past six lists, and a list the person may not write
 *   to is never a target;
 * * the create form is one request, and only the server's answer closes it.
 */
@Composable
fun PlaylistPickerSheet(
    state: TrackActionsModel.State,
    onFilter: (String) -> Unit,
    onPick: (PlaylistChoice) -> Unit,
    onCreate: () -> Unit,
    onStartCreating: () -> Unit,
    onCancelCreating: () -> Unit,
    onName: (String) -> Unit,
    onRetry: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val picker = state.picker ?: return
    val colors = Sillon.colors
    NzSheet(visible = !state.closed, onDismiss = onDismiss, modifier = modifier) {
        Column(
            modifier = Modifier
                .widthIn(max = 560.dp)
                .fillMaxWidth()
                .align(Alignment.CenterHorizontally)
                .semantics { contentDescription = PICKER_MARKER },
            verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = if (picker.creating) "Nouvelle playlist" else PICKER_MARKER,
                        style = Sillon.type.titleSmall,
                        color = colors.tx,
                    )
                    Text(
                        text = state.row.title + if (state.row.artists.isBlank()) "" else " · ${state.row.artists}",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                NzIconButton(
                    glyph = NzGlyph.Close,
                    contentDescription = "Fermer",
                    onClick = onDismiss,
                )
            }

            if (picker.creating) {
                CreateForm(
                    name = picker.name,
                    suggestion = picker.suggestion,
                    busy = picker.busy,
                    onName = onName,
                    onCancel = onCancelCreating,
                    onCreate = onCreate,
                )
            } else {
                NzButton(
                    text = "Nouvelle playlist",
                    kind = NzButtonKind.Secondary,
                    onClick = onStartCreating,
                    glyph = NzGlyph.Plus,
                )
                if (picker.filterShown) {
                    SearchField(
                        value = picker.query,
                        placeholder = "Filtrer les playlists…",
                        label = "Filtrer les playlists",
                        onValue = onFilter,
                    )
                }
                Box(modifier = Modifier.fillMaxWidth().heightIn(max = 420.dp)) {
                    val lists = picker.lists
                    when {
                        lists == null && picker.error != null -> StateLine(
                            text = "Impossible de charger tes playlists : ${picker.error}",
                            action = "Réessayer",
                            onAction = onRetry,
                        )
                        lists == null -> StateLine(text = "Chargement des playlists…")
                        picker.shown.isEmpty() && picker.query.isNotBlank() -> StateLine(
                            text = "Aucune playlist ne correspond à « ${picker.query.trim()} ».",
                        )
                        picker.shown.isEmpty() -> StateLine(
                            text = if (picker.targets.isEmpty()) {
                                "Aucune playlist que vous pouvez modifier."
                            } else {
                                "Aucune playlist pour l'instant."
                            },
                        )
                        else -> Column(
                            // The sheet mounts at most its own ceiling (the SPA's
                            // `LIST_MAX`, 200) and scrolls: a bounded list, so a
                            // plain column IS the window and it wraps its content
                            // (three playlists do not leave an empty tail).
                            modifier = Modifier
                                .fillMaxWidth()
                                .verticalScroll(rememberScrollState()),
                            verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
                        ) {
                            picker.shown.take(LIST_MAX).forEach { choice ->
                                key(choice.id) {
                                    PlaylistRow(
                                        choice = choice,
                                        added = picker.added.contains(choice.id),
                                        onClick = { onPick(choice) },
                                    )
                                }
                            }
                        }
                    }
                }
                if (picker.stale) {
                    Text(
                        text = "Hors ligne · version mémorisée",
                        style = Sillon.type.labelSmall,
                        color = colors.dim,
                    )
                }
                NzButton(
                    text = "Terminé",
                    kind = NzButtonKind.Primary,
                    onClick = onDismiss,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

@Composable
private fun PlaylistRow(
    choice: PlaylistChoice,
    added: Boolean,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    val count = "${choice.nbTracks} titre" + if (choice.nbTracks > 1u) "s" else ""
    val subtitle = if (!choice.role.isNullOrEmpty() && choice.role != "owner" && !choice.owner.isNullOrEmpty()) {
        "$count · ${choice.owner}"
    } else {
        count
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .sizeIn(minHeight = NzMinTouchTarget + 8.dp)
            .selectable(selected = added, role = Role.Checkbox, onClick = onClick)
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        CoverImage(url = choice.cover, sizeDp = 40.dp, seed = choice.id)
        Column(modifier = Modifier.weight(1f)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Text(
                    text = choice.title,
                    style = Sillon.type.body,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (choice.recent) {
                    Text(
                        text = "RÉCENTE",
                        style = Sillon.type.labelMono,
                        color = colors.dim,
                        modifier = Modifier
                            .background(colors.s2, RoundedCornerShape(NzRadius.chip))
                            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xxs),
                    )
                }
            }
            Text(text = subtitle, style = Sillon.type.mono, color = colors.dim, maxLines = 1)
        }
        Box(
            modifier = Modifier
                .size(24.dp)
                .background(
                    if (added) colors.ac else colors.s2,
                    RoundedCornerShape(NzRadius.field),
                ),
            contentAlignment = Alignment.Center,
        ) {
            if (added) {
                Icon(glyph = NzGlyph.Check, size = 14.dp, tint = colors.onac, contentDescription = null)
            }
        }
    }
}

@Composable
private fun CreateForm(
    name: String,
    suggestion: String,
    busy: Boolean,
    onName: (String) -> Unit,
    onCancel: () -> Unit,
    onCreate: () -> Unit,
) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        Text(text = "Nom de la playlist", style = Sillon.type.label, color = colors.dim)
        SearchField(
            value = name,
            placeholder = suggestion,
            label = "Nom de la playlist",
            onValue = onName,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s, Alignment.End),
        ) {
            NzButton(text = "Annuler", kind = NzButtonKind.Ghost, onClick = onCancel)
            NzButton(
                text = "Créer et ajouter",
                kind = NzButtonKind.Primary,
                onClick = onCreate,
                enabled = !busy,
            )
        }
    }
}

@Composable
private fun SearchField(
    value: String,
    placeholder: String,
    label: String,
    onValue: (String) -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.s2, RoundedCornerShape(NzRadius.chip))
            .sizeIn(minHeight = NzMinTouchTarget)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = NzGlyph.Search, size = 16.dp, tint = colors.dim, contentDescription = null)
        BasicTextField(
            value = value,
            onValueChange = onValue,
            singleLine = true,
            textStyle = Sillon.type.body.copy(color = colors.tx),
            cursorBrush = SolidColor(colors.acText),
            modifier = Modifier
                .weight(1f)
                .semantics { contentDescription = label },
            decorationBox = { inner ->
                if (value.isEmpty()) {
                    Text(text = placeholder, style = Sillon.type.body, color = colors.dim)
                }
                inner()
            },
        )
    }
}

@Composable
private fun StateLine(text: String, action: String? = null, onAction: (() -> Unit)? = null) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = NzSpacing.l, horizontal = NzSpacing.s),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(
            text = text,
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        if (action != null && onAction != null) {
            NzButton(text = action, kind = NzButtonKind.Ghost, onClick = onAction)
        }
    }
}
