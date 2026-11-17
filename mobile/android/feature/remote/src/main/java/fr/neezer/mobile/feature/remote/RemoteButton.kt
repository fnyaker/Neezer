package fr.neezer.mobile.feature.remote

import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

/**
 * The remote-control entry the player and Réglages show (lot MB19): take the
 * link, or say who is controlling this player.
 *
 * MB0 draws it inert; MB19 fills the sheet and the banner.
 */
@Composable
fun RemoteButton(modifier: Modifier = Modifier) {
    OutlinedButton(onClick = { /* MB19 */ }, modifier = modifier) {
        Text("Télécommande")
    }
}
