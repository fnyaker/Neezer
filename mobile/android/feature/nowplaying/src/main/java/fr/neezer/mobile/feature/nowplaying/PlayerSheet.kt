package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.input.pointer.pointerInput
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.currentStateAsState
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzDuration
import fr.neezer.mobile.feature.nowplaying.NowPlayingState.Pane
import kotlinx.coroutines.launch
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.Repeat

/**
 * The full-screen player — `MobileNowPlaying.svelte` on a phone, the reading
 * columns of `DesktopNowPlaying.svelte` from 640 dp up.
 *
 * What it does NOT do, on purpose:
 *
 * - it does not hold the queue: the queue is a pane over it ([QueueSheet]) fed by
 *   the player's `ListHandle`, never a copied list;
 * - it does not run the transport: every control sends a `PlayerCommand`, and the
 *   glyph it draws comes from the snapshot Rust pushed (MB3b's machine is the one
 *   truth — a facade with its own `playing` flag is the bug MB3c measured);
 * - it does not count the position: [PlayerColumn]'s frame loop reads it once per
 *   visible frame and stops when the surface leaves, so "0 FFI calls with the
 *   screen off" is a fact of the loop, not a hope.
 *
 * The look is Sillon's: the sleeve's own blur behind an ink ground, one accent
 * (the current lyric line, the played part of the rail, a chosen shuffle), 48 dp
 * under every glyph and a 68 dp play button — the SPA's measurements, which are
 * the ones a thumb misses when they shrink.
 */
@Composable
fun PlayerSurface(
    state: NowPlayingModel.State,
    model: NowPlayingModel,
    nav: NzNavigator,
    lyrics: LyricsModel?,
    about: AboutModel?,
    onClose: () -> Unit,
    /**
     * The two doors MB20 and MB19 own, at their SPA place in the thumb row. Empty
     * until those lots hand back compact icon buttons (see [Doors]).
     */
    party: (@Composable () -> Unit)? = null,
    remote: (@Composable () -> Unit)? = null,
    /**
     * Whether the rail reads the engine's position on every frame. A screenshot
     * bench hands `false`: an image has no player to read, and a frame loop that
     * never ends would keep the bench from ever being idle. The DEVICE always
     * takes the default.
     */
    liveReads: Boolean = true,
) {
    val colors = Sillon.colors
    val lyricsState = lyrics?.state?.collectAsState()?.value
    Box(Modifier.fillMaxSize().background(colors.bg)) {
        PlayerBackdrop(cover = state.current?.cover, seed = state.current?.id)
        BoxWithConstraints(Modifier.fillMaxSize()) {
            // The SPA's breakpoints: the phone column under 640 dp, the queue and
            // « À propos » as a side column from 900 dp, the words beside them from
            // 1100 dp (`DesktopNowPlaying.svelte`'s own media queries). A tablet in
            // portrait keeps the phone column rather than three squeezed strips.
            val wide = maxWidth >= 640.dp
            val showSide = maxWidth >= 900.dp
            val showLyrics = maxWidth >= 1100.dp
            if (wide) {
                Row(Modifier.fillMaxSize()) {
                    if (showLyrics && lyricsState != null && !lyricsState.none) {
                        Column(
                            Modifier
                                .widthIn(min = 240.dp, max = 300.dp)
                                .fillMaxHeight()
                                .padding(top = NzSpacing.xxl, bottom = NzSpacing.l),
                        ) {
                            Text(
                                text = "PAROLES",
                                style = Sillon.type.labelMono,
                                color = colors.dim,
                                modifier = Modifier.padding(horizontal = NzSpacing.l),
                            )
                            LyricsView(lyricsState, Modifier.weight(1f).padding(horizontal = NzSpacing.l))
                        }
                    }
                    Box(Modifier.weight(1f)) {
                        PlayerColumn(state, model, nav, lyricsState, onClose, wide = true, liveReads, party, remote)
                    }
                    if (showSide) {
                        SideColumn(state, model, nav, about, Modifier.width(360.dp))
                    }
                }
            } else {
                PlayerColumn(state, model, nav, lyricsState, onClose, wide = false, liveReads, party, remote)
            }
        }
    }
}

