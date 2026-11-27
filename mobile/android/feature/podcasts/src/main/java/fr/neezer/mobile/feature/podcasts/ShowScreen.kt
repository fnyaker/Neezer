package fr.neezer.mobile.feature.podcasts

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwitch
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import fr.neezer.mobile.designsystem.nzPaneEnter
import fr.neezer.mobile.designsystem.rememberListWindow
import fr.neezer.mobile.designsystem.rememberPaneEnter
import fr.neezer.mobile.feature.offline.DownloadButton
import kotlinx.coroutines.delay
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.EpisodeData
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.TranscriptHitData

/**
 * Podcast — lot MB13 (docs/agents/mobile.md, feature inventory).
 *
 * `routes/Show.svelte` ported: the header, « Lire », « Tout télécharger »
 * (MB15's `DownloadButton`, never a hand-made one), the archive switch, the
 * transcription card, the search inside the transcripts, and the episode list —
 * windowed over a `ListHandle` and paged by cursor, so a show of 2 000 episodes
 * scrolls and never loads whole.
 *
 * The sheet it hosts is [EpisodePanelSheet]: the SPA's *À propos* panel for a
 * podcast (Chapitres · Transcription, then Description and Informations).
 */
@Composable
fun ShowEntry(
    id: String,
    nav: NzNavigator,
    modifier: Modifier = Modifier,
    service: PodcastService? = AppPodcasts.service(),
    player: PodcastsPlayer = AppPodcasts.player(),
    isAdmin: Boolean = AppPodcasts.isAdmin(),
) {
    val scope = rememberCoroutineScope()
    val model = remember(service, player, scope) { service?.let { ShowModel(it, player, scope) } }
    LaunchedEffect(model, id) { model?.load(id) }
    if (model == null) {
        CoreUnavailable(modifier = modifier)
        return
    }
    val state by model.state.collectAsState()
    val sheetOpen = state.sheet != null
    val snapshot by rememberPlayerState(player, sheetOpen)
    val position by rememberPosition(
        player = player,
        active = sheetOpen && snapshot?.current?.key == state.sheet,
    )

    // The panel's poll: while this episode is being transcribed and the sheet is
    // open, one `limit=1` read each 10 s (`TranscriptPanel#poll`).
    LaunchedEffect(state.sheet) {
        val sheet = state.sheet ?: return@LaunchedEffect
        model.watchTranscript(sheet)
    }

    ShowScreen(
        state = state,
        actions = ShowActions(
            retry = model::retry,
            onVisible = model::onVisible,
            loadMore = model::loadMore,
            play = model::play,
            playAll = model::playAll,
            playAt = model::playAt,
            toggleAuto = model::toggleAuto,
            retryTranscribe = { model.loadTranscribeSettings(force = true) },
            toggleTranscribe = model::toggleShowTranscribe,
            setLanguage = model::setLanguage,
            setBacklog = model::setBacklog,
            setBatchCount = model::setBatchCount,
            startBatch = model::startBatch,
            startOne = model::startOne,
            stopOne = model::stopTranscript,
            setSearchQuery = model::setSearchQuery,
            retrySearch = model::retrySearch,
            clearSearch = model::clearSearch,
            toggleChapters = { episode -> model.loadChapters(episode) },
            openSheet = model::openSheet,
            addMarker = { episode, seconds -> model.addMarker(episode, seconds) },
            removeMarker = model::removeMarker,
            openEpisode = { episodeId -> model.openSheet(episodeId) },
            consumeMessage = model::consumeMessage,
        ),
        isAdmin = isAdmin,
        currentKey = snapshot?.current?.key,
        playing = snapshot?.state == uniffi.nz_ffi.PlayState.PLAYING,
        positionS = position,
        modifier = modifier,
    )

    // The sheet: the episode the person opened, its chapters and its transcript.
    val sheet = state.sheet
    if (sheet != null) {
        val card = state.cards.values.firstOrNull { it.id == sheet }
        EpisodePanelSheet(
            episode = card,
            service = service,
            chapters = state.chapters[sheet],
            chaptersBusy = state.chapterBusy.contains(sheet),
            chaptersFailure = state.chapterFailure[sheet],
            markers = state.markers[sheet] ?: card?.markers.orEmpty(),
            isCurrent = currentKeyIs(snapshot, sheet),
            positionS = position,
            admin = isAdmin,
            onLoadChapters = { model.loadChapters(sheet, force = true) },
            onSeek = { seconds -> model.playAt(sheet, seconds) },
            onResume = { model.playAt(sheet, 0.0) },
            onAddMarker = { seconds -> model.addMarker(sheet, seconds) },
            onRemoveMarker = model::removeMarker,
            onStartTranscript = { model.startOne(sheet) },
            onStopTranscript = { model.stopTranscript(sheet) },
            onRestartTranscript = { language -> model.restartTranscript(sheet, language) },
            onDeleteTranscript = { model.deleteTranscript(sheet) },
            onOpenShow = { showId -> nav.go(AppLink.Podcast(showId)) },
            onDismiss = model::closeSheet,
        )
    }
}

