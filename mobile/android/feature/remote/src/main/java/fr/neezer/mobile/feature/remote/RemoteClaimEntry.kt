package fr.neezer.mobile.feature.remote

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Télécommande — lot MB19 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB19 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun RemoteClaimEntry(token: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Télécommande",
        body = "Réclamer un lien de télécommande arrive avec le lot MB19.",
        modifier = modifier,
    )
}
