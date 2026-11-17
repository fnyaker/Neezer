package fr.neezer.mobile.feature.nowplaying

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator

/**
 * The mini player above the bottom bar (lots MB3b + MB11).
 *
 * MB0 draws nothing: a bar with no track in it would be a lie, and the shell
 * must not grow a 64 dp strip for a player that does not exist yet. MB3b gives
 * it the snapshot and MB11 its controls.
 */
@Composable
@Suppress("UNUSED_PARAMETER")
fun MiniPlayer(modifier: Modifier = Modifier, onClick: () -> Unit = {}) {
    // MB11 draws the bar here.
}

/**
 * The full-screen player's host: a sheet on a phone, a reading column on a
 * tablet (docs/agents/mobile.md, "Navigation, screens and the UI").
 *
 * It also hosts the worlds' surface, which is where the animation is seen
 * (MB22): `:feature:nowplaying` declares `:feature:worlds` for exactly that.
 */
@Composable
@Suppress("UNUSED_PARAMETER")
fun NowPlayingHost(nav: NzNavigator, modifier: Modifier = Modifier) {
    // MB11 shows the sheet here, with `WorldSurface` inside it.
}
