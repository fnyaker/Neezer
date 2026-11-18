package fr.neezer.mobile.feature.admin

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
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
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.AdminCleanup
import uniffi.nz_ffi.AdminCredits
import uniffi.nz_ffi.AdminEnrich
import uniffi.nz_ffi.AdminInviteRow
import uniffi.nz_ffi.AdminMigrate
import uniffi.nz_ffi.AdminPlan
import uniffi.nz_ffi.AdminRules
import uniffi.nz_ffi.AdminSimilar
import uniffi.nz_ffi.AdminTranscribe
import uniffi.nz_ffi.AdminUserRow

/**
 * Réglages → *Utilisateurs* et *Serveur* (lot MB17).
 *
 * L'écran de MB16 est le point d'extension : son menu porte les deux catégories
 * et sa `when` appelle CETTE fonction. Rien d'autre de Réglages ne bouge.
 *
 * Ce que la SPA fait, dans le même ordre :
 *
 * - **Utilisateurs** (`AdminUsers.svelte`) : le compte et son rôle, l'état, la
 *   dernière connexion, la recherche par nom (sur le SERVEUR), les six pastilles,
 *   « Charger plus » jusqu'à 250 lignes, puis les invitations en attente avec
 *   « Inviter quelqu'un » et une feuille par ligne.
 * - **Serveur** (`Settings.svelte` *Serveur*) : l'archive et sa jauge, la mise à
 *   niveau des fichiers, les faits MusicBrainz, les crédits, les règles
 *   d'archivage, le nettoyage (aperçu AVANT de supprimer), le stockage et son
 *   bouton de cache, le quota d'upload, l'analyse et ses trois étapes, les
 *   voisins sonores, le remplacement automatique, la transcription, et l'état du
 *   compte Deezer.
 *
 * Aucune de ces cartes n'invente un réglage : chacune est un contrôle de la SPA.
 */
@Composable
fun AdminSections(
    section: String? = null,
    modifier: Modifier = Modifier,
    service: AdminService? = AppAdmin.service(),
) {
    val scope = rememberCoroutineScope()
    val model = remember(service) { service?.let { AdminModel(it, scope) } }
    LaunchedEffect(section, service) {
        model?.open(section)
    }
    if (model == null) {
        AdminCoreUnavailable(modifier)
        return
    }
    AdminPage(section = section, model = model, modifier = modifier)
}

/** The dispatch alone, over a model a test built — the real pages, no route. */
@Composable
internal fun AdminPage(section: String?, model: AdminModel, modifier: Modifier = Modifier) {
    val state = model.state
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        when {
            state.admin == false -> AdminMemberNotice()
            section == AdminModel.SECTION_USERS -> UsersPage(model, state)
            section == AdminModel.SECTION_SERVER -> ServerPage(model, state)
            else -> Unit
        }
    }
}

/** A member reached the page by a deep link: no request, and why. */
@Composable
private fun AdminMemberNotice() {
    AdminCard(
        title = "Administration du serveur",
        subtitle = "Ces pages sont réservées aux administrateurs de ce serveur. Votre compte n'a plus ce rôle : reconnectez-vous pour le vérifier.",
    ) {}
}

/** The native core could not be built: say so instead of drawing empty cards. */
@Composable
private fun AdminCoreUnavailable(modifier: Modifier) {
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        AdminCard(
            title = "Administration du serveur",
            subtitle = "Le noyau de l'application n'a pas pu être chargé : ces pages sont indisponibles.",
        ) {
            Text(
                text = fr.neezer.mobile.bridge.NzBridge.failure ?: "cause inconnue",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
        }
    }
}

// --- *Utilisateurs* ----------------------------------------------------------

@Composable
private fun UsersPage(model: AdminModel, state: AdminModel.State) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        // A tap on a row opens a sheet, and the sheet IS the page until it is
        // closed: the SPA shows a modal, and a phone has no second column. The
        // list behind it is one back tap away (`closeSheet`).
        val sheet = state.sheet
        val openUser = state.openUser
        if (sheet != null || openUser != null) {
            when (sheet) {
                is AdminModel.Sheet.Invite -> InviteSheet(model, state, sheet.source)
                is AdminModel.Sheet.Confirm -> ConfirmSheet(model, state, sheet)
                null -> state.rows.users().firstOrNull { it.id == openUser }?.let { UserSheet(model, state, it) }
            }
            return@Column
        }
        if (state.usersFailure != null && state.rows.users().isEmpty()) {
            LoadFailed(cause = state.usersFailure, onRetry = model::retryUsers)
            return@Column
        }

        // The SPA's head, re-flowed for a phone: the two actions on their own
        // line (a count line squeezed beside them wrapped mid-phrase), then what
        // the list is and when it was refreshed, full width.
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s, Alignment.End),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            NzButton(
                text = "Inviter",
                onClick = { model.openInvite() },
                glyph = NzGlyph.PersonAdd,
            )
            NzIconButton(
                glyph = NzGlyph.Refresh,
                contentDescription = "Rafraîchir la liste",
                onClick = model::refresh,
                enabled = !state.usersLoading,
            )
        }
        if (state.usersFailure == null) {
            Text(
                text = state.countLine(),
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
        state.refreshedAt?.let { at ->
            Text(
                text = "Actualisé à " + AdminCopy.clockTimeOf(at),
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }

        if (state.usersStale) AdminStaleNote()

        AdminSearchField(
            value = state.query,
            onValue = model::setQuery,
            onDone = model::submitQuery,
            onClear = model::clearQuery,
        )
        if (state.query.trim().length == 1) {
            AdminHint("Saisissez au moins 2 caractères pour filtrer.")
        }

        AdminChips(
            options = AdminCopy.FILTERS.map { it.id to it.label },
            selected = state.filter.id,
            onSelect = { model.setFilter(AccountFilter.of(it)) },
        )
        if (state.hasFilters) {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                NzButton(
                    text = "Effacer les filtres",
                    onClick = model::clearFilters,
                    kind = NzButtonKind.Secondary,
                )
            }
        }

        state.actionError?.let { AdminError(it) }

        when {
            state.usersLoading && state.rows.isEmpty() -> AdminHint("Chargement des comptes…")
            state.visible.isEmpty() -> EmptyAccounts(model, state)
            else -> AccountTable(model, state)
        }

        if (state.invitesLoading) AdminHint("Chargement des invitations…")
        state.invitesFailure?.let { cause ->
            AdminCardFailure(cause = cause, onRetry = model::retryInvites)
        }
        state.moreFailure?.let { cause ->
            AdminCardFailure(cause = cause, onRetry = model::loadMore)
        }

        if (state.showMore) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Text(text = state.rangeLine(), style = Sillon.type.bodySmall, color = colors.dim)
                Spacer(Modifier.weight(1f))
                NzButton(
                    text = if (state.moreLoading) "Chargement…" else "Charger plus",
                    onClick = model::loadMore,
                    kind = NzButtonKind.Secondary,
                    enabled = !state.moreLoading,
                )
            }
        } else if (state.visible.isNotEmpty()) {
            Text(text = state.rangeLine(), style = Sillon.type.bodySmall, color = colors.dim)
        }
        if (state.capped) {
            AdminHint("${AdminCopy.MAX_ROWS} comptes chargés. Affinez la recherche pour trouver les autres.")
        }

    }
}

