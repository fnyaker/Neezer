package fr.neezer.mobile.feature.party

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Soirée d'écoute — lot MB20 (docs/agents/mobile.md, feature inventory).
 *
 * The route a party LINK opens (`/party/<id>`, `NavGraph.kt`'s `Routes.Party`):
 * the guest's screen. MB0 drew a placeholder here and this lot keeps the
 * signature, so `NavGraph.kt` never changes.
 *
 * A device with no native core, or a server that predates the party, gets the
 * placeholder — with the reason — instead of a screen that cannot work.
 */
@Composable
fun PartyEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    val service = remember { PartyCore.service() }
    if (service == null) {
        NzPlaceholder(
            title = PartyCopy.TITLE,
            body = PartyCopy.UNSUPPORTED,
            modifier = modifier,
        )
        return
    }
    val model = remember(id, service) { PartyGuestModel(service, scope, id) }
    PartyGuestScreen(model = model, nav = nav, modifier = modifier)
}
