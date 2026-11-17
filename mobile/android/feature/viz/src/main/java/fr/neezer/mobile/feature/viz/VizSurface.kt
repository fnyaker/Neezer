package fr.neezer.mobile.feature.viz

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

/**
 * The visualiser's drawing surface (lot MB21): bars or the oscilloscope, fed by
 * `VizApi::frame()`.
 *
 * The surface exists from MB0 on, so the screens that host it (the Viz route,
 * the full-screen player) compile and lay out today. MB0 paints the cause
 * instead of a canvas: a visualiser that cannot draw says why (rule 3).
 */
@Composable
fun VizSurface(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = "Le visualiseur arrive avec le lot MB21",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