/**
 * The sleeve under the player, dimmed by the gradient below it.
 *
 * **No `Modifier.blur`**: a full-screen blurred layer made SwiftShader compile a
 * NEW pipeline for every primitive drawn under it (`CircleOp`, `FillRectOp`,
 * `EllipseOp`, `TextureOp`: 5 programs, 511 ms of `flush commands`) and the
 * player's first frame cost p95 **775 ms** (MQ2's journey, 2026-11-24). The
 * sleeve is an image drawn at 900 dp behind a 72–94 % gradient; the blur bought
 * a softness the gradient already gives, at the price of the frame budget. A
 * real sleeve is a texture, and the phone's GPU compiles that pipeline once.
 */
@Composable
private fun PlayerBackdrop(cover: String?, seed: String?) {
    if (cover == null) return
    Box(Modifier.fillMaxSize()) {
        CoverImage(url = cover, sizeDp = 900.dp, seed = seed)
    }
    Box(
        Modifier
            .fillMaxSize()
            .background(
                Brush.verticalGradient(
                    listOf(
                        Sillon.colors.bg.copy(alpha = 0.72f),
                        Sillon.colors.bg.copy(alpha = 0.94f),
                    ),
                ),
            ),
    )
}

/** The one column a thumb reaches: header, sleeve, names, rail, transport, doors. */
@Composable
private fun PlayerColumn(
    state: NowPlayingModel.State,
    model: NowPlayingModel,
    nav: NzNavigator,
    lyrics: LyricsModel.State?,
    onClose: () -> Unit,
    wide: Boolean,
    liveReads: Boolean,
    party: (@Composable () -> Unit)?,
    remote: (@Composable () -> Unit)?,
) {
    val lifecycleState by LocalLifecycleOwner.current.lifecycle.currentStateAsState()
    val visible = lifecycleState.isAtLeast(Lifecycle.State.STARTED)

    // The rail reads the position once per visible FRAME, and touches no binding
    // at all once the surface is gone (the acceptance's count).
    LaunchedEffect(visible, state.current?.key, liveReads) {
        if (!visible || !liveReads) return@LaunchedEffect
        while (true) {
            androidx.compose.runtime.withFrameNanos { }
            model.framePosition(drawn = true)
        }
    }

    Column(
        Modifier
            .fillMaxSize()
            .padding(horizontal = if (wide) NzSpacing.xl else NzSpacing.pageX)
            .padding(top = if (wide) NzSpacing.xl else NzSpacing.xs, bottom = NzSpacing.xs),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        PlayerHeader(state, model, nav, onClose, wide)
        Spacer(Modifier.height(NzSpacing.xs))
        Column(
            Modifier.weight(1f),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            val line = lyrics?.activeText
            if (!line.isNullOrBlank()) {
                Text(
                    text = line,
                    style = Sillon.type.titleSmall,
                    color = Sillon.colors.acText,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(horizontal = NzSpacing.s, vertical = NzSpacing.xxs),
                )
            }
            CoverStage(state, model, wide)
            Spacer(Modifier.height(NzSpacing.s))
            TrackLine(state, model, nav)
            Spacer(Modifier.height(NzSpacing.xs))
            SeekRail(state, model)
            Transport(state, model)
            // The thumb row belongs to the same centred block: the SPA's `.mid`
            // holds everything above the page's bottom edge, and a 360 dp phone
            // keeps the transport within the thumb's arc.
            Spacer(Modifier.height(NzSpacing.xl))
            Doors(state, nav, party, remote)
        }
    }
}

