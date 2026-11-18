package fr.neezer.mobile.feature.search

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.ListWindow
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
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import fr.neezer.mobile.designsystem.nzDuration
import fr.neezer.mobile.feature.discovery.ExplorerStylesSection
import uniffi.nz_ffi.EntityKind
import uniffi.nz_ffi.ImportOffer
import uniffi.nz_ffi.SearchEntity
import uniffi.nz_ffi.SearchFallback
import uniffi.nz_ffi.SearchScope
import uniffi.nz_ffi.TopResult
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.TranscriptHit

/**
 * The search screen's root marker, for TalkBack and for the smoke
 * (`mobile/tools/ci-emulator.sh`): a LITERAL, present in every state — the empty
 * page, the skeletons, a failed load — and independent of the query and of the
 * clock, so a needle taken from it cannot drift (MB5's lesson on `HOME_MARKER`).
 */
const val SEARCH_MARKER = "Recherche Neezer"

/**
 * Everything the search can do, as one value.
 *
 * The screen is a pure function of [SearchModel.State] and these callbacks, so
 * the screenshot test draws the REAL screen from a fixture with no core, no
 * player and no network.
 */
data class SearchActions(
    val query: (String) -> Unit = { _ -> },
    val submit: () -> Unit = {},
    val retry: () -> Unit = {},
    val chooseScope: (SearchScope) -> Unit = { _ -> },
    /** The YouTube chip: the fallback off for this screen (0 requests). */
    val toggleYoutube: () -> Unit = {},
    val tab: (SearchTab) -> Unit = { _ -> },
    val useRecent: (String) -> Unit = { _ -> },
    val clearRecent: () -> Unit = {},
    /** « Lancer la radio » (44 dp, full width under the field on a phone). */
    val launchRadio: () -> Unit = {},
    /** The fallback's own « Réessayer ». */
    val retryFallback: () -> Unit = {},
    /** A title: play it, then its radio (the SPA's `TrackList radio`). */
    val playTrack: (TrackRow) -> Unit = { _ -> },
    /** A row's ⋯ — MB10's ONE context menu. */
    val more: (TrackRow) -> Unit = { _ -> },
    /** An entity card: its page (`AppLink.Artist`/`Album`/`Playlist`/`Podcast`). */
    val openEntity: (EntityKind, String) -> Unit = { _, _ -> },
    /** The best result's own control: play the artist, subscribe to the show. */
    val playTop: (TopResult) -> Unit = { _ -> },
    /** The transcript hit: the episode at that moment (MB13's panel). */
    val openPassage: (TranscriptHit) -> Unit = { _ -> },
    /** The import offer: MB9's route for a pasted playlist link. */
    val importLink: (ImportOffer) -> Unit = { _ -> },
)

/** The headings of the shelves: the four families, and the titles above them. */
enum class SearchSection { Titles, Artists, Albums, Playlists, Shows }

/** One block of the page: the list is BUILT by [searchBlocks], so its order is testable. */
sealed interface SearchBlock {
    /** The page's title, the source chips, the field and « Lancer la radio ». */
    data object Header : SearchBlock

    /** The whole page when the box holds nothing: « Récents », the lead, the styles. */
    data object Landing : SearchBlock

    /** MB14's style tiles, on the empty state. */
    data object Styles : SearchBlock

    /** Tout · Titres · Artistes · Albums · Playlists. */
    data object Tabs : SearchBlock

    /** The podcasts scope's two tabs, for an admin. */
    data object PodcastTabs : SearchBlock

    /** « Meilleur résultat ». */
    data object TopCard : SearchBlock

    /** A shelf heading, with « Tout afficher » when [more] is not zero. */
    data class Section(val section: SearchSection, val more: Int) : SearchBlock

    /** One title of the list, by index in the answer. */
    data class Track(val index: Int) : SearchBlock

    /** A sideways rail of cards ("Tout") or a two-column grid (the family's own tab). */
    data class Shelf(val section: SearchSection, val grid: Boolean) : SearchBlock

