package fr.neezer.mobile.feature.admin

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * Réglages' administrator block (lot MB17): users and invites, the server's
 * archive and rules, the jobs with their progress and the CAUSE of a failure,
 * the backfills, the transcription settings.
 *
 * Called from :feature:settings, behind `isAdmin`.
 */
@Composable
fun AdminSections(modifier: Modifier = Modifier) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Administration du serveur", style = MaterialTheme.typography.titleMedium)
        Text(
            text = "Utilisateurs, invitations, archive, règles, travaux et transcriptions arrivent avec le lot MB17.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
