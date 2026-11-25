package fr.neezer.mobile.feature.setup

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import android.app.Activity
import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.platform.LocalAutofillManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.launch
import uniffi.nz_ffi.ProbeResult
import uniffi.nz_ffi.Screens

/**
 * Serveur — lot MB4 (docs/agents/mobile.md, feature inventory).
 *
 * The app's first screen on a fresh install, in the order a person meets it:
 *
 * 1. **the server** — host, optional port, and "Vérifier le certificat" (off
 *    means trust on first use, never trust-all); the certificate the handshake
 *    presented is shown with its SHA-256 before it is trusted;
 * 2. **then the account**, on the same screen: one `POST /api/login`, the cause
 *    on failure, and the session kept by the jar so the person is asked once.
 *
 * It is one screen and not two because `NzNavigator` speaks `AppLink`s (the
 * routes a link can name) and the setup/login routes are not links. The old
 * `SetupActivity` then opened the app; here the second half appears where the
 * first one ended, which is also what a first launch deserves.
 *
 * Every action is ONE request (rule 1), every failure is a cause with
 * "Réessayer" (rule 3), and nothing is persisted until "Se connecter" — the
 * lesson the old setup screen learned when a bad host stayed saved after a probe
 * followed by a back press.
 */
@Composable
fun SetupEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens, core) {
        if (screens != null && core != null) NzScreens(screens, core) else null
    }
    val scope = rememberCoroutineScope()
    val setup = remember(service) { service?.let { SetupModel(it, scope) } }
    val login = remember(service) { service?.let { LoginModel(it, scope) } }
    val migration by MigrationRunner.state.collectAsState()

    // The migration runs at process start (a content provider); this is the
    // second door, for a launch where the core was not ready yet.
    LaunchedEffect(Unit) { MigrationRunner.startIfNeeded(context) }

    if (setup == null || login == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }

    val server by setup.state.collectAsState()
    val account by login.state.collectAsState()

    Column(
        modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = NzSpacing.l, vertical = NzSpacing.m),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        MigrationNotice(migration)

        // Où l'on est déjà : sans ça, « Changer de serveur » demandait de
        // retaper une adresse sans dire laquelle on quitte.
        val current = remember(core) { runCatching { core?.server()?.origin }.getOrNull() }
        if (current != null && !server.saved) {
            Text(
                text = "Serveur actuel : $current",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
        }

        if (!server.saved) {
            NzCard {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                    Text("Adresse du serveur", style = Sillon.type.titleSmall)
                    Text(
                        "Le serveur Neezer qui héberge ta musique. Sur un réseau local, " +
                            "laisse la vérification active si ton serveur a un vrai certificat.",
                        style = Sillon.type.bodySmall,
                    )
                    NzField(
                        value = server.host,
                        onValueChange = setup::onHost,
                        label = "Serveur",
                        placeholder = "music.example.org",
                        keyboard = KeyboardType.Uri,
                    )
                    NzField(
                        value = server.port,
                        onValueChange = setup::onPort,
                        label = "Port (facultatif)",
                        placeholder = "4533",
                        keyboard = KeyboardType.Number,
                    )
                    Row(
                        Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text("Vérifier le certificat", style = Sillon.type.label)
                        NzSwitch(
                            checked = server.verifySsl,
                            onCheckedChange = setup::onVerifySsl,
                            label = "Vérifier le certificat",
                        )
                    }
                    server.cause?.let { cause ->
                        Text(cause, style = Sillon.type.bodySmall, color = Sillon.colors.hrt)
                    }
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        NzButton(
                            text = if (server.checking) "Vérification…" else "Vérifier le serveur",
                            onClick = { setup.check() },
                            enabled = server.canCheck,
                            glyph = NzGlyph.Check,
                        )
                        if (server.cause != null) {
                            NzButton(
                                text = "Continuer quand même",
                                onClick = { setup.continueAnyway() },
                                kind = NzButtonKind.Secondary,
                            )
                        }
                        if (server.checking) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(18.dp),
                                color = Sillon.colors.ac,
                                strokeWidth = 2.dp,
                            )
                        }
                    }
                }
            }
            server.probe?.let { probe -> CertificateCard(probe, onAccept = { setup.acceptPin() }) }
        } else {
            LoginForm(login, nav)
        }
    }
}

