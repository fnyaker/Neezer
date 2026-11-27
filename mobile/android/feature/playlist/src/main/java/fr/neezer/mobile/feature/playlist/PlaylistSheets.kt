package fr.neezer.mobile.feature.playlist

import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSegmented
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.PlaylistMember
import uniffi.nz_ffi.PlaylistRole

/**
 * The playlist's sheets (lot MB9): the ⋯ menu, the collaborators
 * (`PlaylistMembersSheet.svelte`), the export panel (`ExportSheet.svelte`) with its
 * four tabs, and the one confirmation before a destructive act.
 *
 * Each one draws from the model's own state and reports presses through
 * [PlaylistSheetActions]; nothing here reaches for the core.
 */
data class PlaylistSheetActions(
    val close: () -> Unit = {},
    val openMembers: () -> Unit = {},
    val openExport: () -> Unit = {},
    val openRename: () -> Unit = {},
    val delete: () -> Unit = {},
    val leave: () -> Unit = {},
    val confirm: () -> Unit = {},
    val cancelConfirm: () -> Unit = {},
    val inviteName: (String) -> Unit = {},
    val inviteRole: (PlaylistRole) -> Unit = {},
    val invite: () -> Unit = {},
    val setRole: (PlaylistMember, PlaylistRole) -> Unit = { _, _ -> },
    val removeMember: (PlaylistMember) -> Unit = {},
    val chooseTab: (ExportTab) -> Unit = {},
    val chooseFormat: (String) -> Unit = {},
    val chooseListFormat: (String) -> Unit = {},
    val chooseService: (String) -> Unit = {},
    val exportName: (String) -> Unit = {},
    val togglePublic: () -> Unit = {},
    val sendExport: () -> Unit = {},
    val resetJob: () -> Unit = {},
    val toggleMissing: () -> Unit = {},
    val downloadList: () -> Unit = {},
    val copyMissing: () -> Unit = {},
    val createLink: () -> Unit = {},
    val shareLink: () -> Unit = {},
    val revokeLink: () -> Unit = {},
    val renameTitle: (String) -> Unit = {},
    val renameDescription: (String) -> Unit = {},
    val saveRename: () -> Unit = {},
)

/** The ⋯ menu: the acts a playlist is for, never a bare delete icon. */
@Composable
fun PlaylistMoreSheet(state: PlaylistModel.State, actions: PlaylistSheetActions) {
    NzSheet(visible = state.more, onDismiss = actions.close) {
        Text("Playlist", style = Sillon.type.titleSmall, color = Sillon.colors.tx)
        MoreEntry("Exporter", NzGlyph.Upload) {
            actions.close()
            actions.openExport()
        }
        MoreEntry("Collaborateurs", NzGlyph.Users) {
            actions.close()
            actions.openMembers()
        }
        if (state.canManage) {
            MoreEntry("Renommer", NzGlyph.Edit) {
                actions.close()
                actions.openRename()
            }
            MoreEntry("Supprimer la playlist", NzGlyph.Trash, danger = true) {
                actions.close()
                actions.delete()
            }
        }
        if (state.role == PlaylistRole.EDITOR || state.role == PlaylistRole.VIEWER) {
            MoreEntry("Quitter la playlist", NzGlyph.LogOut) {
                actions.close()
                actions.leave()
            }
        }
    }
}

@Composable
private fun MoreEntry(
    label: String,
    glyph: NzGlyph,
    danger: Boolean = false,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.field))
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Icon(
            glyph = glyph,
            size = 18.dp,
            tint = if (danger) colors.hrt else colors.tx,
            contentDescription = null,
        )
        Text(
            label,
            style = Sillon.type.body,
            color = if (danger) colors.hrt else colors.tx,
        )
    }
}

/**
 * The collaborators (`PlaylistMembersSheet.svelte`): who is on it, and — for an
 * owner or an admin — one exact account name to invite, a role per member, and
 * « Retirer ». A viewer sees the list and can leave.
 */
