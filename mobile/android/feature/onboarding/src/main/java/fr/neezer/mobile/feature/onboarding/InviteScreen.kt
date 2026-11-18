package fr.neezer.mobile.feature.onboarding

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
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
import uniffi.nz_ffi.InviteRules
import uniffi.nz_ffi.PasswordStrength

/** Everything the invitation page can do. */
data class InviteActions(
    val username: (String) -> Unit = { _ -> },
    val password: (String) -> Unit = { _ -> },
    val confirm: (String) -> Unit = { _ -> },
    val blur: (InviteModel.Field) -> Unit = { _ -> },
    val submit: () -> Unit = {},
    val toggleShowPassword: () -> Unit = {},
    val retry: () -> Unit = {},
    val openApp: () -> Unit = {},
    val useInvitation: () -> Unit = {},
)

/** What the form's rules say right now, computed by the model's port. */
data class InviteFeedback(
    val usernameError: String = "",
    val passwordError: String = "",
    val confirmError: String = "",
    val strength: PasswordStrength = PasswordStrength.TOO_SHORT,
    val score: Int = 0,
    val missing: Int = 0,
    val rules: InviteRules = InviteRules(
        minUsername = 3u,
        maxUsername = 32u,
        minPassword = 12u,
        maxPassword = 128u,
    ),
    val expiry: String = "",
)

/** The page's marker in every state: the emulator/bench waits for this node. */
const val INVITE_MARKER = "Rejoindre Neezer"

/**
 * L'invitation — lot MB5b: `routes/Invite.svelte`, ported.
 *
 * The link is checked before the form (a malformed token costs nothing), the
 * form validates as the page does, and a refusal from the server goes UNDER the
 * field it names. Every state has its sentence: "plus valide" has no retry, a
 * failed check has its cause and « Réessayer », and a created account says it is
 * opening.
 */
