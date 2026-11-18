package fr.neezer.mobile.feature.lan

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Sur vos appareils — lot LN14 (docs/agents/lan-nodes.md).
 *
 * The sets the connected peers expose, multi-selectable, with « Télécharger ».
 * LN0 draws the route's name; LN14 replaces this body and keeps the signature, so
 * `NavGraph.kt` never changes.
 */
@Composable
fun PeerSetsEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Sur vos appareils",
        body = "Les playlists que vos appareils ont déjà, à télécharger d'un geste, arrivent avec le lot LN14.",
        modifier = modifier,
    )
}
