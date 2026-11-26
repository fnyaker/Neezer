package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.ListWindow
import fr.neezer.mobile.feature.trackactions.TrackActions
import fr.neezer.mobile.feature.trackactions.TrackActionsContext
import uniffi.nz_ffi.TrackRow

/** One queue row's height (`--q-row-h`: 56 px in the SPA, 56 dp here). */
private val QUEUE_ROW_HEIGHT = 56.dp

/**
 * The queue editor — `QueueList.svelte`, the ONE component the SPA shares by the
 * phone's sheet and the desktop's two panels.
 *
 * The rules it keeps, and why each one is here rather than in the player:
 *
 * - **Only what is AFTER the playhead can move or be removed.** A queue is for
 *   what is coming; moving the title being heard is not something a queue is for.
 *   [NowPlayingModel.move] and [NowPlayingModel.remove] refuse it as well, so a
 *   gesture and the model cannot disagree.
 * - **A drag never depends on a row being mounted.** The list is windowed
 *   (`rememberListWindow`: pages of 64 as they near the viewport), so the target
 *   comes from the finger's DISTANCE divided by the row height, never from hit
 *   testing a row that may not exist; the rows between the origin and the target
 *   slide one row out of the way, which is the SPA's `rowShift`.
 * - **The playing row is an accent, the played ones recede** (the SPA's `.now` /
 *   `.past`), and neither carries controls.
 * - **4 000 rows never cross whole.** The handle is the player's; a window is 64
 *   rows, and the memory is the window's (`docs/agents/mobile.md`, rule 4).
 */
@Composable
fun QueueSheet(
    model: NowPlayingModel,
    nav: NzNavigator,
    lyrics: LyricsModel?,
    onClose: () -> Unit,
) {
    val state by model.state.collectAsState()
    val colors = Sillon.colors
    NzSheet(visible = true, onDismiss = onClose) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = NzMinTouchTarget),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "File d'attente",
                style = Sillon.type.titleSmall,
                color = colors.tx,
                modifier = Modifier.weight(1f),
            )
            NzIconButton(
                glyph = NzGlyph.Close,
                contentDescription = "Fermer la file d'attente",
                onClick = onClose,
            )
        }
        QueueEditor(state = state, model = model, nav = nav, modifier = Modifier.heightIn(max = 520.dp))
    }
}

/** The list itself: shared by the phone's sheet and the wide layout's column. */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
internal fun QueueEditor(
    state: NowPlayingModel.State,
    model: NowPlayingModel,
    nav: NzNavigator,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val source = state.queue
    val listState = rememberLazyListState()
    // The source is the player's own handle, adapted by `:designsystem`'s
    // `asRowSource`; a new version means Rust replaced it, so the window starts
    // again on the fresh one (`docs/agents/mobile.md`: re-window).
    val window: ListWindow? = remember(source, state.queueVersion) {
        source?.let { ListWindow(it) }
    }

    // The window asks for the page under the viewport, and for the one ahead once
    // the finger stops — never the pages a fling only crossed (`ListWindow`).
    LaunchedEffect(listState, window) {
        val engine = window ?: return@LaunchedEffect
        snapshotFlow { listState.firstVisibleItemIndex to listState.isScrollInProgress }
            .collect { (first, scrolling) ->
                engine.onVisible(first, first + VISIBLE_ROWS, scrolling)
            }
    }
    // Opening on the queue shows the PLAYING row in its first quarter (the SPA's
    // `followQueue`), and follows an advance.
    LaunchedEffect(state.index, state.queueVersion) {
        if (state.index >= 0) listState.scrollToItem((state.index - 2).coerceAtLeast(0))
    }

    var dragFrom by remember { mutableStateOf<Int?>(null) }
    var dragTo by remember { mutableIntStateOf(-1) }
    val density = LocalDensity.current
    val rowPx = with(density) { QUEUE_ROW_HEIGHT.toPx() }

    Column(modifier.fillMaxWidth()) {
        if (state.upcoming > 0) {
            Row(
                Modifier.fillMaxWidth().padding(bottom = NzSpacing.xs),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "À suivre · ${state.upcoming} titre${if (state.upcoming > 1) "s" else ""}",
                    style = Sillon.type.labelMono,
                    color = colors.dim,
                    modifier = Modifier.weight(1f),
                )
                NzIconButton(
                    glyph = NzGlyph.Trash,
                    contentDescription = "Vider la file",
                    onClick = { model.clearUpcoming() },
                )
            }
        }

        if (window == null || state.len == 0) {
            Text(
                text = "La file est vide.",
                style = Sillon.type.body,
                color = colors.dim,
                modifier = Modifier.padding(vertical = NzSpacing.l),
            )
            return@Column
        }

        // A row's insert / remove / reorder plays the design system's own specs
        // (X0's `itemSpecs()`): no mount fade (M4), a 200 ms placement, a 140 ms
        // fade-out — and nothing at all at eco, no displacement at reduced.
        val specs = Sillon.motion.itemSpecs()
        LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
            items(
                count = window.len,
                // The identity, never the index: the remove case the person does
                // most is the one `animateItem` can only move if the rows below it
                // keep their key (see [itemKey]).
                key = { index -> window.itemKey(index) },
            ) { index ->
                val row = window.rowAt(index)
                val shift = rowShift(index, dragFrom, dragTo)
                val motion = Modifier.animateItem(specs.fadeIn, specs.placement, specs.fadeOut)
                if (row == null) {
                    // A page still in flight: a skeleton row the height of a real
                    // one, never a blank (rule 3).
                    Box(
                        motion
                            .fillMaxWidth()
                            .height(QUEUE_ROW_HEIGHT)
                            .padding(vertical = NzSpacing.xxs)
                            .clip(RoundedCornerShape(NzRadius.field))
                            .background(colors.s1.copy(alpha = 0.4f)),
                    )
                } else {
                    QueueRow(
                        modifier = motion,
                        row = row,
                        index = index,
                        playing = index == state.index,
                        past = index < state.index,
                        editable = index > state.index,
                        shiftRows = shift,
                        onPick = { model.jump(index, row.key) },
                        onRemove = { model.remove(index, row.key) },
                        onMore = {
                            TrackActions.open(
                                row = row,
                                nav = nav,
                                context = TrackActionsContext(
                                    surface = "queue",
                                    index = index,
                                    currentIndex = state.index,
                                ),
                            )
                        },
                        onDragStart = {
                            dragFrom = index
                            dragTo = index
                        },
                        onDrag = { dy ->
                            val from = dragFrom ?: return@QueueRow
                            val steps = (dy / rowPx).toInt()
                            // Nothing can land on or before the playing row.
                            dragTo = (from + steps).coerceIn(state.index + 1, state.len - 1)
                        },
                        onDragEnd = {
                            val from = dragFrom
                            val to = dragTo
                            dragFrom = null
                            dragTo = -1
                            if (from != null && to >= 0 && to != from) model.move(from, to)
                        },
                    )
                }
            }
        }
    }
}

