package fr.neezer.mobile.feature.playlist

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.ListWindow
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTopBar
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.TrackRowView
import fr.neezer.mobile.designsystem.nzDuration
import fr.neezer.mobile.designsystem.rememberListWindow
import fr.neezer.mobile.feature.offline.DownloadButton
import uniffi.nz_ffi.PlaylistRole
import uniffi.nz_ffi.TrackRow

/**
 * One playlist, drawn (lot MB9): the band, the actions, the windowed tracklist,
 * the suggestions at the bottom.
 *
 * A pure function of [PlaylistModel.State] and [PlaylistActions] — the entry
 * wires it, the Model drives it, and a screenshot or a JVM test can draw it over
 * a fake 4 000-row source (the standard's rule 5: what the captures show is what
 * a person sees).
 *
 * Layout, phone first: a 96 dp sleeve beside the facts (the SPA's mobile band),
 * the actions on ONE scrolling line rather than three stacked rows, then the
 * list. A wide screen (≥ 600 dp) reads as two columns: the band on the left, the
 * list on the right — the tablet shell's own shape.
 */
@Composable
fun PlaylistScreen(
    state: PlaylistModel.State,
    actions: PlaylistActions,
    modifier: Modifier = Modifier,
    wide: Boolean = false,
) {
    val colors = Sillon.colors
    Column(modifier = modifier.fillMaxSize()) {
        NzTopBar(
            title = state.displayTitle.ifEmpty { "Playlist" },
            onBack = actions.back,
            actions = {
                if (state.canManage) {
                    NzIconButton(
                        glyph = NzGlyph.More,
                        contentDescription = "Plus d'actions : exporter, collaborateurs, renommer, supprimer",
                        onClick = actions.openMore,
                    )
                }
            },
        )
        when {
            state.page == null && state.failure != null -> LoadFailed(
                cause = state.failure,
                onRetry = actions.retry,
                modifier = Modifier.padding(NzSpacing.l),
            )

            state.page == null -> SkeletonBand()

            else -> if (wide) {
                Row(modifier = Modifier.fillMaxSize()) {
                    Column(
                        modifier = Modifier
                            .width(320.dp)
                            .fillMaxSize()
                            .padding(NzSpacing.l),
                        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
                    ) {
                        Band(state, actions)
                        ActionLine(state, actions)
                        if (state.readOnlyNote) ReadOnlyNote(state)
                    }
                    Box(Modifier.weight(1f).fillMaxSize()) {
                        Tracks(state, actions)
                    }
                }
            } else {
                Tracks(state, actions)
            }
        }
    }
}

/**
 * What a press on this screen does. An interface so the screen stays a pure
 * function: the entry binds it to the model, a test or a screenshot to nothing.
 */
data class PlaylistActions(
    val back: () -> Unit = {},
    val retry: () -> Unit = {},
    val refresh: () -> Unit = {},
    val play: () -> Unit = {},
    val shuffle: () -> Unit = {},
    val toggleEdit: () -> Unit = {},
    val move: (Int, Int) -> Unit = { _, _ -> },
    val remove: (Int) -> Unit = {},
    val openRow: (TrackRow, Int) -> Unit = { _, _ -> },
    val more: (TrackRow, Int) -> Unit = { _, _ -> },
    val queueNext: (TrackRow) -> Unit = {},
    val query: (String) -> Unit = {},
    val sort: (PlaylistSort) -> Unit = {},
    val direction: () -> Unit = {},
    val openMore: () -> Unit = {},
    val openMeta: () -> Unit = {},
    val addTracks: () -> Unit = {},
    val addRecommendation: (TrackRow) -> Unit = {},
    val addAllRecommendations: () -> Unit = {},
    val retryRecommendations: () -> Unit = {},
    val toggleFavorite: (() -> Unit)? = null,
    val staleRefresh: () -> Unit = {},
    val leavePlaylist: () -> Unit = {},
)