@Composable
fun PlaylistMembersSheet(state: PlaylistModel.State, actions: PlaylistSheetActions) {
    val sheet = state.members ?: return
    val colors = Sillon.colors
    NzSheet(visible = true, onDismiss = actions.close) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Collaborateurs", style = Sillon.type.titleSmall, color = colors.tx)
            Spacer(Modifier.weight(1f))
            NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = actions.close)
        }
        if (sheet.canManage) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                PlaylistField(
                    value = sheet.username,
                    onValueChange = actions.inviteName,
                    placeholder = "Nom d'utilisateur",
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(
                        imeAction = ImeAction.Done,
                        capitalization = KeyboardCapitalization.None,
                    ),
                )
                NzButton(
                    text = if (sheet.busy == "invite") "…" else "Inviter",
                    onClick = actions.invite,
                    enabled = sheet.busy.isEmpty() && sheet.username.isNotBlank(),
                )
            }
            NzSegmented(
                options = listOf("Éditeur", "Lecteur"),
                selectedIndex = if (sheet.newRole == PlaylistRole.VIEWER) 1 else 0,
                onSelect = { index ->
                    actions.inviteRole(if (index == 1) PlaylistRole.VIEWER else PlaylistRole.EDITOR)
                },
            )
        }
        if (sheet.error != null) {
            Text(sheet.error, style = Sillon.type.bodySmall, color = colors.hrt)
        }
        LazyColumn(modifier = Modifier.heightIn(max = 420.dp)) {
            items(sheet.members, key = { it.id }) { member ->
                MemberRow(
                    member = member,
                    canManage = sheet.canManage,
                    busy = sheet.busy == "member:${member.id}",
                    onRole = { role -> actions.setRole(member, role) },
                    onRemove = { actions.removeMember(member) },
                )
            }
        }
        if (sheet.role == PlaylistRole.EDITOR || sheet.role == PlaylistRole.VIEWER) {
            NzButton(
                text = "Quitter la playlist",
                onClick = actions.leave,
                kind = NzButtonKind.Ghost,
                glyph = NzGlyph.LogOut,
            )
        }
    }
}

@Composable
private fun MemberRow(
    member: PlaylistMember,
    canManage: Boolean,
    busy: Boolean,
    onRole: (PlaylistRole) -> Unit,
    onRemove: () -> Unit,
) {
    val colors = Sillon.colors
    val owner = member.role == PlaylistRole.OWNER
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Avatar(name = member.name, sizeDp = 32.dp)
            Column(modifier = Modifier.weight(1f)) {
                Text(member.name, style = Sillon.type.body, color = colors.tx, maxLines = 1)
                Text(roleLabel(member.role), style = Sillon.type.labelSmall, color = colors.dim)
            }
            if (busy) {
                CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
            } else if (canManage && !owner) {
                NzIconButton(
                    glyph = NzGlyph.Trash,
                    contentDescription = "Retirer ${member.name}",
                    onClick = onRemove,
                )
            }
        }
        if (canManage && !owner) {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                NzChip(
                    text = "Éditeur",
                    selected = member.role == PlaylistRole.EDITOR,
                    onClick = { onRole(PlaylistRole.EDITOR) },
                )
                NzChip(
                    text = "Lecteur",
                    selected = member.role == PlaylistRole.VIEWER,
                    onClick = { onRole(PlaylistRole.VIEWER) },
                )
            }
        }
    }
}

/**
 * A one-line field in Sillon's own shape: the design system draws buttons, chips
 * and switches, and a text field is Material3's themed here rather than copied
 * into `:designsystem` (that module is MB1's, one writer per file).
 */
@Composable
private fun PlaylistField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    singleLine: Boolean = false,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        placeholder = { Text(placeholder, style = Sillon.type.bodySmall) },
        singleLine = singleLine,
        keyboardOptions = keyboardOptions,
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(NzRadius.chip),
    )
}