    /** What the fallback is doing, when it has nothing to add (`Search.svelte`'s line). */
    data class Fallback(val status: FallbackStatus) : SearchBlock

    /** One spoken passage. */
    data class Passage(val index: Int) : SearchBlock

    /** « Hors ligne · version mémorisée ». */
    data object Stale : SearchBlock

    data object Loading : SearchBlock

    /** A quiet line: an empty answer, a count, the podcasts' scope. */
    data class Note(val text: String) : SearchBlock

    /** A read that failed with nothing to draw: the cause and « Réessayer ». */
    data class Failure(val cause: String) : SearchBlock
}

/** Where the fallback stands, as its one line says it. */
sealed interface FallbackStatus {
    data object Waiting : FallbackStatus
    data class NothingMore(val term: String) : FallbackStatus
    data class Failed(val failure: SearchFallback) : FallbackStatus
}

/**
 * The page, in order — `Search.svelte`'s structure as a list a test can read.
 *
 * The SPA's own order: the best result above the titles on a phone, ten titles
 * then « Tout afficher », then the artists, the albums with the shows mixed in,
 * the playlists; a family's tab shows it whole. The podcasts scope shows its
 * summary and the passages; its « Émissions » tab is the [SearchScope.SHOWS]
 * scope — ONE request of its own, where the SPA reuses the library's payload (the
 * phone has none to reuse: the two are separate answers on the disk).
 */
fun searchBlocks(state: SearchModel.State, topTracks: Int): List<SearchBlock> {
    val blocks = mutableListOf<SearchBlock>()
    blocks += SearchBlock.Header
    if (state.importOffer != null) return blocks
    val term = state.term
    if (term.isBlank()) {
        blocks += SearchBlock.Landing
        blocks += SearchBlock.Styles
        return blocks
    }
    val answer = state.answer
    // The tabs are a SOURCE's own row: they stay while a read fails, exactly as
    // the SPA draws them above its failure card.
    if (state.scope == SearchScope.LIBRARY) {
        blocks += SearchBlock.Tabs
    } else if (state.showsAvailable) {
        blocks += SearchBlock.PodcastTabs
    }
    if (state.failure != null && answer == null) {
        blocks += SearchBlock.Failure(state.failure)
        return blocks
    }

    if (state.scope != SearchScope.LIBRARY) {
        if (answer == null) {
            blocks += SearchBlock.Loading
            return blocks
        }
        if (state.stale) blocks += SearchBlock.Stale
        if (state.scope == SearchScope.PODCASTS) {
            if (answer.transcripts.isEmpty()) {
                blocks += SearchBlock.Note("Rien pour « $term » dans les podcasts.")
                blocks += SearchBlock.Note(
                    "La recherche porte sur les titres des émissions et sur le texte des " +
                        "épisodes déjà transcrits.",
                )
            } else {
                blocks += SearchBlock.Note(summaryOf(answer))
                if (answer.transcriptMore) {
                    blocks += SearchBlock.Note("20 passages affichés — la recherche s'arrête là.")
                }
                for (index in answer.transcripts.indices) blocks += SearchBlock.Passage(index)
            }
        } else {
            blocks += SearchBlock.Note(showsCount(answer.podcasts.size))
            blocks += SearchBlock.Shelf(SearchSection.Shows, grid = true)
        }
        return blocks
    }

    if (answer == null) {
        blocks += SearchBlock.Loading
        return blocks
    }
    if (state.stale) blocks += SearchBlock.Stale
    if (!answer.hasResults) {
        if (state.youtube !is SearchModel.YoutubeUi.Waiting) {
            blocks += SearchBlock.Note("Aucun résultat pour « $term ».")
        } else {
            blocks += SearchBlock.Loading
        }
        return blocks
    }
    when (state.tab) {
        SearchTab.All -> {
            if (answer.top != null) blocks += SearchBlock.TopCard
            val shown = minOf(topTracks, answer.trackCount)
            if (shown > 0) {
                blocks += SearchBlock.Section(SearchSection.Titles, answer.trackCount - shown)
                for (index in 0 until shown) blocks += SearchBlock.Track(index)
            }
            fallbackLine(state, answer)?.let { blocks += it }
            for ((section, rows) in families(answer)) {
                if (rows.isEmpty()) continue
                blocks += SearchBlock.Section(section, more = if (rows.size > 1) 1 else 0)
                blocks += SearchBlock.Shelf(section, grid = false)
            }
        }
        SearchTab.Tracks -> {
            for (index in 0 until answer.trackCount) blocks += SearchBlock.Track(index)
            fallbackLine(state, answer)?.let { blocks += it }
        }
        SearchTab.Artists -> whole(blocks, answer, SearchSection.Artists)
        SearchTab.Albums -> whole(blocks, answer, SearchSection.Albums)
        SearchTab.Playlists -> whole(blocks, answer, SearchSection.Playlists)
    }
    return blocks
}