/** No row to draw: three different nothings, as the SPA distinguishes them. */
@Composable
private fun EmptyAccounts(model: AdminModel, state: AdminModel.State) {
    when {
        state.filter == AccountFilter.Invited && state.term.isNotEmpty() -> EmptyState(
            title = "Aucune invitation ne correspond.",
            body = "Essayez un autre nom ou retirez la recherche.",
            actionLabel = "Effacer les filtres",
            onAction = model::clearFilters,
            glyph = NzGlyph.PersonAdd,
        )
        state.filter == AccountFilter.Invited -> EmptyState(
            title = "Aucune invitation en attente.",
            body = "Un lien d'invitation crée un compte membre, une seule fois.",
            actionLabel = "Inviter quelqu'un",
            onAction = { model.openInvite() },
            glyph = NzGlyph.PersonAdd,
        )
        state.hasFilters -> EmptyState(
            title = "Aucun compte ne correspond.",
            body = "Essayez un autre nom ou retirez les filtres.",
            actionLabel = "Effacer les filtres",
            onAction = model::clearFilters,
            glyph = NzGlyph.Users,
        )
        else -> EmptyState(
            title = "Aucun compte pour l'instant.",
            body = "Invitez la première personne : le lien crée un compte membre, une seule fois.",
            actionLabel = "Inviter quelqu'un",
            onAction = { model.openInvite() },
            glyph = NzGlyph.PersonAdd,
        )
    }
}

/** The table: the accounts, then the pending invitations under their own line. */
@Composable
private fun AccountTable(model: AdminModel, state: AdminModel.State) {
    val colors = Sillon.colors
    // A plain Column, not a LazyColumn, and deliberately: the parent (Réglages)
    // already scrolls, and this list is bounded by the SPA's own cap (250 rows,
    // 25 per page) with the search running on the SERVER. There is no 4 000-row
    // list here to window (docs/agents/mobile.md, MB17).
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.card))
            .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.card))
            .background(colors.s1),
    ) {
        state.visibleUsers.forEach { user ->
            AccountRowItem(
                name = user.name,
                subtitle = "${AdminCopy.roleLabel(user)} · ${AdminCopy.stateLabel(user)}",
                detail = "Dernière connexion : " + AdminCopy.lastSeenLabel(user),
                isAdmin = user.admin,
                isMe = user.name == state.adminName,
                isInvite = false,
                onClick = { model.openUser(user) },
            )
        }
        if (state.visibleInvites.isNotEmpty()) {
            if (state.filter == AccountFilter.All && state.visibleUsers.isNotEmpty()) {
                Text(
                    text = "INVITATIONS EN ATTENTE",
                    style = Sillon.type.labelMono,
                    color = colors.dim,
                    modifier = Modifier.padding(horizontal = NzSpacing.m, vertical = NzSpacing.s),
                )
            }
            state.visibleInvites.forEach { invite ->
                AccountRowItem(
                    name = AdminCopy.inviteName(invite),
                    subtitle = "Membre · " + AdminCopy.inviteStateLabel(invite),
                    detail = AdminCopy.lastSeenLabel(invite),
                    isAdmin = false,
                    isMe = false,
                    isInvite = true,
                    onClick = { model.openInvite(invite) },
                )
            }
        }
    }
}

/** One line of the table: a groove avatar, the name, the role, the state. */
@Composable
private fun AccountRowItem(
    name: String,
    subtitle: String,
    detail: String,
    isAdmin: Boolean,
    isMe: Boolean,
    isInvite: Boolean,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 72.dp)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .semantics {
                contentDescription = if (isInvite) "Ouvrir l'invitation $name" else "Ouvrir le compte de $name"
            },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Avatar(name = name, sizeDp = 40.dp)
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                Text(
                    text = name,
                    style = Sillon.type.label,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (isMe) {
                    Text(text = "VOUS", style = Sillon.type.labelMono, color = colors.dim)
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                if (isAdmin) Icon(glyph = NzGlyph.Shield, size = 13.dp, tint = colors.acText)
                Text(text = subtitle, style = Sillon.type.bodySmall, color = colors.dim, maxLines = 1)
            }
            Text(
                text = detail,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Icon(glyph = NzGlyph.ChevronRight, size = 18.dp, tint = colors.dim)
    }
}

/** « Chercher un compte » — the pill field, 48 dp, with its clear button. */
@Composable
private fun AdminSearchField(
    value: String,
    onValue: (String) -> Unit,
    onDone: () -> Unit,
    onClear: () -> Unit,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.chip)
    BasicTextField(
        value = value,
        onValueChange = onValue,
        singleLine = true,
        textStyle = Sillon.type.body.copy(color = colors.tx),
        cursorBrush = SolidColor(colors.acText),
        modifier = Modifier
            .fillMaxWidth()
            .height(NzMinTouchTarget)
            .semantics { contentDescription = "Rechercher un compte" },
        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
            imeAction = androidx.compose.ui.text.input.ImeAction.Search,
        ),
        keyboardActions = androidx.compose.foundation.text.KeyboardActions(
            onSearch = { onDone() },
        ),
        decorationBox = { inner ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = NzMinTouchTarget)
                    .clip(shape)
                    .background(colors.s1)
                    .border(1.dp, colors.ln, shape)
                    .padding(horizontal = NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Icon(glyph = NzGlyph.Search, size = 16.dp, tint = colors.dim)
                Box(Modifier.weight(1f)) {
                    if (value.isEmpty()) {
                        Text(
                            text = "Rechercher un nom…",
                            style = Sillon.type.body,
                            color = colors.dim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    inner()
                }
                if (value.isNotEmpty()) {
                    NzIconButton(
                        glyph = NzGlyph.Close,
                        contentDescription = "Effacer la recherche",
                        onClick = onClear,
                        size = 15.dp,
                    )
                }
            }
        },
    )
}