/** The role in French (`actions.js#roleLabel`). */
internal fun roleLabel(role: PlaylistRole): String = when (role) {
    PlaylistRole.OWNER -> "Propriétaire"
    PlaylistRole.EDITOR -> "Éditeur"
    PlaylistRole.VIEWER -> "Lecteur"
    PlaylistRole.ADMIN -> "Administrateur"
    PlaylistRole.PUBLIC -> "Lien public"
}

/**
 * The export panel (`ExportSheet.svelte`): the audio ZIP, another service, a LIST
 * file, and the public read-only link — the last tab only for a playlist the
 * caller may manage.
 */
@Composable
fun PlaylistExportSheet(state: PlaylistModel.State, actions: PlaylistSheetActions) {
    val export = state.export ?: return
    val colors = Sillon.colors
    val canLink = state.canManage
    NzSheet(visible = true, onDismiss = actions.close) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Exporter", style = Sillon.type.titleSmall, color = colors.tx)
            Spacer(Modifier.weight(1f))
            NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = actions.close)
        }
        Text(
            "« ${state.displayTitle} »",
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        val tabs = if (canLink) listOf("Service", "Audio", "Liste", "Lien") else listOf("Service", "Audio", "Liste")
        val index = when (export.tab) {
            ExportTab.Service -> 0
            ExportTab.Audio -> 1
            ExportTab.List -> 2
            ExportTab.Link -> if (canLink) 3 else 0
        }
        NzSegmented(
            options = tabs,
            selectedIndex = index,
            onSelect = { at ->
                actions.chooseTab(
                    when (tabs[at]) {
                        "Service" -> ExportTab.Service
                        "Audio" -> ExportTab.Audio
                        "Liste" -> ExportTab.List
                        else -> ExportTab.Link
                    },
                )
            },
        )
        when (export.tab) {
            ExportTab.Audio -> AudioTab(export, state, actions)
            ExportTab.List -> ListTab(export, actions)
            ExportTab.Service -> ServiceTab(export, state, actions)
            ExportTab.Link -> LinkTab(export, actions)
        }
    }
}

@Composable
private fun AudioTab(
    export: PlaylistModel.ExportUi,
    state: PlaylistModel.State,
    actions: PlaylistSheetActions,
) {
    val colors = Sillon.colors
    when {
        export.formatsLoading -> Text("Chargement des formats…", style = Sillon.type.bodySmall, color = colors.dim)
        export.formatsFailure != null -> Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text("Impossible de lire les formats : ${export.formatsFailure}", style = Sillon.type.bodySmall, color = colors.dim)
        }
        export.formats.isEmpty() -> Text(
            "Aucun format d'export disponible sur ce serveur.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        else -> Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            export.formats.forEach { format ->
                ChoiceRow(
                    label = format.label,
                    detail = ".${format.ext}",
                    selected = export.fmt == format.id,
                    onClick = { actions.chooseFormat(format.id) },
                )
            }
            Text(
                "Un fichier ZIP avec un fichier par titre et une playlist .m3u. " +
                    "Les titres pas encore archivés sont récupérés au passage : un gros export peut prendre un moment.",
                style = Sillon.type.labelSmall,
                color = colors.dim,
            )
        }
    }
}

@Composable
private fun ListTab(export: PlaylistModel.ExportUi, actions: PlaylistSheetActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        export.listFormats.forEach { format ->
            ChoiceRow(
                label = format.label,
                detail = ".${format.ext}",
                selected = export.listFmt == format.id,
                onClick = { actions.chooseListFormat(format.id) },
            )
        }
        Text(
            "Le nom des titres, pas leur son. Pour Apple Music, Tidal ou Amazon Music : " +
                "importez le CSV sur TuneMyMusic ou Soundiiz, qui le recréent là-bas.",
            style = Sillon.type.labelSmall,
            color = colors.dim,
        )
        NzButton(
            text = "Partager la liste",
            onClick = actions.downloadList,
            glyph = NzGlyph.Share,
            kind = NzButtonKind.Secondary,
        )
    }
}