private fun currentKeyIs(snapshot: PlayerSnapshot?, episodeId: String): Boolean =
    snapshot?.current?.key == episodeId

/** What the screen may ask of its model — one function per gesture. */
data class ShowActions(
    val retry: () -> Unit,
    val onVisible: (Int, Int, Boolean) -> Unit,
    val loadMore: () -> Unit,
    val play: (Int) -> Unit,
    val playAll: () -> Unit,
    val playAt: (String, Double) -> Unit,
    val toggleAuto: () -> Unit,
    val retryTranscribe: () -> Unit,
    val toggleTranscribe: () -> Unit,
    val setLanguage: (String) -> Unit,
    val setBacklog: (String) -> Unit,
    val setBatchCount: (Int) -> Unit,
    val startBatch: () -> Unit,
    val startOne: (String) -> Unit,
    val stopOne: (String) -> Unit,
    val setSearchQuery: (String) -> Unit,
    val retrySearch: () -> Unit,
    val clearSearch: () -> Unit,
    val toggleChapters: (String) -> Unit,
    val openSheet: (String) -> Unit,
    val addMarker: (String, Double) -> Unit,
    val removeMarker: (uniffi.nz_ffi.EpisodeMarkerData) -> Unit,
    val openEpisode: (String) -> Unit,
    val consumeMessage: () -> Unit,
)

/** The show page itself: a pure function of the model's state. */
@Composable
fun ShowScreen(
    state: ShowModel.State,
    actions: ShowActions,
    isAdmin: Boolean,
    currentKey: String?,
    playing: Boolean,
    positionS: Double,
    modifier: Modifier = Modifier,
    toast: @Composable (String) -> Unit = {},
) {
    val colors = Sillon.colors
    // The page's phases are one pane (X4's decision note, D2): the body is a
    // `LazyColumn`, so an `AnimatedContent` around it would carry the show's
    // header with it. The header's item stays out and every item of the panel
    // carries `Modifier.nzPaneEnter(pane)`; a re-emit of the same phase plays
    // nothing, and a page of the window landing does not change the phase.
    val pane = rememberPaneEnter(showPhase(state))
    val enter = Modifier.nzPaneEnter(pane)
    Box(
        modifier
            .fillMaxSize()
            .semantics { contentDescription = SHOW_MARKER }
            .background(colors.bg),
    ) {
        when {
            state.loading && state.show == null -> {
                Column(enter.padding(NzSpacing.pageX)) {
                    Skeleton(kind = NzSkeletonKind.Header, count = 1)
                    Skeleton(kind = NzSkeletonKind.List, count = 6)
                }
            }
            state.show == null -> {
                LoadFailed(
                    cause = state.failure ?: "cause inconnue",
                    onRetry = actions.retry,
                    modifier = enter.padding(NzSpacing.pageX),
                    title = "Podcast introuvable",
                )
            }
            else -> ShowBody(state, actions, isAdmin, currentKey, playing, positionS, enter)
        }
        state.message?.let { message ->
            toast(message)
            LaunchedEffect(message) { actions.consumeMessage() }
        }
    }
}