@Composable
fun InviteScreen(
    state: InviteModel.State,
    feedback: InviteFeedback,
    actions: InviteActions,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Box(
        modifier = modifier
            .fillMaxSize()
            .background(colors.bg)
            .semantics { contentDescription = INVITE_MARKER },
        contentAlignment = Alignment.TopCenter,
    ) {
        Column(
            modifier = Modifier
                .verticalScroll(rememberScrollState())
                .widthIn(max = 460.dp)
                .fillMaxWidth()
                .padding(
                    start = NzSpacing.pageX,
                    end = NzSpacing.pageX,
                    top = NzSpacing.xxl,
                    bottom = NzSpacing.xxl,
                ),
        ) {
            NzCard(Modifier.fillMaxWidth()) {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.l)) {
                    Brand()
                    when (state.phase) {
                        InviteModel.Phase.Checking -> Checking()
                        InviteModel.Phase.Invalid -> Invalid(
                            accountName = state.accountName,
                            onOpenApp = actions.openApp,
                        )
                        InviteModel.Phase.Error -> Failed(
                            title = state.failureTitle,
                            cause = state.failure.orEmpty(),
                            onRetry = actions.retry,
                        )
                        InviteModel.Phase.Created -> Created()
                        InviteModel.Phase.Ready -> {
                            if (state.accountName != null) {
                                AlreadySignedIn(state, actions)
                            } else {
                                Form(state, feedback, actions)
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Brand() {
    val colors = Sillon.colors
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Box(
            Modifier.size(30.dp).clip(CircleShape).background(colors.ac),
            contentAlignment = Alignment.Center,
        ) {
            Icon(NzGlyph.Disc, size = 17.dp, tint = colors.onac)
        }
        Text("Neezer", style = Sillon.type.titleSmall.copy(color = colors.tx))
    }
}

@Composable
private fun Checking() {
    val colors = Sillon.colors
    Column(
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        modifier = Modifier.semantics { contentDescription = "Vérification de l'invitation…" },
    ) {
        SkeletonBlock(width = 0.68f, height = 30.dp)
        SkeletonBlock(width = 1f, height = 12.dp)
        SkeletonBlock(width = 0.72f, height = 12.dp)
        SkeletonBlock(width = 1f, height = 48.dp, radius = NzRadius.field)
        SkeletonBlock(width = 1f, height = 48.dp, radius = NzRadius.field)
        SkeletonBlock(width = 1f, height = 48.dp, radius = NzRadius.field)
        Spacer(Modifier.height(NzSpacing.xs))
        SkeletonBlock(width = 1f, height = 48.dp, radius = NzRadius.pill)
        Text("", style = Sillon.type.bodySmall.copy(color = colors.dim))
    }
}

@Composable
private fun SkeletonBlock(width: Float, height: androidx.compose.ui.unit.Dp, radius: androidx.compose.ui.unit.Dp = NzRadius.field) {
    Box(
        Modifier
            .fillMaxWidth(width)
            .height(height)
            .clip(RoundedCornerShape(radius))
            .background(Sillon.colors.s2),
    )
}

@Composable
private fun Invalid(accountName: String?, onOpenApp: () -> Unit) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        Box(
            Modifier.size(44.dp).clip(CircleShape).background(colors.s2),
            contentAlignment = Alignment.Center,
        ) {
            Icon(NzGlyph.Alert, size = 24.dp, tint = colors.dim)
        }
        Text(
            "Cette invitation n'est plus valide.",
            style = Sillon.type.title.copy(color = colors.tx),
            modifier = Modifier.semantics { heading() },
        )
        Text(
            "Demande un nouveau lien à la personne qui t'a invitée.",
            style = Sillon.type.body.copy(color = colors.dim),
        )
        NzButton(
            text = if (accountName != null) "Retour à l'application" else "Se connecter",
            onClick = onOpenApp,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun Failed(title: String, cause: String, onRetry: () -> Unit) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        Text(
            title,
            style = Sillon.type.title.copy(color = colors.tx),
            modifier = Modifier.semantics { heading() },
        )
        LoadFailed(cause = cause, onRetry = onRetry)
        NzButton(
            text = "Réessayer",
            onClick = onRetry,
            kind = NzButtonKind.Secondary,
            glyph = NzGlyph.Refresh,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun Created() {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Text(
            "Ton compte est créé.",
            style = Sillon.type.title.copy(color = colors.tx),
            modifier = Modifier.semantics { heading() },
        )
        Text(
            "Ouverture de Neezer…",
            style = Sillon.type.body.copy(color = colors.dim),
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        )
    }
}

@Composable
private fun AlreadySignedIn(state: InviteModel.State, actions: InviteActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        Text(
            "Tu es déjà connecté",
            style = Sillon.type.title.copy(color = colors.tx),
            modifier = Modifier.semantics { heading() },
        )
        Text(
            "Tu es connecté en tant que ${state.accountName}. Créer un compte ici te déconnectera " +
                "de celui-ci.",
            style = Sillon.type.body.copy(color = colors.dim),
        )
        if (state.leaveError.isNotEmpty()) {
            AlertText(state.leaveError)
        }
        NzButton(
            text = "Retour à l'application",
            onClick = actions.openApp,
            modifier = Modifier.fillMaxWidth(),
        )
        NzButton(
            text = if (state.leaving) "Déconnexion…" else "Utiliser cette invitation",
            onClick = actions.useInvitation,
            kind = NzButtonKind.Ghost,
            enabled = !state.leaving,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun Form(
    state: InviteModel.State,
    feedback: InviteFeedback,
    actions: InviteActions,
) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.l)) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            if (state.label.isNotEmpty()) {
                Text(
                    state.label.uppercase(),
                    style = Sillon.type.labelMono.copy(color = colors.dim),
                )
            }
            Text(
                "Rejoindre Neezer",
                style = Sillon.type.displaySmall.copy(color = colors.tx),
                modifier = Modifier.semantics { heading() },
            )
            Text(
                "Crée ton compte. Ce lien ne fonctionne qu'une fois.",
                style = Sillon.type.body.copy(color = colors.dim),
            )
            if (feedback.expiry.isNotEmpty()) {
                Text(
                    feedback.expiry,
                    style = Sillon.type.labelMono.copy(color = colors.dim),
                )
            }
        }
        NzField(
            label = "Nom d'utilisateur",
            placeholder = "elodie",
            value = state.username,
            onValue = actions.username,
            hint = "${feedback.rules.minUsername} à ${feedback.rules.maxUsername} caractères : " +
                "a–z, 0–9, point, tiret, tiret bas.",
            error = if (feedback.usernameError.isNotEmpty()) feedback.usernameError else "",
            keyboardType = KeyboardType.Ascii,
        )
        PasswordField(state, feedback, actions)
        NzField(
            label = "Confirmer le mot de passe",
            placeholder = "",
            value = state.confirm,
            onValue = actions.confirm,
            hint = "",
            masked = !state.showPassword,
            error = feedback.confirmError,
        )
        if (state.formError.isNotEmpty()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(NzRadius.field))
                    .background(colors.s2)
                    .padding(NzSpacing.m)
                    .semantics { liveRegion = LiveRegionMode.Assertive },
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(NzGlyph.Alert, size = 16.dp, tint = colors.acText)
                Text(
                    state.formError,
                    style = Sillon.type.bodySmall.copy(color = colors.tx),
                    modifier = Modifier.weight(1f),
                )
                if (state.pendingRetry) {
                    NzButton("Réessayer", actions.submit, kind = NzButtonKind.Ghost)
                }
            }
        }
        NzButton(
            text = if (state.busy) "Création…" else "Créer mon compte",
            onClick = actions.submit,
            enabled = !state.busy,
            modifier = Modifier.fillMaxWidth(),
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Déjà un compte ?", style = Sillon.type.bodySmall.copy(color = colors.dim))
            Text(
                "Se connecter",
                style = Sillon.type.label.copy(color = colors.tx),
                modifier = Modifier
                    .heightIn(min = NzMinTouchTarget)
                    .clickable(role = Role.Button, onClick = actions.openApp)
                    .padding(horizontal = NzSpacing.s, vertical = NzSpacing.m),
            )
        }
    }
}

@Composable
private fun PasswordField(
    state: InviteModel.State,
    feedback: InviteFeedback,
    actions: InviteActions,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.field)
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Text("Mot de passe", style = Sillon.type.label.copy(color = colors.tx))
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = NzMinTouchTarget)
                .clip(shape)
                .background(colors.bg)
                .border(
                    1.dp,
                    if (feedback.passwordError.isNotEmpty()) colors.acText else colors.ln,
                    shape,
                )
                .padding(start = NzSpacing.m),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            val transformation = if (state.showPassword) {
                VisualTransformation.None
            } else {
                PasswordVisualTransformation()
            }
            BasicTextField(
                value = state.password,
                onValueChange = actions.password,
                singleLine = true,
                textStyle = Sillon.type.body.copy(color = colors.tx),
                cursorBrush = SolidColor(colors.acText),
                visualTransformation = transformation,
                keyboardOptions = KeyboardOptions(
                    imeAction = ImeAction.Next,
                    keyboardType = KeyboardType.Password,
                ),
                modifier = Modifier
                    .weight(1f)
                    .semantics { contentDescription = "Mot de passe" },
                decorationBox = { inner ->
                    CompositionLocalProvider(LocalContentColor provides colors.tx) { inner() }
                },
            )
            NzIconButton(
                glyph = NzGlyph.Eye,
                contentDescription = if (state.showPassword) {
                    "Masquer le mot de passe"
                } else {
                    "Afficher le mot de passe"
                },
                onClick = actions.toggleShowPassword,
                size = 17.dp,
            )
        }
        if (state.password.isNotEmpty()) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Row(
                    modifier = Modifier
                        .weight(1f)
                        .semantics {
                            progressBarRangeInfo = ProgressBarRangeInfo(
                                current = feedback.score.toFloat(),
                                range = 0f..4f,
                            )
                            contentDescription = "Force du mot de passe : " +
                                OnboardingCopy.strengthLabel(feedback.strength)
                        },
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    repeat(4) { level ->
                        Box(
                            Modifier
                                .weight(1f)
                                .height(4.dp)
                                .clip(RoundedCornerShape(2.dp))
                                .background(
                                    if (level < feedback.score) colors.acText else colors.ln,
                                ),
                        )
                    }
                }
                Text(
                    OnboardingCopy.strengthLabel(feedback.strength),
                    style = Sillon.type.labelSmall.copy(color = colors.tx),
                )
                if (feedback.missing > 0) {
                    Text(
                        "${state.password.length} / ${feedback.rules.minPassword}",
                        style = Sillon.type.mono.copy(color = colors.dim),
                    )
                }
            }
        }
        Text(
            "${feedback.rules.minPassword} caractères minimum, ${feedback.rules.maxPassword} au plus. " +
                "Une phrase de passe fait très bien l'affaire.",
            style = Sillon.type.bodySmall.copy(color = colors.dim),
        )
        if (feedback.passwordError.isNotEmpty()) {
            AlertText(feedback.passwordError)
        }
    }
}

@Composable
private fun AlertText(text: String) {
    val colors = Sillon.colors
    Row(
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Icon(NzGlyph.Alert, size = 14.dp, tint = colors.acText)
        Text(text, style = Sillon.type.bodySmall.copy(color = colors.acText))
    }
}
