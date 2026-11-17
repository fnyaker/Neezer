package fr.neezer.mobile.feature.trackactions

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import uniffi.nz_ffi.TrackRow

/**
 * The ONE track-actions spec (lot MB10), shared by every list.
 *
 * Every surface that lists a title opens the SAME menu — a playlist row, the
 * queue, a search hit, a podcast's episode list — so the actions cannot drift
 * apart. MB0 declares the entry points; MB10 fills the sheet.
 */
object TrackActions {
    /**
     * Opens the context sheet for [row]. Called by a long press, a "..." button,
     * a right click on a Chromebook, or Shift+F10 from a hardware keyboard.
     */
    @Suppress("UNUSED_PARAMETER")
    fun open(row: TrackRow?, nav: NzNavigator) {
        // MB10: the sheet's state lives in the core; nothing to show yet.
    }
}

/**
 * The sheet's host, placed once by the shell: one menu instance for the whole
 * app, so two lists can never show two menus.
 */
@Composable
fun TrackActionsHost(modifier: Modifier = Modifier) {
    // MB10 draws the sheet here when the core says one is open.
    @Suppress("UNUSED_EXPRESSION")
    modifier
}