@Composable
private fun SkeletonBand() {
    Column(
        modifier = Modifier.fillMaxWidth().padding(NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            Box(
                Modifier
                    .size(96.dp)
                    .clip(RoundedCornerShape(NzRadius.cover))
                    .background(Sillon.colors.s2),
            )
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                Box(Modifier.width(180.dp).height(14.dp).background(Sillon.colors.s2))
                Box(Modifier.width(120.dp).height(12.dp).background(Sillon.colors.s2))
            }
        }
        repeat(3) {
            Box(Modifier.fillMaxWidth().height(56.dp).background(Sillon.colors.s2))
        }
    }
}

@Composable
private fun Band(state: PlaylistModel.State, actions: PlaylistActions) {
    val colors = Sillon.colors
    val meta = state.page?.meta
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            verticalAlignment = Alignment.Bottom,
        ) {
            CoverImage(
                url = meta?.cover,
                sizeDp = if (state.editing) 80.dp else 96.dp,
                seed = state.id,
                modifier = Modifier.semantics { contentDescription = "Pochette de ${state.displayTitle}" },
            )
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs), modifier = Modifier.weight(1f)) {
                Text(
                    text = factsLine(state),
                    style = Sillon.type.mono,
                    color = colors.dim,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = state.displayTitle,
                    style = Sillon.type.displaySmall,
                    color = colors.tx,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (state.displayDescription.isNotEmpty()) {
            Text(
                text = state.displayDescription,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (state.collaborative) {
            MembersStack(state, actions)
        }
        val editedBy = state.lastEditedBy
        if (!editedBy.isNullOrEmpty()) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                Icon(glyph = NzGlyph.Edit, size = 13.dp, tint = colors.dim, contentDescription = null)
                Text(
                    text = "Modifié par $editedBy",
                    style = Sillon.type.mono,
                    color = colors.dim,
                    maxLines = 1,
                )
            }
        }
        if (state.stale) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                Text("Hors ligne · version mémorisée", style = Sillon.type.labelSmall, color = colors.dim)
                NzButton(
                    text = "Actualiser",
                    onClick = actions.staleRefresh,
                    kind = NzButtonKind.Ghost,
                    glyph = NzGlyph.Refresh,
                )
            }
        }
    }
}

/** The facts line, assembled as ONE string (the SPA's own rule). */
internal fun factsLine(state: PlaylistModel.State): String {
    val parts = ArrayList<String>(5)
    parts += "Playlist"
    if (state.ownerName.isNotEmpty()) parts += state.ownerName
    val count = state.trackCount
    parts += "$count titre" + if (count > 1) "s" else ""
    val members = state.page?.members?.size ?: 0
    if (members > 1) parts += "$members collaborateurs"
    if (state.readOnlyNote) parts += "Lecture seule"
    return parts.joinToString(" · ")
}

@Composable
private fun MembersStack(state: PlaylistModel.State, actions: PlaylistActions) {
    val colors = Sillon.colors
    val members = state.page?.members.orEmpty()
    val faces = members.take(3)
    val extra = (members.size - faces.size).coerceAtLeast(0)
    Row(
        modifier = Modifier
            .heightIn(min = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.pill))
            .clickable(role = Role.Button, onClick = actions.openMore)
            .padding(end = NzSpacing.m),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        faces.forEach { member ->
            Avatar(name = member.name, sizeDp = 22.dp)
        }
        if (extra > 0) {
            Text("+$extra", style = Sillon.type.mono, color = colors.dim)
        }
        Text(
            text = "Collaborateurs",
            style = Sillon.type.label,
            color = colors.dim,
        )
    }
}