// --- the sheets ------------------------------------------------------------------

/** *Ouvrir le compte* — the account's facts, then its actions. */
@Composable
private fun UserSheet(model: AdminModel, state: AdminModel.State, user: AdminUserRow) {
    val colors = Sillon.colors
    val me = user.name == state.adminName
    AdminCard(
        title = user.name,
        subtitle = "${AdminCopy.roleLabel(user)} · ${AdminCopy.stateLabel(user)}" + if (me) " · vous" else "",
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = "Dernière connexion : " + AdminCopy.relativeTime(user.lastLogin),
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
                Text(
                    text = "Adresse e-mail : " + (user.mail ?: "—"),
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
                val detail = state.userDetail.data
                if (detail != null && detail.user.id == user.id) {
                    Text(
                        text = "${detail.uploads} fichier(s) importé(s) · ${detail.playlists} playlist(s)",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                } else if (state.userDetail.loading) {
                    Text(
                        text = "Chargement des fichiers et des playlists…",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
            }
            NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = model::closeSheet)
        }

        if (me) {
            AdminHint("Vous ne pouvez pas modifier votre propre compte ici. Demandez à un autre administrateur.")
        }
        state.userDetail.failure?.let { AdminHint(it, warn = true) }
        state.actionError?.let { AdminError(it) }

        AdminActionRow(
            title = if (user.admin) "Retirer les droits d'administrateur" else "Nommer administrateur",
            description = if (user.admin) {
                "Garde le compte, perd les réglages du serveur et les comptes."
            } else {
                "Accès aux réglages du serveur et à tous les comptes."
            },
            glyph = NzGlyph.Shield,
            enabled = !me && !state.busy,
            onClick = { model.setRole(user, !user.admin) },
        )
        AdminActionRow(
            title = if (user.disabled) "Réactiver le compte" else "Désactiver le compte",
            description = if (user.disabled) {
                "La personne pourra se reconnecter."
            } else {
                "Déconnecte la personne partout. Ses données restent."
            },
            glyph = if (user.disabled) NzGlyph.Refresh else NzGlyph.MinusCircle,
            enabled = !me && !state.busy,
            onClick = { model.setDisabled(user, !user.disabled) },
        )
        AdminSubtitle("Zone dangereuse")
        AdminActionRow(
            title = "Supprimer le compte",
            description = "Définitif. Ses playlists et son historique partent ; l'audio archivé reste.",
            glyph = NzGlyph.Trash,
            enabled = !me && !state.busy,
            danger = true,
            onClick = { model.askDelete(user) },
        )
    }
}

/** *Inviter quelqu'un* / *Renvoyer l'invitation* — and the link, shown ONCE. */
@Composable
private fun InviteSheet(model: AdminModel, state: AdminModel.State, source: AdminInviteRow?) {
    val colors = Sillon.colors
    val context = LocalContext.current
    var note by remember(source?.id) { mutableStateOf(source?.note.orEmpty()) }
    var ttl by remember(source?.id) { mutableStateOf(AdminCopy.INVITE_DEFAULT_TTL) }
    val issued = state.issuedUrl

    AdminCard(
        title = if (issued != null) "Lien d'invitation créé" else if (source != null) "Renvoyer l'invitation" else "Inviter quelqu'un",
        subtitle = when {
            issued != null -> "Ce lien ne sera plus affiché. Il ne sert qu'à un compte."
            source != null -> "L'ancien lien sera révoqué immédiatement. Seul le nouveau fonctionnera."
            else -> "Le lien crée un compte membre. Il ne sert qu'une fois."
        },
    ) {
        if (issued != null) {
            androidx.compose.foundation.text.selection.SelectionContainer {
                Text(text = issued, style = Sillon.type.mono, color = colors.tx)
            }
            AdminHint("Copiez ce lien maintenant : il ne sera plus affiché.", warn = true)
            AdminActions {
                NzButton(
                    text = "Copier le lien",
                    onClick = {
                        copyToClipboard(context, issued)
                        model.notify("Lien copié")
                    },
                    glyph = NzGlyph.Copy,
                )
                NzButton(text = "Fermer", onClick = model::dismissIssued, kind = NzButtonKind.Secondary)
            }
            return@AdminCard
        }

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    text = if (source != null) "Pour qui" else "Pour qui ? (facultatif)",
                    style = Sillon.type.labelSmall,
                    color = colors.dim,
                )
            }
            NzIconButton(glyph = NzGlyph.Close, contentDescription = "Fermer", onClick = model::closeSheet)
        }
        if (source != null) {
            Text(text = source.note ?: "Sans note", style = Sillon.type.body, color = colors.tx)
        } else {
            AdminField(
                label = "Note, pour vous y retrouver",
                value = note,
                onValue = { if (it.length <= 64) note = it },
                numeric = false,
                hint = "${note.length}/64 — ce nom ne crée pas de compte et n'est pas montré à la personne invitée.",
            )
        }
        AdminSubtitle("Durée de validité")
        AdminChips(
            options = AdminCopy.INVITE_DURATIONS.map { it.first.toString() to it.second },
            selected = if (source != null) source.id else ttl.toString(),
            onSelect = { id -> if (source == null) ttl = id.toUInt() },
        )
        state.actionError?.let { AdminError(it) }
        AdminActions {
            NzButton(
                text = if (state.busy) "En cours…" else if (source != null) "Créer un nouveau lien" else "Créer le lien",
                onClick = { model.createInvite(note, ttl, source) },
                glyph = NzGlyph.Link,
                enabled = !state.busy,
            )
            if (source != null) {
                NzButton(
                    text = "Révoquer",
                    onClick = { model.confirmRevoke(source) },
                    kind = NzButtonKind.Secondary,
                    enabled = !state.busy,
                )
            }
        }
    }
}

