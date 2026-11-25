package fr.neezer.mobile.feature.nowplaying

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
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
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.currentStateAsState
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzBlurredBackdrop
import fr.neezer.mobile.designsystem.asRowSource
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzDuration
import fr.neezer.mobile.feature.nowplaying.NowPlayingState.Pane
import fr.neezer.mobile.feature.party.PartyButton
import fr.neezer.mobile.feature.remote.RemoteButton
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
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
 * Empty until something plays (« Rien en lecture »), and a tap ANYWHERE on the
 * card opens the full-screen player (the SPA's `cardTap`): the sleeve and the
 * names are one target, and the play/pause button answers for itself.
 */
@Composable
fun MiniPlayer(modifier: Modifier = Modifier, onClick: () -> Unit = {}) {
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

    if (!state.playing && now == null) {
        // The skeleton's rule: a bar with nothing in it is not a lie, it is a
        // strip 64 dp tall for a player that does not exist. The SPA shows it
        // empty (« Rien en lecture ») only once a player EXISTS, so the strip
        // appears with the first title and never before.
        return
    }

    val colors = Sillon.colors
    val progress = if (state.durationS > 0.0) {
        (state.position.positionS / state.durationS).coerceIn(0.0, 1.0).toFloat()
    } else {
        0f
    }
    Box(modifier = modifier.fillMaxWidth().background(colors.glassBg)) {
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
            CoverImage(url = now?.cover, sizeDp = 44.dp, seed = now?.id)
            Column(Modifier.weight(1f)) {
                Text(
                    text = now?.title ?: "Rien en lecture",
                    style = Sillon.type.body,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = playbackLine(state),
                    style = Sillon.type.bodySmall,
                    color = if (state.snapshot?.state == PlayState.BUFFERING) colors.acText else colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            NzIconButton(
                glyph = if (state.playing) NzGlyph.Pause else NzGlyph.Play,
                contentDescription = if (state.playing) "Pause" else "Lecture",
                onClick = { model.toggle() },
            )
        }
        // The card's own 2 dp line along its bottom edge: played on the accent,
        // the rest on the hairline — the SPA's `.mini-progress`.
        Box(
            modifier = Modifier
                .align(Alignment.BottomStart)
                .fillMaxWidth(progress)
                .height(2.dp)
                .background(colors.acMark),
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
    AnimatedVisibility(
        visible = opened,
        enter = slideInVertically(initialOffsetY = { it }),
        exit = slideOutVertically(targetOffsetY = { it }),
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