@Composable
private fun ServiceTab(
    export: PlaylistModel.ExportUi,
    state: PlaylistModel.State,
    actions: PlaylistSheetActions,
) {
    val colors = Sillon.colors
    val services = export.services
    val job = export.job
    when {
        export.servicesFailure != null -> Text(
            "Impossible de lire les comptes reliés : ${export.servicesFailure}",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )

        services == null -> Text("Chargement des services…", style = Sillon.type.bodySmall, color = colors.dim)

        job != null -> Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text(
                text = jobLabel(job),
                style = Sillon.type.bodySmall,
                color = if (job.error != null) colors.hrt else colors.dim,
            )
            job.error?.let {
                Text(it, style = Sillon.type.bodySmall, color = colors.hrt)
            }
            val url = job.url
            if (url != null) {
                Text(url, style = Sillon.type.mono, color = colors.acText, maxLines = 1)
            }
            if (job.missingCount > 0u) {
                NzButton(
                    text = if (export.missingShown) "Masquer les titres introuvables" else "Voir les titres introuvables",
                    onClick = actions.toggleMissing,
                    kind = NzButtonKind.Ghost,
                    glyph = if (export.missingShown) NzGlyph.ChevronUp else NzGlyph.ChevronDown,
                )
                if (export.missingShown) {
                    job.missing.forEach { missing ->
                        Text(
                            "${missing.artist} — ${missing.title}",
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    if (job.missingCount.toInt() > job.missing.size) {
                        Text(
                            "… et ${job.missingCount.toInt() - job.missing.size} autres",
                            style = Sillon.type.labelSmall,
                            color = colors.dim,
                        )
                    }
                    NzButton(
                        text = "Copier la liste",
                        onClick = actions.copyMissing,
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.Copy,
                    )
                }
            }
            if (!job.running) {
                NzButton(text = "Autre export", onClick = actions.resetJob, kind = NzButtonKind.Ghost)
            }
        }

        else -> Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            services.forEach { service ->
                val usable = service.linked
                ChoiceRow(
                    label = service.name,
                    detail = when {
                        usable && service.account != null -> service.account.orEmpty()
                        usable -> "compte du serveur"
                        service.ready -> "Relier dans Réglages"
                        else -> "indisponible"
                    },
                    selected = export.service == service.id,
                    enabled = usable,
                    onClick = { if (usable) actions.chooseService(service.id) },
                )
            }
            if (services.any { it.linked }) {
                PlaylistField(
                    value = export.name,
                    onValueChange = actions.exportName,
                    placeholder = "Nom de la playlist",
                    singleLine = true,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    NzSwitch(
                        checked = export.isPublic,
                        onCheckedChange = { actions.togglePublic() },
                        label = "Playlist publique",
                    )
                    Spacer(Modifier.weight(1f))
                }
                NzButton(
                    text = if (export.sending) "…" else "Exporter",
                    onClick = actions.sendExport,
                    glyph = NzGlyph.Upload,
                    enabled = !export.sending && export.service != null,
                )
            } else {
                Text(
                    "Aucun compte relié. Reliez Spotify ou YouTube Music dans Réglages → Sources.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
            Text(
                "Pour Apple Music, Tidal… : l'onglet Liste exporte un CSV à importer sur TuneMyMusic ou Soundiiz.",
                style = Sillon.type.labelSmall,
                color = colors.dim,
            )
            if (state.canManage) {
                Spacer(Modifier.height(NzSpacing.xxs))
            }
        }
    }
}

/** The job's one line: what it is doing, or what it did (`jobSummary`). */
internal fun jobLabel(job: uniffi.nz_ffi.PlaylistExportJob): String = when {
    job.running && job.stage == "create" -> "Création de la playlist…"
    job.running -> "Recherche ${job.done} / ${job.total} · ${job.matched} trouvés"
    job.error != null -> "Échec : ${job.error}"
    else -> "${job.matched} titres trouvés sur ${job.total}"
}

@Composable
private fun LinkTab(export: PlaylistModel.ExportUi, actions: PlaylistSheetActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Text(
            "Toute personne avec ce lien peut voir la playlist et écouter les titres déjà archivés. Rien n'est modifiable.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        when {
            export.linkLoading -> Text("Chargement…", style = Sillon.type.bodySmall, color = colors.dim)

            export.link != null -> {
                NzCard {
                    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                        Text("Lien public", style = Sillon.type.labelSmall, color = colors.dim)
                        Text(export.link.url, style = Sillon.type.mono, color = colors.tx, maxLines = 1)
                        Text(
                            expiryLabel(export.link.expires),
                            style = Sillon.type.labelSmall,
                            color = colors.dim,
                        )
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    NzButton(text = "Partager", onClick = actions.shareLink, glyph = NzGlyph.Share)
                    NzButton(
                        text = "Révoquer",
                        onClick = actions.revokeLink,
                        kind = NzButtonKind.Secondary,
                        glyph = NzGlyph.Trash,
                        enabled = !export.linkBusy,
                    )
                }
            }

            else -> NzButton(
                text = if (export.linkBusy) "…" else "Créer un lien public",
                onClick = actions.createLink,
                glyph = NzGlyph.Link,
                enabled = !export.linkBusy,
            )
        }
        export.linkFailure?.let {
            Text(it, style = Sillon.type.bodySmall, color = colors.hrt)
        }
    }
}

/** « Sans expiration » / « Expire le 1 décembre 2026 ». */
internal fun expiryLabel(expires: String?): String {
    if (expires.isNullOrEmpty()) return "Sans expiration"
    val date = runCatching { java.time.LocalDate.parse(expires.take(10)) }.getOrNull()
        ?: return "Expire bientôt"
    val formatted = date.format(
        java.time.format.DateTimeFormatter.ofPattern("d MMMM yyyy", java.util.Locale.FRANCE),
    )
    return "Expire le $formatted"
}

@Composable
private fun ChoiceRow(
    label: String,
    detail: String,
    selected: Boolean,
    enabled: Boolean = true,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.field))
            .background(if (selected) colors.s2 else colors.s1)
            .clickable(enabled = enabled, role = Role.RadioButton, onClick = onClick)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                label,
                style = Sillon.type.body,
                color = if (enabled) colors.tx else colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(detail, style = Sillon.type.labelSmall, color = colors.dim, maxLines = 1)
        }
        if (selected) {
            Icon(glyph = NzGlyph.Check, size = 18.dp, tint = colors.acText, contentDescription = "Choisi")
        }
    }
}

