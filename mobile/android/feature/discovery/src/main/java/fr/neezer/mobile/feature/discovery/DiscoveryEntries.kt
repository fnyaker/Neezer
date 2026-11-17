package fr.neezer.mobile.feature.discovery

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Nouveautés — lot MB14 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB14 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun ReleasesEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Nouveautés",
        body = "Les deux étagères de sorties et leurs réglages arrivent avec le lot MB14.",
        modifier = modifier,
    )
}

/**
 * Sorties — lot MB14 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB14 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun ReleaseShelfEntry(kind: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Sorties",
        body = "Une étagère de sorties (artistes suivis ou découvertes) arrive avec le lot MB14.",
        modifier = modifier,
    )
}

/**
 * Récap — lot MB14 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB14 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun RecapEntry(year: Int?, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Récap",
        body = "Les années, les stories et le partage du récap arrivent avec le lot MB14.",
        modifier = modifier,
    )
}