/** The destructive confirmation: the name is typed, the server checks it. */
@Composable
private fun ConfirmSheet(model: AdminModel, state: AdminModel.State, sheet: AdminModel.Sheet.Confirm) {
    var typed by remember(sheet.user.id) { mutableStateOf("") }
    AdminCard(
        title = when (sheet.action) {
            ConfirmAction.Delete -> "Supprimer ${sheet.user.name} ?"
            ConfirmAction.Revoke -> "Révoquer l'invitation ?"
        },
        subtitle = when (sheet.action) {
            ConfirmAction.Delete ->
                "Cette action est définitive. Ses playlists et son historique seront supprimés. L'audio déjà archivé reste sur le serveur."
            ConfirmAction.Revoke ->
                "Le lien cessera de fonctionner immédiatement. La personne n'aura plus de moyen de créer son compte avec ce lien."
        },
    ) {
        if (sheet.action == ConfirmAction.Delete) {
            state.userDetail.data?.let { detail ->
                AdminHint(
                    "${detail.uploads} fichier(s) importé(s), attribués à votre compte, aucun fichier supprimé · " +
                        "${detail.playlists} playlist(s).",
                )
            }
            AdminField(
                label = "Tapez ${sheet.user.name} pour confirmer",
                value = typed,
                onValue = { typed = it },
                numeric = false,
            )
        }
        state.actionError?.let { AdminError(it) }
        AdminActions {
            NzButton(
                text = when (sheet.action) {
                    ConfirmAction.Delete -> if (state.busy) "En cours…" else "Supprimer définitivement"
                    ConfirmAction.Revoke -> if (state.busy) "Révocation…" else "Révoquer l'invitation"
                },
                onClick = {
                    when (sheet.action) {
                        ConfirmAction.Delete -> model.confirmDelete(sheet.user, typed)
                        ConfirmAction.Revoke -> {
                            val invite = state.invites.firstOrNull { it.id == sheet.user.id }
                            if (invite != null) model.confirmRevoke(invite)
                        }
                    }
                },
                glyph = NzGlyph.Trash,
                enabled = !state.busy && (sheet.action == ConfirmAction.Revoke || typed.trim() == sheet.user.name.trim()),
            )
            NzButton(text = "Annuler", onClick = model::closeSheet, kind = NzButtonKind.Secondary)
        }
    }
}

/** The platform clipboard: the SPA's `navigator.clipboard`, ported. */
internal fun copyToClipboard(context: Context, text: String) {
    val manager = context.getSystemService(ClipboardManager::class.java) ?: return
    manager.setPrimaryClip(ClipData.newPlainText("Neezer", text))
}

// --- *Serveur* -------------------------------------------------------------------

@Composable
private fun ServerPage(model: AdminModel, state: AdminModel.State) {
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        state.actionError?.let { AdminError(it) }
        ArchiveCard(model, state)
        MigrateCard(model, state)
        EnrichCard(model, state)
        CreditsCard(model, state)
        RulesCard(model, state)
        CleanupCard(model, state)
        StorageCard(model, state)
        QuotaSection(model, state)
        AnalysisCard(model, state)
        SimilarCard(model, state)
        AutomatchCard(model, state)
        TranscribeCard(model, state)
        DeezerCard(model, state)
    }
}

@Composable
private fun ArchiveCard(model: AdminModel, state: AdminModel.State) {
    val section = state.archive
    AdminCard(
        title = "Archive du serveur",
        subtitle = "Un titre archivé est à vous : il reste lisible même si Deezer le retire de son catalogue, ou si Deezer est injoignable. L'archivage est automatique ; ce bouton rattrape ce qui manquerait encore. Rien n'est jamais supprimé de l'archive.",
    ) {
        section.data?.let { status ->
            if (status.running) {
                AdminGauge(
                    fraction = if (status.total == 0u) 0f else status.done.toFloat() / status.total.toFloat(),
                    label = "Archivage en cours… ${status.done} / ${status.total}",
                    detail = "${status.archived} archivé(s)" +
                        (if (status.unavailable > 0u) " · ${status.unavailable} indisponible(s)" else ""),
                )
                AdminHint("Vous pouvez fermer l'application, le serveur continue.")
            } else {
                AdminActions {
                    AdminCopy.ARCHIVE_SCOPES.forEach { (scope, label) ->
                        NzButton(
                            text = label,
                            onClick = { model.startArchive(scope) },
                            kind = if (scope == "all") NzButtonKind.Primary else NzButtonKind.Secondary,
                            glyph = if (scope == "all") NzGlyph.Archive else null,
                        )
                    }
                }
                if (status.total > 0u) {
                    AdminHint(
                        "Dernier passage : ${status.archived} archivé(s)" +
                            (if (status.unavailable > 0u) ", ${status.unavailable} indisponible(s)" else "") +
                            (if (status.failed > 0u) ", ${status.failed} échec(s)" else "") + ".",
                    )
                }
            }
            status.error?.let { AdminHint(it, warn = true) }
        }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("archive") }) }
        if (section.data == null && section.failure == null) AdminHint("Chargement de l'archive…")
    }
}

@Composable
private fun MigrateCard(model: AdminModel, state: AdminModel.State) {
    val section = state.migrate
    AdminCard(
        title = "Mise à niveau des fichiers",
        subtitle = "Réécrit les titres archivés avant le détachement de Deezer : plus de volume Deezer dans les étiquettes, la date de sortie la plus ancienne, les faits dans le fichier compagnon. Sans risque : relancer ne change plus rien.",
    ) {
        section.data?.let { migrate -> MigrateBody(model, migrate) }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("migrate") }) }
    }
}

@Composable
private fun MigrateBody(model: AdminModel, migrate: AdminMigrate) {
    if (migrate.running) {
        AdminGauge(
            fraction = 0f,
            label = "Mise à niveau en cours… ${migrate.done} fichier(s)",
            detail = (migrate.counts.firstOrNull { it.name == "tags" }?.count ?: 0u).toString() + " étiquette(s) · " +
                (migrate.counts.firstOrNull { it.name == "bpm" }?.count ?: 0u).toString() + " tempo(s) appris",
            indeterminate = true,
        )
    } else {
        NzButton(
            text = "Mettre à niveau",
            onClick = model::startMigrate,
            glyph = NzGlyph.Refresh,
            kind = NzButtonKind.Secondary,
        )
        if (migrate.done > 0u || migrate.failed > 0u) {
            AdminHint(
                "Dernier passage : ${migrate.done} fichier(s)" +
                    (if (migrate.failed > 0u) ", ${migrate.failed} échec(s)" else "") + ".",
            )
        }
    }
    migrate.error?.let { AdminHint(it, warn = true) }
}

@Composable
private fun EnrichCard(model: AdminModel, state: AdminModel.State) {
    val section = state.enrich
    AdminCard(
        title = "Faits complétés par MusicBrainz",
        subtitle = "Date de première sortie, compositeur, parolier et producteur, sans jamais ralentir la lecture.",
    ) {
        section.data?.let { enrich -> EnrichBody(model, enrich) }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("enrich") }) }
    }
}