/** A family's own tab: its heading and its grid, whole. */
private fun whole(
    blocks: MutableList<SearchBlock>,
    answer: SearchAnswer,
    section: SearchSection,
) {
    if (entitiesOf(answer, section).isEmpty()) return
    blocks += SearchBlock.Section(section, more = 0)
    blocks += SearchBlock.Shelf(section, grid = true)
}

/** The four card families of the answer, in the SPA's order. */
fun families(answer: SearchAnswer): List<Pair<SearchSection, List<SearchEntity>>> = listOf(
    SearchSection.Artists to answer.artists,
    SearchSection.Albums to answer.albums,
    SearchSection.Playlists to answer.playlists,
)

/** The cards a shelf draws. */
fun entitiesOf(answer: SearchAnswer, section: SearchSection): List<SearchEntity> = when (section) {
    SearchSection.Titles -> emptyList()
    SearchSection.Artists -> answer.artists
    SearchSection.Albums -> answer.albums
    SearchSection.Playlists -> answer.playlists
    SearchSection.Shows -> answer.podcasts
}

private fun fallbackLine(state: SearchModel.State, answer: SearchAnswer): SearchBlock? =
    when (val youtube = state.youtube) {
        SearchModel.YoutubeUi.Idle -> null
        SearchModel.YoutubeUi.Waiting -> SearchBlock.Fallback(FallbackStatus.Waiting)
        is SearchModel.YoutubeUi.Failed -> SearchBlock.Fallback(FallbackStatus.Failed(youtube.failure))
        is SearchModel.YoutubeUi.Added ->
            if (youtube.rows == 0 && answer.trackCount > 0) {
                SearchBlock.Fallback(FallbackStatus.NothingMore(state.term))
            } else {
                null
            }
    }

private fun showsCount(count: Int): String =
    if (count == 1) "1 émission" else "$count émissions"

private fun summaryOf(answer: SearchAnswer): String {
    val passages = if (answer.transcriptTotal == 1) {
        "1 passage"
    } else {
        "${answer.transcriptTotal} passages"
    }
    val episodes = answer.transcriptEpisodes
    return buildString {
        append(passages)
        append(" · ")
        append(if (episodes == 1) "1 épisode" else "$episodes épisodes")
        answer.transcriptSegments?.let { append(" · $it segments") }
        if (answer.transcriptTruncated) append(" · les plus récents")
    }
}

/**
 * Rechercher — lot MB6, `routes/Search.svelte` ported to Compose.
 *
 * One `LazyColumn` over the blocks above, so 50 titles are windowed like any
 * other list: the rows come from the answer's `ListWindow` (pages of 64, one call
 * for a search), and the viewport reports which TITLE indices it is showing.
 * Nothing waits on the network to paint: the kept answer is the first frame.
 */
