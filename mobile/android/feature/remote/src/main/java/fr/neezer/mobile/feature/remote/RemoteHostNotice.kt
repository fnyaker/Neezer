package fr.neezer.mobile.feature.remote

import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import fr.neezer.mobile.designsystem.NzSpacing

/**
 * « Quelqu'un pilote ce lecteur » — `components/RemoteHostChip.svelte`, porté.
 *
 * Monté dans la coquille (au-dessus du mini-lecteur sur téléphone, en haut de la
 * colonne de contenu sur large), pour être visible depuis N'IMPORTE QUEL écran :
 * le propriétaire ne doit jamais chercher comment reprendre son lecteur (audit,
 * bloqueur 6). Il ne dessine rien tant qu'aucun contrôleur n'est connecté.
 *
 * Le statut vient de Rust sur CHANGEMENT (`RemoteListener`), jamais d'un
 * minuteur ; `resumeHost()` interroge le serveur une fois au lancement, pour un
 * appareil prêté lors d'une session précédente.
 *
 * `:app` l'appelle : `:feature:settings` et `:feature:nowplaying` ne peuvent pas
 * monter une barre de coquille, et `HostChip` reste `internal` à ce module.
 */
@Composable
fun RemoteHostNotice(modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val model = remember(scope) { RemoteModel(NzRemoteService, scope) }
    val state by model.state.collectAsState()

    LaunchedEffect(model) { model.resumeHostAtStart() }

    val status = state.status ?: return
    if (status.controllers.isEmpty()) return
    HostChip(
        status = status,
        onCut = model::cutAllNow,
        modifier = modifier.padding(horizontal = NzSpacing.m, vertical = NzSpacing.xxs),
    )
}
