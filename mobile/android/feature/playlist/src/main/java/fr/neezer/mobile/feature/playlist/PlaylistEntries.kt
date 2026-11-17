package fr.neezer.mobile.feature.playlist

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Playlist — lot MB9 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB9 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun PlaylistEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Playlist",
        body = "Les 4 000 lignes, l'édition, les membres, le partage et les recommandations arrivent avec le lot MB9.",
        modifier = modifier,
    )
}

/**
 * Playlist partagée — lot MB9 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB9 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun SharedPlaylistEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Playlist partagée",
        body = "Le lien public en lecture seule arrive avec le lot MB9.",
        modifier = modifier,
    )
}

/**
 * Importer — lot MB9 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB9 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun ImportEntry(url: String?, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Importer",
        body = "L'import d'un lien partagé arrive avec le lot MB9.",
        modifier = modifier,
    )
}
