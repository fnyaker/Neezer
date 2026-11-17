package fr.neezer.mobile.feature.audiofx

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * The audio-effects block Réglages shows (lot MB12): equalizer, bass lift,
 * mono/balance, output name and verdict, volume warning.
 *
 * One composable, called from :feature:settings, so the effects have a single
 * definition instead of a copy in each settings section. MB0 draws the list of
 * what will be there; MB12 fills the faders.
 */
@Composable
fun AudioFxSections(modifier: Modifier = Modifier) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Effets audio", style = MaterialTheme.typography.titleMedium)
        for (item in listOf("Égaliseur", "Basses", "Mono et balance", "Sortie", "Avertissement de volume")) {
            Text(
                text = "$item — arrive avec le lot MB12",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
