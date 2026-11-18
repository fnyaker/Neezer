package fr.neezer.mobile.feature.nowplaying

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/**
 * What the full-screen player is showing, for the WHOLE app.
 *
 * The player is not a route in the inventory: the SPA opens it as a sheet over
 * whatever screen the person was on, and it survives a tap through to an album
 * (the sheet closes, the album opens). One object holds it, so the mini player,
 * the sheet and `NzNavigator.openPlayer()` all speak about the same thing without
 * a `NavController` in a feature module.
 *
 * [Pane] is the SPA's rule that **one sheet is open at a time**: the queue, the
 * words and « À propos » live over the same player, and two stacked scrollers
 * would fight for the gesture and for the close.
 */
object NowPlayingState {
    /** Which sheet is up over the player, if any. */
    enum class Pane { None, Queue, Lyrics, About, Card }

    /** The full-screen player itself (the sheet on a phone, the column set wide). */
    var open: Boolean by mutableStateOf(false)
        private set

    /** The sheet over it. */
    var pane: Pane by mutableStateOf(Pane.None)
        private set

    /** The album/artist a tap must leave the player for (it closes on the way). */
    fun show() {
        open = true
    }

    fun hide() {
        open = false
        pane = Pane.None
    }

    /** Opens a pane, closing any other (the SPA's `openQueue`/`openLyrics`). */
    fun showPane(target: Pane) {
        open = true
        pane = if (pane == target) Pane.None else target
    }

    fun closePane() {
        pane = Pane.None
    }

    /** `NzNavigator.openQueue()` — the queue editor, the player behind it. */
    fun openQueue() {
        open = true
        pane = Pane.Queue
    }
}
