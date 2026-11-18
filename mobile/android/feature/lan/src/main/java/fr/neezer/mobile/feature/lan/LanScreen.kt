package fr.neezer.mobile.feature.lan

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.style.TextOverflow
import androidx.core.content.ContextCompat
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.launch
import uniffi.nz_ffi.LanDeviceFfi
import uniffi.nz_ffi.LanFriendFfi
import uniffi.nz_ffi.LanPeerFfi
import uniffi.nz_ffi.LanPlaylistFfi
import uniffi.nz_ffi.LanSettingsFfi
import uniffi.nz_ffi.LanSnapshotFfi
import uniffi.nz_ffi.LanStateFfi

/**
 * « Appareils à proximité » (lot LN14): the two switches, the tolerated
 * qualities, the devices seen now, my enrolled devices, my trusted friends and
 * the advanced port/key line — the note's *Screens and Réglages*, on a phone.
 *
 * It is drawn in two places from ONE body: Réglages' own category (MB16's
 * extension point, [LanSettingsSection]) and the route `Routes.Lan`
 * ([LanEntry]), which is what a deep link or a future link opens. Nothing is
 * duplicated: a second copy would drift (MB16's rule for hosted sections).
 *
 * French copy, glyphs through the design system, no emoji, and every control
 * labelled for TalkBack. A switch's label sits on its own line and is repeated as
 * the control's `contentDescription` — inside a weighted Row an inline label
 * squeezes the row (MB16 measured it).
 */
@Composable
fun LanSettingsSection(
    modifier: Modifier = Modifier,
    model: LanModel? = null,
) {
    val host = rememberLanModel(model)
    val state by host.state.collectAsState()
    LaunchedEffect(host) { host.start() }

    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        val snapshot = state.snapshot
        if (snapshot != null && snapshot.state == LanStateFfi.Unsupported) {
            LanCard(
                title = "Échange entre appareils",
                subtitle = "Ce serveur ne propose pas l'échange entre appareils proches. " +
                    "Rien n'est activé sur ce téléphone, et rien ne changera tant qu'il n'aura pas été mis à jour.",
            ) {}
            return@Column
        }
        val settings = state.settings
        if (settings == null) {
            Text(
                text = "L'échange entre appareils n'est pas disponible : le noyau de l'application n'a pas démarré.",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
            return@Column
        }

        ExchangeCard(host, settings, snapshot)
        if (settings.enabled) {
            SeenCard(state)
            MyDevicesCard(host, state.devices)
            FriendsCard(host, state)
            AdvancedCard(host, state, settings, snapshot)
        }
        state.notice?.let { cause ->
            CauseLine(cause = cause, onDismiss = host::dismissNotice)
        }
        Spacer(Modifier.height(NzSpacing.s))
    }
}

/** The route's own screen: the section, under the shell's title bar. */
@Composable
fun LanEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    androidx.compose.foundation.layout.Box(
        modifier = modifier
            .fillMaxSize()
            .background(Sillon.colors.bg)
            .verticalScroll(rememberScrollState()),
    ) {
        LanSettingsScreenBody(nav)
    }
}

/**
 * The route's body: what the standalone screen draws. Kept apart so the
 * instrumented capture can render exactly it without a `NavHost`.
 */
@Composable
internal fun LanSettingsScreenBody(nav: NzNavigator) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Text(
            text = "Vos appareils et vos amis de confiance sur ce Wi-Fi. Les titres déjà téléchargés " +
                "ailleurs s'écoutent ici sans passer par le serveur.",
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
        LanSettingsSection()
    }
}

// -- the cards ---------------------------------------------------------------

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun ExchangeCard(host: LanModel, settings: LanSettingsFfi, snapshot: LanSnapshotFfi?) {
    LanCard(
        title = "Échange entre appareils",
        subtitle = snapshot?.let { stateText(it.state) }.orEmpty(),
    ) {
        SwitchLine(
            label = "Échanger avec mes appareils à proximité",
            hint = "Recherche et annonce sur ce Wi-Fi, tant que l'application est ouverte, qu'elle joue " +
                "ou qu'un téléchargement tourne.",
            checked = settings.enabled,
            onChange = host::setEnabled,
            tag = "lan-enabled",
        )
        if (!settings.enabled) return@LanCard
        LocalNetworkPermissionLine(enabled = true)
        SwitchLine(
            label = "Partager mes sons",
            hint = "Vos autres appareils peuvent prendre les titres que ce téléphone a déjà téléchargés.",
            checked = settings.serve,
            onChange = host::setServe,
            tag = "lan-serve",
        )
        Text(text = "Qualités tolérées", style = Sillon.type.label, color = Sillon.colors.tx)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            TOLERATED_CHOICES.forEach { (id, label) ->
                NzChip(
                    text = label,
                    selected = id in settings.tolerated,
                    onClick = { host.toggleTolerated(id) },
                    modifier = Modifier.semantics {
                        contentDescription = "Qualité tolérée $label, " +
                            if (id in settings.tolerated) "acceptée" else "refusée"
                    },
                )
            }
        }
        Text(text = TOLERATED_HINT, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
    }
}