/**
 * What the page is drawing: the show arrived and holds episodes, the show
 * arrived and holds none, the read failed, or nothing yet — the pane's key (D2),
 * from the same fields the branches above test.
 */
private enum class Phase { Loading, Failed, Empty, Content }

private fun showPhase(state: ShowModel.State): Phase = when {
    state.show == null && state.loading -> Phase.Loading
    state.show == null -> Phase.Failed
    state.loaded == 0 -> Phase.Empty
    else -> Phase.Content
}

@Composable
private fun ShowBody(
    state: ShowModel.State,
    actions: ShowActions,
    isAdmin: Boolean,
    currentKey: String?,
    playing: Boolean,
    positionS: Double,
    enter: Modifier,
) {
    val colors = Sillon.colors
    val show = state.show ?: return
    val listState = rememberLazyListState()
    val handle = state.handle
    val window = handle?.let { rememberListWindow(it) }
    val rowsOnScreen = 12
    // An episode inserted or removed moves the rows below it over 200 ms, and a
    // MARK added inside a row moves them the same way once that row grows (M6:
    // no mount fade, M4; a 140 ms fade on the row that leaves). Measured: X5's
    // `ShowScreenMotionTest`.
    val specs = Sillon.motion.itemSpecs()

    LaunchedEffect(listState, window) {
        if (window == null) return@LaunchedEffect
        snapshotFlow {
            Triple(
                listState.firstVisibleItemIndex,
                listState.isScrollInProgress,
                window.len,
            )
        }.collect { (first, scrolling, len) ->
            actions.onVisible(first, minOf(len - 1, first + rowsOnScreen), scrolling)
        }
    }

    LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
        // The header is NOT part of the panel: a phase change leaves it where it
        // is (D2, measured ± 0 dp in the change's first frame).
        item(key = "header") {
            Column(
                modifier = Modifier.padding(
                    start = NzSpacing.pageX,
                    end = NzSpacing.pageX,
                    top = NzSpacing.s,
                    bottom = NzSpacing.m,
                ),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Row(verticalAlignment = Alignment.Bottom) {
                    CoverImage(url = show.cover, sizeDp = 96.dp, seed = show.id)
                    Column(
                        modifier = Modifier
                            .padding(start = NzSpacing.l)
                            .weight(1f),
                        verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
                    ) {
                        Text(
                            text = if (show.local) "Podcast · Local" else "Podcast",
                            style = Sillon.type.labelMono,
                            color = colors.dim,
                        )
                        Text(
                            text = show.title,
                            style = Sillon.type.title,
                            color = colors.tx,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            text = PodcastsCopy.episodeCount(state.loaded),
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                        )
                    }
                }
                if (show.local) {
                    Text(
                        text = PodcastsCopy.LOCAL_NOTE,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                if (show.description.isNotBlank()) {
                    ExpandableText(show.description)
                }
                if (state.stale) {
                    Text(
                        text = PodcastsCopy.STALE,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                Row(
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(top = NzSpacing.xs),
                ) {
                    NzButton(
                        text = "Lire",
                        onClick = actions.playAll,
                        glyph = NzGlyph.Play,
                        enabled = state.loaded > 0,
                    )
                    // MB15's button, never a hand-made one: it already knows its
                    // five states, its failures and its cause.
                    DownloadButton(context = "show:${show.id}", label = "Tout télécharger")
                }
            }
        }

        if (isAdmin && !show.local) {
            item(key = "archive") {
                ArchiveSwitch(
                    state = state,
                    onToggle = actions.toggleAuto,
                    modifier = enter.padding(horizontal = NzSpacing.pageX),
                )
            }
        }

        if (isAdmin && state.transcribeSupported) {
            item(key = "transcribe") {
                TranscriptionCard(
                    state = state,
                    actions = actions,
                    modifier = enter.padding(
                        horizontal = NzSpacing.pageX,
                        vertical = NzSpacing.s,
                    ),
                )
            }
        }

        if (state.showSearch) {
            item(key = "search") {
                TranscriptSearchField(
                    state = state,
                    actions = actions,
                    modifier = enter.padding(
                        horizontal = NzSpacing.pageX,
                        vertical = NzSpacing.s,
                    ),
                )
            }
            if (state.isSearching) {
                item(key = "hits") {
                    SearchHits(state, actions, enter.padding(horizontal = NzSpacing.pageX))
                }
            }
        }

        val showRows = !state.isSearching
        if (showRows) {
            if (state.loaded == 0) {
                item(key = "empty") {
                    EmptyState(
                        title = "Aucun épisode",
                        body = PodcastsCopy.NO_EPISODE,
                        modifier = enter,
                        glyph = NzGlyph.Mic,
                    )
                }
            } else {
                items(count = state.loaded, key = { index -> state.cards[index]?.id ?: "skeleton:$index" }) { index ->
                    val card = state.cards[index]
                    // `enter` FIRST in the item's chain (D2), then the placement.
                    val motion = enter.animateItem(specs.fadeIn, specs.placement, specs.fadeOut)
                    if (card == null) {
                        Skeleton(
                            kind = NzSkeletonKind.List,
                            count = 1,
                            modifier = motion.padding(horizontal = NzSpacing.pageX),
                        )
                    } else {
                        EpisodeRow(
                            episode = card,
                            index = index,
                            isCurrent = currentKey == card.id,
                            playing = playing && currentKey == card.id,
                            positionS = if (currentKey == card.id) positionS else 0.0,
                            chapters = state.chapters[card.id],
                            chaptersBusy = state.chapterBusy.contains(card.id),
                            chaptersFailure = state.chapterFailure[card.id],
                            onPlay = { actions.play(index) },
                            onOpen = { actions.openEpisode(card.id) },
                            onToggleChapters = { actions.toggleChapters(card.id) },
                            onSeek = { seconds -> actions.playAt(card.id, seconds) },
                            onRemoveMarker = actions.removeMarker,
                            modifier = motion.padding(horizontal = NzSpacing.pageX),
                        )
                    }
                }
            }
            if (state.next != null || state.loadingMore) {
                item(key = "more") {
                    Row(
                        modifier = enter
                            .fillMaxWidth()
                            .padding(NzSpacing.pageX),
                        horizontalArrangement = Arrangement.Center,
                    ) {
                        NzButton(
                            text = if (state.loadingMore) "Chargement…" else "Charger plus d'épisodes",
                            onClick = actions.loadMore,
                            kind = NzButtonKind.Ghost,
                            enabled = !state.loadingMore,
                        )
                    }
                }
            }
            if (state.moreFailure != null) {
                item(key = "more-error") {
                    LoadFailed(
                        cause = state.moreFailure,
                        onRetry = actions.loadMore,
                        modifier = enter.padding(horizontal = NzSpacing.pageX),
                        title = "Les épisodes suivants n'ont pas pu être chargés",
                    )
                }
            }
            item(key = "footer") { Spacer(enter.height(NzSpacing.pageY)) }
        }
    }
}

/** A description that unfolds on a tap (`Show.svelte#toggleExpand`). */
@Composable
private fun ExpandableText(text: String) {
    val colors = Sillon.colors
    var open by remember(text) { mutableStateOf(false) }
    Column {
        Text(
            text = text,
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = if (open) Int.MAX_VALUE else 3,
            overflow = TextOverflow.Ellipsis,
        )
        Row(
            modifier = Modifier
                .heightIn(min = NzMinTouchTarget)
                .clickable(role = Role.Button) { open = !open }
                .semantics { contentDescription = if (open) "Réduire la description" else "Lire la suite" },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = if (open) "Réduire" else "Lire la suite",
                style = Sillon.type.label,
                color = colors.btn,
            )
            Icon(
                glyph = if (open) NzGlyph.ChevronUp else NzGlyph.ChevronDown,
                size = 15.dp,
                tint = colors.btn,
            )
        }
    }
}

/**
 * « Archiver automatiquement les nouveaux épisodes » — the show's own switch,
 * with the line that says what it has done and the bar that moves with it
 * (`Show.svelte`'s `.auto` row).
 */
@Composable
private fun ArchiveSwitch(
    state: ShowModel.State,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val show = state.show ?: return
    NzCard(modifier = modifier.fillMaxWidth(), onClick = onToggle) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                modifier = Modifier
                    .size(36.dp)
                    .background(
                        colors.ac.copy(alpha = 0.16f),
                        RoundedCornerShape(NzRadius.field),
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Icon(glyph = NzGlyph.Archive, size = 18.dp, tint = colors.acText)
            }
            Column(
                Modifier
                    .weight(1f)
                    .padding(horizontal = NzSpacing.m),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
            ) {
                Text(
                    text = "Archiver automatiquement les nouveaux épisodes",
                    style = Sillon.type.label,
                    color = colors.tx,
                )
                Text(
                    text = when {
                        show.autoArchive && state.archiving -> "Archivage en cours · ${show.archiveLine}"
                        show.autoArchive -> "Activé · ${show.archiveLine}"
                        else -> "Désactivé · ${show.archiveLine}"
                    },
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
            NzSwitch(
                checked = show.autoArchive,
                onCheckedChange = { onToggle() },
                enabled = !state.archiveBusy,
                // The sentence is DRAWN beside it, so it is the switch's NAME,
                // not its label: `NzSwitch(label = …)` renders a second copy.
                modifier = Modifier.semantics {
                    contentDescription = "Archiver automatiquement les nouveaux épisodes"
                },
            )
        }
    }
}

/**
 * « Transcription des épisodes » — the admin card: the switch, the catch-up
 * mode, the language, the estimate and the batch (`Show.svelte`'s `.tr-card`).
 */
@Composable
private fun TranscriptionCard(
    state: ShowModel.State,
    actions: ShowActions,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val settings = state.transcribeSettings
    NzCard(modifier = modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = "Transcription des épisodes",
                    style = Sillon.type.label,
                    color = colors.tx,
                    modifier = Modifier.weight(1f),
                )
                Text(
                    text = "${state.readyCount} sur ${state.archivedCount}",
                    style = Sillon.type.labelMono,
                    color = colors.dim,
                )
            }
            Text(
                text = PodcastsCopy.TRANSCRIPT_LOCAL,
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            when {
                settings == null && state.transcribeFailure != null -> {
                    Text(
                        text = "Impossible de vérifier la transcription.",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                    NzButton(
                        text = "Réessayer",
                        onClick = actions.retryTranscribe,
                        kind = NzButtonKind.Ghost,
                        glyph = NzGlyph.Refresh,
                    )
                }
                settings == null -> {
                    Skeleton(kind = NzSkeletonKind.List, count = 1)
                }
                settings.available -> {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                text = "Transcrire automatiquement les nouveaux épisodes",
                                style = Sillon.type.body,
                                color = colors.tx,
                            )
                        }
                        NzSwitch(
                            checked = state.show?.autoTranscribe == true,
                            onCheckedChange = { actions.toggleTranscribe() },
                            enabled = !state.transcribeBusy,
                            modifier = Modifier.semantics {
                                contentDescription =
                                    "Transcrire automatiquement les nouveaux épisodes"
                            },
                        )
                    }
                    ChoiceRow(
                        title = "Rattrapage à l'activation",
                        options = listOf(
                            "all" to "Tout l'historique archivé",
                            "recent" to "Les 10 épisodes archivés les plus récents",
                            "none" to "Aucun rattrapage",
                        ),
                        selected = state.backlog,
                        onSelect = actions.setBacklog,
                    )
                    ChoiceRow(
                        title = "Langue de l'émission",
                        options = TRANSCRIBE_LANGUAGES.map { code ->
                            code to PodcastsCopy.language(code)
                        },
                        selected = state.show?.transcribeLanguage ?: "auto",
                        onSelect = actions.setLanguage,
                    )
                    if (state.eligible.isNotEmpty()) {
                        Text(
                            text = PodcastsCopy.estimate(
                                state.eligible.size,
                                state.audioSeconds,
                            ),
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                        )
                        FlowRow(
                            verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                        ) {
                            state.batchChoices.forEach { count ->
                                NzChip(
                                    text = if (count > 1) "$count épisodes" else "1 épisode",
                                    selected = state.batchCount == count,
                                    onClick = { actions.setBatchCount(count) },
                                )
                            }
                        }
                        NzButton(
                            text = if (state.transcribeBusy) "Envoi…" else "Transcrire",
                            onClick = actions.startBatch,
                            enabled = !state.transcribeBusy,
                            glyph = NzGlyph.Transcript,
                        )
                    } else {
                        Text(
                            text = "Aucun épisode archivé à transcrire.",
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                        )
                    }
                    state.activeTranscript?.let { active ->
                        TranscriptProgress(active, onStop = { actions.stopOne(active.id) })
                    }
                    if (state.queuedCount > 0) {
                        Text(
                            text = if (state.queuedCount > 1) {
                                "${state.queuedCount} épisodes en attente de transcription."
                            } else {
                                "1 épisode en attente de transcription."
                            },
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                        )
                    }
                }
                else -> {
                    Text(
                        text = PodcastsCopy.reason(settings.reason),
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
            }
        }
    }
}