/** The header: collapse, what is playing, the room's chips, the ⋯ menu. */
@Composable
private fun PlayerHeader(
    state: NowPlayingModel.State,
    model: NowPlayingModel,
    nav: NzNavigator,
    onClose: () -> Unit,
    wide: Boolean,
) {
    val colors = Sillon.colors
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
    ) {
        NzIconButton(
            glyph = NzGlyph.ChevronDown,
            contentDescription = "Réduire le lecteur",
            onClick = onClose,
            size = 24.dp,
        )
        Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
            // « En lecture » on a wide window; on a phone the sheet IS the player,
            // and at 200 % the label is the first thing to squeeze the chips — so
            // the line that stays is the one with information in it: what the
            // player is doing (« Chargement… », the cause of a fault).
            if (wide) {
                Text(
                    text = "EN LECTURE",
                    style = Sillon.type.labelMono,
                    color = colors.dim,
                    maxLines = 1,
                )
            }
            playbackStatus(state)?.let { status ->
                Text(
                    text = status,
                    style = Sillon.type.mono,
                    color = colors.acText,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        // The room's chips, in the SPA's order (§9): the episode's speed, the
        // sleep timer, then the quality badge — each a 44 dp target whatever the
        // glyph's own size.
        state.speedRate?.let { rate -> SpeedChip(rate = rate, model = model) }
        SleepButton(sleep = state.sleep, model = model)
        QualityButton(quality = state.quality, model = model)
        NzIconButton(
            glyph = NzGlyph.MoreVertical,
            contentDescription = "Plus d'options",
            onClick = {
                val now = state.current ?: return@NzIconButton
                fr.neezer.mobile.feature.trackactions.TrackActions.open(
                    row = nowPlayingRow(now),
                    nav = nav,
                    context = fr.neezer.mobile.feature.trackactions.TrackActionsContext(
                        surface = "player",
                        index = state.index,
                        currentIndex = state.index,
                        episode = now.isEpisode,
                    ),
                )
            },
        )
    }
}

/**
 * The sleeve, its neighbours peeking: a swipe changes track, as in the SPA.
 *
 * The square is the SMALLER of 80 % of the width and 48 % of the room the block
 * has (the SPA's `min(78vw, 42vh)`), so a 360 × 640 phone shrinks the cover
 * instead of pushing the transport off the screen, and a 600 dp window does not
 * grow a sleeve taller than the column it sits in.
 */
@Composable
private fun CoverStage(state: NowPlayingModel.State, model: NowPlayingModel, wide: Boolean) {
    val colors = Sillon.colors
    val pager = rememberPagerState(initialPage = 1) { 3 }
    val neighbours = remember(state.index, state.queueVersion, state.len) { neighboursOf(state) }

    // A settled page that is not the middle one is a swipe: change track, then put
    // the strip back on the centre without animating (the SPA's `centerCurrent`).
    LaunchedEffect(pager.settledPage, state.index) {
        when (pager.settledPage) {
            0 -> {
                if (state.index > 0) model.jump(state.index - 1, neighbours.prevKey.orEmpty())
                pager.scrollToPage(1)
            }
            2 -> {
                if (state.index in 0 until state.len - 1) {
                    model.jump(state.index + 1, neighbours.nextKey.orEmpty())
                }
                pager.scrollToPage(1)
            }
            else -> Unit
        }
    }
    BoxWithConstraints(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        val fraction = if (wide) 0.62f else 0.80f
        val size = minOf(maxWidth * fraction, maxHeight * 0.48f)
        HorizontalPager(
            state = pager,
            modifier = Modifier.fillMaxWidth().height(size),
            pageSpacing = 14.dp,
            contentPadding = PaddingValues(horizontal = ((maxWidth - size) / 2).coerceAtLeast(0.dp)),
        ) { page ->
            val cover = when (page) {
                0 -> neighbours.prevCover
                2 -> neighbours.nextCover
                else -> state.current?.cover
            }
            val seed = when (page) {
                0 -> neighbours.prevId
                2 -> neighbours.nextId
                else -> state.current?.id
            }
            Box(
                Modifier
                    .fillMaxHeight()
                    .aspectRatio(1f)
                    .clip(RoundedCornerShape(NzRadius.cover))
                    .background(colors.s1),
            ) {
                CoverImage(url = cover, sizeDp = 420.dp, seed = seed)
            }
        }
    }
}

/** The two neighbours of the playhead, read from the queue's own handle. */
private data class Neighbours(
    val prevCover: String? = null,
    val prevId: String? = null,
    val prevKey: String? = null,
    val nextCover: String? = null,
    val nextId: String? = null,
    val nextKey: String? = null,
)

private fun neighboursOf(state: NowPlayingModel.State): Neighbours {
    val rows = state.queue ?: return Neighbours()
    val index = state.index
    if (index < 0 || rows.len() == 0) return Neighbours()
    val prev = if (index > 0) rows.window(index - 1, 1).firstOrNull() else null
    val next = if (index in 0 until state.len - 1) rows.window(index + 1, 1).firstOrNull() else null
    return Neighbours(
        prevCover = prev?.cover,
        prevId = prev?.id,
        prevKey = prev?.key,
        nextCover = next?.cover,
        nextId = next?.id,
        nextKey = next?.key,
    )
}

/** The title, the names, and the heart (MB10's own optimistic `like`/`unlike`). */
@Composable
private fun TrackLine(state: NowPlayingModel.State, model: NowPlayingModel, nav: NzNavigator) {
    val colors = Sillon.colors
    val now = state.current
    val scope = rememberCoroutineScope()
    val screens = remember { NowPlayingCore.screens() }
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                text = now?.title ?: "Rien en lecture",
                style = Sillon.type.displaySmall,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = if (now?.albumId != null) {
                    Modifier.clickable(role = Role.Button) {
                        now.albumId?.let { nav.go(uniffi.nz_ffi.AppLink.Album(it)) }
                    }
                } else {
                    Modifier
                },
            )
            Text(
                text = now?.artists ?: "",
                style = Sillon.type.body,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        val liked = now?.liked == true
        NzIconButton(
            glyph = if (liked) NzGlyph.HeartFilled else NzGlyph.Heart,
            contentDescription = if (liked) "Retirer des favoris" else "Ajouter aux favoris",
            onClick = {
                val key = now?.key ?: return@NzIconButton
                val target = screens ?: return@NzIconButton
                // Optimistic: the glyph flips on the next snapshot; a failure says
                // why and offers the retry (rule 3), never a silent revert.
                scope.launch {
                    runCatching { if (liked) target.unlike(key) else target.like(key) }
                        .onFailure {
                            fr.neezer.mobile.designsystem.NzToasts.show(
                                text = NowPlayingCore.cause(it),
                                actionLabel = "Réessayer",
                                onAction = { scope.launch { runCatching { target.like(key) } } },
                            )
                        }
                    model.refresh()
                }
            },
            size = 24.dp,
            tint = if (liked) colors.hrt else colors.dim,
        )
    }
}

