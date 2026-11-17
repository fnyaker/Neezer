package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * Explorer's ten parent styles, as a block other screens can place (lot MB14):
 * a style tile opens its genre station.
 *
 * MB0 draws the list; MB14 fills the tiles and the station behind them.
 */
@Composable
fun ExplorerStylesSection(modifier: Modifier = Modifier) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Explorer par style", style = MaterialTheme.typography.titleMedium)
        Text(
            text = "Les dix styles parents et leurs stations arrivent avec le lot MB14.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