@Composable
fun SearchScreen(
    state: SearchModel.State,
    actions: SearchActions,
    modifier: Modifier = Modifier,
    topTracks: Int = 10,
) {
    val blocks = remember(state, topTracks) { searchBlocks(state, topTracks) }
    val source = state.answer?.tracks
    val window = remember(source) { source?.let { ListWindow(it) } }
    val listState = rememberLazyListState()

    // The viewport, reported as TITLE indices (the page's other blocks are not
    // rows): `ListWindow` then fetches the page under the finger, and one ahead
    // once the list is read.
    LaunchedEffect(listState, blocks, window) {
        val rows = window ?: return@LaunchedEffect
        snapshotFlow {
            val visible = listState.layoutInfo.visibleItemsInfo
                .mapNotNull { item -> (blocks.getOrNull(item.index) as? SearchBlock.Track)?.index }
            Triple(
                visible.minOrNull() ?: -1,
                visible.maxOrNull() ?: -1,
                listState.isScrollInProgress,
            )
        }.collect { (first, last, scrolling) ->
            if (first >= 0) rows.onVisible(first, last, scrolling)
        }
    }

    LazyColumn(
        state = listState,
        modifier = modifier
            .fillMaxSize()
            // The screen's own marker, in EVERY state (see `SEARCH_MARKER`).
            .semantics { contentDescription = SEARCH_MARKER },
        contentPadding = PaddingValues(
            start = NzSpacing.pageX,
            end = NzSpacing.pageX,
            top = NzSpacing.l,
            bottom = NzSpacing.xl,
        ),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        items(count = blocks.size, key = { index -> keyOf(blocks[index], index) }) { index ->
            Block(blocks[index], state, window, actions)
        }
    }
}

private fun keyOf(block: SearchBlock, index: Int): String = when (block) {
    is SearchBlock.Track -> "track:${block.index}"
    is SearchBlock.Passage -> "passage:${block.index}"
    else -> "${block::class.simpleName}:$index"
}

@Composable
private fun Block(
    block: SearchBlock,
    state: SearchModel.State,
    window: ListWindow?,
    actions: SearchActions,
) {
    val answer = state.answer
    when (block) {
        SearchBlock.Header -> Header(state, actions)
        SearchBlock.Landing -> Landing(state, actions)
        SearchBlock.Styles -> CompositionLocalProvider(
            LocalContentColor provides Sillon.colors.tx,
        ) {
            ExplorerStylesSection(Modifier.fillMaxWidth())
        }
        SearchBlock.Tabs -> TabsRow(state, actions)
        SearchBlock.PodcastTabs -> PodcastTabsRow(state, actions)
        SearchBlock.TopCard -> answer?.top?.let { TopCard(it, actions) }
        is SearchBlock.Section -> SectionHeading(block, answer, actions)
        is SearchBlock.Shelf -> ShelfView(block, answer, actions)
        is SearchBlock.Track -> TrackBlock(window?.rowAt(block.index), state, actions)
        is SearchBlock.Fallback -> FallbackLine(block.status, actions)
        is SearchBlock.Passage -> answer?.transcripts?.getOrNull(block.index)
            ?.let { PassageRow(it, actions) }
        SearchBlock.Stale -> StaleNote()
        SearchBlock.Loading -> Skeleton(kind = NzSkeletonKind.List, count = 5)
        is SearchBlock.Note -> Text(
            text = block.text,
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
            modifier = Modifier.padding(top = NzSpacing.s),
        )
        is SearchBlock.Failure -> LoadFailed(
            cause = block.cause,
            onRetry = { actions.retry() },
            title = "La recherche n'a pas pu se charger",
        )
    }
}