/** The rail: played on the accent, buffered in a lighter ink, the rest a hairline. */
@Composable
private fun SeekRail(state: NowPlayingModel.State, model: NowPlayingModel) {
    val colors = Sillon.colors
    var scrubbing by remember { mutableStateOf<Float?>(null) }
    val duration = state.durationS
    val played = if (duration > 0.0) {
        ((scrubbing?.toDouble() ?: state.position.positionS).coerceIn(0.0, duration)) / duration
    } else {
        0.0
    }
    val buffered = if (duration > 0.0) {
        (state.position.bufferedS / duration).coerceIn(played, 1.0)
    } else {
        played
    }

    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(nzDuration((played * duration).toInt()), style = Sillon.type.mono, color = colors.dim)
        BoxWithConstraints(
            Modifier
                .weight(1f)
                .height(NzMinTouchTarget)
                .semantics { contentDescription = "Position dans le titre" }
                .pointerInput(duration) {
                    if (duration <= 0.0) return@pointerInput
                    detectTapGestures { offset ->
                        model.seek((offset.x / size.width).coerceIn(0f, 1f) * duration)
                    }
                }
                .pointerInput(duration) {
                    if (duration <= 0.0) return@pointerInput
                    detectHorizontalDragGestures(
                        onDragStart = { offset ->
                            scrubbing = ((offset.x / size.width).coerceIn(0f, 1f) * duration).toFloat()
                        },
                        onDragEnd = {
                            scrubbing?.let { model.seek(it.toDouble()) }
                            scrubbing = null
                        },
                        onDragCancel = { scrubbing = null },
                    ) { change, _ ->
                        scrubbing = ((change.position.x / size.width).coerceIn(0f, 1f) * duration)
                            .toFloat()
                    }
                },
            contentAlignment = Alignment.CenterStart,
        ) {
            val track = maxWidth
            Box(Modifier.fillMaxWidth().height(6.dp).clip(CircleShape).background(colors.ln))
            Box(
                Modifier
                    .width(track * buffered.toFloat())
                    .height(6.dp)
                    .clip(CircleShape)
                    .background(colors.tx.copy(alpha = 0.42f)),
            )
            Box(
                Modifier
                    .width(track * played.toFloat())
                    .height(6.dp)
                    .clip(CircleShape)
                    .background(colors.acMark),
            )
            // The thumb's centre follows the position: a 0 % title has it at the
            // very start, a 100 % one at the very end (the SPA's 15 px thumb).
            Box(
                Modifier
                    .offset(x = (track * played.toFloat()) - 7.5.dp)
                    .size(15.dp)
                    .clip(CircleShape)
                    .background(colors.tx),
            )
        }
        Text(nzDuration(duration.toInt()), style = Sillon.type.mono, color = colors.dim)
    }
}

