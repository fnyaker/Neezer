package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
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
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzDuration
import uniffi.nz_ffi.TrackRow

/** What a shelf page's controls do. */
data class ShelfActions(
    val back: () -> Unit = {},
    val playAll: () -> Unit = {},
    val shuffle: () -> Unit = {},
    val search: (String) -> Unit = {},
    val sort: (String) -> Unit = {},
    val reverse: () -> Unit = {},
    val playRow: (TrackRow) -> Unit = {},
    val queueNext: (TrackRow) -> Unit = {},
    val more: (TrackRow) -> Unit = {},
    val retry: () -> Unit = {},
    val openReleases: () -> Unit = {},
)

/**
 * One release shelf (lot MB14), `routes/ReleaseShelf.svelte` ported.
 *
 * A read-only sibling of a playlist page: the banner (a mosaic, the facts, the
 * shelf's own description), « Lire » and « Mélanger », then the windowed titles
 * with their search and sort. On a phone the banner is a 96 dp sleeve beside
 * the title, and the toolbar puts the search on its own line — the page's own
 * two breakpoints.
 */
@Composable
fun ReleaseShelfScreen(
    state: ReleaseShelfModel.State,
    actions: ShelfActions,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val type = Sillon.type
    CompositionLocalProvider(LocalContentColor provides colors.tx) {
    when (val screen = state.screen) {
        is ScreenState.Loading -> Column(modifier.padding(NzSpacing.pageX)) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = 120.dp)
                    .clip(RoundedCornerShape(20.dp))
                    .background(colors.s1),
            )
        }

        is ScreenState.Failed -> Box(modifier.padding(NzSpacing.l)) {
            LoadFailed(
                cause = screen.cause,
                onRetry = { screen.retry() },
                title = DiscoveryCopy.SHELF_FAILED,
            )
        }

        is ScreenState.Ready -> {
            val page = screen.data
            LazyColumn(
                modifier = modifier,
                contentPadding = PaddingValues(bottom = NzSpacing.xxl),
            ) {
                item(key = "banner") {
                    Banner(page, back = actions.back, stale = state.stale)
                }
                item(key = "actions") {
                    Row(
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        NzButton(text = "Lire", glyph = NzGlyph.Play, onClick = actions.playAll, enabled = state.rowCount > 0)
                        NzButton(
                            text = "Mélanger",
                            glyph = NzGlyph.Shuffle,
                            kind = NzButtonKind.Ghost,
                            onClick = actions.shuffle,
                            enabled = state.rowCount > 0,
                        )
                    }
                }
                item(key = "toolbar") {
                    Toolbar(state, actions)
                }
                if (state.rows.isEmpty() && state.searching) {
                    item(key = "no-match") {
                        Text(
                            "Aucun titre ne correspond à « ${state.query} ».",
                            style = type.body,
                            color = colors.dim,
                            modifier = Modifier.padding(NzSpacing.pageX),
                        )
                    }
                }
                if (!state.searching && state.rowCount == 0) {
                    item(key = "empty") {
                        EmptyState(
                            title = "Rien par ici pour l'instant",
                            body = if (page.key == "artists") {
                                DiscoveryCopy.SHELF_EMPTY_ARTISTS
                            } else {
                                DiscoveryCopy.SHELF_EMPTY_DISCOVERY
                            },
                            glyph = NzGlyph.Sparkles,
                            actionLabel = "Voir les nouveautés",
                            onAction = actions.openReleases,
                        )
                    }
                }
                items(state.rows, key = { "row-${it.key}" }) { row ->
                    // The designsystem's `TrackRowView` is a FIXED 56 dp and its
                    // two lines overlap at the 200 % scale (MB6's note, whose
                    // search draws its own row for the same reason): this row is
                    // `heightIn`, and it keeps the swipe that queues the title.
                    ShelfRowView(
                        row = row,
                        onClick = { actions.playRow(row) },
                        onMore = { actions.more(row) },
                        onSwipeQueue = { actions.queueNext(row) },
                    )
                }
            }
        }
    }
    }
}

