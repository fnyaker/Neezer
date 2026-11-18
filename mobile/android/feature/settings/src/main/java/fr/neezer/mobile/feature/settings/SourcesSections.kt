package fr.neezer.mobile.feature.settings

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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.launch
import uniffi.nz_ffi.AccessAccountData
import uniffi.nz_ffi.AccountData
import uniffi.nz_ffi.ProviderData
import uniffi.nz_ffi.SourceData

/**
 * The sections the server feeds: *Sources* (the order, my accounts, who may use
 * what, and the administrator's Deezer card), *Compte* (the person, the export
 * services) — the SPA's `SourceOrder`, `MyAccounts`, `AccessSettings`,
 * `ServicesSettings` and its ARL card, ported.
 *
 * Every card states its own four states: the device's copy, the fresh answer,
 * « Hors ligne · version mémorisée », or the cause and « Réessayer » (rule 3).
 * Nothing here is a spinner: a card that has nothing to draw says why.
 */
@Composable
internal fun SourcesSection(model: SettingsModel, state: SettingsModel.State) {
    SourceOrderCard(model, state)
    AccountsCard(model, state)
    AccessCard(model, state)
    if (state.server != null) DeezerArlCard(model, state)
}

// -- Ordre des sources ---------------------------------------------------------

@Composable
private fun SourceOrderCard(model: SettingsModel, state: SettingsModel.State) {
    val section = state.sources
    val colors = Sillon.colors
    SettingsCard(
        title = "Ordre des sources",
        subtitle = "La source principale sert la recherche, les pages et la musique. Quand elle n'a pas un titre, les sources de secours sont interrogées dans l'ordre ci-dessous.",
    ) {
        when {
            section.failure != null && section.data == null -> LoadFailed(cause = section.failure, onRetry = { model.retry("sources") })
            section.data == null -> Text("Chargement…", style = Sillon.type.bodySmall, color = colors.dim)
            else -> {
                if (section.stale) StaleNote()
                val sources = state.orderedSources
                val fallbacks = sources.filter { it.place == "fallback" }
                val admin = section.data.admin
                if (sources.isEmpty()) {
                    Text(
                        text = "Aucune source de musique n'est installée sur ce serveur : seule la bibliothèque est lisible.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                sources.forEach { source ->
                    SourceRow(
                        source = source,
                        fallbacks = fallbacks,
                        admin = admin,
                        enabledBusy = state.busy,
                        onMove = { delta ->
                            val ordered = fallbacks.sortedBy { it.rank ?: 0 }.toMutableList()
                            val index = ordered.indexOfFirst { it.id == source.id }
                            val target = index + delta
                            if (index < 0 || target < 0 || target >= ordered.size) return@SourceRow
                            ordered[index] = ordered[target].also { ordered[target] = ordered[index] }
                            val main = sources.firstOrNull { it.place == "main" }
                            val off = sources.filter { it.place == "off" }
                            model.saveSourceOrder((listOfNotNull(main) + ordered.sortedBy { ordered.indexOf(it) } + off).map { it.id })
                        },
                        onToggle = { on -> model.setSourceEnabled(source.id, on) },
                    )
                }
                if (admin && fallbacks.size > 1) {
                    NzButton(
                        text = "Classer les secours par qualité",
                        onClick = { model.sortFallbacksByQuality() },
                        kind = NzButtonKind.Secondary,
                        glyph = NzGlyph.Sort,
                        enabled = !state.busy,
                    )
                }
                if (!admin) {
                    Text(
                        text = "Réglé par l'administrateur du serveur.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
            }
        }
    }
}

/** One source: its place, its glyph, what it is and where it stands. */
@Composable
private fun SourceRow(
    source: SourceData,
    fallbacks: List<SourceData>,
    admin: Boolean,
    enabledBusy: Boolean,
    onMove: (Int) -> Unit,
    onToggle: (Boolean) -> Unit,
) {
    val colors = Sillon.colors
    val fallbackIndex = fallbacks.indexOfFirst { it.id == source.id }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            // The place, on its own line's height: at 360 dp a fixed 78 dp badge
            // clipped « Principale » (measured in this lot's first capture).
            Text(
                text = placeLabel(source),
                style = Sillon.type.labelMono,
                color = if (source.place == "main") colors.acText else colors.dim,
                modifier = Modifier
                    .widthIn(min = 74.dp, max = 104.dp)
                    .background(
                        if (source.place == "main") colors.ac.copy(alpha = 0.1f) else colors.s2,
                        RoundedCornerShape(NzRadius.field),
                    )
                    .padding(vertical = NzSpacing.xs, horizontal = NzSpacing.xs),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Box(
                modifier = Modifier
                    .size(34.dp)
                    .background(colors.s2, RoundedCornerShape(NzRadius.field)),
                contentAlignment = Alignment.Center,
            ) {
                Icon(sourceGlyph(source.id), size = 17.dp, tint = if (source.place == "off") colors.dim else colors.tx)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                    Text(
                        text = source.name,
                        style = Sillon.type.label,
                        color = if (source.place == "off") colors.dim else colors.tx,
                    )
                    if (source.qualityLabel.isNotBlank()) {
                        Text(
                            text = source.qualityLabel + if (source.lossless) " · sans perte" else "",
                            style = Sillon.type.labelMono,
                            color = colors.dim,
                        )
                    }
                }
                Text(
                    text = sourceStatus(source, fallbacks),
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
        }
        if (admin) {
            // The controls keep their own line: a switch's label inside the row
            // squeezed the status into a one-letter column (this lot's capture).
            Row(
                modifier = Modifier.fillMaxWidth().padding(start = NzSpacing.xs),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                if (source.place == "fallback" && fallbacks.size > 1) {
                    NzIconButton(
                        glyph = NzGlyph.ChevronUp,
                        contentDescription = "Monter ${source.name}",
                        onClick = { onMove(-1) },
                        enabled = fallbackIndex > 0 && !enabledBusy,
                        size = 16.dp,
                    )
                    NzIconButton(
                        glyph = NzGlyph.ChevronDown,
                        contentDescription = "Descendre ${source.name}",
                        onClick = { onMove(1) },
                        enabled = fallbackIndex in 0 until fallbacks.size - 1 && !enabledBusy,
                        size = 16.dp,
                    )
                }
                Spacer(Modifier.weight(1f))
                if (source.switchable) {
                    Box(Modifier.semantics {
                        contentDescription = if (source.enabled) "Couper ${source.name}" else "Activer ${source.name}"
                    }) {
                        NzSwitch(
                            checked = source.enabled,
                            onCheckedChange = onToggle,
                            enabled = source.configured && !enabledBusy,
                        )
                    }
                }
            }
        }
    }
}

// -- Mes comptes ---------------------------------------------------------------

@Composable
private fun AccountsCard(model: SettingsModel, state: SettingsModel.State) {
    val section = state.accounts
    val colors = Sillon.colors
    SettingsCard(
        title = "Mes comptes",
        subtitle = "Reliez votre propre compte pour retrouver ici vos artistes suivis, vos titres aimés, vos albums et vos playlists. Chaque élément n'est importé qu'une fois, et rien n'est jamais écrit sur votre compte.",
    ) {
        when {
            section.failure != null && section.data == null -> LoadFailed(cause = section.failure, onRetry = { model.retry("sources") })
            section.data == null -> Text("Chargement…", style = Sillon.type.bodySmall, color = colors.dim)
            else -> {
                if (section.stale) StaleNote()
                val accounts = section.data.accounts
                if (accounts.isNotEmpty()) {
                    Text("Plateforme par défaut", style = Sillon.type.label, color = colors.tx)
                    Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                        accounts.forEach { account ->
                            NzChip(
                                text = account.name,
                                onClick = { model.chooseDefault(account.id) },
                                selected = account.isDefault,
                                enabled = !state.busy && account.status !in setOf("auth", "broken"),
                            )
                        }
                        NzChip(
                            text = "Aucune",
                            onClick = { model.chooseDefault(null) },
                            selected = section.data.default == null,
                            enabled = !state.busy,
                        )
                    }
                    Text(
                        text = if (section.data.default != null) {
                            "Ses favoris sont relus chaque jour et ce qui est nouveau est ajouté. Les autres comptes ne sont importés que lorsque vous le demandez."
                        } else {
                            "Aucun compte n'est relu automatiquement : vous importez quand vous le demandez."
                        },
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                accounts.forEach { account -> AccountBlock(model, state, account) }
                if (section.data.canLink.isNotEmpty()) LinkAccountBlock(model, section.data.canLink)
                if (accounts.none { it.provider == "youtube" }) {
                    Text(
                        text = "YouTube Music : reliez votre session dans Compte → Services de streaming ; elle sert aussi à l'import.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
            }
        }
    }
}

@Composable
private fun AccountBlock(model: SettingsModel, state: SettingsModel.State, account: AccountData) {
    val colors = Sillon.colors
    var confirmUnlink by remember(account.id) { mutableStateOf(false) }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Icon(if (account.provider == "youtube") NzGlyph.Youtube else NzGlyph.Music, size = 18.dp, tint = colors.tx)
            Text(
                text = if (account.server) "${account.name} · compte du serveur" else account.name,
                style = Sillon.type.label,
                color = colors.tx,
            )
            if (account.isDefault) {
                Text("Par défaut", style = Sillon.type.labelMono, color = colors.acText)
            }
        }
        val status = accountStatus(account, clockNow())
        if (status.isNotBlank()) {
            Text(status, style = Sillon.type.bodySmall, color = colors.dim)
        }
        val use = accountUse(account)
        if (use.isNotBlank()) {
            Text(use, style = Sillon.type.bodySmall, color = colors.dim)
        }
        val progress = importProgress(account)
        if (progress != null) {
            Text(progress.text, style = Sillon.type.bodySmall, color = colors.tx)
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(4.dp)
                    .background(colors.s2, RoundedCornerShape(NzRadius.pill)),
            ) {
                Box(
                    Modifier
                        .fillMaxWidth((progress.share ?: 1.0).toFloat().coerceIn(0.02f, 1f))
                        .height(4.dp)
                        .background(colors.btn, RoundedCornerShape(NzRadius.pill)),
                )
            }
            Text(
                text = "L'import continue sur le serveur si vous quittez cette page.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
        if (account.kinds.isNotEmpty()) {
            Text(
                text = if (account.isDefault) "Synchronisé chaque jour" else "Importé quand vous le demandez",
                style = Sillon.type.labelMono,
                color = colors.dim,
            )
            Column {
                account.kinds.forEach { kind ->
                    NzSwitch(
                        checked = kind in account.autoOn,
                        onCheckedChange = { model.toggleAccountAuto(account, kind) },
                        label = KIND_LABELS[kind] ?: kind,
                        enabled = !state.busy,
                    )
                }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            NzButton(
                text = if (account.runningPhase != null) "Import en cours…" else "Importer maintenant",
                onClick = { model.importNow(account) },
                glyph = NzGlyph.Download,
                kind = NzButtonKind.Secondary,
                enabled = !state.busy && account.runningPhase == null && account.status !in setOf("auth", "broken"),
            )
            if (!account.server) {
                NzButton(
                    text = if (confirmUnlink) "Délier ?" else "Délier",
                    onClick = {
                        if (confirmUnlink) {
                            confirmUnlink = false
                            model.unlinkAccount(account)
                        } else {
                            confirmUnlink = true
                        }
                    },
                    kind = NzButtonKind.Ghost,
                )
            }
        }
        if (confirmUnlink) {
            Text(
                text = "Touchez « Délier ? » une seconde fois : ce qui a été importé reste dans votre bibliothèque.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
    }
}

/** The link form: an ARL pasted (the phone has no Deezer password path, MB4). */
@Composable
private fun LinkAccountBlock(model: SettingsModel, canLink: List<String>) {
    val colors = Sillon.colors
    var open by remember { mutableStateOf(false) }
    var arl by remember { mutableStateOf("") }
    if (!open) {
        NzButton(
            text = "Relier un compte",
            onClick = { open = true },
            glyph = NzGlyph.Plus,
            kind = NzButtonKind.Secondary,
        )
        return
    }
    Text(
        text = "ARL de votre session Deezer — la clé de votre compte : il reste chiffré sur le serveur et n'est montré à personne.",
        style = Sillon.type.bodySmall,
        color = colors.dim,
    )
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Box(
            modifier = Modifier
                .weight(1f)
                .height(NzMinTouchTarget)
                .background(colors.s2, RoundedCornerShape(NzRadius.field))
                .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.field))
                .padding(horizontal = NzSpacing.m),
            contentAlignment = Alignment.CenterStart,
        ) {
            if (arl.isEmpty()) {
                Text("ARL de votre session Deezer", style = Sillon.type.body, color = colors.dim)
            }
            BasicTextField(
                value = arl,
                onValueChange = { arl = it },
                singleLine = true,
                textStyle = Sillon.type.body.copy(color = colors.tx),
                cursorBrush = SolidColor(colors.acText),
                modifier = Modifier
                    .fillMaxWidth()
                    .semantics { contentDescription = "ARL de votre session Deezer" },
            )
        }
        NzButton(
            text = "Relier",
            onClick = {
                model.linkDeezer(arl)
                arl = ""
                open = false
            },
            enabled = arl.isNotBlank(),
        )
    }
    if (canLink.none { it == "deezer" }) {
        Text(
            text = "Le compte Deezer du serveur tient déjà cette place ; ce formulaire relie VOTRE compte.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
    }
}

// -- Sources et accès ----------------------------------------------------------

@Composable
private fun AccessCard(model: SettingsModel, state: SettingsModel.State) {
    val section = state.access
    val colors = Sillon.colors
    var who by remember { mutableStateOf<GrantTarget?>(null) }
    SettingsCard(
        title = "Sources et accès",
        subtitle = "Tout ce qui est déjà sur le serveur est à tout le monde. Ici se règle le reste : qui peut chercher, écouter et ouvrir des pages chez chaque service, et avec quel compte.",
    ) {
        when {
            section.failure != null && section.data == null -> LoadFailed(cause = section.failure, onRetry = { model.retry("sources") })
            section.data == null -> Text("Chargement…", style = Sillon.type.bodySmall, color = colors.dim)
            else -> {
                if (section.stale) StaleNote()
                val access = section.data
                access.providers.filter { it.role != "info" }.forEach { provider ->
                    ProviderBlock(model, state, access, provider, onWho = { who = it })
                }
                if (access.admin) GroupsBlock(model, access)
            }
        }
    }
    who?.let { target ->
        WhoSheet(
            target = target,
            access = section.data,
            onDismiss = { who = null },
            onConfirm = { subjects ->
                model.setGrants(target.provider, target.account, target.feature, subjects)
                who = null
            },
        )
    }
}

@Composable
private fun ProviderBlock(
    model: SettingsModel,
    state: SettingsModel.State,
    access: uniffi.nz_ffi.AccessData,
    provider: ProviderData,
    onWho: (GrantTarget) -> Unit,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Icon(sourceGlyph(provider.id), size = 18.dp, tint = colors.tx)
            Text(provider.name, style = Sillon.type.label, color = colors.tx)
            Text(
                text = when (provider.role) {
                    "main" -> "principale"
                    "fallback" -> "secours"
                    else -> ""
                },
                style = Sillon.type.labelMono,
                color = colors.dim,
            )
        }
        provider.accounts.forEach { account ->
            Text(holderLine(account), style = Sillon.type.bodySmall, color = colors.dim)
            provider.features.forEach { feature ->
                val subjects = subjectsOf(account.grants, feature.id)
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(NzMinTouchTarget)
                        .clickable(enabled = account.editable && !state.busy, role = Role.Button) {
                            onWho(
                                GrantTarget(
                                    provider = provider.id,
                                    account = account.id,
                                    feature = feature.id,
                                    featureLabel = feature.id,
                                    subjects = subjects,
                                ),
                            )
                        },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                ) {
                    Text(feature.id, style = Sillon.type.body, color = colors.tx, modifier = Modifier.weight(1f))
                    Text(
                        text = subjectsLabel(subjects, access),
                        style = Sillon.type.bodySmall,
                        color = colors.acText,
                    )
                }
            }
        }
        if (provider.anonymousEditable) {
            val subjects = subjectsOf(provider.anonymousGrants, provider.features.firstOrNull()?.id ?: "")
            Text(
                text = "Sans compte : ${subjectsLabel(subjects, access)}",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
    }
    Spacer(Modifier.height(NzSpacing.s))
    Box(Modifier.fillMaxWidth().height(1.dp).background(colors.ln))
}

@Composable
private fun GroupsBlock(model: SettingsModel, access: uniffi.nz_ffi.AccessData) {
    val colors = Sillon.colors
    var name by remember { mutableStateOf("") }
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Spacer(Modifier.height(NzSpacing.m))
        Text("Groupes", style = Sillon.type.label, color = colors.tx)
        Text(
            text = "Un groupe est un ensemble de personnes à qui un accès se donne en une fois.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        access.groups.forEach { group ->
            Row(
                modifier = Modifier.fillMaxWidth().height(NzMinTouchTarget),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Text(group.name, style = Sillon.type.body, color = colors.tx, modifier = Modifier.weight(1f))
                Text("${group.members.size}", style = Sillon.type.labelMono, color = colors.dim)
                NzIconButton(
                    glyph = NzGlyph.Trash,
                    contentDescription = "Supprimer le groupe ${group.name}",
                    onClick = { model.deleteGroup(group.id) },
                    size = 16.dp,
                )
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Box(
                modifier = Modifier
                    .weight(1f)
                    .height(NzMinTouchTarget)
                    .background(colors.s2, RoundedCornerShape(NzRadius.field))
                    .padding(horizontal = NzSpacing.m),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (name.isEmpty()) {
                    Text("Nouveau groupe", style = Sillon.type.body, color = colors.dim)
                }
                BasicTextField(
                    value = name,
                    onValueChange = { name = it },
                    singleLine = true,
                    textStyle = Sillon.type.body.copy(color = colors.tx),
                    cursorBrush = SolidColor(colors.acText),
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics { contentDescription = "Nom du nouveau groupe" },
                )
            }
            NzButton(
                text = "Créer",
                onClick = {
                    model.createGroup(name)
                    name = ""
                },
                enabled = name.isNotBlank() && !model.state.busy,
                kind = NzButtonKind.Secondary,
                glyph = NzGlyph.Plus,
            )
        }
    }
}

// -- Compte --------------------------------------------------------------------

@Composable
internal fun AccountSection(model: SettingsModel, state: SettingsModel.State) {
    val colors = Sillon.colors
    val scope = rememberCoroutineScope()
    val identity = remember { runCatching { AppSettings.api()?.identity() }.getOrNull() }
    var confirmLogout by remember { mutableStateOf(false) }
    SettingsCard(title = "Compte") {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            Column(Modifier.weight(1f)) {
                Text(identity?.name ?: "Personne", style = Sillon.type.titleSmall, color = colors.tx)
                Text(
                    text = if (identity?.admin == true) "Administrateur" else "Utilisateur",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
            NzButton(
                text = if (confirmLogout) "Confirmer" else "Déconnexion",
                onClick = {
                    if (!confirmLogout) {
                        confirmLogout = true
                    } else {
                        confirmLogout = false
                        scope.launch {
                                runCatching { NzBridge.core?.logout() }
                                .onFailure { fr.neezer.mobile.designsystem.NzToasts.show(fr.neezer.mobile.bridge.NzErrors.text(it)) }
                        }
                    }
                },
                kind = NzButtonKind.Ghost,
                glyph = NzGlyph.LogOut,
            )
        }
    }
    ServicesCard(model, state)
}

/** `ServicesSettings.svelte`: the export targets, and the Spotify app. */
@Composable
private fun ServicesCard(model: SettingsModel, state: SettingsModel.State) {
    val section = state.services
    val colors = Sillon.colors
    var clientId by remember { mutableStateOf<String?>(null) }
    val context = androidx.compose.ui.platform.LocalContext.current
    // A consent URL the model produced: the platform's browser opens it, and the
    // app never sees a password (`ServicesSettings.svelte#linkSpotify`).
    androidx.compose.runtime.LaunchedEffect(model.pendingLink.toList()) {
        val link = model.pendingLink.firstOrNull() ?: return@LaunchedEffect
        runCatching {
            context.startActivity(
                android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(link.url)),
            )
        }.onFailure { fr.neezer.mobile.designsystem.NzToasts.show("Aucun navigateur pour ouvrir cette page") }
        model.pendingLink.clear()
    }
    SettingsCard(
        title = "Services de streaming",
        subtitle = "Pour envoyer une playlist vers vos propres comptes : chaque titre y est recherché et ceux qui manquent vous sont listés.",
    ) {
        when {
            section.failure != null && section.data == null -> LoadFailed(cause = section.failure, onRetry = { model.retry("account") })
            section.data == null -> Text("Chargement…", style = Sillon.type.bodySmall, color = colors.dim)
            else -> {
                if (section.stale) StaleNote()
                section.data.services.filter { it.id != "apple" }.forEach { service ->
                    Row(
                        modifier = Modifier.fillMaxWidth().height(NzMinTouchTarget),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    ) {
                        Icon(if (service.id == "youtube") NzGlyph.Youtube else NzGlyph.Music, size = 18.dp, tint = if (service.linked) colors.acText else colors.dim)
                        Column(Modifier.weight(1f)) {
                            Text(service.name, style = Sillon.type.label, color = colors.tx)
                            Text(
                                text = serviceStatus(service, state.access.data?.admin == true),
                                style = Sillon.type.bodySmall,
                                color = colors.dim,
                            )
                        }
                        when {
                            service.id == "deezer" || service.file -> Unit
                            service.linked -> NzButton(
                                text = "Délier",
                                onClick = { model.disconnectService(service.id, service.name) },
                                kind = NzButtonKind.Ghost,
                                enabled = !state.busy,
                            )
                            service.ready -> NzButton(
                                text = "Relier",
                                onClick = { model.linkService(service.id) },
                                kind = NzButtonKind.Secondary,
                                enabled = !state.busy,
                            )
                        }
                    }
                }
                if (section.data.formats.isNotEmpty()) {
                    Text(
                        text = "Formats de liste : ${section.data.formats.joinToString(", ")}",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                // The operator's own Spotify app: only an administrator sets it.
                if (state.access.data?.admin == true) {
                    val current = section.data.services.firstOrNull { it.id == "spotify" }?.clientId.orEmpty()
                    val value = clientId ?: current
                    Text("Application Spotify (administrateur)", style = Sillon.type.label, color = colors.tx)
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                        Box(
                            modifier = Modifier
                                .weight(1f)
                                .height(NzMinTouchTarget)
                                .background(colors.s2, RoundedCornerShape(NzRadius.field))
                                .padding(horizontal = NzSpacing.m),
                            contentAlignment = Alignment.CenterStart,
                        ) {
                            if (value.isEmpty()) {
                                Text("Identifiant client", style = Sillon.type.body, color = colors.dim)
                            }
                            BasicTextField(
                                value = value,
                                onValueChange = { clientId = it },
                                singleLine = true,
                                textStyle = Sillon.type.body.copy(color = colors.tx),
                                cursorBrush = SolidColor(colors.acText),
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .semantics { contentDescription = "Identifiant client Spotify" },
                            )
                        }
                        NzButton(
                            text = "Enregistrer",
                            onClick = {
                                model.saveSpotifyApp(value)
                                clientId = null
                            },
                            enabled = !state.busy,
                            kind = NzButtonKind.Secondary,
                        )
                    }
                }
            }
        }
    }
}

/** `ServicesSettings.svelte#status`. */
private fun serviceStatus(service: uniffi.nz_ffi.ServiceData, admin: Boolean): String {
    if (service.id == "deezer") return if (service.ready) "Le compte Deezer du serveur" else "Proxy Deezer désactivé"
    if (service.linked) return service.account?.let { "Relié : $it" } ?: "Relié"
    if (!service.ready) {
        return if (service.id == "spotify") "Pas encore configuré sur ce serveur" else "Indisponible sur ce serveur"
    }
    return "Non relié"
}

// -- the administrator's Deezer card -------------------------------------------

@Composable
private fun DeezerArlCard(model: SettingsModel, state: SettingsModel.State) {
    val colors = Sillon.colors
    val card = state.server ?: return
    var draft by remember { mutableStateOf("") }
    var confirmClear by remember { mutableStateOf(false) }
    SettingsCard(
        title = "Compte Deezer (ARL)",
        subtitle = "Tout le catalogue passe par ce seul identifiant de session. Il expire régulièrement : collez-en un nouveau ici et il remplacera aussitôt celui de la configuration du serveur, sans redémarrage.",
    ) {
        val deezer = card.deezer
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text(
                text = when {
                    state.deezerChecking -> "Vérification…"
                    state.deezer?.ok == true -> "Connecté"
                    state.deezer?.reason == "arl" -> "ARL invalide ou expiré"
                    state.deezer?.reason == "network" -> "Deezer injoignable"
                    state.deezer?.reason == "disabled" -> "Proxy Deezer désactivé"
                    else -> "État inconnu"
                },
                style = Sillon.type.label,
                color = if (state.deezer?.ok == true) colors.acText else colors.tx,
            )
            NzButton(
                text = "Tester",
                onClick = { model.testDeezer() },
                kind = NzButtonKind.Ghost,
                glyph = NzGlyph.Refresh,
                enabled = !state.deezerChecking,
            )
        }
        Text(
            text = if (deezer.arlSet) {
                "ARL actif ${deezer.arlHint ?: ""} · source : " +
                    if (deezer.arlSource == "database") "saisi ici" else "configuration serveur"
            } else {
                "Aucun ARL configuré"
            },
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Box(
                modifier = Modifier
                    .weight(1f)
                    .height(NzMinTouchTarget)
                    .background(colors.s2, RoundedCornerShape(NzRadius.field))
                    .padding(horizontal = NzSpacing.m),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (draft.isEmpty()) {
                    Text("Coller le nouvel ARL…", style = Sillon.type.body, color = colors.dim)
                }
                BasicTextField(
                    value = draft,
                    onValueChange = { draft = it },
                    singleLine = true,
                    textStyle = Sillon.type.body.copy(color = colors.tx),
                    cursorBrush = SolidColor(colors.acText),
                    modifier = Modifier
                        .fillMaxWidth()
                        .semantics { contentDescription = "Coller le nouvel ARL" },
                )
            }
            NzButton(
                text = "Enregistrer",
                onClick = {
                    model.saveDeezerArl(draft)
                    draft = ""
                },
                enabled = draft.isNotBlank() && !state.busy,
            )
        }
        Text(
            text = "L'ARL est vérifié auprès de Deezer avant d'être enregistré : s'il est refusé, rien n'est modifié. Il n'est jamais réaffiché ensuite.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        if (deezer.arlSource == "database") {
            NzButton(
                text = if (confirmClear) "Revenir à l'ARL du serveur ?" else "Revenir à l'ARL du serveur",
                onClick = {
                    if (confirmClear) {
                        model.clearDeezerArl()
                        confirmClear = false
                    } else {
                        confirmClear = true
                    }
                },
                kind = NzButtonKind.Ghost,
                glyph = NzGlyph.Trash,
                enabled = !state.busy,
            )
        }
    }
}

// -- the grants' plumbing ------------------------------------------------------

/** One feature of one account whose audience is being chosen. */
internal data class GrantTarget(
    val provider: String,
    val account: String,
    val feature: String,
    val featureLabel: String,
    val subjects: List<String>,
)

/** The subjects a feature is granted to (`"*"` is everyone). */
internal fun subjectsOf(grants: List<uniffi.nz_ffi.GrantData>, feature: String): List<String> =
    grants.filter { it.features.any { granted -> granted == "all" || granted == feature } }
        .map { it.subject }

/** `AccessSettings.svelte#holderLine`. */
private fun holderLine(account: AccessAccountData): String {
    if (account.server) return account.owner?.let { "Compte du serveur · tenu par $it" } ?: "Compte du serveur"
    return when {
        account.mine -> "Votre compte"
        account.owner != null -> "Compte de ${account.owner}"
        else -> "Compte"
    }
}

/** Who a grant names, in words. */
internal fun subjectsLabel(subjects: List<String>, access: uniffi.nz_ffi.AccessData?): String {
    if (subjects.isEmpty()) return "Personne"
    if (subjects.contains("*")) return "Tout le monde"
    val names = subjects.map { subject ->
        access?.users?.firstOrNull { it.id == subject }?.name
            ?: access?.groups?.firstOrNull { it.id == subject }?.name
            ?: subject
    }
    return names.joinToString(", ")
}

/** The clock, read once per composition of a card (never a ticking state). */
@Composable
private fun clockNow(): Long = remember { System.currentTimeMillis() }
