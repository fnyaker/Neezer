package fr.neezer.mobile.feature.party

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Soirée d'écoute — lot MB20 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB20 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun PartyEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Soirée d'écoute",
        body = "Rejoindre une soirée et écouter ensemble arrive avec le lot MB20.",
        modifier = modifier,
    )
}