/** The banner: a mosaic, "Nouveautés · 12 titres · 3 sorties · 1:02:10", the title. */
@Composable
private fun Banner(page: ShelfPage, back: () -> Unit, stale: Boolean) {
    val colors = Sillon.colors
    val type = Sillon.type
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.s2)
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.pageY),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            NzIconButton(glyph = NzGlyph.ChevronLeft, contentDescription = "Retour aux nouveautés", onClick = back)
            Text(
                page.facts + if (page.totalDurationS > 0u) " · ${discDuration(page.totalDurationS.toLong())}" else "",
                style = type.label,
                color = colors.dim,
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m), verticalAlignment = Alignment.CenterVertically) {
            Mosaic(covers = page.mosaic, seed = page.key, size = 96.dp)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                Text(
                    page.title,
                    style = type.displaySmall,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.semantics { heading() },
                )
                Text(page.description, style = type.bodySmall, color = colors.dim, maxLines = 3, overflow = TextOverflow.Ellipsis)
                if (stale) {
                    Text(DiscoveryCopy.OFFLINE_KEPT, style = type.label, color = colors.dim)
                }
            }
        }
    }
}

/** The search, the sort picker and the arrow that reverses the order. */
@Composable
private fun Toolbar(state: ReleaseShelfModel.State, actions: ShelfActions) {
    val colors = Sillon.colors
    val type = Sillon.type
    Column(
        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        SearchField(query = state.query, onQuery = actions.search)
        // `FlowRow`: the SPA's toolbar wraps, and six pills at the 200 % scale
        // are wider than a phone — a LazyRow would hide the sort behind a swipe.
        androidx.compose.foundation.layout.FlowRow(
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            ReleaseShelfModel.SORTS.forEach { (key, label) ->
                NzChip(
                    text = label,
                    selected = state.sort == key,
                    onClick = { actions.sort(key) },
                )
            }
            NzChip(
                text = "Inverser l'ordre",
                glyph = NzGlyph.Sort,
                selected = state.descending,
                onClick = actions.reverse,
            )
        }
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                discTracks(state.rowCount),
                style = type.label,
                color = colors.dim,
                modifier = Modifier.semantics { contentDescription = "Titres affichés" },
            )
        }
    }
}


/**
 * One title of the shelf: a sleeve, the two lines, the duration, the ⋯ — and a
 * right swipe that queues it (« Lire ensuite », the list's gesture).
 */
@Composable
private fun ShelfRowView(
    row: TrackRow,
    onClick: () -> Unit,
    onMore: () -> Unit,
    onSwipeQueue: () -> Unit,
) {
    val colors = Sillon.colors
    val type = Sillon.type
    val queueLabel = "Lire ensuite"
    Box(Modifier.fillMaxWidth()) {
        Text(
            queueLabel,
            style = type.label,
            color = colors.acText,
            modifier = Modifier.align(Alignment.CenterStart).padding(start = NzSpacing.l),
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .background(colors.bg)
                .pointerInput(row.key) {
                    var offset = 0f
                    detectHorizontalDragGestures(
                        onDragEnd = {
                            if (offset > 72.dp.toPx()) onSwipeQueue()
                            offset = 0f
                        },
                        onDragCancel = { offset = 0f },
                    ) { change, amount ->
                        offset = (offset + amount).coerceIn(0f, size.width.toFloat() * 0.75f)
                        change.consume()
                    }
                }
                .clickable(role = Role.Button, onClick = onClick)
                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xs),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            CoverImage(url = row.cover, sizeDp = 44.dp, seed = row.album.ifEmpty { row.key })
            Column(Modifier.weight(1f)) {
                Text(row.title, style = type.body, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(row.artists, style = type.label, color = colors.dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Text(nzDuration(row.durationS.toInt()), style = type.mono, color = colors.dim)
            NzIconButton(
                glyph = NzGlyph.MoreVertical,
                contentDescription = "Actions pour ${row.title}",
                onClick = onMore,
            )
        }
    }
}
