package fr.neezer.mobile.feature.remote

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon

/**
 * Ouverture d'un lien de télécommande (`/rc/<token>`): claim it, then DRIVE.
 *
 * `routes/RemoteClaim.svelte`, ported with one deliberate difference: the SPA
 * reloads the whole app into remote mode, because its player store can be
 * rebound. The phone lands on the loan's own screen instead — the same
 * "Contrôle distant" the desktop has (see `RemoteControl.kt` for why the shell's
 * transport is not the place to do it) — and « Quitter » hands the account back.
 *
 * A `token` of `""` is not an error: it means "this session is already a loan"
 * (the shell's button opens this screen after a restart, when there is no link to
 * paste), and the grant `/api/me` reports is the truth.
 */
@Composable
fun RemoteClaimEntry(
    token: String,
    nav: NzNavigator,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    val model = remember(token) { RemoteModel(NzRemoteService, scope) }
    val state by model.state.collectAsState()

    LaunchedEffect(token) { model.claim(token) }

    val colors = Sillon.colors
    Box(
        modifier = modifier
            .fillMaxSize()
            .background(colors.bg),
    ) {
        when {
            state.ended != null -> RemoteEnded(
                restored = state.ended,
                onBack = {
                    nav.back()
                },
            )

            state.claim == ClaimPhase.Connected -> RemoteControl(
                model = model,
                onLeave = { nav.back() },
            )

            else -> ClaimCard(
                phase = state.claim,
                owner = state.grant?.owner,
                level = state.grant?.levelLabel,
                cause = state.claimCause,
                onRetry = { model.claim(token) },
                onHome = { nav.back() },
            )
        }
    }
}

@Composable
private fun ClaimCard(
    phase: ClaimPhase,
    owner: String?,
    level: String?    ,
    cause: String?,
    onRetry: () -> Unit,
    onHome: () -> Unit,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier.fillMaxSize().padding(NzSpacing.xl),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            Modifier
                .size(64.dp)
                .background(
                    if (phase == ClaimPhase.Connected) colors.ac else colors.s2,
                    RoundedCornerShape(NzRadius.card),
                ),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                NzGlyph.Remote,
                size = 28.dp,
                tint = if (phase == ClaimPhase.Connected) colors.onac else colors.tx,
            )
        }
        when (phase) {
            ClaimPhase.Claiming, ClaimPhase.Idle -> {
                Text("Connexion au lecteur…", style = Sillon.type.title, color = colors.tx)
                Text(
                    "Le lien est vérifié, puis l'app se met à piloter ce lecteur.",
                    style = Sillon.type.body,
                    color = colors.dim,
                    textAlign = TextAlign.Center,
                )
            }

            ClaimPhase.Offline -> {
                Text("Pas de connexion", style = Sillon.type.title, color = colors.tx)
                Text(
                    "Le serveur est injoignable pour l'instant.",
                    style = Sillon.type.body,
                    color = colors.dim,
                    textAlign = TextAlign.Center,
                )
                NzButton(text = "Réessayer", glyph = NzGlyph.Refresh, onClick = onRetry)
            }

            ClaimPhase.Invalid -> {
                Text("Ce lien n'est plus valide", style = Sillon.type.title, color = colors.tx)
                Text(
                    "Il a été coupé ou il a expiré. Demandez-en un nouveau à la personne qui vous l'a envoyé.",
                    style = Sillon.type.body,
                    color = colors.dim,
                    textAlign = TextAlign.Center,
                )
                // The CAUSE the core named, never a guess: a link that failed
                // because the phone has no server yet reads differently from one
                // that expired, and both are true sentences.
                if (!cause.isNullOrBlank()) {
                    Text(cause, style = Sillon.type.bodySmall, color = colors.dim, textAlign = TextAlign.Center)
                }
                NzButton(
                    text = "Ouvrir l'app",
                    kind = NzButtonKind.Secondary,
                    glyph = NzGlyph.Home,
                    onClick = onHome,
                )
            }

            ClaimPhase.Connected -> {
                Text(
                    "Vous pilotez le lecteur de ${owner ?: "…"}",
                    style = Sillon.type.title,
                    color = colors.tx,
                    textAlign = TextAlign.Center,
                )
                if (!level.isNullOrBlank()) {
                    Text(level, style = Sillon.type.body, color = colors.dim)
                }
            }
        }
    }
}