/** The one confirmation before a destructive act (delete, leave, revoke). */
@Composable
fun PlaylistConfirmSheet(state: PlaylistModel.State, actions: PlaylistSheetActions) {
    val confirm = state.confirmForDialog() ?: return
    NzSheet(visible = true, onDismiss = actions.cancelConfirm) {
        Text(confirm.title, style = Sillon.type.titleSmall, color = Sillon.colors.tx)
        Text(confirm.body, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            NzButton(text = "Annuler", onClick = actions.cancelConfirm, kind = NzButtonKind.Secondary)
            NzButton(text = confirm.action, onClick = actions.confirm)
        }
    }
}

/** A confirmation as the sheet draws it. */
internal data class Confirmation(val title: String, val body: String, val action: String)

internal fun PlaylistModel.State.confirmForDialog(): Confirmation? {
    val title = displayTitle
    return when (confirm) {
        PlaylistModel.Confirm.Delete -> Confirmation(
            title = "Supprimer la playlist « $title » ?",
            body = "Les titres ne sont pas supprimés : seule la playlist disparaît.",
            action = "Supprimer",
        )
        PlaylistModel.Confirm.Leave -> Confirmation(
            title = "Quitter la playlist « $title » ?",
            body = "Vous pourrez y revenir si quelqu'un vous y invite de nouveau.",
            action = "Quitter",
        )
        PlaylistModel.Confirm.RevokeLink -> Confirmation(
            title = "Révoquer le lien public ?",
            body = "Le lien est mort dès la prochaine ouverture : personne ne pourra plus lire la playlist par ce lien.",
            action = "Révoquer",
        )
        null -> null
    }
}