// -- the head of the page ------------------------------------------------------

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Header(state: SearchModel.State, actions: SearchActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Text(text = "Explorer", style = Sillon.type.title, color = colors.tx)
        FlowRow(
            modifier = Modifier
                .fillMaxWidth()
                .semantics { role = Role.Tab },
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            NzChip(
                text = "Ma bibliothèque",
                onClick = { actions.chooseScope(SearchScope.LIBRARY) },
                selected = state.scope == SearchScope.LIBRARY,
            )
            if (state.scope == SearchScope.LIBRARY && state.youtubeAvailable) {
                NzChip(
                    text = "YouTube",
                    onClick = { actions.toggleYoutube() },
                    selected = state.youtubeOn,
                    glyph = NzGlyph.Youtube,
                )
            }
            if (state.podcastsAvailable) {
                NzChip(
                    text = "Podcasts",
                    onClick = { actions.chooseScope(SearchScope.PODCASTS) },
                    selected = state.scope != SearchScope.LIBRARY,
                )
            }
        }
        SearchField(
            value = state.query,
            placeholder = if (state.scope == SearchScope.LIBRARY) {
                "Titres, artistes, albums, podcasts…"
            } else {
                "Rechercher dans les podcasts…"
            },
            onValue = actions.query,
            onSubmit = actions.submit,
        )
        // The import offer of a pasted link, and the radio offer of a broad
        // search: the SPA's two doors under the field.
        state.importOffer?.let { ImportCard(it, actions) }
        if (answer_offers_radio(state)) {
            NzButton(
                text = if (state.radioBusy) "Lancement…" else "Lancer la radio",
                onClick = { actions.launchRadio() },
                glyph = NzGlyph.Radio,
                enabled = !state.radioBusy,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

/** The button's gate: a term, results FOR it, the library scope, no pasted link. */
private fun answer_offers_radio(state: SearchModel.State): Boolean =
    state.scope == SearchScope.LIBRARY &&
        state.importOffer == null &&
        state.term.length >= 2 &&
        state.answer?.radioOffered == true &&
        state.answer.term == state.term &&
        !state.loading

@Composable
private fun SearchField(
    value: String,
    placeholder: String,
    onValue: (String) -> Unit,
    onSubmit: () -> Unit,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.chip)
    val keyboard = LocalSoftwareKeyboardController.current
    // Enter searches AND drops the keyboard, as `Search.svelte#onKey` blurs the
    // box (`e.currentTarget.blur()`): the results are what the person came for,
    // and the keyboard covered half of them in this lot's first capture.
    val submit = {
        onSubmit()
        keyboard?.hide()
    }
    BasicTextField(
        value = value,
        onValueChange = onValue,
        singleLine = true,
        textStyle = Sillon.type.body.copy(color = colors.tx),
        cursorBrush = SolidColor(colors.acText),
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        keyboardActions = KeyboardActions(onSearch = { submit() }),
        modifier = Modifier
            .fillMaxWidth()
            .height(48.dp)
            .semantics { contentDescription = "Rechercher" },
        decorationBox = { inner ->
            Row(
                modifier = Modifier
                    .fillMaxSize()
                    .clip(shape)
                    .background(colors.s1)
                    .border(1.dp, colors.ln, shape)
                    .padding(horizontal = NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Icon(glyph = NzGlyph.Search, size = 16.dp, tint = colors.dim)
                Box(Modifier.weight(1f)) {
                    if (value.isEmpty()) {
                        Text(
                            text = placeholder,
                            style = Sillon.type.body,
                            color = colors.dim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    inner()
                }
                if (value.isNotEmpty()) {
                    NzIconButton(
                        glyph = NzGlyph.Close,
                        contentDescription = "Effacer la recherche",
                        onClick = { onValue("") },
                        size = 16.dp,
                    )
                }
            }
        },
    )
}

@Composable
private fun ImportCard(offer: ImportOffer, actions: SearchActions) {
    val colors = Sillon.colors
    NzCard(onClick = { actions.importLink(offer) }, modifier = Modifier.fillMaxWidth()) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Box(
                modifier = Modifier
                    .size(40.dp)
                    .clip(RoundedCornerShape(NzRadius.chip))
                    .background(colors.ac),
                contentAlignment = Alignment.Center,
            ) {
                Icon(glyph = NzGlyph.Open, size = 18.dp, tint = colors.onac)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                Text(
                    text = "Importer cette playlist ${offer.label}",
                    style = Sillon.type.body,
                    color = colors.tx,
                )
                Text(
                    text = "Retrouver ses titres pour les écouter ici, et la garder si tu veux.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
        }
    }
}

/** A row of chips that WRAPS instead of running off the screen (the SPA's ≤640 rule). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun SearchChipFlow(content: @Composable () -> Unit) {
    FlowRow(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        content()
    }
}

// -- the empty page ------------------------------------------------------------

@Composable
private fun Landing(state: SearchModel.State, actions: SearchActions) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
        if (state.recent.isNotEmpty()) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text("Récents", style = Sillon.type.titleSmall, color = colors.tx)
                Box(
                    modifier = Modifier
                        .sizeIn(minWidth = NzMinTouchTarget, minHeight = NzMinTouchTarget)
                        .clickable(role = Role.Button) { actions.clearRecent() },
                    contentAlignment = Alignment.Center,
                ) {
                    Text("Effacer", style = Sillon.type.label, color = colors.dim)
                }
            }
            SearchChipFlow {
                state.recent.forEach { term ->
                    NzChip(
                        text = term,
                        onClick = { actions.useRecent(term) },
                        glyph = NzGlyph.Search,
                    )
                }
            }
        } else {
            Text(
                text = "Cherche un titre, un artiste, un album ou un podcast — les titres de ta " +
                    "bibliothèque d'abord, puis le catalogue, et YouTube quand il a ce que le " +
                    "catalogue n'a pas.",
                style = Sillon.type.body,
                color = colors.dim,
            )
        }
    }
}

// -- the tabs ------------------------------------------------------------------

@Composable
private fun TabsRow(state: SearchModel.State, actions: SearchActions) {
    val tabs = listOf(
        SearchTab.All to "Tout",
        SearchTab.Tracks to "Titres",
        SearchTab.Artists to "Artistes",
        SearchTab.Albums to "Albums",
        SearchTab.Playlists to "Playlists",
    )
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState())
            .semantics { role = Role.Tab },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        tabs.forEach { (tab, label) ->
            NzChip(
                text = label,
                onClick = { actions.tab(tab) },
                selected = state.tab == tab,
                modifier = Modifier.semantics { selected = state.tab == tab },
            )
        }
    }
}

@Composable
private fun PodcastTabsRow(state: SearchModel.State, actions: SearchActions) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .semantics { role = Role.Tab },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        NzChip(
            text = "Passages",
            onClick = { actions.chooseScope(SearchScope.PODCASTS) },
            selected = state.scope == SearchScope.PODCASTS,
        )
        NzChip(
            text = "Émissions",
            onClick = { actions.chooseScope(SearchScope.SHOWS) },
            selected = state.scope == SearchScope.SHOWS,
        )
    }
}

// -- the results ---------------------------------------------------------------

@Composable
private fun SectionHeading(
    block: SearchBlock.Section,
    answer: SearchAnswer?,
    actions: SearchActions,
) {
    val colors = Sillon.colors
    val title = when (block.section) {
        SearchSection.Titles -> "Titres"
        SearchSection.Artists -> "Artistes"
        SearchSection.Albums -> "Albums"
        SearchSection.Playlists -> "Playlists"
        SearchSection.Shows -> "Émissions"
    }
    val total = when (block.section) {
        SearchSection.Titles -> answer?.trackCount ?: 0
        else -> answer?.let { entitiesOf(it, block.section).size } ?: 0
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(title, style = Sillon.type.titleSmall, color = colors.tx)
        if (block.more > 0) {
            Row(
                modifier = Modifier
                    .sizeIn(minHeight = NzMinTouchTarget)
                    .clickable(role = Role.Button) {
                        actions.tab(
                            when (block.section) {
                                SearchSection.Titles -> SearchTab.Tracks
                                SearchSection.Artists -> SearchTab.Artists
                                SearchSection.Albums -> SearchTab.Albums
                                else -> SearchTab.Playlists
                            },
                        )
                    },
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                Text("Tout afficher", style = Sillon.type.label, color = colors.dim)
                if (block.section == SearchSection.Titles && total > 0) {
                    Text(
                        text = total.toString(),
                        style = Sillon.type.mono,
                        color = colors.dim,
                    )
                }
            }
        }
    }
}

@Composable
private fun TrackBlock(row: TrackRow?, state: SearchModel.State, actions: SearchActions) {
    if (row == null) {
        Skeleton(kind = NzSkeletonKind.List, count = 1)
        return
    }
    SearchTrackRow(
        row = row,
        onClick = { actions.playTrack(row) },
        onMore = { actions.more(row) },
    )
}

/**
 * One title of the answer.
 *
 * Not `TrackRowView`: that row is a FIXED 56 dp, and at the 200 % font scale its
 * two lines overlap (seen in this lot's own screenshot). This one grows, keeps
 * the 44 dp sleeve and the 56 dp minimum, and says `YT` on a fallback row — the
 * badge `webapp.md` asks for ("Every YouTube row says YT and its Opus rate").
 */
@Composable
private fun SearchTrackRow(row: TrackRow, onClick: () -> Unit, onMore: () -> Unit) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .clip(RoundedCornerShape(NzRadius.chip))
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        CoverImage(url = row.cover, sizeDp = 44.dp, seed = row.id)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = row.title,
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            // The fallback's mark rides the ARTIST line, where a narrow row hides
            // the quality cell (`webapp.md`: `.qinline`) — a trailing cell squeezed
            // the title to « Titr… » at 200 % (this lot's own screenshot).
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                Text(
                    text = row.artists,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (row.source == "youtube") {
                    Text("YT", style = Sillon.type.mono, color = colors.dim)
                }
            }
        }
        Text(
            text = nzDuration(row.durationS.toInt()),
            style = Sillon.type.mono,
            color = colors.dim,
        )
        NzIconButton(
            glyph = NzGlyph.MoreVertical,
            contentDescription = "Plus d'actions pour ${row.title}",
            onClick = onMore,
        )
    }
}

@Composable
private fun TopCard(top: TopResult, actions: SearchActions) {
    val colors = Sillon.colors
    val row = top.row
    val round = top.kind == EntityKind.ARTIST
    NzCard(onClick = { actions.openEntity(row.kind, row.id) }, modifier = Modifier.fillMaxWidth()) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            if (round) {
                Avatar(name = row.title, sizeDp = 64.dp, url = row.cover)
            } else {
                CoverImage(url = row.cover, sizeDp = 64.dp, seed = row.id)
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                Text(
                    text = row.title,
                    style = Sillon.type.titleSmall,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = if (round) "Artiste" else "Podcast",
                    style = Sillon.type.mono,
                    color = colors.dim,
                )
            }
            NzIconButton(
                glyph = if (round) NzGlyph.Play else NzGlyph.Plus,
                contentDescription = if (round) "Lire" else "S'abonner",
                onClick = { actions.playTop(top) },
                selected = true,
            )
        }
    }
}

