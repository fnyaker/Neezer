package fr.neezer.mobile.feature.entity

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Artiste — lot MB7 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB7 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun ArtistEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Artiste",
        body = "La discographie, les crédits et les personnes arrivent avec le lot MB7.",
        modifier = modifier,
    )
}

/**
 * Album — lot MB7 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB7 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun AlbumEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Album",
        body = "Les titres, les crédits et les personnes arrivent avec le lot MB7.",
        modifier = modifier,
    )
}

/**
 * Crédits — lot MB7 (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB7 replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun CreditsEntry(name: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Crédits",
        body = "La fiche d'une personne et ce qu'elle a fait arrivent avec le lot MB7.",
        modifier = modifier,
    )
}