@Composable
private fun EnrichBody(model: AdminModel, enrich: AdminEnrich) {
    if (enrich.running) {
        AdminGauge(
            fraction = if (enrich.tried + enrich.untried == 0u) 0f else
                enrich.tried.toFloat() / (enrich.tried + enrich.untried).toFloat(),
            label = "${enrich.tried} titre(s) essayé(s)",
            indeterminate = true,
        )
    } else {
        AdminActions {
            NzButton(
                text = "Compléter les faits",
                onClick = { model.startEnrich(false) },
                glyph = NzGlyph.Refresh,
                kind = NzButtonKind.Secondary,
                enabled = enrich.enabled,
            )
            NzButton(
                text = "Tout recommencer",
                onClick = { model.startEnrich(true) },
                kind = NzButtonKind.Secondary,
                enabled = enrich.enabled,
            )
        }
        val found = enrich.found.sumOf { it.count.toInt() }
        AdminHint(
            when {
                !enrich.enabled -> "Désactivé dans la configuration ([enrich] enabled)."
                enrich.tried > 0u -> buildString {
                    append("$found titre(s) complété(s)")
                    if (enrich.found.isNotEmpty()) {
                        append(" (")
                        append(enrich.found.joinToString(", ") { "${it.count} par ${AdminCopy.ENRICH_METHODS[it.name] ?: it.name}" })
                        append(")")
                    }
                    append(", ${enrich.untried} pas encore essayé(s).")
                }
                else -> "Un titre archivé ou téléversé est complété tout seul, à une requête par seconde."
            },
        )
        if (!enrich.acoustid) {
            AdminHint("L'empreinte sonore (AcoustID) demande l'outil fpcalc et une clé.", warn = true)
        }
    }
    enrich.error?.let { AdminHint(it, warn = true) }
}

@Composable
private fun CreditsCard(model: AdminModel, state: AdminModel.State) {
    val section = state.credits
    AdminCard(
        title = "Crédits des titres",
        subtitle = "Compositeur, auteur, producteur de chaque titre, gardés en base pour le lecteur plein écran, sans jamais ralentir la lecture.",
    ) {
        section.data?.let { credits -> CreditsBody(model, credits) }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("credits") }) }
    }
}

@Composable
private fun CreditsBody(model: AdminModel, credits: AdminCredits) {
    if (credits.running) {
        AdminGauge(
            fraction = if (credits.total == 0u) 0f else
                (credits.done + credits.failed).toFloat() / credits.total.toFloat(),
            label = "${credits.done + credits.failed}/${credits.total} titres",
            detail = credits.sources.joinToString(" · ") { "${it.count} par ${it.name}" },
        )
    } else {
        AdminActions {
            NzButton(
                text = "Compléter les crédits",
                onClick = { model.startCredits(false) },
                glyph = NzGlyph.Users,
                kind = NzButtonKind.Secondary,
            )
            NzButton(
                text = "Tout recommencer",
                onClick = { model.startCredits(true) },
                kind = NzButtonKind.Secondary,
            )
        }
        AdminHint(
            if (credits.tracks > 0u) {
                "${credits.tracks} titre(s) ont leurs crédits." +
                    (if (credits.completed) " La bibliothèque archivée avant a été complétée."
                    else " La bibliothèque archivée avant sera complétée au prochain démarrage.")
            } else {
                "Compositeur, auteur, producteur : conservés pour le lecteur, jamais pendant la lecture."
            },
        )
    }
    credits.error?.let { AdminHint(it, warn = true) }
    JobFailuresBlock(credits.failures, "crédits")
}

@Composable
private fun RulesCard(model: AdminModel, state: AdminModel.State) {
    val section = state.rules
    AdminCard(
        title = "Règles d'archivage",
        subtitle = "Ce qui déclenche un archivage. La lecture n'est pas dans la liste : un titre lu est toujours archivé, c'est ainsi que fonctionne le transcodage.",
    ) {
        section.data?.let { rules -> RulesBody(model, rules) }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("rules") }) }
    }
}

@Composable
private fun RulesBody(model: AdminModel, rules: AdminRules) {
    if (!rules.archiveLibrary) {
        AdminHint(
            "L'archivage est désactivé côté serveur (archive_library) : ces règles sont sans effet.",
            warn = true,
        )
    }
    rules.events.forEach { event ->
        AdminSwitchRow(
            title = AdminCopy.EVENT_LABELS[event.id] ?: event.id,
            checked = event.on,
            onChange = { on -> model.saveRule(event.id, if (on) "true" else "false") },
        )
    }
    AdminSubtitle("Artiste mis en favori")
    AdminHint("Une discographie complète peut représenter des dizaines de Go. Limitez-la si le disque est petit.")
    AdminChips(
        options = rules.artistScopes.map { it to (AdminCopy.SCOPE_LABELS[it] ?: it) },
        selected = rules.rules.artistScope,
        onSelect = { scope -> model.saveRule("artist_scope", "\"$scope\"") },
        enabled = rules.events.firstOrNull { it.id == "on_fav_artist" }?.on == true,
    )
    if (rules.rules.artistScope != "all") {
        AdminSubtitle("Combien en prendre (N)")
        AdminChips(
            options = listOf("10", "25", "50", "100", "250").map { it to "N = $it" },
            selected = rules.rules.artistLimit.toString(),
            onSelect = { limit -> model.saveRule("artist_limit", limit) },
            enabled = rules.events.firstOrNull { it.id == "on_fav_artist" }?.on == true,
        )
    }
    AdminSubtitle("Chapitres des épisodes")
    AdminSwitchRow(
        title = "Lire les chapitres à l'archivage",
        hint = "Depuis le fichier de l'épisode, sans requête à personne.",
        checked = rules.rules.chapters,
        onChange = { on -> model.saveRule("chapters", if (on) "true" else "false") },
    )
    AdminSwitchRow(
        title = "Suivre aussi le lien de chapitres du fichier",
        hint = "Une requête tierce par épisode ; désactivé par défaut.",
        checked = rules.rules.chapterUrls,
        onChange = { on -> model.saveRule("chapter_urls", if (on) "true" else "false") },
    )
}

