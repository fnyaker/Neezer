package fr.neezer.mobile.feature.nowplaying

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.input.pointer.changedToUpIgnoreConsumed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntOffset
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.currentStateAsState
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.NzBlurredBackdrop
import fr.neezer.mobile.designsystem.NzGlyphSwap
import fr.neezer.mobile.designsystem.NzSwap
import fr.neezer.mobile.designsystem.asRowSource
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzDuration
import fr.neezer.mobile.feature.nowplaying.NowPlayingState.Pane
import fr.neezer.mobile.feature.party.PartyButton
import fr.neezer.mobile.feature.remote.RemoteButton
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlin.coroutines.cancellation.CancellationException
import kotlin.math.abs
import kotlin.math.roundToInt
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.PlayerListener
import uniffi.nz_ffi.PlayerSnapshot

/**
 * The player's two surfaces, as the shell mounts them (lot MB11).
 *
 * `NavGraph.kt` already places them — the mini bar above the phone's tab bar and
 * `NowPlayingHost` once for the whole app — so this file fills MB0's two stubs and
 * edits nothing shared. [NowPlayingState] says which of them is up and which pane
 * is open; `AppGraph.openPlayer()`/`openQueue()` (the shell's own two lines) show
 * it, and so does a tap on the bar.
 *
 * # The position rule, in one place
 *
 * **`position()` is read on a frame, and only while a bar is DRAWING.** The mini
 * bar reads it at the SPA's own 4 Hz (`timeupdate`) while its lifecycle is at
 * least STARTED; the full-screen bar reads it once per frame while it is on
 * screen. With the screen off Android stops the frames, the lifecycle leaves
 * STARTED, and [PositionTicker] answers `null` without touching a binding — the
 * acceptance's "0 FFI calls with the screen off" is a count
 * (`output_stats`-free, no timer), and `NowPlayingDeviceTest` holds it.
 */

/** The SPA's `timeupdate` rate, for a bar that is 2 dp tall. */
private const val MINI_READ_MS = 250L

/**
 * The mini player above the phone's bottom bar — `PlayerBar.svelte`'s card.
 *
 * **The room is reserved from the first frame** (`PlayerBar.svelte`'s `--mini-h`,
 * lot X2): with nothing playing the same 64 dp card is drawn, the groove as its
 * cover, « Rien en lecture » as its title and a disabled transport. The first
 * « Lire » therefore moves nothing above the bar — an empty strip that appears
 * with the first title used to shift the whole screen by 76 dp under the finger.
 *
 * The content swaps on the track's KEY (`NzSwap`, 140 ms + 4 dp) and the
 * play↔pause glyph is a `NzGlyphSwap`; the cover and the 2 dp progress line stay
 * OUTSIDE the swap, because a sleeve is kept until its replacement is ready (P1)
 * and a live position is never animated (M6).
 *
 * A tap ANYWHERE on the card opens the full-screen player (the SPA's `cardTap`):
 * the sleeve and the names are one target, and the play/pause button answers for
 * itself.
 *
 * The card carries its OWN margins and its own [shape] (L1): the shell mounts it
 * bare between the « Contrôlé par … » notice and the tab bar.
 */
