package fr.neezer.mobile.feature.viz

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Visualiseur — lot MB21 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB21 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun VizEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Visualiseur",
        body = "Les barres, l'oscilloscope et leurs réglages arrivent avec le lot MB21.",
        modifier = modifier,
    )
}