/**
 * Connexion — lot MB2/MB4.
 *
 * The route `NavGraph` declares, for a shell that opens the login screen
 * directly (a session that ended, a first launch after the server step). Same
 * form as the setup screen's second half, because it is the same component.
 */
@Composable
fun LoginEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens, core) {
        if (screens != null && core != null) NzScreens(screens, core) else null
    }
    val scope = rememberCoroutineScope()
    val login = remember(service) { service?.let { LoginModel(it, scope) } }

    if (login == null) {
        EmptyState(
            title = "Le noyau de l'application n'est pas disponible",
            body = NzBridge.failure ?: "Redémarre l'application.",
            modifier = modifier,
        )
        return
    }
    // A launch that already has a cookie restores silently: this screen is
    // shown only when nobody is signed in.
    LaunchedEffect(Unit) { login.restore { nav.openHomeAfterSignIn() } }
    Column(
        modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = NzSpacing.l, vertical = NzSpacing.m),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        LoginForm(login, nav)
    }
}

/**
 * The account form: two fields, one request, the cause under it.
 *
 * The two fields are declared to the platform's autofill service (the
 * `ContentType`s below), which is why a password manager offers the saved
 * Neezer login instead of guessing — and the password field's "Done" submits,
 * with `commit()` on success so the manager offers to save a new password.
 * Before this, both fields were plain text to autofill, and Proton Pass
 * mis-detected them (owner's report on v1.6.5).
 */
@Composable
private fun LoginForm(model: LoginModel, nav: NzNavigator) {
    val state by model.state.collectAsState()
    val autofill = LocalAutofillManager.current
    // ONE submit path for the button and the keyboard's "Done": the same
    // request, the same commit of a successful save.
    val submit = {
        model.submit {
            autofill?.commit()
            nav.openHomeAfterSignIn()
        }
    }
    NzCard {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            Text("Ton compte", style = Sillon.type.titleSmall)
            NzField(
                value = state.username,
                onValueChange = model::onUsername,
                label = "Nom d'utilisateur",
                placeholder = "",
                keyboard = KeyboardType.Text,
                contentType = ContentType.Username,
                imeAction = ImeAction.Next,
            )
            NzField(
                value = state.password,
                onValueChange = model::onPassword,
                label = "Mot de passe",
                placeholder = "",
                keyboard = KeyboardType.Password,
                secret = true,
                contentType = ContentType.Password,
                imeAction = ImeAction.Done,
                onImeAction = submit,
            )
            state.cause?.let { cause ->
                Text(cause, style = Sillon.type.bodySmall, color = Sillon.colors.hrt)
                NzButton(
                    text = "Réessayer",
                    onClick = submit,
                    kind = NzButtonKind.Secondary,
                    enabled = state.canSubmit,
                )
            }
            NzButton(
                text = if (state.busy) "Connexion…" else "Se connecter",
                onClick = submit,
                enabled = state.canSubmit,
                glyph = NzGlyph.User,
            )
            Text(
                "La session reste ouverte sur cet appareil : tu ne seras pas redemandé à " +
                    "chaque lancement.",
                style = Sillon.type.bodySmall,
            )
        }
    }
}

/**
 * The certificate the handshake presented, shown BEFORE it is trusted: the
 * fingerprint a person compares with
 * `openssl x509 -noout -fingerprint -sha256` on the server, and the one line
 * that matters when it is not the pinned leaf ("le certificat a changé").
 */