@Composable
private fun ActionLine(state: PlaylistModel.State, actions: PlaylistActions) {
    val colors = Sillon.colors
    LazyRow(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        item {
            NzButton(text = "Lire", onClick = actions.play, glyph = NzGlyph.Play)
        }
        item {
            NzButton(
                text = "Mélanger",
                onClick = actions.shuffle,
                glyph = NzGlyph.Shuffle,
                kind = NzButtonKind.Secondary,
                enabled = state.trackCount > 0,
            )
        }
        item {
            // MB15's own control: its states (absent / queued / n of m / done /
            // failed) are that lot's, the entry point is this one.
            DownloadButton(context = "playlist:${state.id}")
        }
        if (state.editable) {
            item {
                NzButton(
                    text = "Ajouter des titres",
                    onClick = actions.addTracks,
                    glyph = NzGlyph.Plus,
                    kind = NzButtonKind.Secondary,
                )
            }
            item {
                NzButton(
                    text = if (state.editing) "Terminé" else "Modifier",
                    onClick = actions.toggleEdit,
                    glyph = if (state.editing) NzGlyph.Check else NzGlyph.Grip,
                    kind = if (state.editing) NzButtonKind.Primary else NzButtonKind.Secondary,
                    enabled = state.manualView || state.editing,
                )
            }
        } else if (actions.toggleFavorite != null) {
            item {
                NzIconButton(
                    glyph = NzGlyph.Heart,
                    contentDescription = "Ajouter aux favoris",
                    selected = state.page?.meta?.isFavorite == true,
                    onClick = actions.toggleFavorite,
                )
            }
        }
        if (state.role == PlaylistRole.EDITOR || state.role == PlaylistRole.VIEWER) {
            item {
                NzButton(
                    text = "Quitter",
                    onClick = actions.leavePlaylist,
                    glyph = NzGlyph.LogOut,
                    kind = NzButtonKind.Ghost,
                )
            }
        }
        item { Spacer(Modifier.width(NzSpacing.xxs)) }
        if (state.canManage) {
            item {
                NzIconButton(
                    glyph = NzGlyph.More,
                    contentDescription = "Plus d'actions : exporter, collaborateurs, renommer, supprimer",
                    onClick = actions.openMore,
                )
            }
        }
    }
}

@Composable
private fun ReadOnlyNote(state: PlaylistModel.State) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.card))
            .background(colors.s1)
            .padding(NzSpacing.m),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(glyph = NzGlyph.Eye, size = 15.dp, tint = colors.dim, contentDescription = null)
        Text(
            text = "Lecture seule — demandez à ${state.ownerName} de vous donner l'accès en modification.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
    }
}

