package fr.neezer.mobile.feature.genres

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Genres — lot MB18 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB18 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun GenresEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Genres",
        body = "Le studio des genres, l'arbre des styles et l'analyse locale arrivent avec le lot MB18.",
        modifier = modifier,
    )
}