@Composable
private fun TranscriptProgress(episode: EpisodeData, onStop: () -> Unit) {
    val colors = Sillon.colors
    val info = episode.transcript
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                text = episode.title,
                style = Sillon.type.bodySmall,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = "${info?.percent ?: 0} %",
                style = Sillon.type.labelMono,
                color = colors.dim,
            )
        }
        NzButton(text = "Arrêter", onClick = onStop, kind = NzButtonKind.Ghost)
    }
}

/** A labelled row of exclusive choices (the SPA's two `<select>`s). */
@Composable
private fun ChoiceRow(
    title: String,
    options: List<Pair<String, String>>,
    selected: String,
    onSelect: (String) -> Unit,
) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Text(text = title, style = Sillon.type.bodySmall, color = colors.dim)
        // A FLOW, not a row: at 360 dp « Les 10 épisodes archivés les plus
        // récents » beside « Tout l'historique archivé » does not fit, and a
        // crushed chip (one letter per line) is a bug, not a compromise.
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            options.forEach { (value, label) ->
                NzChip(
                    text = label,
                    selected = selected == value,
                    onClick = { onSelect(value) },
                )
            }
        }
    }
}

/** The search field and its states (`runShowSearch`). */
@Composable
private fun TranscriptSearchField(
    state: ShowModel.State,
    actions: ShowActions,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.chip)
    Column(modifier, verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        BasicTextField(
            value = state.searchQuery,
            onValueChange = actions.setSearchQuery,
            singleLine = true,
            textStyle = Sillon.type.body.copy(color = colors.tx),
            cursorBrush = SolidColor(colors.acText),
            keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                imeAction = androidx.compose.ui.text.input.ImeAction.Search,
            ),
            modifier = Modifier
                .fillMaxWidth()
                .height(NzMinTouchTarget)
                .semantics { contentDescription = "Rechercher dans les épisodes" },
            decorationBox = { inner ->
                Row(
                    modifier = Modifier
                        .fillMaxSize()
                        .background(colors.s1, shape)
                        .padding(horizontal = NzSpacing.m),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                ) {
                    Icon(glyph = NzGlyph.SearchText, size = 16.dp, tint = colors.dim)
                    Box(Modifier.weight(1f)) {
                        if (state.searchQuery.isEmpty()) {
                            Text(
                                text = "Rechercher dans les épisodes…",
                                style = Sillon.type.body,
                                color = colors.dim,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        inner()
                    }
                    if (state.searchQuery.isNotEmpty()) {
                        NzIconButton(
                            glyph = NzGlyph.Close,
                            contentDescription = "Effacer la recherche",
                            onClick = actions.clearSearch,
                            size = 14.dp,
                        )
                    }
                }
            },
        )
        val term = state.searchQuery.trim()
        when {
            term.length == 1 -> Text(
                text = PodcastsCopy.SEARCH_MIN,
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            state.searchFailure != null -> Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .semantics {
                        liveRegion = LiveRegionMode.Polite
                    },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = "La recherche a échoué.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    modifier = Modifier.weight(1f),
                )
                NzButton(
                    text = "Réessayer",
                    onClick = actions.retrySearch,
                    kind = NzButtonKind.Ghost,
                )
            }
        }
    }
}