/** How many rows a phone shows; the window's preload looks one page past it. */
private const val VISIBLE_ROWS = 12

/**
 * A row's `LazyColumn` key, stable across a removal.
 *
 * The key this list used to carry was `"<index>:<track key>"`, which is a NEW
 * item after every removal: the rows below shift by one, so each of them was
 * torn down and rebuilt instead of moving, and `animateItem` had nothing to
 * animate — the 4 000-row case the owner asked for moved in 0 frames. The
 * identity is the track's own key; the rank only disambiguates a queue that
 * holds the same title twice (the SPA's `VirtualList` does the same), and it is
 * counted over the window's LOADED rows, so it costs no request and no whole
 * list: a page that is not loaded draws a skeleton, which has its own key.
 */
private fun ListWindow.itemKey(index: Int): String {
    val row = rowAt(index) ?: return "skeleton:$index"
    var rank = 0
    for ((other, value) in rows) if (other < index && value.key == row.key) rank += 1
    return if (rank == 0) row.key else "${row.key}#$rank"
}

/**
 * The SPA's `rowShift`: the rows between the dragged origin and the target slide
 * one row out of the way, so the list MAKES ROOM instead of drawing a line.
 */
private fun rowShift(index: Int, from: Int?, to: Int): Int {
    if (from == null || to < 0 || to == from) return 0
    return when {
        to > from && index > from && index <= to -> -1
        to < from && index >= to && index < from -> 1
        else -> 0
    }
}

/** One queue row: the sleeve, the names, and — for an upcoming one — its controls. */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
private fun QueueRow(
    row: TrackRow,
    index: Int,
    playing: Boolean,
    past: Boolean,
    editable: Boolean,
    shiftRows: Int,
    onPick: () -> Unit,
    onRemove: () -> Unit,
    onMore: () -> Unit,
    onDragStart: () -> Unit,
    onDrag: (Float) -> Unit,
    onDragEnd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val density = LocalDensity.current
    var dragY by remember { mutableStateOf(0f) }
    Row(
        modifier
            .fillMaxWidth()
            .height(QUEUE_ROW_HEIGHT)
            .graphicsLayer {
                translationY = shiftRows * QUEUE_ROW_HEIGHT.toPx() + dragY
            }
            .clip(RoundedCornerShape(NzRadius.field))
            .combinedClickable(
                role = Role.Button,
                onClick = onPick,
                onLongClick = onMore,
            )
            .padding(horizontal = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Box(contentAlignment = Alignment.Center) {
            CoverImage(url = row.cover, sizeDp = 40.dp, seed = row.id)
            if (playing) {
                Box(
                    Modifier.size(22.dp).clip(CircleShape).background(colors.bg.copy(alpha = 0.72f)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(glyph = NzGlyph.Play, size = 12.dp, tint = colors.acText, contentDescription = null)
                }
            }
        }
        Column(Modifier.weight(1f)) {
            Text(
                text = row.title,
                style = Sillon.type.body,
                color = when {
                    playing -> colors.acText
                    past -> colors.dim
                    else -> colors.tx
                },
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
        if (editable) {
            NzIconButton(
                glyph = NzGlyph.Close,
                contentDescription = "Retirer « ${row.title} » de la file",
                onClick = onRemove,
                size = 16.dp,
            )
            Box(
                Modifier
                    .size(NzMinTouchTarget)
                    .semantics {
                        contentDescription =
                            "Déplacer « ${row.title} » : glisser, ou flèches haut et bas"
                    }
                    .pointerInput(index) {
                        detectDragGestures(
                            onDragStart = {
                                onDragStart()
                                dragY = 0f
                            },
                            onDragEnd = {
                                dragY = 0f
                                onDragEnd()
                            },
                            onDragCancel = {
                                dragY = 0f
                                onDragEnd()
                            },
                        ) { change, amount ->
                            change.consume()
                            dragY += amount.y
                            onDrag(dragY)
                        }
                    },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    glyph = NzGlyph.Grip,
                    size = 18.dp,
                    tint = if (dragY != 0f) colors.acText else colors.dim,
                    contentDescription = null,
                )
            }
        } else {
            // The playing and played rows keep the column the controls occupy, so
            // their durations line up with everyone else's.
            Box(Modifier.size(NzMinTouchTarget)) {
                Text(
                    text = "",
                    style = Sillon.type.mono,
                    color = colors.dim,
                    modifier = Modifier.align(Alignment.Center),
                )
            }
        }
    }
}
