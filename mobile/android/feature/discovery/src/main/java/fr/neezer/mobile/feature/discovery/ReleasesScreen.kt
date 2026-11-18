package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzDuration
import uniffi.nz_ffi.ReleaseGroupData
import uniffi.nz_ffi.ReleaseRowData
import uniffi.nz_ffi.ReleaseShelfData
import uniffi.nz_ffi.TrackRow

/** What Nouveautés' buttons do (one record, so the screen stays a function). */
data class ReleasesActions(
    val playAll: () -> Unit = {},
    val playGroup: (ReleaseGroupData) -> Unit = {},
    val playGroupAt: (ReleaseGroupData, Int) -> Unit = { _, _ -> },
    val toggleGroup: (String) -> Unit = {},
    val toggleAll: () -> Unit = {},
    val openShelf: (String) -> Unit = {},
    val retry: () -> Unit = {},
)

/**
 * Nouveautés (lot MB14), `routes/Releases.svelte` ported.
 *
 * It draws the feed's own hierarchy the page's way: the lead, the bar ("Tout
 * lire · N" and the unseen pill), then either the two shelf cards or — when no
 * shelf holds a title yet — the per-release list already open, never a
 * collapsed heading over nothing. Every row is keyed, and the long part is a
 * `LazyColumn`: a 120-release feed is windowed, not composed whole.
 */
@Composable
fun ReleasesScreen(
    state: ReleasesModel.State,
    actions: ReleasesActions,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val type = Sillon.type
    // The app's root provides a BLACK `LocalContentColor` (docs/agents/mobile.md,
    // MB14's note): every Text without an explicit colour would be invisible on
    // Encre. One provider per screen fixes the whole tree.
    CompositionLocalProvider(LocalContentColor provides colors.tx) {
    when (val screen = state.screen) {
        is ScreenState.Loading -> SkeletonList(modifier)
        is ScreenState.Failed -> Box(modifier.padding(NzSpacing.l)) {
            LoadFailed(cause = screen.cause, onRetry = { screen.retry() })
        }

        is ScreenState.Ready -> {
            val data = screen.data
            if (data.empty) {
                Box(modifier.padding(NzSpacing.l)) {
                    EmptyState(
                        title = DiscoveryCopy.NOTHING_NEW_TITLE,
                        body = DiscoveryCopy.NOTHING_NEW_BODY,
                        glyph = NzGlyph.Sparkles,
                        actionLabel = "Explorer",
                        onAction = { actions.openShelf("discovery") },
                    )
                }
            } else {
            LazyColumn(
                modifier = modifier,
                contentPadding = androidx.compose.foundation.layout.PaddingValues(
                    start = NzSpacing.pageX,
                    end = NzSpacing.pageX,
                    top = NzSpacing.m,
                    bottom = NzSpacing.xxl,
                ),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                item(key = "lead") {
                    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                        Text(
                            DiscoveryCopy.RELEASES_LEAD,
                            style = type.body,
                            color = colors.dim,
                            modifier = Modifier.semantics { heading() },
                        )
                        if (state.stale) {
                            Text(
                                DiscoveryCopy.OFFLINE_KEPT,
                                style = type.label,
                                color = colors.dim,
                            )
                        }
                    }
                }
                item(key = "bar") {
                    // `flex-wrap` in the SPA: at 200 % the pill goes to a line of
                    // its own instead of being clipped (this lot's first capture).
                    androidx.compose.foundation.layout.FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                        itemVerticalAlignment = Alignment.CenterVertically,
                    ) {
                        if (data.trackCount > 0u) {
                            NzButton(
                                text = "Tout lire · ${data.trackCount}",
                                glyph = NzGlyph.Play,
                                onClick = actions.playAll,
                            )
                        }
                        if (data.unseen > 0u) {
                            FreshPill(data.freshLabel)
                        }
                    }
                }
                items(state.shelves, key = { "shelf-${it.key}" }) { shelf ->
                    ShelfCard(shelf, onOpen = { actions.openShelf(shelf.key) })
                }
                if (data.releaseTotal > 0u) {
                    item(key = "all-head") {
                        AllReleasesHead(
                            total = data.releaseTotal.toInt(),
                            open = state.showAll,
                            onToggle = actions.toggleAll,
                        )
                    }
                }
                if (state.showAll) {
                    item(key = "all-hint") {
                        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                            Text(DiscoveryCopy.ALL_RELEASES_HINT, style = type.bodySmall, color = colors.dim)
                            if (data.pendingLabel.isNotEmpty()) {
                                Text(
                                    data.pendingLabel,
                                    style = type.bodySmall,
                                    color = colors.dim,
                                )
                            }
                        }
                    }
                    items(state.groups, key = { "group-${it.release.key}" }) { group ->
                        ReleaseGroupCard(
                            group = group,
                            open = state.opened.contains(group.release.key),
                            rows = state.groupRows[group.release.key].orEmpty(),
                            onToggle = { actions.toggleGroup(group.release.key) },
                            onPlay = { actions.playGroup(group) },
                            onRow = { position -> actions.playGroupAt(group, position) },
                        )
                    }
                    if (state.others.isNotEmpty()) {
                        item(key = "others-head") {
                            Text(
                                if (state.groups.isEmpty()) "Sorties" else "Autres sorties",
                                style = type.title,
                                modifier = Modifier.semantics { heading() },
                            )
                        }
                        items(state.others, key = { "other-${it.key}" }) { release ->
                            OtherReleaseCard(release)
                        }
                    }
                }
            }
            }
        }
    }
    }
}