@Composable
private fun ShelfView(block: SearchBlock.Shelf, answer: SearchAnswer?, actions: SearchActions) {
    val rows = answer?.let { entitiesOf(it, block.section) }.orEmpty()
    if (rows.isEmpty()) return
    if (block.grid) {
        EntityGrid(rows, actions)
    } else {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            rows.forEach { row -> EntityCard(row, actions, Modifier.width(132.dp), 104.dp) }
        }
    }
}

/** A two-column grid: the phone's answer to the SPA's `grid` class. */
@Composable
private fun EntityGrid(rows: List<SearchEntity>, actions: SearchActions) {
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val columns = if (maxWidth >= 520.dp) 3 else 2
        val gap = NzSpacing.s
        val width = (maxWidth - gap * (columns - 1)) / columns
        Column(verticalArrangement = Arrangement.spacedBy(gap)) {
            rows.chunked(columns).forEach { line ->
                Row(horizontalArrangement = Arrangement.spacedBy(gap)) {
                    val image = (width - NzSpacing.s * 2).coerceAtLeast(64.dp)
                    line.forEach { row -> EntityCard(row, actions, Modifier.width(width), image) }
                    repeat(columns - line.size) { Spacer(Modifier.width(width)) }
                }
            }
        }
    }
}

@Composable
private fun EntityCard(
    row: SearchEntity,
    actions: SearchActions,
    modifier: Modifier,
    imageDp: Dp,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.card))
            .background(colors.glassBg)
            .clickable(role = Role.Button) { actions.openEntity(row.kind, row.id) }
            .padding(NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        if (row.kind == EntityKind.ARTIST) {
            Avatar(name = row.title, sizeDp = imageDp, url = row.cover)
        } else {
            CoverImage(url = row.cover, sizeDp = imageDp, seed = row.id)
        }
        Text(
            text = row.title,
            style = Sillon.type.body,
            color = colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = if (row.subtitle.isNotEmpty()) {
                row.subtitle
            } else if (row.kind == EntityKind.SHOW) {
                "Podcast"
            } else {
                ""
            },
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun FallbackLine(status: FallbackStatus, actions: SearchActions) {
    val colors = Sillon.colors
    when (status) {
        FallbackStatus.Waiting -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Icon(glyph = NzGlyph.Youtube, size = 14.dp, tint = colors.dim)
            Text(
                text = "Recherche sur YouTube…",
                style = Sillon.type.bodySmall,
                color = colors.dim,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
        is FallbackStatus.NothingMore -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Icon(glyph = NzGlyph.Youtube, size = 14.dp, tint = colors.dim)
            Text(
                text = "Rien de plus sur YouTube pour « ${status.term} ».",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
        is FallbackStatus.Failed -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            Text(
                text = sentenceOf(status.failure),
                style = Sillon.type.bodySmall,
                color = colors.dim,
                modifier = Modifier
                    .weight(1f)
                    .semantics { liveRegion = LiveRegionMode.Polite },
            )
            if (status.failure != SearchFallback.OFFLINE) {
                NzButton(
                    text = "Réessayer",
                    onClick = { actions.retryFallback() },
                    kind = NzButtonKind.Secondary,
                    glyph = NzGlyph.Refresh,
                )
            }
        }
    }
}

/** The SPA's three sentences for a refused fallback, plus the offline one. */
private fun sentenceOf(failure: SearchFallback): String = when (failure) {
    SearchFallback.DENIED -> "Vous n'avez pas accès à YouTube sur ce serveur."
    SearchFallback.UNAVAILABLE -> "YouTube ne répond pas pour le moment."
    SearchFallback.OFFLINE -> "Hors ligne : YouTube n'a pas été interrogé."
    SearchFallback.FAILED -> "La recherche sur YouTube a échoué."
}

@Composable
private fun PassageRow(hit: TranscriptHit, actions: SearchActions) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 72.dp)
            .clip(RoundedCornerShape(NzRadius.chip))
            .clickable(role = Role.Button) { actions.openPassage(hit) }
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.s),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Box(Modifier.size(36.dp).clip(RoundedCornerShape(NzRadius.chip))) {
            CoverImage(url = hit.cover, sizeDp = 36.dp, seed = hit.episodeId)
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                Text(
                    text = nzDuration(hit.startS.toInt()),
                    style = Sillon.type.mono,
                    color = colors.dim,
                )
                Text(
                    text = hit.title,
                    style = Sillon.type.body,
                    color = colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            // The words that were typed are MARKED, as `TranscriptHits.svelte`
            // paints them (`<mark>`: `--ac` behind `--onac`).
            Text(
                text = buildAnnotatedString {
                    hit.text.forEach { part ->
                        if (part.mark) {
                            withStyle(SpanStyle(background = colors.ac, color = colors.onac)) {
                                append(part.text)
                            }
                        } else {
                            append(part.text)
                        }
                    }
                },
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            if (hit.channel.isNotEmpty()) {
                Text(hit.channel, style = Sillon.type.bodySmall, color = colors.dim, maxLines = 1)
            }
        }
    }
}

/** The SPA's stale sentence, under the list it applies to (rule 3). */
@Composable
private fun StaleNote() {
    Text(
        text = "Hors ligne · version mémorisée",
        style = Sillon.type.label,
        color = Sillon.colors.dim,
        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
    )
}
