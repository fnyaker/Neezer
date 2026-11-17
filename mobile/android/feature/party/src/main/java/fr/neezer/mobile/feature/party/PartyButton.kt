package fr.neezer.mobile.feature.party

import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

/**
 * The listen-party entry the player shows (lot MB20): open a party from the
 * queue, or join one.
 *
 * MB0 draws it inert; MB20 fills the sheet.
 */
@Composable
fun PartyButton(modifier: Modifier = Modifier) {
    OutlinedButton(onClick = { /* MB20 */ }, modifier = modifier) {
        Text("Soirée d'écoute")
    }
}
