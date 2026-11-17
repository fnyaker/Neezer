package fr.neezer.mobile.feature.home

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Accueil — lot MB5 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB5 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun HomeEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Accueil",
        body = "Le hero, la reprise, les humeurs, les nouveautés, le récap et le Flow arrivent avec le lot MB5.",
        modifier = modifier,
    )
}
