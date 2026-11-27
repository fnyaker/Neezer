package fr.neezer.mobile.feature.party

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.PartyStateData

/**
 * The player's door to the listen party (lot MB20): one glyph in the transport
 * row, and the sheet behind it.
 *
 * It is a GLYPH and not a labelled button because of the row it lives in: five
 * 48 dp targets share a 360 dp phone with two other doors, and MB0's stub (an
 * `OutlinedButton` with a label) does not fit there (rule 5, MB11's measurement).
 * The label is the icon's `contentDescription`, which is what TalkBack reads.
 *
 * The door HIDES itself in three cases, and says nothing instead of failing
 * behind a tap: the native core could not be bound (no `.so`), the server has no
 * party routes (rule 7, one `OPTIONS`), or the person is not signed in — the
 * party's host side needs a session, and a guest arrives by a link, never here.
 * It tints its glyph while this device hosts, so the room is visible from the
 * player without opening the sheet.
 *
 * The sheet opens in a transparent, undecorated `Dialog` (the MB19 pattern): a
 * sheet that only painted inside a 48 dp button would be invisible. The door
 * reads `state.unsupported` instead of a copy taken before the probe answered.
 */
@Composable
fun PartyButton(
    modifier: Modifier = Modifier,
    /** A screen that already has a model (the full-screen player) hands it in. */
    model: PartyModel? = null,
    /** What the intro card shows under the arguments: the player's own row. */
    current: PartyStateData? = null,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val bound = model ?: remember {
        PartyCore.service()?.let { service -> PartyModel(service, scope, AndroidPartyShare(context)) }
    } ?: return
    val state by bound.state.collectAsStateWithLifecycle()

    // One probe when the door is first drawn, and the STATE it answers decides:
    // a server that really said "no" hides the door (rule 7), a doubt never does.
    LaunchedEffect(bound) { bound.probe() }
    if (state.unsupported) return

    NzIconButton(
        glyph = NzGlyph.Party,
        contentDescription = if (state.hosting) "Soirée d'écoute en cours" else PartyCopy.DOOR,
        onClick = { bound.open() },
        modifier = modifier,
        tint = if (state.hosting) Sillon.colors.acMark else null,
    )
    if (state.open) {
        Dialog(
            onDismissRequest = { bound.close() },
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            Box(Modifier.fillMaxSize()) { PartySheet(model = bound, current = current) }
        }
    }
}