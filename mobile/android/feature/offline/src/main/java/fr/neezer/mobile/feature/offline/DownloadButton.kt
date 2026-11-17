package fr.neezer.mobile.feature.offline

import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

/**
 * The download action every list and every entity page shows (lot MB15):
 * "Télécharger" for a whole playlist, album or show.
 *
 * Its state is the queue's own (a running download says so and shows its
 * progress, rule 2); MB0 draws the inert button so a caller's signature is
 * already final.
 */
@Composable
fun DownloadButton(
    context: String,
    modifier: Modifier = Modifier,
    label: String = "Télécharger",
) {
    OutlinedButton(onClick = { /* MB15 */ }, modifier = modifier) {
        Text(label)
    }
}