/** Shuffle, previous, play/pause (68 dp), next, repeat. */
@Composable
private fun Transport(state: NowPlayingModel.State, model: NowPlayingModel) {
    val colors = Sillon.colors
    Row(
        Modifier.fillMaxWidth().padding(top = NzSpacing.xs),
        horizontalArrangement = Arrangement.SpaceEvenly,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        NzIconButton(
            glyph = NzGlyph.Shuffle,
            contentDescription = if (state.shuffle) "Aléatoire activé" else "Aléatoire",
            onClick = { model.setShuffle(!state.shuffle) },
            selected = state.shuffle,
            size = 22.dp,
        )
        NzIconButton(
            glyph = NzGlyph.Prev,
            contentDescription = "Précédent",
            onClick = { model.prev() },
            size = 30.dp,
        )
        Box(
            Modifier
                .size(68.dp)
                .clip(CircleShape)
                .background(colors.btn)
                .clickable(role = Role.Button) { model.toggle() },
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                glyph = if (state.playing) NzGlyph.Pause else NzGlyph.Play,
                size = 28.dp,
                tint = colors.onbtn,
                contentDescription = if (state.playing) "Pause" else "Lecture",
            )
        }
        NzIconButton(
            glyph = NzGlyph.Next,
            contentDescription = "Suivant",
            onClick = { model.next() },
            size = 30.dp,
        )
        NzIconButton(
            glyph = if (state.repeat == Repeat.ONE) NzGlyph.Repeat1 else NzGlyph.Repeat,
            contentDescription = repeatLabel(state.repeat),
            onClick = { model.cycleRepeat() },
            selected = state.repeat != Repeat.OFF,
            size = 22.dp,
        )
    }
}