@Composable
private fun CleanupCard(model: AdminModel, state: AdminModel.State) {
    val section = state.cleanup
    val rules = state.rules.data
    AdminCard(
        title = "Nettoyage automatique",
        subtitle = "La SEULE fonction qui supprime de l'audio archivé. Elle ne touche que des titres re-téléchargeables depuis Deezer — un fichier que vous avez importé vous-même n'est jamais concerné. Le titre reste dans vos playlists et se réarchive à la prochaine lecture ; seuls les octets partent.",
    ) {
        if (rules != null) {
            AdminSwitchRow(
                title = "Activer le nettoyage quand le disque se remplit",
                checked = rules.rules.cleanOn,
                onChange = { on -> model.saveRule("clean_on", if (on) "true" else "false") },
            )
            if (rules.rules.cleanOn) {
                CleanupPolicy(model, rules)
                CleanupPreview(model, section.data)
            }
        } else {
            section.failure?.let { AdminCardFailure(it, { model.retryCard("cleanup") }) }
        }
    }
}

@Composable
private fun CleanupPolicy(model: AdminModel, rules: AdminRules) {
    AdminChips(
        options = listOf("5", "10", "20", "50", "100").map { it to "$it Go libres" },
        selected = rules.rules.cleanMinFreeGb.toInt().toString(),
        onSelect = { gb -> model.saveRule("clean_min_free_gb", gb) },
    )
    if (rules.rules.cleanMinFreeGb <= 0.0) {
        AdminHint("Seuil à 0 Go : rien ne sera jamais supprimé.", warn = true)
    }
    AdminChips(
        options = listOf("30", "90", "180", "365").map { it to "pas écouté depuis $it j" },
        selected = rules.rules.cleanStaleDays.toString(),
        onSelect = { days -> model.saveRule("clean_stale_days", days) },
    )
    AdminSubtitle("Ce qui est protégé")
    AdminSwitchRow(
        title = "Mes favoris",
        checked = rules.rules.cleanKeepFav,
        onChange = { on -> model.saveRule("clean_keep_fav", if (on) "true" else "false") },
    )
    AdminSwitchRow(
        title = "Les titres présents dans une playlist",
        checked = rules.rules.cleanKeepPlaylist,
        onChange = { on -> model.saveRule("clean_keep_playlist", if (on) "true" else "false") },
    )
    AdminSwitchRow(
        title = "Les épisodes de podcast",
        hint = "Un podcast disparu de Deezer reste protégé quoi qu'il arrive.",
        checked = rules.rules.cleanKeepPodcast,
        onChange = { on -> model.saveRule("clean_keep_podcast", if (on) "true" else "false") },
    )
    AdminSubtitle("Priorité de suppression")
    AdminChips(
        options = rules.cleanupOrders.map { it to (AdminCopy.ORDER_LABELS[it] ?: it) },
        selected = rules.rules.cleanOrder,
        onSelect = { order -> model.saveRule("clean_order", "\"$order\"") },
    )
}

@Composable
private fun CleanupPreview(model: AdminModel, cleanup: AdminCleanup?) {
    AdminActions {
        NzButton(
            text = "Prévisualiser",
            onClick = model::previewCleanup,
            glyph = NzGlyph.Refresh,
            kind = NzButtonKind.Secondary,
        )
        NzButton(
            text = "Libérer maintenant",
            onClick = model::runCleanup,
            glyph = NzGlyph.Trash,
            kind = NzButtonKind.Secondary,
            enabled = cleanup != null && cleanup.neededBytes > 0uL,
        )
    }
    if (cleanup == null) return
    AdminHint(
        if (cleanup.neededBytes > 0uL) {
            "Il manque ${AdminCopy.bytes(cleanup.neededBytes)} pour revenir au seuil ; ${cleanup.tracks.size} titre(s) partiraient, soit ${AdminCopy.bytes(cleanup.wouldFree)}."
        } else {
            "Le disque est au-dessus du seuil : rien ne sera supprimé. ${cleanup.eligible} titre(s) seraient éligibles si nécessaire."
        },
    )
    cleanup.tracks.take(12).forEach { track ->
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text(
                text = track.title,
                style = Sillon.type.bodySmall,
                color = Sillon.colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Text(text = track.artist, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
            Text(text = AdminCopy.bytes(track.bytes), style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        }
    }
}

@Composable
private fun StorageCard(model: AdminModel, state: AdminModel.State) {
    val section = state.storage
    AdminCard(
        title = "Stockage serveur",
        subtitle = "Le cache (transcodages Opus, pochettes proxifiées) se reconstruit tout seul : le vider ne coûte que du CPU. L'archive, elle, n'est pas un cache — aucun bouton ici ne peut y toucher.",
    ) {
        section.data?.let { store ->
            AdminGauge(
                fraction = state.diskUsedShare,
                label = "${AdminCopy.bytes(store.diskFree)} libres sur le disque d'archives",
                detail = "${AdminCopy.bytes(store.diskTotal)} au total",
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.l),
            ) {
                AdminStat(AdminCopy.bytes(store.archiveBytes), "Archive")
                AdminStat("${store.tracksArchived} / ${store.tracksTotal}", "Titres archivés")
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.l),
            ) {
                AdminStat(AdminCopy.bytes(store.transcodeBytes), "Cache transcodage")
                AdminStat(AdminCopy.bytes(store.cacheBytes), "Cache images")
            }
            if (store.archiveDir.isNotBlank()) {
                Text(text = store.archiveDir, style = Sillon.type.mono, color = Sillon.colors.dim)
            }
            NzButton(
                text = "Vider le cache",
                onClick = model::flushCache,
                glyph = NzGlyph.Trash,
                kind = NzButtonKind.Secondary,
                enabled = !state.busy,
            )
        }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("storage") }) }
    }
}

@Composable
private fun AnalysisCard(model: AdminModel, state: AdminModel.State) {
    val plan = state.plan
    val analysis = state.analysis
    AdminCard(
        title = "Analyse de la bibliothèque",
        subtitle = "Tempo, style et empreinte de chaque titre, mesurés par le serveur. L'analyse s'efface devant vous : elle se met en pause dès qu'on écoute de la musique, et reprend après.",
    ) {
        plan.data?.let { view -> PlanBody(model, view) }
        plan.failure?.let { AdminCardFailure(it, { model.retryCard("plan") }) }
        analysis.data?.let { job -> AnalysisBody(model, job) }
        analysis.failure?.let { AdminCardFailure(it, { model.retryCard("analysis") }) }
    }
}

