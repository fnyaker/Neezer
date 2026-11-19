package fr.neezer.mobile.feature.party

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
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
    val service = remember { PartyCore.service() } ?: return
    val share = remember(context) { AndroidPartyShare(context) }
    val bound = model ?: remember(service) { PartyModel(service, scope, share) }
    val state by bound.state.collectAsStateWithLifecycle()
    var hidden by remember(service) { mutableStateOf(false) }

    // One probe when the door is first drawn: a server that predates the party
    // hides it instead of failing behind it (rule 7).
    LaunchedEffect(bound) {
        bound.probe()
        hidden = bound.state.value.unsupported
    }
    if (hidden) return

    NzIconButton(
        glyph = NzGlyph.Party,
        contentDescription = if (state.hosting) "Soirée d'écoute en cours" else PartyCopy.DOOR,
        onClick = { bound.open() },
        modifier = modifier,
        tint = if (state.hosting) Sillon.colors.acMark else null,
    )
    PartySheet(model = bound, current = current)
}