/** The hits of a show search, with their moment and the excerpt's marks. */
@Composable
private fun SearchHits(state: ShowModel.State, actions: ShowActions, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    val search = state.search
    Column(modifier, verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        when {
            state.searchBusy -> Text(
                text = "Recherche dans les épisodes…",
                style = Sillon.type.bodySmall,
                color = colors.dim,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            )
            search == null || search.hits.isEmpty() -> Text(
                text = PodcastsCopy.noHit(state.searchQuery.trim()),
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
            else -> {
                Text(
                    text = PodcastsCopy.hitCount(search.total.toInt()),
                    style = Sillon.type.labelMono,
                    color = colors.dim,
                )
                if (search.truncated) {
                    Text(
                        text = PodcastsCopy.searchScope(search.episodes.toInt()),
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                search.hits.forEach { hit ->
                    TranscriptHitRow(hit, onPick = { actions.playAt(hit.episodeId, hit.start) })
                }
            }
        }
    }
}

@Composable
private fun TranscriptHitRow(hit: TranscriptHitData, onPick: () -> Unit) {
    val colors = Sillon.colors
    NzCard(modifier = Modifier.fillMaxWidth(), onClick = onPick) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                Text(
                    text = PodcastsCopy.duration(hit.start.toInt()),
                    style = Sillon.type.labelMono,
                    color = colors.btn,
                )
                Text(
                    text = hit.episodeTitle,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                text = hit.text.replace("[", "").replace("]", ""),
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** The languages the transcription picker offers (`transcript.js#LANGUAGES`). */
internal val TRANSCRIBE_LANGUAGES = listOf("auto", "fr", "en", "es", "de", "it", "pt")

/** A confirmation for the admin's « Se désabonner ». */
@Composable
internal fun UnsubscribeDialog(title: String, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    val colors = Sillon.colors
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Se désabonner de ce podcast ?", style = Sillon.type.titleSmall) },
        text = {
            Text(
                "« $title » quittera la liste des abonnements. Ce qui est déjà archivé reste " +
                    "sur le serveur et jouable.",
                style = Sillon.type.body,
                color = colors.dim,
            )
        },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text("Se désabonner", color = colors.btn)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Annuler", color = colors.dim) }
        },
    )
}

/** The player's state, refreshed while [active] — the sheet is the only caller. */
@Composable
internal fun rememberPlayerState(
    player: PodcastsPlayer,
    active: Boolean,
): androidx.compose.runtime.State<PlayerSnapshot?> =
    produceState(initialValue = player.snapshot(), active) {
        if (!active) return@produceState
        while (true) {
            value = player.snapshot()
            delay(500)
        }
    }

/** The hot position, read while [active] (a visible bar or chapter card). */
@Composable
internal fun rememberPosition(player: PodcastsPlayer, active: Boolean): androidx.compose.runtime.State<Double> =
    produceState(initialValue = player.position()?.positionS ?: 0.0, active) {
        if (!active) return@produceState
        while (true) {
            value = player.position()?.positionS ?: value
            delay(250)
        }
    }

/**
 * Le marqueur de la page d'une émission, pour la même raison que
 * [PODCASTS_MARKER] : le stub MB0 supprimé par ce lot, aucun titre d'émission
 * n'est un littéral (il vient du serveur). Posé sur la racine, il est présent
 * dans tous les états — squelettes, page chargée, échec.
 */
const val SHOW_MARKER = "Podcast Neezer"