@Composable
private fun SeenCard(state: LanModel.State) {
    val snapshot = state.snapshot
    val peers = snapshot?.peers.orEmpty()
    val transfers = snapshot?.transfers.orEmpty()
    LanCard(
        title = "Appareils vus maintenant",
        subtitle = when {
            peers.isEmpty() -> "Aucun appareil connecté pour l'instant."
            peers.size == 1 -> "1 appareil authentifié sur ce Wi-Fi."
            else -> "${peers.size} appareils authentifiés sur ce Wi-Fi."
        },
    ) {
        if (peers.isEmpty()) {
            Text(
                text = if (state.showNoPeerHint) {
                    NO_PEER_HINT
                } else {
                    "La recherche tourne ; un appareil qui ouvre Neezer apparaît ici en quelques secondes."
                },
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
                modifier = Modifier.testTag("lan-no-peer"),
            )
        }
        peers.forEach { peer -> PeerRow(peer) }
        transfers.forEach { transfer ->
            Text(text = transferText(transfer), style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        }
        snapshot?.failures?.take(3)?.forEach { failure ->
            Text(
                text = "${failure.what} : ${failure.cause}",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
        }
    }
}

@Composable
private fun PeerRow(peer: LanPeerFfi) {
    Row(
        modifier = Modifier.fillMaxWidth().semantics {
            contentDescription = "${peer.name.ifBlank { platformText(peer.platform) }}, " +
                "${relationText(peer.own)}, ${signalText(peer)}"
        },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = relationGlyph(peer.own), size = 18.dp, tint = Sillon.colors.dim)
        Column(Modifier.weight(1f)) {
            Text(
                text = peer.name.ifBlank { platformText(peer.platform) },
                style = Sillon.type.body,
                color = Sillon.colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = "${relationText(peer.own)} · ${signalText(peer)} · ${peerDetailText(peer)}",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
        }
    }
}

@Composable
private fun MyDevicesCard(host: LanModel, devices: ScreenState<List<LanDeviceFfi>>) {
    LanCard(
        title = "Mes appareils",
        subtitle = "Les appareils inscrits sur votre compte. « Révoquer » les coupe du LAN au prochain contact.",
    ) {
        when (devices) {
            is ScreenState.Loading -> Text("Chargement…", style = Sillon.type.bodySmall, color = Sillon.colors.dim)
            is ScreenState.Failed -> CauseLine(cause = devices.cause, onRetry = devices.retry)
            is ScreenState.Ready -> {
                if (devices.data.isEmpty()) {
                    Text(
                        text = "Aucun appareil inscrit pour l'instant.",
                        style = Sillon.type.bodySmall,
                        color = Sillon.colors.dim,
                    )
                }
                devices.data.forEach { device -> DeviceRow(host, device) }
            }
        }
    }
}

@Composable
private fun DeviceRow(host: LanModel, device: LanDeviceFfi) {
    var renaming by remember(device.id) { mutableStateOf(false) }
    var draft by remember(device.id, device.name) { mutableStateOf(device.name) }
    var confirming by remember(device.id) { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Icon(glyph = NzGlyph.User, size = 18.dp, tint = Sillon.colors.dim)
            Column(Modifier.weight(1f)) {
                if (renaming) {
                    LanTextField(
                        value = draft,
                        onValue = { draft = it },
                        label = "Nom de l'appareil",
                        onSubmit = {
                            host.renameDevice(device.id, draft)
                            renaming = false
                        },
                    )
                } else {
                    Text(
                        text = device.name,
                        style = Sillon.type.body,
                        color = Sillon.colors.tx,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Text(
                    text = buildString {
                        append(platformText(device.platform))
                        if (device.current) append(" · cet appareil")
                        seenText(device.lastSeen).takeIf { it.isNotEmpty() }?.let { append(" · $it") }
                    },
                    style = Sillon.type.bodySmall,
                    color = Sillon.colors.dim,
                )
            }
            if (!device.current) {
                if (renaming) {
                    NzButton(text = "OK", onClick = {
                        host.renameDevice(device.id, draft)
                        renaming = false
                    }, kind = NzButtonKind.Ghost)
                } else {
                    NzButton(text = "Renommer", onClick = { renaming = true }, kind = NzButtonKind.Ghost)
                }
            }
        }
        if (confirming) {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                NzButton(
                    text = "Confirmer la révocation",
                    onClick = {
                        host.revokeDevice(device.id)
                        confirming = false
                    },
                    kind = NzButtonKind.Primary,
                )
                NzButton(text = "Annuler", onClick = { confirming = false }, kind = NzButtonKind.Ghost)
            }
        } else if (!device.current) {
            NzButton(text = "Révoquer", onClick = { confirming = true }, kind = NzButtonKind.Ghost)
        }
    }
}

@Composable
private fun FriendsCard(host: LanModel, state: LanModel.State) {
    var name by remember { mutableStateOf("") }
    LanCard(
        title = "Amis de confiance",
        subtitle = "Un ami ne voit que ce que vous partagez avec lui. Ajoutez-le par le nom exact de son compte.",
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            LanTextField(
                value = name,
                onValue = { name = it },
                label = "Nom du compte",
                modifier = Modifier.weight(1f),
                onSubmit = {
                    host.addFriend(name)
                    name = ""
                },
            )
            NzButton(
                text = "Ajouter",
                onClick = {
                    host.addFriend(name)
                    name = ""
                },
                kind = NzButtonKind.Secondary,
                glyph = NzGlyph.PersonAdd,
                enabled = name.isNotBlank() && state.busy == null,
            )
        }
        val friends = state.friends
        when (friends) {
            is ScreenState.Loading -> Text("Chargement…", style = Sillon.type.bodySmall, color = Sillon.colors.dim)
            is ScreenState.Failed -> CauseLine(cause = friends.cause, onRetry = friends.retry)
            is ScreenState.Ready -> {
                if (friends.data.sharesTo.isEmpty() && friends.data.trustedBy.isEmpty()) {
                    Text(
                        text = "Personne pour l'instant : vos propres appareils suffisent à l'échange.",
                        style = Sillon.type.bodySmall,
                        color = Sillon.colors.dim,
                    )
                }
                friends.data.sharesTo.forEach { friend ->
                    FriendRow(host, friend, state, sharing = true)
                }
                if (friends.data.trustedBy.isNotEmpty()) {
                    Text(text = "Qui partage avec vous", style = Sillon.type.label, color = Sillon.colors.tx)
                    friends.data.trustedBy.forEach { friend ->
                        Text(
                            text = friend.name.ifBlank { "Un ami" },
                            style = Sillon.type.bodySmall,
                            color = Sillon.colors.dim,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun FriendRow(host: LanModel, friend: LanFriendFfi, state: LanModel.State, sharing: Boolean) {
    var confirming by remember(friend.userId) { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Icon(glyph = NzGlyph.User, size = 18.dp, tint = Sillon.colors.dim)
            Column(Modifier.weight(1f)) {
                Text(
                    text = friend.name.ifBlank { "Un ami" },
                    style = Sillon.type.body,
                    color = Sillon.colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = when {
                        friend.all -> "Tout partagé"
                        friend.playlists.isEmpty() -> "Aucune liste partagée"
                        friend.playlists.size == 1 -> "1 liste · ${friend.playlists.first().title}"
                        else -> "${friend.playlists.size} listes partagées"
                    },
                    style = Sillon.type.bodySmall,
                    color = Sillon.colors.dim,
                )
            }
            NzSwitch(
                checked = friend.all,
                onCheckedChange = { host.setFriendAll(friend.userId, it) },
                modifier = Modifier.semantics { contentDescription = "Tout partager avec ${friend.name}" },
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            NzButton(
                text = if (state.pickerFor == friend.userId) "Fermer les listes" else "Choisir des listes",
                onClick = {
                    if (state.pickerFor == friend.userId) host.closePicker() else host.openPicker(friend.userId)
                },
                kind = NzButtonKind.Ghost,
            )
            if (confirming) {
                NzButton(
                    text = "Confirmer le retrait",
                    onClick = {
                        host.removeFriend(friend.userId)
                        confirming = false
                    },
                    kind = NzButtonKind.Primary,
                )
                NzButton(text = "Annuler", onClick = { confirming = false }, kind = NzButtonKind.Ghost)
            } else {
                NzButton(text = "Retirer", onClick = { confirming = true }, kind = NzButtonKind.Ghost)
            }
        }
        if (state.pickerFor == friend.userId && !friend.all) {
            PlaylistPicker(host, friend, state.playlistChoices)
        }
    }
}

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun PlaylistPicker(host: LanModel, friend: LanFriendFfi, choices: List<LanPlaylistFfi>) {
    if (choices.isEmpty()) {
        Text(
            text = "Vos playlists apparaîtront ici dès qu'elles seront chargées.",
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
        return
    }
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        choices.forEach { playlist ->
            val picked = friend.playlists.any { it.id == playlist.id }
            NzChip(
                text = playlist.title.ifBlank { "Playlist" },
                selected = picked,
                onClick = { host.toggleFriendPlaylist(friend.userId, playlist.id) },
                modifier = Modifier.semantics {
                    contentDescription = "${playlist.title}, ${if (picked) "partagée" else "non partagée"}"
                },
            )
        }
    }
}

@Composable
private fun AdvancedCard(
    host: LanModel,
    state: LanModel.State,
    settings: LanSettingsFfi,
    snapshot: LanSnapshotFfi?,
) {
    LanCard(
        title = "Avancé",
        subtitle = "Le port UDP de l'échange, et l'état de la clé qui vous identifie sur le réseau local.",
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            LanTextField(
                value = state.portDraft,
                onValue = host::setPortDraft,
                label = "Port UDP (0 = automatique)",
                modifier = Modifier.weight(1f),
                numeric = true,
                onSubmit = host::commitPort,
            )
            if (state.portDraft != settings.port.toString()) {
                NzButton(text = "Appliquer", onClick = host::commitPort, kind = NzButtonKind.Ghost)
            }
        }
        Text(
            text = portText(snapshot?.port?.toInt() ?: 0, snapshot?.portFallback == true),
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
        Text(
            text = keyText(snapshot?.keyUntil, snapshot?.keyError),
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
    }
}

// -- the small pieces --------------------------------------------------------

@Composable
private fun SwitchLine(
    label: String,
    hint: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    tag: String,
) {
    Row(
        modifier = Modifier.fillMaxWidth().testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Column(Modifier.weight(1f)) {
            Text(text = label, style = Sillon.type.body, color = Sillon.colors.tx)
            Text(text = hint, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
        }
        NzSwitch(
            checked = checked,
            onCheckedChange = onChange,
            modifier = Modifier.semantics { contentDescription = label },
        )
    }
}

/**
 * The local-network permission, asked when the person TURNS THE FEATURE ON and
 * never at start-up, with the reason in the same card.
 *
 * Android 13–16 gate Wi-Fi APIs behind `NEARBY_WIFI_DEVICES`; Android 17 (API 37)
 * replaces it with a runtime `ACCESS_LOCAL_NETWORK` for everything that reaches
 * the LAN, `NsdManager` included (Android Developers, *Local network permission*,
 * read 2026-11-03). This build targets 35, where access is implicitly granted,
 * so the line only appears the day a device actually asks.
 */
@Composable
private fun LocalNetworkPermissionLine(enabled: Boolean) {
    val permission = localNetworkPermission() ?: return
    val context = LocalContext.current
    var granted by remember { mutableStateOf(isGranted(context, permission)) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { answered ->
        granted = answered
    }
    if (granted) return
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Text(
            text = "Android doit autoriser l'accès au réseau local pour que les appareils se voient. " +
                "Rien n'est demandé au démarrage : seulement quand vous activez l'échange.",
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
        NzButton(
            text = "Autoriser l'accès au réseau local",
            onClick = { if (enabled) launcher.launch(permission) },
            kind = NzButtonKind.Secondary,
            glyph = NzGlyph.Shield,
            modifier = Modifier.semantics { contentDescription = "Autoriser l'accès au réseau local" },
        )
    }
}

/** The permission a local-network feature must hold on this OS, or `null`. */
fun localNetworkPermission(): String? = when {
    Build.VERSION.SDK_INT >= 37 -> ACCESS_LOCAL_NETWORK
    Build.VERSION.SDK_INT >= 33 -> Manifest.permission.NEARBY_WIFI_DEVICES
    else -> null
}

/** Android 17's name; no `Manifest.permission` constant exists before API 37. */
const val ACCESS_LOCAL_NETWORK: String = "android.permission.ACCESS_LOCAL_NETWORK"

/** Whether the local-network permission is already granted on this device. */
fun isGranted(context: android.content.Context, permission: String): Boolean =
    ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

@Composable
private fun LanCard(title: String, subtitle: String, content: @Composable () -> Unit) {
    NzCard(Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text(text = title, style = Sillon.type.label, color = Sillon.colors.tx)
            Text(text = subtitle, style = Sillon.type.bodySmall, color = Sillon.colors.dim)
            content()
        }
    }
}

/** A cause, in the server's or the core's own French, with its retry (rule 3). */
@Composable
internal fun CauseLine(cause: String, onRetry: (() -> Unit)? = null, onDismiss: (() -> Unit)? = null) {
    Row(
        modifier = Modifier.fillMaxWidth().semantics { contentDescription = "Échec : $cause" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = NzGlyph.Alert, size = 16.dp, tint = Sillon.colors.dim)
        Text(text = cause, style = Sillon.type.bodySmall, color = Sillon.colors.dim, modifier = Modifier.weight(1f))
        if (onRetry != null) {
            NzButton(text = "Réessayer", onClick = onRetry, kind = NzButtonKind.Ghost, glyph = NzGlyph.Refresh)
        }
        if (onDismiss != null) {
            NzButton(text = "Fermer", onClick = onDismiss, kind = NzButtonKind.Ghost)
        }
    }
}

/** A one-line field: Sillon's colours, 48 dp, accent cursor (the SPA's `.field`). */
@Composable
internal fun LanTextField(
    value: String,
    onValue: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    numeric: Boolean = false,
    onSubmit: () -> Unit = {},
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(fr.neezer.mobile.designsystem.NzRadius.field)
    BasicTextField(
        value = value,
        onValueChange = onValue,
        singleLine = true,
        textStyle = Sillon.type.body.copy(color = colors.tx),
        cursorBrush = SolidColor(colors.acText),
        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
            keyboardType = if (numeric) {
                androidx.compose.ui.text.input.KeyboardType.Number
            } else {
                androidx.compose.ui.text.input.KeyboardType.Text
            },
            imeAction = ImeAction.Done,
        ),
        keyboardActions = androidx.compose.foundation.text.KeyboardActions(onDone = { onSubmit() }),
        modifier = modifier.semantics { contentDescription = label },
        decorationBox = { inner ->
            androidx.compose.foundation.layout.Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(48.dp)
                    .background(colors.s1, shape)
                    .border(1.dp, colors.ln, shape)
                    .padding(horizontal = NzSpacing.m),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (value.isEmpty()) {
                    Text(
                        text = label,
                        style = Sillon.type.body,
                        color = colors.dim,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                inner()
            }
        },
    )
}

/** A device of mine is a person's device, a friend's is a group: no platform glyph. */
internal fun relationGlyph(own: Boolean): NzGlyph = if (own) NzGlyph.User else NzGlyph.People

/**
 * The section's model: the app's own (one per process, so the switches and the
 * route agree) unless a test passes one.
 */
@Composable
private fun rememberLanModel(model: LanModel?): LanModel {
    if (model != null) return model
    return remember { LanHost.model() }
}

/** The playlist picker's source: the library's own `/api/me/playlists` read. */
internal object BridgePlaylists : LanPlaylistSource {
    override fun cached(): List<LanPlaylistFfi> = runCatching {
        fr.neezer.mobile.bridge.NzBridge.core?.screens()?.cachedLibraryPlaylists()
    }.getOrNull()?.playlists.orEmpty().map { LanPlaylistFfi(id = it.id, title = it.title) }

    override suspend fun fresh(): List<LanPlaylistFfi> {
        val tree = fr.neezer.mobile.bridge.NzBridge.core?.screens()?.libraryPlaylists() ?: return emptyList()
        return tree.playlists.map { LanPlaylistFfi(id = it.id, title = it.title) }
    }
}