@Composable
private fun PlanBody(model: AdminModel, plan: AdminPlan) {
    AdminGauge(
        fraction = if (plan.total == 0u) 0f else plan.current.toFloat() / plan.total.toFloat(),
        label = "${plan.current} à jour sur ${plan.total}",
        detail = buildString {
            append("${plan.never} jamais analysés")
            val stale = (plan.total - plan.current - plan.never).coerceAtLeast(0u)
            if (stale > 0u) append(" · $stale à remettre à jour")
        },
    )
    plan.stages.forEach { stage ->
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(
                text = AdminCopy.STAGES[stage.id] ?: stage.id,
                style = Sillon.type.bodySmall,
                color = Sillon.colors.tx,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = when {
                    stage.off -> "—"
                    stage.late == 0u -> "à jour"
                    else -> "${stage.late} à faire"
                },
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
        }
    }
    AdminSubtitle("Qui mesure")
    AdminChips(
        options = listOf("server" to "Serveur", "client" to "Cet ordinateur"),
        selected = plan.place,
        onSelect = model::saveAnalysisPlace,
    )
    AdminSubtitle("Analyse automatique")
    AdminSwitchRow(
        title = "Chaque nuit",
        checked = plan.schedule.enabled,
        onChange = { on -> model.saveSchedule(on, plan.schedule.at, plan.schedule.hours) },
    )
    if (plan.schedule.enabled) {
        AdminChips(
            options = listOf("01:00", "02:00", "03:00", "04:00").map { it to "à $it" },
            selected = plan.schedule.at,
            onSelect = { at -> model.saveSchedule(true, at, plan.schedule.hours) },
        )
        AdminChips(
            options = AdminCopy.SCHEDULE_HOURS.map { it.toString() to "pendant $it h" },
            selected = plan.schedule.hours.toString(),
            onSelect = { hours -> model.saveSchedule(true, plan.schedule.at, hours.toUInt()) },
        )
    }
    AdminActions {
        NzButton(
            text = "Mesurer maintenant",
            onClick = { model.startAnalysis(false, 0u) },
            glyph = NzGlyph.Activity,
            kind = NzButtonKind.Secondary,
        )
        NzButton(
            text = "Tout remesurer",
            onClick = { model.startAnalysis(true, 0u) },
            kind = NzButtonKind.Secondary,
        )
    }
}

@Composable
private fun AnalysisBody(model: AdminModel, job: uniffi.nz_ffi.AdminAnalysisJob) {
    if (job.running) {
        AdminGauge(
            fraction = if (job.total == 0u) 0f else job.scanned.toFloat() / job.total.toFloat(),
            label = (AdminCopy.STAGES[job.stage ?: ""] ?: "Analyse") + " · ${job.done} analysés",
            detail = "${job.scanned} / ${job.total} parcourus",
            indeterminate = job.total == 0u,
        )
        job.paused?.let { why ->
            AdminHint("En pause : ${AdminCopy.PAUSE_WHY[why] ?: why}. Reprend toute seule.")
        }
        NzButton(
            text = "Arrêter",
            onClick = model::stopAnalysis,
            kind = NzButtonKind.Secondary,
        )
    } else {
        job.error?.let { AdminHint(it, warn = true) }
        AdminHint("${job.workersSetting} titre(s) en parallèle (0 = auto · ${job.workersAuto} sur cette machine).")
    }
    JobFailuresBlock(job.failures, "analyse")
}

@Composable
private fun SimilarCard(model: AdminModel, state: AdminModel.State) {
    val section = state.similar
    AdminCard(
        title = "Voisins sonores",
        subtitle = "Ce qui fait marcher la radio, le flow et les « sons proches » : les 50 titres qui sonnent le plus comme chacun. Un titre mesuré les reçoit tout seul ; ce bouton rattrape une bibliothèque mesurée avant.",
    ) {
        section.data?.let { similar -> SimilarBody(model, similar) }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("similar") }) }
    }
}

@Composable
private fun SimilarBody(model: AdminModel, similar: AdminSimilar) {
    if (similar.running) {
        AdminGauge(fraction = 0f, label = "Calcul des voisins…", indeterminate = true)
    } else {
        AdminActions {
            NzButton(
                text = "Recalculer les voisins",
                onClick = { model.startSimilar(false) },
                glyph = NzGlyph.Activity,
                kind = NzButtonKind.Secondary,
            )
            NzButton(
                text = "Tout recommencer",
                onClick = { model.startSimilar(true) },
                kind = NzButtonKind.Secondary,
            )
        }
        if (similar.tracks > 0u) AdminHint("${similar.tracks} titre(s) ont leurs voisins.")
    }
}

@Composable
private fun AutomatchCard(model: AdminModel, state: AdminModel.State) {
    val section = state.automatch
    AdminCard(
        title = "Remplacement automatique",
        subtitle = "L'extrait du titre perdu, servi par sa source, est comparé à l'audio des candidats. À 99 % ou plus, c'est le même enregistrement : il est remplacé partout, sans rien demander.",
    ) {
        section.data?.let { status -> AutomatchBody(model, status) }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("automatch") }) }
    }
}

@Composable
private fun AutomatchBody(model: AdminModel, status: uniffi.nz_ffi.AdminAutomatch) {
    if (!status.available) {
        AdminHint(
            "Comparaison audio indisponible sur ce serveur (numpy et ffmpeg sont nécessaires, tous deux dans l'image Docker).",
            warn = true,
        )
        return
    }
    AdminSwitchRow(
        title = "Remplacement automatique",
        hint = if (status.youtube) "Le repli YouTube peut servir de stand-in." else null,
        checked = status.enabled,
        onChange = model::setAutomatch,
    )
    if (status.running) {
        AdminGauge(
            fraction = if ((status.total ?: 0u) == 0u) 0f else status.done.toFloat() / (status.total ?: 1u).toFloat(),
            label = if (status.total == null) "Préparation…" else "Écoute… ${status.done} / ${status.total}",
            detail = if (status.matched > 0u) "${status.matched} remplacé(s)" else null,
            indeterminate = status.total == null,
        )
        NzButton(
            text = if (status.cancel) "Arrêt…" else "Arrêter",
            onClick = model::cancelAutomatch,
            kind = NzButtonKind.Secondary,
            enabled = !status.cancel,
        )
    } else {
        NzButton(
            text = "Analyser maintenant",
            onClick = model::runAutomatch,
            glyph = NzGlyph.Zap,
            kind = NzButtonKind.Secondary,
        )
        if (status.lastCounts.isNotEmpty()) {
            AdminHint(
                "Dernier passage : " + status.lastCounts.joinToString(" · ") { count ->
                    "${count.count} " + (AdminCopy.AUTOMATCH_STATUS[count.name] ?: count.name)
                },
            )
        }
        status.lastStopped?.let { AdminHint("Interrompu : Deezer injoignable.", warn = true) }
    }
    status.recent.take(10).forEach { row ->
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Text(
                text = row.fromTitle + " → " + row.toTitle,
                style = Sillon.type.bodySmall,
                color = Sillon.colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Text(text = AdminCopy.percent(row.score), style = Sillon.type.bodySmall, color = Sillon.colors.acText)
        }
    }
    status.misses.take(10).forEach { miss ->
        AdminHint(
            "${miss.title} — " + (AdminCopy.automatchReason(miss.reason)
                .ifBlank { AdminCopy.AUTOMATCH_STATUS[miss.status] ?: miss.status }),
        )
    }
}

