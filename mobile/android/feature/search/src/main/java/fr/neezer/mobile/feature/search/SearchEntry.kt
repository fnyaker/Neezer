package fr.neezer.mobile.feature.search

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Rechercher — lot MB6 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB6 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun SearchEntry(q: String?, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Rechercher",
        body = "Titres, albums, artistes, playlists et podcasts, le meilleur résultat et la recherche qui devient une radio arrivent avec le lot MB6.",
        modifier = modifier,
    )
}
