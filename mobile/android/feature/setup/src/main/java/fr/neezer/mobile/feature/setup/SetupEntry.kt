package fr.neezer.mobile.feature.setup

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Serveur — lot MB4 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB4 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun SetupEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Serveur",
        body = "L'adresse du serveur, le port, la vérification du certificat et la connexion arrivent avec le lot MB4.",
        modifier = modifier,
    )
}

/**
 * Connexion — lot MB2/MB4 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB2/MB4 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun LoginEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Connexion",
        body = "Le compte, la session mémorisée et la reconnexion arrivent avec les lots MB2 et MB4.",
        modifier = modifier,
    )
}
