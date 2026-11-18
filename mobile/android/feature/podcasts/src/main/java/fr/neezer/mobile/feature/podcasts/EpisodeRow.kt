package fr.neezer.mobile.feature.podcasts

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.ChaptersData
import uniffi.nz_ffi.EpisodeData
import uniffi.nz_ffi.EpisodeMarkerData

/**
 * One episode of the show page (`Show.svelte`'s `<li>` rows), with everything
 * the SPA draws: the play button, the title and its date, the description, the
 * length or the resume line, the archive and provider flags, the chapters chip
 * and the transcript chip, the resume bar, and this person's markers.
 *
 * A row is a FUNCTION of the episode it is handed: the screen windows 12 of
 * them, and a row that is not loaded yet is a skeleton, never a blank line.
 */
@Composable
fun EpisodeRow(
    episode: EpisodeData,
    index: Int,
    isCurrent: Boolean,
    playing: Boolean,
    positionS: Double,
    chapters: ChaptersData?,
    chaptersBusy: Boolean,
    chaptersFailure: String?,
    onPlay: () -> Unit,
    onOpen: () -> Unit,
    onToggleChapters: () -> Unit,
    onSeek: (Double) -> Unit,
    onRemoveMarker: (EpisodeMarkerData) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    var expanded by remember(episode.id) { mutableStateOf(false) }
    var chaptersOpen by remember(episode.id) { mutableStateOf(false) }

    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(
                if (isCurrent) colors.s1 else colors.bg,
                RoundedCornerShape(NzRadius.card),
            )
            .padding(vertical = NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Row(verticalAlignment = Alignment.Top) {
            Box(
                modifier = Modifier
                    .size(NzMinTouchTarget)
                    .background(colors.s2, CircleShape)
                    .clickable(role = Role.Button, onClick = onPlay)
                    .semantics {
                        contentDescription = if (isCurrent && playing) {
                            "Mettre en pause ${episode.title}"
                        } else {
                            "Lire ${episode.title}"
                        }
                    },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    glyph = if (isCurrent && playing) NzGlyph.Pause else NzGlyph.Play,
                    size = 18.dp,
                    tint = colors.tx,
                )
            }
            Column(
                modifier = Modifier
                    .weight(1f)
                    .padding(start = NzSpacing.m),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = episode.title,
                        style = Sillon.type.label,
                        color = if (isCurrent) colors.acText else colors.tx,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Text(
                        text = episode.publishedLabel,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                        maxLines = 1,
                        modifier = Modifier.padding(start = NzSpacing.s),
                    )
                }
                if (episode.description.isNotBlank()) {
                    Text(
                        text = episode.description,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                        maxLines = if (expanded) Int.MAX_VALUE else 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable(role = Role.Button) { expanded = !expanded },
                    )
                }
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                ) {
                    Text(
                        text = PodcastsCopy.episodeLine(episode),
                        style = Sillon.type.bodySmall,
                        color = if (episode.position?.finished == true) colors.btn else colors.dim,
                    )
                    if (episode.archived) {
                        Icon(glyph = NzGlyph.Downloaded, size = 13.dp, tint = colors.dim)
                    }
                    if (episode.unavailable) {
                        Icon(glyph = NzGlyph.Alert, size = 13.dp, tint = colors.acText)
                        Text(
                            text = PodcastsCopy.UNAVAILABLE,
                            style = Sillon.type.bodySmall,
                            color = colors.acText,
                        )
                    }
                    if (episode.chapterCount > 0u) {
                        ChaptersChip(
                            label = episode.chaptersLabel,
                            open = chaptersOpen,
                            onClick = {
                                chaptersOpen = !chaptersOpen
                                if (chaptersOpen) onToggleChapters()
                            },
                        )
                    }
                    episode.transcript?.let { info ->
                        val label = PodcastsCopy.transcriptChip(info)
                        if (label.isNotEmpty()) {
                            Box(
                                modifier = Modifier
                                    .heightIn(min = NzMinTouchTarget)
                                    .clickable(role = Role.Button, onClick = onOpen)
                                    .semantics {
                                        contentDescription =
                                            if (info.ready) "Ouvrir la transcription de ${episode.title}"
                                            else "Transcription de ${episode.title} : $label"
                                    },
                                contentAlignment = Alignment.Center,
                            ) {
                                Row(
                                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Icon(glyph = NzGlyph.Transcript, size = 14.dp, tint = colors.dim)
                                    Text(
                                        text = label,
                                        style = Sillon.type.bodySmall,
                                        color = colors.dim,
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
        val resume = episode.position
        if (resume != null && !resume.finished) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(3.dp)
                    .background(colors.s2, RoundedCornerShape(2.dp)),
            ) {
                Box(
                    Modifier
                        .fillMaxWidth(resume.fraction.toFloat().coerceIn(0f, 1f))
                        .height(3.dp)
                        .background(colors.btn, RoundedCornerShape(2.dp)),
                )
            }
        }
        if (chaptersOpen) {
            InlineChapters(
                chapters = chapters,
                busy = chaptersBusy,
                failure = chaptersFailure,
                currentSeconds = positionS,
                isCurrent = isCurrent,
                onSeek = onSeek,
                onOpenAll = onOpen,
            )
        }
        if (episode.markers.isNotEmpty()) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                episode.markers.forEach { marker ->
                    MarkerChip(
                        marker = marker,
                        onJump = { onSeek(marker.seconds.toDouble()) },
                        onRemove = { onRemoveMarker(marker) },
                    )
                }
            }
        }
    }
}

/** The chapters chip: a glyph, the count, and a chevron that turns. */
@Composable
private fun ChaptersChip(label: String, open: Boolean, onClick: () -> Unit) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .heightIn(min = NzMinTouchTarget)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = if (open) "Masquer les chapitres" else "Afficher les chapitres" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Icon(glyph = NzGlyph.Chapters, size = 14.dp, tint = colors.dim)
        Text(text = label, style = Sillon.type.bodySmall, color = colors.dim)
        Icon(
            glyph = if (open) NzGlyph.ChevronUp else NzGlyph.ChevronDown,
            size = 14.dp,
            tint = colors.dim,
        )
    }
}

/** A marker chip: its moment, its label, and its × (`Show.svelte`'s `.mark`). */
@Composable
private fun MarkerChip(
    marker: EpisodeMarkerData,
    onJump: () -> Unit,
    onRemove: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .background(colors.s2, RoundedCornerShape(NzRadius.chip))
            .padding(start = NzSpacing.s),
    ) {
        Row(
            modifier = Modifier
                .heightIn(min = NzMinTouchTarget)
                .clickable(role = Role.Button, onClick = onJump)
                .semantics { contentDescription = "Lire à ${marker.timeLabel}" },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Icon(glyph = NzGlyph.BookmarkFilled, size = 12.dp, tint = colors.btn)
            Text(text = marker.timeLabel, style = Sillon.type.labelMono, color = colors.tx)
            if (marker.label.isNotBlank()) {
                Text(
                    text = marker.label,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        NzIconButton(
            glyph = NzGlyph.Close,
            contentDescription = "Supprimer le marqueur",
            onClick = onRemove,
            size = 12.dp,
        )
    }
}
