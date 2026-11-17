package fr.neezer.mobile.feature.podcasts

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Podcasts — lot MB13 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB13 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun PodcastsEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Podcasts",
        body = "Les abonnements, la recherche d'un podcast et les épisodes arrivent avec le lot MB13.",
        modifier = modifier,
    )
}

/**
 * Podcast — lot MB13 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB13 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun ShowEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Podcast",
        body = "Les épisodes, la progression, les chapitres et la transcription arrivent avec le lot MB13.",
        modifier = modifier,
    )
}