@Composable
private fun TranscribeCard(model: AdminModel, state: AdminModel.State) {
    val section = state.transcribe
    AdminCard(
        title = "Transcription des podcasts",
        subtitle = "Le texte est produit sur ce serveur, à partir des fichiers archivés. Aucun audio n'est envoyé à un tiers. Rien n'est transcrit automatiquement : chaque épisode se demande depuis sa page.",
    ) {
        section.data?.let { settings -> TranscribeBody(model, settings, section.loading) }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("transcribe") }) }
    }
}

@Composable
private fun TranscribeBody(model: AdminModel, settings: AdminTranscribe, probing: Boolean) {
    if (!settings.available) {
        AdminHint("La transcription n'est pas disponible sur ce serveur.", warn = true)
    }
    AdminSwitchRow(
        title = "Transcrire les podcasts",
        hint = "Le moteur reste désactivé tant que cette option est fermée.",
        checked = settings.enabled,
        onChange = { on -> model.saveTranscribe("enabled", if (on) "true" else "false") },
    )
    AdminCopy.transcribeReason(settings.reason).takeIf { it.isNotEmpty() }?.let { AdminHint(it, warn = true) }

    AdminSubtitle("Langue des prochaines transcriptions")
    AdminChips(
        options = settings.languages.map { it to AdminCopy.languageLabel(it) },
        selected = settings.language,
        onSelect = { code -> model.saveTranscribe("language", "\"$code\"") },
    )
    AdminSubtitle("Durée maximale d'un épisode")
    AdminChips(
        options = AdminCopy.MINUTE_SHORTCUTS.map { it.toString() to "$it min" },
        selected = settings.maxMinutes.toString(),
        onSelect = { minutes -> model.saveTranscribe("max_minutes", minutes) },
    )
    AdminField(
        label = "Durée maximale d'un épisode (minutes)",
        value = settings.maxMinutes.toString(),
        onValue = {},
        suffix = "min",
        hint = "Entre 5 et ${AdminCopy.MAX_MINUTES_MAX}.",
    )
    AdminSubtitle("Budget automatique par jour")
    AdminChips(
        options = listOf("0", "30", "60", "120", "240").map { it to "$it min" },
        selected = settings.dailyMinutes.toString(),
        onSelect = { minutes -> model.saveTranscribe("daily_minutes", minutes) },
    )
    AdminHint("0 = illimité. La transcription s'arrête d'elle-même au budget.")
    AdminSubtitle("Recherche dans les transcriptions")
    AdminHint(
        if (settings.searchSegments > 0uL) {
            "${settings.searchSegments} segments parcourus au plus."
        } else {
            "Toute la bibliothèque transcrite."
        },
    )
    settings.engineName?.let { name ->
        AdminHint(
            "Moteur : $name" + (settings.engineVersion?.let { " $it" } ?: "") +
                " — " + (if (settings.engineOk) "trouvé" else "absent"),
        )
    }
    settings.modelName?.let { name ->
        AdminHint(
            "Modèle : $name" +
                (if (settings.modelSizeMb > 0u) " · ${settings.modelSizeMb} Mo" else "") +
                " — " + (if (settings.modelOk) "valide" else "invalide"),
        )
    }
    if (settings.threads > 0u) AdminHint("${settings.threads} fils de calcul sur ce serveur.")
    if (settings.runningNow > 0u || settings.queued > 0u) {
        AdminHint("File : ${settings.runningNow} en cours, ${settings.queued} en attente.")
    }
    NzButton(
        text = if (probing) "Vérification…" else "Tester le moteur",
        onClick = model::probeTranscribe,
        glyph = NzGlyph.Refresh,
        kind = NzButtonKind.Secondary,
        enabled = !probing,
    )
}

@Composable
private fun DeezerCard(model: AdminModel, state: AdminModel.State) {
    val section = state.server
    AdminCard(
        title = "Compte Deezer (ARL)",
        subtitle = "Tout le catalogue passe par ce seul identifiant de session. Il expire régulièrement : collez-en un nouveau ici et il remplacera aussitôt celui défini dans la configuration du serveur, sans redémarrage.",
    ) {
        section.data?.let { card ->
            AdminPill(
                text = when {
                    state.deezerChecking -> "Vérification…"
                    card.statusOk == true -> "Connecté"
                    card.statusReason == "arl" -> "ARL invalide ou expiré"
                    card.statusReason == "network" -> "Deezer injoignable"
                    card.statusReason == "disabled" -> "Proxy Deezer désactivé"
                    else -> "État inconnu"
                },
                ok = card.statusOk,
            )
            Text(
                text = AdminCopy.arlLine(card),
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
            NzButton(
                text = "Tester",
                onClick = model::testDeezer,
                glyph = NzGlyph.Refresh,
                kind = NzButtonKind.Secondary,
                enabled = !state.deezerChecking,
            )
            AdminHint("L'ARL se colle et se vérifie depuis Réglages → Sources, où il n'est jamais réaffiché.")
        }
        section.failure?.let { AdminCardFailure(it, { model.retryCard("server") }) }
    }
}

@Composable
private fun QuotaSection(model: AdminModel, state: AdminModel.State) {
    val section = state.server
    AdminCard(
        title = "Quota d'upload (utilisateurs non-admin)",
        subtitle = "Limite l'espace total que chaque utilisateur non-administrateur peut occuper avec ses fichiers importés. Les administrateurs ne sont pas limités. Mettez 0 pour désactiver la limite.",
    ) {
        AdminField(
            label = "Go par utilisateur",
            value = state.quotaValue,
            onValue = model::setQuotaDraft,
            suffix = "Go",
            onDone = model::saveQuota,
        )
        section.failure?.let { AdminCardFailure(it, { model.retryCard("server") }) }
    }
}