@Composable
private fun Tracks(state: PlaylistModel.State, actions: PlaylistActions) {
    val rows = state.rows
    val page = state.page
    if (rows == null || page == null) return
    // The window is the design system's (`rememberListWindow` over the handle);
    // a fake source (a screenshot, a JVM test) gets the same `ListWindow`.
    val window = rememberPlaylistWindow(rows, state.wake)
    val listState = rememberLazyListState()
    LaunchedEffect(window, state.wake) {
        snapshotFlow { listState.firstVisibleItemIndex to listState.isScrollInProgress }
            .collect { (first, scrolling) ->
                window.onVisible(first, first + VISIBLE_ROWS, scrolling)
            }
    }
    val colors = Sillon.colors
    LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
        item(key = "band") {
            Column(
                modifier = Modifier.padding(horizontal = NzSpacing.l, vertical = NzSpacing.m),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                Band(state, actions)
                ActionLine(state, actions)
                if (state.readOnlyNote) ReadOnlyNote(state)
                if (state.editable) Toolbar(state, actions)
                if (state.editing) {
                    Text(
                        text = "Glissez la poignée, ou utilisez les flèches, pour changer l'ordre.",
                        style = Sillon.type.labelSmall,
                        color = colors.dim,
                    )
                }
            }
        }
        if (state.trackCount == 0) {
            item(key = "empty") {
                if (state.query.isNotBlank()) {
                    Text(
                        text = "Aucun titre ne correspond à « ${state.query} ».",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                        modifier = Modifier.padding(NzSpacing.l),
                    )
                } else {
                    EmptyState(
                        title = "Cette playlist est vide",
                        body = "Ajoutez des titres pour commencer.",
                        actionLabel = if (state.editable) "Ajouter des titres" else null,
                        onAction = if (state.editable) actions.addTracks else null,
                        glyph = NzGlyph.Music,
                    )
                }
            }
        }
        items(count = state.trackCount, key = { index -> "row-$index-${window.rowAt(index)?.key ?: index}" }) { index ->
            val row = window.rowAt(index)
            if (row == null) {
                // A page that has not landed yet: a skeleton, never a blank row.
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(56.dp)
                        .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
                ) {
                    Box(
                        Modifier
                            .fillMaxSize()
                            .clip(RoundedCornerShape(NzRadius.field))
                            .background(colors.s1),
                    )
                }
            } else if (state.editing) {
                EditRow(row, index, state.trackCount, actions)
            } else {
                TrackRowView(
                    row = row,
                    playing = false,
                    onClick = { actions.openRow(row, index) },
                    onMore = { actions.more(row, index) },
                    onSwipeQueue = { actions.queueNext(row) },
                )
            }
        }
        if (state.editable && !state.editing && state.query.isBlank()) {
            item(key = "recs") { Recommendations(state, actions) }
        }
        item(key = "tail") { Spacer(Modifier.height(96.dp)) }
    }
}

@Composable
private fun rememberPlaylistWindow(rows: PlaylistRows, wake: Long): ListWindow {
    val handle = rows.handle
    return if (handle != null) {
        // The design system's own windowed handle (rule 4); `key(wake)` builds a
        // fresh window when a write replaced the order, because the handle a
        // deleted row was in is not the handle the next drag reads.
        androidx.compose.runtime.key(wake) { rememberListWindow(handle) }
    } else {
        remember(rows.source, wake) { ListWindow(rows.source) }
    }
}

/** One edit-mode row: the grip, the up/down pair, the title, the remove button. */
@Composable
private fun EditRow(
    row: TrackRow,
    index: Int,
    count: Int,
    actions: PlaylistActions,
) {
    val colors = Sillon.colors
    val density = LocalDensity.current
    val step = with(density) { 76.dp.toPx() }
    var dragged by remember(row.key) { mutableFloatStateOf(0f) }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(76.dp)
            .background(if (index % 2 == 0) colors.s1 else colors.bg)
            .padding(horizontal = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Box(
            modifier = Modifier
                .size(NzMinTouchTarget)
                .pointerInput(row.key, index, count) {
                    detectDragGesturesAfterLongPress(
                        onDragEnd = { dragged = 0f },
                        onDragCancel = { dragged = 0f },
                        onDrag = { change, amount ->
                            change.consume()
                            dragged += amount.y
                            if (dragged > step && index < count - 1) {
                                actions.move(index, index + 1)
                                dragged = 0f
                            } else if (dragged < -step && index > 0) {
                                actions.move(index, index - 1)
                                dragged = 0f
                            }
                        },
                    )
                }
                .semantics { contentDescription = "Déplacer ${row.title}" },
            contentAlignment = Alignment.Center,
        ) {
            Icon(glyph = NzGlyph.Grip, size = 22.dp, tint = colors.dim, contentDescription = null)
        }
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
            NzIconButton(
                glyph = NzGlyph.ChevronUp,
                contentDescription = "Monter ${row.title}",
                enabled = index > 0,
                onClick = { actions.move(index, index - 1) },
            )
            NzIconButton(
                glyph = NzGlyph.ChevronDown,
                contentDescription = "Descendre ${row.title}",
                enabled = index < count - 1,
                onClick = { actions.move(index, index + 1) },
            )
        }
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
            Text(
                text = row.title,
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = row.artists,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Text(nzDuration(row.durationS.toInt()), style = Sillon.type.mono, color = colors.dim)
        NzIconButton(
            glyph = NzGlyph.MinusCircle,
            contentDescription = "Retirer ${row.title} de la playlist",
            onClick = { actions.remove(index) },
        )
    }
}