@Composable
private fun CertificateCard(probe: ProbeResult, onAccept: () -> Unit) {
    val cert = probe.cert ?: return
    NzCard {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    glyph = if (probe.pinChanged) NzGlyph.Alert else NzGlyph.Check,
                    tint = if (probe.pinChanged) Sillon.colors.hrt else Sillon.colors.ac,
                    contentDescription = null,
                )
                Spacer(Modifier.width(NzSpacing.s))
                Text(
                    when {
                        probe.pinChanged -> "Le certificat a changé"
                        probe.systemTrusted -> "Certificat vérifié par Android"
                        else -> "Certificat auto-signé"
                    },
                    style = Sillon.type.titleSmall,
                )
            }
            Text(cert.sha256, style = Sillon.type.labelMono)
            Text("Émis par ${cert.issuer}", style = Sillon.type.bodySmall)
            Text("Valable jusqu'au ${cert.notAfter.take(10)}", style = Sillon.type.bodySmall)
            Text("Serveur ${probe.version} · ${if (probe.https) "https" else "http"}", style = Sillon.type.bodySmall)
            if (probe.pin != null && (!probe.systemTrusted || probe.pinChanged)) {
                Text(
                    "Android ne connaît pas ce certificat. Accepte-le pour que l'application " +
                        "ne fasse confiance qu'à celui-là, et à aucun autre.",
                    style = Sillon.type.bodySmall,
                )
                NzButton(
                    text = if (probe.pinChanged) {
                        "Accepter le nouveau certificat"
                    } else {
                        "Accepter ce certificat"
                    },
                    onClick = onAccept,
                    kind = NzButtonKind.Secondary,
                    glyph = NzGlyph.Check,
                )
            }
            if (probe.updateApk) {
                Text(
                    probe.androidVersion?.let {
                        "Le serveur publie la version $it : Réglages → Assistance pour l'installer."
                    } ?: "Le serveur publie une mise à jour : Réglages → Assistance pour l'installer.",
                    style = Sillon.type.bodySmall,
                )
            }
        }
    }
}

/**
 * What the one-shot check of the old app found, or the cause that stopped it
 * (rule 2: say it, and name each failure — never just a count). With the two
 * apps on separate ids this is where the "Android ne permet pas…" sentence is
 * read, so the title is about the OLD app, never "données reprises".
 */
@Composable
fun MigrationNotice(state: MigrationRunner.State, modifier: Modifier = Modifier) {
    if (state.running) {
        NzCard(modifier) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(
                    modifier = Modifier.size(18.dp),
                    color = Sillon.colors.ac,
                    strokeWidth = 2.dp,
                )
                Spacer(Modifier.width(NzSpacing.s))
                Text(
                    "Vérification de l'ancienne application…",
                    style = Sillon.type.bodySmall,
                )
            }
        }
        return
    }
    val notice = state.notice
    val cause = state.cause
    if (notice == null && cause == null) return
    NzCard(modifier) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
            Text(
                if (cause != null) "Ancienne application : vérification impossible" else "Ancienne application",
                style = Sillon.type.titleSmall,
            )
            Text(cause ?: notice.orEmpty(), style = Sillon.type.bodySmall)
        }
    }
}

/**
 * The input this design system has no component for yet: Material's text field
 * re-skinned with Sillon's colours and radii, one line, 48 dp tall.
 *
 * A grammar of its own would be worse than one honest re-skin; `NzSheet` does
 * the same for Material's sheet.
 *
 * [contentType] is only for the fields a password manager may fill: Compose
 * publishes it to the autofill framework through `Modifier.semantics`
 * (`androidx.compose.ui.autofill.ContentType`). The server-address fields pass
 * nothing — a host and a port are not credentials, and declaring them as any
 * would put the account login in front of somebody still naming their server.
 */
