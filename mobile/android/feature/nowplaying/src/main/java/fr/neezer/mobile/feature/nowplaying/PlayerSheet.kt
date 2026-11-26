package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
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
import androidx.compose.ui.input.pointer.pointerInput
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.currentStateAsState
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzGlyphSwap
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
 * The one column a thumb reaches: header, sleeve, names, rail, transport, doors.
 *
 * **It sits inside the system bars** (M5). The surface is `enableEdgeToEdge`, so
 * without the two insets the header row was drawn under the status bar and the
 * « À propos » pill under the gesture bar; the insets come BEFORE this lot's own
 * 4 dp paddings, never instead of them.
 */
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
            // P2: the sheet's own gesture has an alternative for a screen reader —
            // the chevron (a 48 dp button, above) stays the control a finger or
            // TalkBack taps, and this sentence says the same thing by swiping.
            .semantics {
                contentDescription = "Lecteur plein écran, glisser vers le bas pour réduire"
            }
            .statusBarsPadding()
            .navigationBarsPadding()
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
            AboutPeek()
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
        GlyphIconButton(
            glyph = if (liked) NzGlyph.HeartFilled else NzGlyph.Heart,
            contentDescription = if (liked) "Retirer des favoris" else "Ajouter aux favoris",
            onClick = {
                val key = now?.key ?: return@GlyphIconButton
                val target = screens ?: return@GlyphIconButton
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
            NzGlyphSwap(
                glyph = if (state.playing) NzGlyph.Pause else NzGlyph.Play,
                contentDescription = if (state.playing) "Pause" else "Lecture",
                size = 28.dp,
                tint = colors.onbtn,
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
 * `RemoteButton()`, both an [NzIconButton] and therefore a 48 dp target by
 * themselves — the row wraps them in a 48 dp box and adds NO padding, because an
 * 8 dp pad on each side leaves the button 32 dp to live in and a thumb that aims
 * at it misses (measured on the M5 capture).
 *
 * **Five doors, and the words yield before a target does.** On a 360 dp phone the
 * row has 320 dp; three icon doors and the two word doors need 296 dp at a 100 %
 * font scale, and 390 dp at 200 %. Two 48 dp targets and a label cannot share
 * that line, so the words become glyphs when they no longer fit (the doors keep
 * their contentDescription, so TalkBack still reads « Paroles » and « File
 * d'attente »). The third answer — letting Compose shrink the last control — is
 * the bug the M5 measurement found: the chevron fell to ~20 dp.
 */
@Composable
private fun Doors(
    state: NowPlayingModel.State,
    nav: NzNavigator,
    party: (@Composable () -> Unit)?,
    remote: (@Composable () -> Unit)?,
) {
    val colors = Sillon.colors
    val density = LocalDensity.current
    val measurer = rememberTextMeasurer()
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        // The words in the type scale IN FORCE: at 200 % the measurer answers
        // double, which is exactly the case the screenshot bench renders.
        val words = with(density) {
            (measurer.measure("Paroles", Sillon.type.label).size.width +
                measurer.measure("File · ${state.upcoming}", Sillon.type.label).size.width).toDp()
        }
        val icons = 1 + (if (party != null) 1 else 0) + (if (remote != null) 1 else 0)
        val needed = NzMinTouchTarget * icons + words + NzSpacing.s * 4 + NzSpacing.xs * (icons + 1)
        val labelled = maxWidth >= needed
        Row(
            Modifier.fillMaxWidth().padding(vertical = NzSpacing.xxs),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
        ) {
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
            if (party != null) Box(Modifier.width(NzMinTouchTarget)) { party() }
            if (remote != null) Box(Modifier.width(NzMinTouchTarget)) { remote() }
            Door(contentDescription = "Paroles", onClick = { NowPlayingState.showPane(Pane.Lyrics) }) {
                if (labelled) {
                    Text(
                        "Paroles",
                        style = Sillon.type.label,
                        color = colors.tx,
                        maxLines = 1,
                        softWrap = false,
                        overflow = TextOverflow.Clip,
                    )
                } else {
                    Icon(NzGlyph.Music, size = 20.dp, tint = colors.dim)
                }
            }
            Door(
                contentDescription = "File d'attente",
                onClick = { NowPlayingState.showPane(Pane.Queue) },
            ) {
                if (labelled) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
                    ) {
                        Text("File ·", style = Sillon.type.label, color = colors.tx, maxLines = 1, softWrap = false)
                        Text("${state.upcoming}", style = Sillon.type.mono, color = colors.dim, maxLines = 1)
                    }
                } else {
                    Icon(NzGlyph.Queue, size = 20.dp, tint = colors.dim)
                }
            }
        }
    }
}

/**
 * « À propos » — the line UNDER the thumb row, the SPA's `.peek`
 * (`MobileNowPlaying.svelte`).
 *
 * It is not a door of the row for a measured reason: six 48 dp targets on a
 * 360 dp phone ask 348 dp of the 320 usable, and a `Row` that overflows shrinks
 * its last child instead of clipping — the chevron fell to ~20 dp (M5's own
 * capture, `mb11-encre-360dp-lecteur.png`). Under the row, the five remaining
 * doors fit in 296 dp and this control keeps its 48 dp.
 *
 * The look is the SPA's `.peek`: a glass pill 26 % of the first tier, the
 * specular hairline on its top edge, the mono label in small capitals and the
 * chevron that says "there is more below".
 */
@Composable
private fun AboutPeek() {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.pill)
    Row(
        modifier = Modifier
            .padding(top = NzSpacing.xs)
            .heightIn(min = NzMinTouchTarget)
            .clip(shape)
            .background(colors.s1.copy(alpha = 0.26f))
            // A 1 dp border costs no size in Compose (it is painted inside the
            // bounds): the ring the SPA draws as an inset shadow.
            .border(1.dp, colors.tx.copy(alpha = 0.07f), shape)
            .clickable(role = Role.Button) { NowPlayingState.showPane(Pane.About) }
            .semantics { contentDescription = "À propos du titre" }
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Icon(NzGlyph.ChevronUp, size = 16.dp, tint = colors.dim)
        Text(
            "À propos".uppercase(),
            style = Sillon.type.labelMono,
            color = colors.dim,
            maxLines = 1,
            softWrap = false,
        )
    }
}

/**
 * One door of the thumb row: a 48 dp target of glass, the SPA's `.fb`.
 *
 * The fill is the first glass tier at 34 % with a hairline on its top edge — a
 * control over a photograph, not a floating glyph, and never a Material button on
 * a Sillon surface.
 */
@Composable
private fun Door(
    contentDescription: String,
    onClick: () -> Unit,
    content: @Composable () -> Unit,
) {
    val colors = Sillon.colors
    Box(
        modifier = Modifier
            .widthIn(min = NzMinTouchTarget)
            .heightIn(min = NzMinTouchTarget)
            .clip(RoundedCornerShape(NzRadius.field))
            .background(colors.s1.copy(alpha = 0.34f))
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { this.contentDescription = contentDescription }
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