@Composable
private fun Toolbar(state: PlaylistModel.State, actions: PlaylistActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        OutlinedTextField(
            value = state.query,
            onValueChange = actions.query,
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
            placeholder = { Text("Rechercher dans la playlist…", style = Sillon.type.bodySmall) },
            leadingIcon = {
                Icon(glyph = NzGlyph.Search, size = 16.dp, tint = colors.dim, contentDescription = null)
            },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            shape = RoundedCornerShape(NzRadius.chip),
        )
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            LazyRow(
                modifier = Modifier.weight(1f),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                items(PlaylistSort.entries.toList()) { sort ->
                    NzChip(
                        text = sort.label,
                        selected = state.sort == sort,
                        onClick = { actions.sort(sort) },
                    )
                }
            }
            NzIconButton(
                glyph = NzGlyph.Sort,
                contentDescription = if (state.descending) "Ordre croissant" else "Ordre décroissant",
                selected = state.descending,
                onClick = actions.direction,
            )
        }
    }
}

@Composable
private fun Recommendations(state: PlaylistModel.State, actions: PlaylistActions) {
    val colors = Sillon.colors
    val recs = state.recs
    Column(
        modifier = Modifier.fillMaxWidth().padding(NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Box(Modifier.fillMaxWidth().height(1.dp).background(colors.ln))
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text("Titres recommandés", style = Sillon.type.titleSmall, color = colors.tx)
            Spacer(Modifier.weight(1f))
            if (recs is RecsUi.Ready && recs.tracks.size > 1) {
                NzButton(
                    text = "Tout ajouter",
                    onClick = actions.addAllRecommendations,
                    glyph = NzGlyph.Plus,
                    kind = NzButtonKind.Ghost,
                    enabled = !state.recsBusy,
                )
            }
        }
        when (recs) {
            is RecsUi.Failed -> Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Text(
                    "Impossible de charger les suggestions.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
                NzButton(
                    text = "Réessayer",
                    onClick = actions.retryRecommendations,
                    glyph = NzGlyph.Refresh,
                    kind = NzButtonKind.Ghost,
                )
            }

            is RecsUi.Loading -> Text(
                "Recherche de suggestions…",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )

            is RecsUi.Ready -> if (recs.tracks.isEmpty()) {
                Text(
                    "Pas de suggestion pour l'instant",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            } else {
                recs.tracks.forEach { track ->
                    RecommendationRow(
                        track = track,
                        busy = state.recsBusy || track.key in state.pendingRecs,
                        onAdd = { actions.addRecommendation(track) },
                        onOpen = { actions.openRow(track, 0) },
                    )
                }
            }

            RecsUi.Idle -> Text(
                "Recherche de suggestions…",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
    }
}

@Composable
private fun RecommendationRow(
    track: TrackRow,
    busy: Boolean,
    onAdd: () -> Unit,
    onOpen: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .clip(RoundedCornerShape(NzRadius.field))
            .clickable(role = Role.Button, onClick = onOpen)
            .padding(horizontal = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        CoverImage(url = track.cover, sizeDp = 40.dp, seed = track.id)
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
            Text(
                track.title,
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                track.artists,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Text(nzDuration(track.durationS.toInt()), style = Sillon.type.mono, color = colors.dim)
        NzIconButton(
            glyph = NzGlyph.Plus,
            contentDescription = "Ajouter ${track.title} à la playlist",
            enabled = !busy,
            onClick = onAdd,
        )
    }
}

/** How many rows a 600 dp viewport shows: what the window asks for per report. */
private const val VISIBLE_ROWS = 12