/** "2 nouvelles sorties" — the page's own pill, not a badge on a sleeve. */
@Composable
private fun FreshPill(label: String) {
    val colors = Sillon.colors
    Box(
        modifier = Modifier
            .clip(RoundedCornerShape(NzRadius.chip))
            .background(colors.ac)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.xs)
            .semantics { contentDescription = label },
    ) {
        Text(
            label,
            style = Sillon.type.label,
            color = colors.onac,
            maxLines = 2,
        )
    }
}

/** One shelf card: a mosaic, its count, six preview rows and "Tout voir". */
@Composable
private fun ShelfCard(shelf: ReleaseShelfData, onOpen: () -> Unit) {
    val colors = Sillon.colors
    val type = Sillon.type
    val count = "${discTracks(shelf.trackCount.toInt())} · ${discReleases(shelf.releaseCount.toInt())}"
    NzCard(onClick = onOpen, modifier = Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                Mosaic(covers = shelf.mosaic, seed = shelf.key, size = 84.dp)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                    Text(
                        shelf.title,
                        style = type.title,
                        modifier = Modifier.semantics { heading() },
                    )
                    Text(shelf.subtitle, style = type.bodySmall, color = colors.dim, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(count, style = type.mono, color = colors.dim)
                }
            }
            shelf.preview.forEach { row -> PreviewRow(row) }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("Tout voir", style = type.label, color = colors.acText)
                Icon(NzGlyph.ChevronRight, size = 15.dp, tint = colors.acText)
            }
        }
    }
}