/** The band's rename fields, drawn in the header when the person edits it. */
@Composable
fun PlaylistRenameSheet(state: PlaylistModel.State, actions: PlaylistSheetActions) {
    if (!state.editingMeta) return
    NzSheet(visible = true, onDismiss = actions.close) {
        Text("Renommer la playlist", style = Sillon.type.titleSmall, color = Sillon.colors.tx)
        PlaylistField(
            value = state.draftTitle,
            onValueChange = actions.renameTitle,
            placeholder = "Nom de la playlist",
            singleLine = true,
        )
        PlaylistField(
            value = state.draftDescription,
            onValueChange = actions.renameDescription,
            placeholder = "Description (optionnel)",
        )
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            NzButton(text = "Annuler", onClick = actions.close, kind = NzButtonKind.Secondary)
            NzButton(
                text = "Enregistrer",
                onClick = actions.saveRename,
                glyph = NzGlyph.Check,
                enabled = state.draftTitle.isNotBlank(),
            )
        }
    }
}

/**
 * « Ajouter des titres » (`AddTracksSheet.svelte`): the library searched, the rows
 * already in the playlist marked, and one « Ajouter » that appends every picked
 * row in ONE request.
 *
 * The sheet asks MB6's own search (`searchTracks`), never a copy of the library:
 * a 4 000-row playlist plus an 8 000-row library is not something a phone holds
 * twice, and the search's folding is `appcore`'s.
 */
@Composable
fun AddTracksSheet(
    state: PlaylistModel.State,
    onTerm: (String) -> Unit,
    onAdd: (List<uniffi.nz_ffi.TrackRow>) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val add = state.add ?: return
    val colors = Sillon.colors
    val picked = androidx.compose.runtime.remember { androidx.compose.runtime.mutableStateListOf<uniffi.nz_ffi.TrackRow>() }
    NzSheet(visible = true, onDismiss = onDismiss, modifier = modifier) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Ajouter des titres", style = Sillon.type.titleSmall, color = colors.tx)
            Spacer(Modifier.weight(1f))
            NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = onDismiss)
        }
        PlaylistField(
            value = add.term,
            onValueChange = onTerm,
            placeholder = "Rechercher dans la bibliothèque…",
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        )
        when {
            add.failure != null -> Text(
                "La recherche a échoué : ${add.failure}",
                style = Sillon.type.bodySmall,
                color = colors.hrt,
            )

            add.loading -> Text("Recherche…", style = Sillon.type.bodySmall, color = colors.dim)

            add.rows.isEmpty() -> Text(
                "Aucun titre ne correspond à « ${add.term} ».",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
        LazyColumn(modifier = Modifier.heightIn(max = 360.dp)) {
            items(add.rows, key = { it.key }) { row ->
                val already = row.key in add.existing
                val chosen = picked.any { it.key == row.key }
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = NzMinTouchTarget)
                        .clip(RoundedCornerShape(NzRadius.field))
                        .background(if (chosen) colors.s2 else colors.s1)
                        .clickable(enabled = !already, role = Role.Checkbox) {
                            if (chosen) picked.removeAll { it.key == row.key } else picked.add(row)
                        }
                        .padding(horizontal = NzSpacing.s),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
                ) {
                    Icon(
                        glyph = if (chosen) NzGlyph.Check else NzGlyph.Plus,
                        size = 18.dp,
                        tint = if (already) colors.dim else colors.acText,
                        contentDescription = null,
                    )
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            row.title,
                            style = Sillon.type.body,
                            color = if (already) colors.dim else colors.tx,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            if (already) "Déjà dans la playlist" else row.artists,
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            NzButton(text = "Annuler", onClick = onDismiss, kind = NzButtonKind.Secondary)
            NzButton(
                text = if (picked.size > 1) "Ajouter ${picked.size} titres" else "Ajouter",
                onClick = { onAdd(picked.toList()) },
                glyph = NzGlyph.Plus,
                enabled = picked.isNotEmpty(),
            )
        }
    }
}