@Composable
fun MiniPlayer(
    modifier: Modifier = Modifier,
    onClick: () -> Unit = {},
    shape: Shape = RoundedCornerShape(NzRadius.card),
) {
    val model = rememberNowPlaying()
    val state by model.state.collectAsState()
    val lifecycleState by LocalLifecycleOwner.current.lifecycle.currentStateAsState()
    val started = lifecycleState.isAtLeast(Lifecycle.State.STARTED)
    val now = state.current

    // The 2 dp line: four reads a second, on the SPA's own cadence, and NONE once
    // the screen is gone (the loop is keyed on `started` and the player's state).
    LaunchedEffect(started, state.playing, now?.key) {
        if (!started) return@LaunchedEffect
        while (true) {
            model.framePosition(drawn = true)
            delay(MINI_READ_MS)
            if (!state.playing) break
        }
    }

    val colors = Sillon.colors
    val progress = if (state.durationS > 0.0) {
        (state.position.positionS / state.durationS).coerceIn(0.0, 1.0).toFloat()
    } else {
        0f
    }
    Box(
        modifier = modifier
            .fillMaxWidth()
            .testTag(MINI_BAR_TAG)
            .padding(start = 8.dp, end = 8.dp, top = 4.dp, bottom = 8.dp),
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(NzSpacing.miniPlayer)
                .clip(shape)
                .background(colors.glassBg),
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(NzSpacing.miniPlayer)
                    .clickable(role = Role.Button) { onClick() }
                    .semantics {
                        contentDescription = "Ouvrir le lecteur plein écran"
                    }
                    .padding(horizontal = NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                // Outside the swap: a sleeve keeps its node (and Coil's painter)
                // across a track change, so the player never flashes a groove.
                CoverImage(
                    url = now?.cover,
                    sizeDp = 44.dp,
                    modifier = Modifier.testTag(MINI_COVER_TAG),
                    seed = now?.id,
                )
                Column(Modifier.weight(1f)) {
                    // The two text lines only: the key is the track, so a re-emit
                    // of the same title plays nothing (M6).
                    NzSwap(targetState = now?.key ?: "", label = "mini-player") {
                        Column {
                            Text(
                                text = now?.title ?: "Rien en lecture",
                                style = Sillon.type.body,
                                color = colors.tx,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Text(
                                text = if (now == null) "" else playbackLine(state),
                                style = Sillon.type.bodySmall,
                                color = if (state.snapshot?.state == PlayState.BUFFERING) colors.acText else colors.dim,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
                GlyphIconButton(
                    glyph = if (state.playing) NzGlyph.Pause else NzGlyph.Play,
                    contentDescription = if (state.playing) "Pause" else "Lecture",
                    onClick = { model.toggle() },
                    enabled = now != null,
                )
            }
            // The card's own 2 dp line along its bottom edge: played on the
            // accent, the rest on the hairline — the SPA's `.mini-progress`. It
            // is INSIDE the clipped card, so the line follows the corner.
            Box(
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .fillMaxWidth(progress)
                    .height(2.dp)
                    .background(colors.acMark),
            )
        }
    }
}

/** The mini bar's root, for the height comparison a test holds (X2). */
internal const val MINI_BAR_TAG = "mini-player-bar"

/** The sleeve's node, kept across a track change (P1's rule, held by a test). */
internal const val MINI_COVER_TAG = "mini-player-cover"

/**
 * An icon-only control whose glyph is a [NzGlyphSwap] — play↔pause, heart↔filled.
 *
 * `NzIconButton` draws its own `Icon` and `Buttons.kt` belongs to X0 (the motion
 * vocabulary lot), so this lot mirrors that button's well here instead of
 * widening the design system's API: the same 48 dp target, the same disabled ink
 * (a disabled control is a WELL, not a faded surface — `Buttons.kt`'s rule), the
 * same 0.92 press scale through the motion token.
 */
@Composable
internal fun GlyphIconButton(
    glyph: NzGlyph,
    contentDescription: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 20.dp,
    tint: Color? = null,
    enabled: Boolean = true,
) {
    val motion = Sillon.motion
    val colors = Sillon.colors
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val scale by animateFloatAsState(
        targetValue = if (pressed && enabled) 0.92f else 1f,
        animationSpec = motion.displace(),
        label = "nz-glyph-icon-button-press",
    )
    val ink = if (enabled) tint ?: colors.tx else colors.dim
    Box(
        modifier = modifier
            .scale(scale)
            .sizeIn(minWidth = NzMinTouchTarget, minHeight = NzMinTouchTarget)
            .clip(CircleShape)
            .clickable(
                interactionSource = interaction,
                indication = null,
                enabled = enabled,
                role = Role.Button,
                onClick = onClick,
            ),
        contentAlignment = Alignment.Center,
    ) {
        NzGlyphSwap(
            glyph = glyph,
            contentDescription = contentDescription,
            size = size,
            tint = ink,
        )
    }
}

/**
 * The full-screen player's host, mounted ONCE by the shell.
 *
 * It owns three things and no rule:
 *
 * 1. **the one subscription** to the player (`PlayerFeed.install`) — Rust pushes
 *    on change, this hands each push to the model and returns at once;
 * 2. the model's lyrics trigger: the words are asked for when the AUDIO is under
 *    way (`lib/lyrics.js`'s `playable`), never on the track change;
 * 3. the surfaces: the player, and ONE pane over it at a time (queue, words,
 *    « À propos », the lyric card).
 */
@Composable
fun NowPlayingHost(nav: NzNavigator, modifier: Modifier = Modifier) {
    val model = rememberNowPlaying()
    val state by model.state.collectAsState()
    val scope = rememberCoroutineScope()
    val service = remember { NowPlayingCore.service() }
    val lyrics = remember(service, scope) {
        service?.let { LyricsModel(it, scope) }
    }
    val about = remember(service, scope) {
        service?.let { AboutModel(it, scope) }
    }
    val card = remember(lyrics) { CardSheetState() }
    val playerPort = remember { NowPlayingCore.player()?.let(::NzPlayerPort) ?: NoPlayerPort }

    // Nothing is installed here any more: [NowPlayingSession] owns the ONE model
    // and its single subscription to [PlayerFeed]. A host that installed its own
    // feed and its own model was why the mini bar never moved: it built a SECOND
    // model, read `player.snapshot()` once at composition and never collected
    // the feed (measured 2026-11-23: session PLAYING at 2 000 ms, bar absent).

    val now = state.current
    val opened = NowPlayingState.open
    // The words are asked for when the AUDIO is under way AND somebody is looking
    // at the player (`lib/lyrics.js`'s `playable`, plus rule 1: a request nobody
    // asked for is not sent — the mini bar draws no words).
    val playable = state.snapshot?.state == PlayState.PLAYING && opened
    LaunchedEffect(now?.key, playable, lyrics) {
        lyrics?.onTrack(now?.key, now?.artists ?: "", now?.title ?: "", playable)
    }
    // The active line follows the position the player hands us — the same value
    // the bar draws, so the words and the rail never disagree.
    LaunchedEffect(state.position.positionS, lyrics) {
        lyrics?.onPosition(state.position.positionS)
    }

    val pane = NowPlayingState.pane
    PlayerSheetFrame(
        opened = opened,
        onDismiss = { NowPlayingState.hide() },
        modifier = modifier.fillMaxSize(),
    ) {
        PlayerSurface(
            state = state,
            model = model,
            nav = nav,
            lyrics = lyrics,
            about = about,
            onClose = { NowPlayingState.hide() },
            // Les deux portes du SPA (`MobileNowPlaying.svelte` §9), à leur place
            // dans la rangée du pouce. `PlayerSurface` a les slots depuis MB11 et
            // la coquille n'y passait rien : ni la soirée ni la télécommande
            // n'avaient de porte (audit 5 et 6).
            party = { PartyButton() },
            remote = { RemoteButton() },
        )
    }

    if (pane != Pane.None) {
        when (pane) {
            Pane.Queue -> QueueSheet(
                model = model,
                nav = nav,
                lyrics = lyrics,
                onClose = { NowPlayingState.closePane() },
            )
            Pane.Lyrics -> LyricsSheet(
                lyrics = lyrics,
                state = state,
                onClose = { NowPlayingState.closePane() },
                onCard = {
                    NowPlayingState.showPane(Pane.Card)
                    lyrics?.openCard()
                },
            )
            Pane.About -> AboutSheet(
                about = about,
                player = playerPort,
                nav = nav,
                track = now,
                onClose = { NowPlayingState.closePane() },
            )
            Pane.Card -> {
                val lyricsState = lyrics?.state?.collectAsState()?.value
                LyricCardSheet(
                    state = lyricsState,
                    model = lyrics,
                    card = card,
                    track = now,
                    onClose = {
                        card.reset()
                        lyrics?.closeCard()
                        NowPlayingState.showPane(Pane.Lyrics)
                    },
                )
            }
            Pane.None -> Unit
        }
    }
}

/**
 * The sheet's frame: the player follows the finger down and settles on **distance
 * or velocity** (P2, `plan-mobile-ui-finish.md` §3 and §4).
 *
 * It replaces an `AnimatedVisibility(slideOutVertically)`, which had two defects
 * the person could see: the exit **started at 0** on every path — the chevron, the
 * back key and a drag all jumped back to the top before leaving (`transitions.md`
 * P1's own trap) — and a finger could not move it at all.
 *
 * The numbers are the plan's shared ones, no more: **28 % of the sheet height** or
 * **125 dp/s** decides, the dismissal springs at `spring(0.8f, 400f)` (Apple's
 * Music, WWDC 2018, S4) and the snap back at `spring(1f, 400f)` — 100 % damping,
 * so it never overshoots the top edge. The settle is seeded with the release's
 * own velocity, so the motion continues from the finger instead of restarting.
 *
 * **Why a `Float` and not `AnchoredDraggableState`:** the plan's §4 sketched the
 * anchored draggable, and it cannot hold §3's table — its `flingBehavior` takes ONE
 * animation spec for both targets (so a snap back would carry the dismissal's 80 %
 * damping), it cannot land at once under eco, and a cancelled drag leaves the
 * sheet mid-way instead of returning it. A restricted pointer-input coroutine also
 * may not call `snapTo`/`animateTo`, so the frame would allocate a coroutine per
 * move event. This is `SwipeRow`'s proven shape: one state write per move, ONE
 * settle coroutine cancelled by the next `down` (`docs/agents/transitions.md`
 * S1/S2/S4, M14).
 *
 * **One truth for the three doors.** `NowPlayingState` stays the only state: the
 * chevron and the back key call `onDismiss()`, and a dismissive settle calls it
 * too, so none of them can disagree with the sheet's position. Dismissing from
 * rest animates from the CURRENT offset (never from 0), and a second open starts
 * from wherever the sheet is — the entrance snaps under the screen in the layout
 * phase, so the first painted frame is never a flash of the player mid-air.
 *
 * **A cancel is never a decision** (S8): a gesture cut by the system returns to
 * the anchor it started from.
 *
 * **Reduced motion / eco** (`Sillon.motion.moves == false`): every settle lands at
 * once, exactly as M14 asks — no displacement, no duration. The drag itself still
 * follows the finger: that is the person's own movement, not an animation.
 */
@Composable
internal fun PlayerSheetFrame(
    opened: Boolean,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    var height by remember { mutableFloatStateOf(0f) }
    /** The sheet's top, in px: 0 is open, [height] is closed. */
    var offsetY by remember { mutableFloatStateOf(0f) }
    var visible by remember { mutableStateOf(false) }
    var pendingOpen by remember { mutableStateOf(false) }
    /** Where this drag started, for a cancel that must not decide (S8). */
    var dragStart by remember { mutableFloatStateOf(0f) }
    val scope = rememberCoroutineScope()
    val moves = Sillon.motion.moves
    val tracker = remember { VelocityTracker() }
    val settleJob = remember { mutableStateOf<Job?>(null) }

    // One settle at a time, from the offset the sheet has NOW and with the speed
    // the finger left it: the next `down` cancels it, and so does the next door.
    fun settleTo(target: Float, velocityPx: Float, damping: Float, onDone: () -> Unit = {}) {
        settleJob.value?.cancel()
        if (!moves) {
            offsetY = target
            onDone()
            return
        }
        settleJob.value = scope.launch {
            animate(
                initialValue = offsetY,
                targetValue = target,
                initialVelocity = velocityPx,
                animationSpec = spring(
                    dampingRatio = damping,
                    stiffness = Spring.StiffnessMediumLow,
                    // The settle is DONE when it is within half a pixel: the tail
                    // below that is not motion, and waiting for Compose's 0.01 px
                    // threshold made a dismissal read 752 ms of animation for a
                    // move the eye finishes in ~400 (SwipeRow's own constant).
                    visibilityThreshold = 0.5f,
                ),
            ) { value, _ -> offsetY = value }
            onDone()
        }
    }

    // The latest `opened` for a settle that finishes later than the state it was
    // started by: a re-open in the exit's last frame must not hide the sheet.
    val openedNow by rememberUpdatedState(opened)

    // The entrance — and a re-open that interrupts an exit: both end at 0. The
    // first entrance has no height yet, so the size callback owns its snap.
    LaunchedEffect(opened) {
        if (opened) {
            visible = true
            if (height > 0f) {
                pendingOpen = false
                settleTo(0f, 0f, RETURN_DAMPING)
            } else {
                pendingOpen = true
            }
        } else if (visible) {
            // The chevron and the back key: from the CURRENT offset (never 0).
            settleTo(height, 0f, DISMISS_DAMPING) { if (!openedNow) visible = false }
        }
    }
    BackHandler(enabled = opened) { onDismiss() }

    if (!visible) return
    Box(
        modifier = modifier
            .fillMaxSize()
            .onSizeChanged { size ->
                val measured = size.height.toFloat()
                if (measured <= 0f || measured == height) return@onSizeChanged
                height = measured
                if (pendingOpen) {
                    pendingOpen = false
                    offsetY = measured
                    settleTo(0f, 0f, RETURN_DAMPING)
                } else if (!opened) {
                    // A rotation mid-close: aim at the NEW bottom.
                    settleTo(measured, 0f, DISMISS_DAMPING) { if (!openedNow) visible = false }
                } else {
                    offsetY = offsetY.coerceIn(0f, measured)
                }
            }
            .offset { IntOffset(0, offsetY.roundToInt()) }
            .pointerInput(height) {
                if (height <= 0f) return@pointerInput
                // The shape [SwipeRow] proved (its five gesture tests run on this
                // same Robolectric host): one state write per move, and a claimed
                // gesture consumes only after the platform's slop. The finger's
                // travel BEYOND the slop is the sheet's motion, so the follow is 1:1
                // from the first claimed pixel.
                awaitEachGesture {
                    // Read at GESTURE time: the node's size, not a state that may
                    // still be 0 when the pointer-input block was built.
                    val sheetHeight = size.height.toFloat()
                    if (sheetHeight <= 0f) return@awaitEachGesture
                    val slop = viewConfiguration.touchSlop
                    val flingPx = DISMISS_VELOCITY.toPx()
                    val start = offsetY
                    var claimed = false
                    var cancelled = false
                    var travelY = 0f
                    var travelX = 0f
                    val down = awaitFirstDown(requireUnconsumed = false)
                    tracker.resetTracking()
                    tracker.addPosition(down.uptimeMillis, down.position)
                    try {
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (change.changedToUpIgnoreConsumed()) {
                                // A REAL up is never consumed; Compose delivers a
                                // cancelled gesture as a consumed up (`drag()` returns
                                // false on `isConsumed` for the same reason). That is the
                                // only way to tell them apart, and S8 needs to: a cancel
                                // must not decide.
                                if (change.isConsumed) cancelled = true
                                tracker.addPosition(change.uptimeMillis, change.position)
                                break
                            }
                            if (!change.pressed) {
                                cancelled = true
                                break
                            }
                            val delta = change.positionChange()
                            // Fed from the first move, before the axis is decided: the
                            // tracker needs the travel before the slop, or a flick under
                            // the distance reports no velocity at all (SwipeRow measured
                            // the same with two samples).
                            tracker.addPosition(change.uptimeMillis, change.position)
                            if (!claimed) {
                                travelY += delta.y
                                travelX += delta.x
                                if (abs(travelY) <= slop) continue
                                // A mostly horizontal drag belongs to the cover carousel
                                // or the seek rail; an upward one has nowhere to go (the
                                // sheet is already at 0). Neither is ours: consume
                                // nothing, so the child keeps its gesture.
                                if (abs(travelX) > abs(travelY) || (start <= 0f && travelY < 0f)) {
                                    break
                                }
                                claimed = true
                                settleJob.value?.cancel()
                                change.consume()
                                offsetY = (start + travelY - slop).coerceIn(0f, sheetHeight)
                                continue
                            }
                            change.consume()
                            offsetY = (offsetY + delta.y).coerceIn(0f, sheetHeight)
                        }
                    } catch (interrupted: CancellationException) {
                        // `onCancelPointerInput` cancels this coroutine outright (a
                        // system gesture, a call, another window taking the pointer):
                        // the sheet must not be left half-open, so the cancel still
                        // returns it, then the cancellation continues.
                        if (claimed) settleTo(start, 0f, RETURN_DAMPING)
                        throw interrupted
                    }
                    if (!claimed) return@awaitEachGesture
                    if (cancelled) {
                        // A cancel is never a decision: back to where it started.
                        settleTo(start, 0f, RETURN_DAMPING)
                        return@awaitEachGesture
                    }
                    val velocity = tracker.calculateVelocity().y.let { if (it.isNaN()) 0f else it }
                    val dismiss = offsetY > sheetHeight * 0.28f || velocity > flingPx
                    if (dismiss) {
                        settleTo(sheetHeight, velocity, DISMISS_DAMPING) { onDismiss() }
                    } else {
                        settleTo(0f, velocity, RETURN_DAMPING)
                    }
                }
            },
    ) {
        content()
    }
}

/** The plan's sheet-dismiss speed, `AnchoredDraggableMinFlingVelocity` (S2/S4). */
private val DISMISS_VELOCITY = 125.dp

/**
 * The plan's two springs (S4, Apple's Music on WWDC 2018): 80 % damping on the way
 * out — a little momentum, no rebound — and 100 % on the way back, so the sheet
 * never overshoots the top edge it is returning to. `Spring.StiffnessMediumLow`
 * (400f) is the stiffness both use.
 */
private const val DISMISS_DAMPING = 0.8f
private const val RETURN_DAMPING = 1f

/** The line under the title: what the player is DOING, else who sings. */
internal fun playbackLine(state: NowPlayingModel.State): String = when (state.snapshot?.state) {
    PlayState.BUFFERING -> "Chargement…"
    null, PlayState.STOPPED -> state.current?.artists ?: "Rien en lecture"
    else -> state.errorOrArtists()
}

private fun NowPlayingModel.State.errorOrArtists(): String {
    val cause = snapshot?.error
    if (!cause.isNullOrBlank()) return cause
    return current?.artists ?: "Rien en lecture"
}

/**
 * The playing title as a `TrackRow`, for MB10's ONE context menu: the player's ⋯
 * and a queue row open the same sheet, with the same entries.
 */
internal fun nowPlayingRow(now: uniffi.nz_ffi.NowPlaying): uniffi.nz_ffi.TrackRow =
    uniffi.nz_ffi.TrackRow(
        key = now.key,
        id = now.id,
        title = now.title,
        artists = now.artists,
        album = now.album,
        albumId = now.albumId,
        durationS = now.durationS.toUInt(),
        cover = now.cover,
        explicit = false,
        liked = now.liked,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = now.source,
        position = 0u,
    )

/** The one player of the process, as the views' seam. */
class NzPlayerPort(private val api: uniffi.nz_ffi.PlayerApi) : PlayerPort {
    override fun snapshot(): PlayerSnapshot = api.snapshot()

    override fun position(): uniffi.nz_ffi.PositionInfo = api.position()

    override fun command(cmd: PlayerCommand, tapNs: Long) = api.command(cmd, tapNs.toULong())

    override fun queueSource(): fr.neezer.mobile.designsystem.NzRowSource? =
        runCatching { api.queue().asRowSource() }.getOrNull()

    override fun audio(): uniffi.nz_ffi.AudioSettings = api.audio()

    override fun setAudio(settings: uniffi.nz_ffi.AudioSettings) = api.setAudio(settings)

    override fun setSleep(timer: uniffi.nz_ffi.SleepTimer?) = api.setSleep(timer)
}

/** `NzCore::player().subscribe(listener)` — the process's ONE player. */
internal fun subscribePlayer(listener: PlayerListener) {
    runCatching { NowPlayingCore.player()?.subscribe(listener) }
}

/**
 * The ONE player model of the process — the mini bar and the full-screen player
 * read the same `StateFlow`, fed by the same [PlayerFeed] subscription.
 *
 * Why a singleton and not one per composition: the bar is composed in the shell
 * at every launch and the sheet only when somebody opens it, so two models meant
 * two states — and the bar's copy, built before anything played, never heard a
 * change (measured 2026-11-23: the session said PLAYING at 2 000 ms while the
 * bar stayed absent, which also made the full-screen player unreachable from a
 * phone). One holder, one feed, one state: a second surface can only disagree if
 * it invents its own.
 *
 * The subscription is installed HERE and not in a composable's `LaunchedEffect`:
 * an effect is cancelled with its composition (the sheet closes, the bar
 * recomposes), and a player that stops being observed when a screen goes away is
 * the old app's frozen-page bug with extra steps.
 */
internal object NowPlayingSession {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    /** The one model, built on first use (no core → a silent player). */
    val model: NowPlayingModel by lazy {
        // The feed's port IS the process's player: install it first (a no-op when
        // a test already owns the feed) and build the model on what the feed
        // holds. Building it on `NowPlayingCore.player()` instead made the bar a
        // second reader of a different object, and `refresh()` then overwrote the
        // feed's first snapshot with an empty one (measured: a PLAYING snapshot
        // installed before composition vanished).
        NowPlayingCore.player()?.let(::NzPlayerPort)?.let { port ->
            PlayerFeed.install(port, subscribe = { listener -> subscribePlayer(listener) })
        }
        val model = NowPlayingModel(PlayerFeed.port, NowPlayingCore.service() ?: NoService, scope)
        // The first picture, BEFORE the collector: a snapshot that arrived while
        // the model was being built must be the last word, not the first.
        model.refresh()
        scope.launch {
            PlayerFeed.snapshot.filterNotNull().collect { snapshot ->
                model.onSnapshot(snapshot)
                // A podcast's rate: the show's own, applied to the engine — on
                // this thread, never inside the Rust callback that pushed it.
                PodcastSpeed.reconcile(PlayerFeed.port, snapshot)
            }
        }
        scope.launch {
            PlayerFeed.queueVersion.collect { model.onQueueChanged(it) }
        }
        model
    }
}

/** Both surfaces take this: the process's one model. */
@Composable
private fun rememberNowPlaying(): NowPlayingModel = NowPlayingSession.model

/** A service that answers nothing, for a process without a core. */
private object NoService : NowPlayingService {
    override suspend fun cachedLyrics(key: String) = null
    override suspend fun lyrics(key: String) = uniffi.nz_ffi.LyricsData(
        synced = emptyList(),
        plain = "",
        source = "",
        empty = true,
        bytes = 0uL,
    )
    override suspend fun similarTracks(key: String) = null
    override fun cardLines(lyrics: uniffi.nz_ffi.LyricsData) = emptyList<uniffi.nz_ffi.CardLineData>()
    override fun cardDefaultLine(lyrics: uniffi.nz_ffi.LyricsData, activeSource: Long) = -1
    override fun cardToggle(selected: List<Int>, index: Int) = CardSelection(selected, null)
    override fun cardTexts(lyrics: uniffi.nz_ffi.LyricsData, selected: List<Int>) = emptyList<String>()
    override fun cardFilename(artist: String, title: String) = "neezer-titre.png"
    override fun speedChoices() = listOf(0.75f, 1f, 1.25f, 1.5f, 1.75f, 2f, 2.5f, 3f)
    override fun speed(show: String) = 1f
    override fun setSpeed(show: String, rate: Float) = Unit
    override suspend fun coverPath(url: String, sizePx: Int): String? = null
    override suspend fun cachedCredits(key: String) = null
    override suspend fun credits(key: String) = uniffi.nz_ffi.TrackCreditsData(
        credits = emptyList(),
        info = uniffi.nz_ffi.CreditsInfoData(null, null, null, null, null),
        empty = true,
    )
}