/** One preview row: a 40 px sleeve, the two lines, the duration. Not a tracklist. */
@Composable
private fun PreviewRow(row: TrackRow) {
    val colors = Sillon.colors
    val type = Sillon.type
    Row(
        modifier = Modifier.fillMaxWidth().heightIn(min = 40.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        CoverImage(url = row.cover, sizeDp = 32.dp, seed = row.album.ifEmpty { row.key })
        Column(Modifier.weight(1f)) {
            Text(row.title, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(row.artists, style = type.label, color = colors.dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Text(nzDuration(row.durationS.toInt()), style = type.mono, color = colors.dim)
    }
}

/** "Toutes les sorties 120" and its Afficher / Replier. */
@Composable
private fun AllReleasesHead(total: Int, open: Boolean, onToggle: () -> Unit) {
    val type = Sillon.type
    val colors = Sillon.colors
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(
            DiscoveryCopy.ALL_RELEASES,
            style = type.title,
            modifier = Modifier.semantics { heading() },
        )
        Text("$total", style = type.mono, color = colors.dim)
        Spacer(Modifier.weight(1f))
        NzButton(
            text = if (open) "Replier" else "Afficher",
            kind = NzButtonKind.Ghost,
            glyph = if (open) NzGlyph.ChevronUp else NzGlyph.ChevronDown,
            onClick = onToggle,
        )
    }
}

/** One release with its titles: the sleeve, its label, « Lire », the disclosure. */
@Composable
private fun ReleaseGroupCard(
    group: ReleaseGroupData,
    open: Boolean,
    rows: List<TrackRow>,
    onToggle: () -> Unit,
    onPlay: () -> Unit,
    onRow: (Int) -> Unit,
) {
    val colors = Sillon.colors
    val type = Sillon.type
    NzCard(modifier = Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            CoverImage(url = group.release.cover, sizeDp = 72.dp, seed = group.release.key)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                Text(group.label, style = type.labelMono, color = colors.dim, maxLines = 2)
                Text(
                    group.release.title,
                    style = type.title,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.semantics { heading() },
                )
                Text(group.artist, style = type.bodySmall, color = colors.dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        Row(
            modifier = Modifier.padding(top = NzSpacing.s),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            NzButton(text = "Lire", glyph = NzGlyph.Play, onClick = onPlay)
            NzButton(
                text = if (open) "Masquer" else "Les ${group.trackCount} titre${if (group.trackCount > 1u) "s" else ""}",
                kind = NzButtonKind.Ghost,
                glyph = if (open) NzGlyph.ChevronUp else NzGlyph.ChevronDown,
                onClick = onToggle,
            )
        }
        if (open) {
            Column(Modifier.padding(top = NzSpacing.s)) {
                rows.forEach { row ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = NzMinTouchTarget)
                            .clickable(role = Role.Button) { onRow(row.position.toInt()) },
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    ) {
                        Text(
                            "${row.position + 1u}",
                            style = type.mono,
                            color = colors.dim,
                            modifier = Modifier.width(20.dp),
                        )
                        Column(Modifier.weight(1f)) {
                            Text(row.title, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(row.artists, style = type.label, color = colors.dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                        Text(nzDuration(row.durationS.toInt()), style = type.mono, color = colors.dim)
                    }
                }
            }
        }
        }
    }
}

/** "Autres sorties": a release the server has not read yet, as one card. */
@Composable
private fun OtherReleaseCard(release: ReleaseRowData) {
    val colors = Sillon.colors
    val type = Sillon.type
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = NzSpacing.xs),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CoverImage(url = release.cover, sizeDp = 56.dp, seed = release.key)
        Column(Modifier.weight(1f)) {
            Text(release.title, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(release.subtitle, style = type.label, color = colors.dim, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        if (release.new) {
            Text("Nouveau", style = type.label, color = colors.acText)
        }
    }
}

/**
 * Two sleeves side by side, one in each corner when there are three or four —
 * `ReleaseCard.svelte#rc-mosaic`, grooves alone reading as "no artwork".
 */
@Composable
internal fun Mosaic(covers: List<String>, seed: String, size: androidx.compose.ui.unit.Dp) {
    val shown = covers.take(4).ifEmpty { listOf("") }
    val perRow = if (shown.size == 1) 1 else 2
    Column(
        modifier = Modifier.size(size).clip(RoundedCornerShape(14.dp)),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        shown.chunked(perRow).forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(2.dp)) {
                row.forEach { cover ->
                    CoverImage(
                        url = cover.ifEmpty { null },
                        sizeDp = (size - 2.dp) / perRow,
                        seed = cover.ifEmpty { seed },
                    )
                }
            }
        }
    }
}

/** The skeleton the page shows while its one request is in flight. */
@Composable
private fun SkeletonList(modifier: Modifier) {
    Column(
        modifier = modifier.padding(NzSpacing.pageX),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        repeat(3) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(112.dp)
                    .clip(RoundedCornerShape(NzRadius.card))
                    .background(Sillon.colors.s1),
            )
        }
    }
}
