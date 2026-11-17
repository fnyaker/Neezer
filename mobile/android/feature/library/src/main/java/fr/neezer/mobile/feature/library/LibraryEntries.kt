package fr.neezer.mobile.feature.library

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Bibliothèque — lot MB8 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB8 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun LibraryEntry(tab: String?, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Bibliothèque",
        body = "Playlists et dossiers, favoris, artistes, albums, envois, dislikes, indisponibles et titres locaux arrivent avec le lot MB8.",
        modifier = modifier,
    )
}

/**
 * Historique — lot MB8 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB8 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun HistoryEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Historique",
        body = "L'historique d'écoute arrive avec le lot MB8.",
        modifier = modifier,
    )
}
