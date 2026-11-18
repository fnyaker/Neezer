package fr.neezer.mobile.feature.lan

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Appareils à proximité — lot LN14 (docs/agents/lan-nodes.md).
 *
 * LN0 draws the route's name and what will live here; LN14 replaces this body and
 * keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun LanEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Appareils à proximité",
        body = "Vos appareils et vos amis de confiance sur ce Wi-Fi arrivent avec le lot LN14.",
        modifier = modifier,
    )
}