/**
 * The thumb row (§9): add to a playlist, the words, what is left of the queue,
 * and the two doors the other lots own.
 *
 * `party` and `remote` are SLOTS: MB20's `PartyButton()` and MB19's
 * `RemoteButton()`. They are empty here on purpose — their MB0 bodies are labelled
 * Material `OutlinedButton`s, and five 44 dp targets do not fit on a 360 dp phone
 * beside two of them (rule 5: no default-looking widget on a Sillon surface). The
 * day those lots replace the bodies, `NowPlayingHost` passes them and nothing here
 * moves.
 */
@Composable
private fun Doors(
    state: NowPlayingModel.State,
    nav: NzNavigator,
    party: (@Composable () -> Unit)?,
    remote: (@Composable () -> Unit)?,
) {
    val colors = Sillon.colors
    Row(
        Modifier.fillMaxWidth().padding(vertical = NzSpacing.xxs),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Every door keeps its 44 dp target and its own width (the SPA's
        // `justify-content: space-between`): the GAPS give way on a 360 dp phone,
        // never a target and never a label (`MobileNowPlaying.svelte` §9).
        Box(Modifier.width(NzMinTouchTarget)) {
            NzIconButton(
                glyph = NzGlyph.Plus,
                contentDescription = "Ajouter à une playlist",
                onClick = {
                    val now = state.current ?: return@NzIconButton
                    fr.neezer.mobile.feature.trackactions.TrackActions.open(
                        row = nowPlayingRow(now),
                        nav = nav,
                        context = fr.neezer.mobile.feature.trackactions.TrackActionsContext(
                            surface = "player",
                            index = state.index,
                            currentIndex = state.index,
                        ),
                    )
                },
            )
        }
        if (party != null) Door(Modifier.width(NzMinTouchTarget)) { party() }
        if (remote != null) Door(Modifier.width(NzMinTouchTarget)) { remote() }
        Door(onClick = { NowPlayingState.showPane(Pane.Lyrics) }) {
            Text(
                "Paroles",
                style = Sillon.type.label,
                color = colors.tx,
                maxLines = 1,
                softWrap = false,
                overflow = TextOverflow.Clip,
            )
        }
        Door(onClick = { NowPlayingState.showPane(Pane.Queue) }) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
            ) {
                Text("File ·", style = Sillon.type.label, color = colors.tx, maxLines = 1, softWrap = false)
                Text("${state.upcoming}", style = Sillon.type.mono, color = colors.dim, maxLines = 1)
            }
        }
        NzIconButton(
            glyph = NzGlyph.ChevronUp,
            contentDescription = "À propos du titre",
            onClick = { NowPlayingState.showPane(Pane.About) },
            tint = colors.dim,
            size = 20.dp,
        )
    }
}

/**
 * One door of the thumb row: a 44 dp target of glass, the SPA's `.fb`.
 *
 * The fill is the first glass tier at 34 % with a hairline on its top edge — a
 * control over a photograph, not a floating glyph, and never a Material button on
 * a Sillon surface.
 */
@Composable
private fun Door(
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    val colors = Sillon.colors
    Box(
        modifier = modifier
            .heightIn(min = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.field))
            .background(colors.s1.copy(alpha = 0.34f))
            .then(
                if (onClick != null) Modifier.clickable(role = Role.Button, onClick = onClick) else Modifier,
            )
            .padding(horizontal = NzSpacing.s),
        contentAlignment = Alignment.Center,
    ) {
        content()
    }
}

/** The line under the header: what the player is DOING, or nothing when idle. */
internal fun playbackStatus(state: NowPlayingModel.State): String? = when (state.snapshot?.state) {
    PlayState.BUFFERING -> "Chargement…"
    PlayState.ENDED -> "Fin de la file"
    else -> state.snapshot?.error?.takeIf { it.isNotBlank() }
}

/** The repeat glyph's own sentence, so TalkBack never reads a bare icon. */
internal fun repeatLabel(repeat: Repeat): String = when (repeat) {
    Repeat.OFF -> "Répéter : désactivé"
    Repeat.ALL -> "Répéter : toute la file"
    Repeat.ONE -> "Répéter : ce titre"
}
