package fr.neezer.mobile.feature.worlds

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

/**
 * The worlds' GLES surface (lot MB22): an `ANativeWindow` handed to
 * `nz-viz-gl`, one context, frame-capped.
 *
 * Separate from [fr.neezer.mobile.feature.viz.VizSurface] because the two answer
 * different questions: this one is the 50 worlds, that one is the plain bars and
 * scope. MB0 paints the reason it draws nothing yet.
 */
@Composable
fun WorldSurface(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = "Les mondes arrivent avec le lot MB22",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