@Composable
private fun NzField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    placeholder: String,
    keyboard: KeyboardType,
    secret: Boolean = false,
    contentType: ContentType? = null,
    imeAction: ImeAction = ImeAction.Next,
    onImeAction: (() -> Unit)? = null,
) {
    val colors = OutlinedTextFieldDefaults.colors(
        focusedTextColor = Sillon.colors.tx,
        unfocusedTextColor = Sillon.colors.tx,
        focusedContainerColor = Sillon.colors.s1,
        unfocusedContainerColor = Sillon.colors.s1,
        focusedBorderColor = Sillon.colors.ac,
        unfocusedBorderColor = Sillon.colors.ln,
        focusedLabelColor = Sillon.colors.dim,
        unfocusedLabelColor = Sillon.colors.dim,
        cursorColor = Sillon.colors.ac,
    )
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        modifier = Modifier
            .fillMaxWidth()
            .then(
                if (contentType == null) {
                    Modifier
                } else {
                    Modifier.semantics { this.contentType = contentType }
                },
            ),
        label = { Text(label, style = Sillon.type.labelSmall) },
        placeholder = { Text(placeholder, style = Sillon.type.bodySmall) },
        singleLine = true,
        textStyle = Sillon.type.body,
        visualTransformation = if (secret) {
            PasswordVisualTransformation()
        } else {
            VisualTransformation.None
        },
        keyboardOptions = KeyboardOptions(keyboardType = keyboard, imeAction = imeAction),
        keyboardActions = if (onImeAction == null) {
            KeyboardActions.Default
        } else {
            KeyboardActions(
                onNext = { onImeAction() },
                onDone = { onImeAction() },
            )
        },
        colors = colors,
        shape = RoundedCornerShape(NzRadius.field),
    )
}

/**
 * Réglages → Compte (MB16 places this card): links the Deezer account through
 * [DeezerLoginActivity], the only route to an ARL that still exists.
 *
 * The value crosses back to the SERVER, never to this device: the activity hands
 * it to `Screens.saveDeezerArl`, which posts it to `/api/settings`. The card says
 * what happened, and when the server refused the ARL, its own reason.
 */
@Composable
fun DeezerAccountCard(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var cause by remember { mutableStateOf<String?>(null) }
    var linked by remember { mutableStateOf<String?>(null) }

    fun submit(arl: String) {
        val target = screens
        if (target == null) {
            cause = "La liaison Deezer a besoin du noyau de l'application"
            busy = false
            return
        }
        scope.launch {
            runCatching { target.saveDeezerArl(arl) }
                .onSuccess { link ->
                    linked = link.hint?.let { "Compte lié ($it)" } ?: "Compte lié"
                    busy = false
                }
                .onFailure { error ->
                    cause = NzErrors.text(error)
                    busy = false
                }
        }
    }

    // The Activity Result contract, not a broadcast: the ARL comes back to the
    // very screen that asked for it, and a cancelled screen closes the loop
    // instead of leaving the button spinning (rule 3).
    val launcher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        val arl = result.data?.getStringExtra(DeezerLoginActivity.EXTRA_ARL)
        if (result.resultCode == Activity.RESULT_OK && !arl.isNullOrEmpty()) {
            submit(arl)
        } else {
            busy = false
        }
    }

    NzCard(modifier) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Text("Compte Deezer", style = Sillon.type.titleSmall)
            Text(
                "La connexion se fait sur la page de Deezer, dans un écran dédié : le mot de " +
                    "passe ne passe ni par l'application ni par le serveur, seul le jeton de " +
                    "session est transmis à ton serveur.",
                style = Sillon.type.bodySmall,
            )
            cause?.let { Text(it, style = Sillon.type.bodySmall, color = Sillon.colors.hrt) }
            linked?.let { Text(it, style = Sillon.type.bodySmall) }
            NzButton(
                text = if (busy) "Liaison…" else "Lier mon compte Deezer",
                onClick = {
                    busy = true
                    cause = null
                    launcher.launch(
                        Intent(context, DeezerLoginActivity::class.java),
                    )
                },
                enabled = !busy && screens != null,
                kind = NzButtonKind.Secondary,
                glyph = NzGlyph.Link,
            )
        }
    }
}
