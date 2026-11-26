package fr.neezer.mobile.feature.podcasts

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwap
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.Skeleton
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.ShowData

/**
 * Podcasts — lot MB13 (docs/agents/mobile.md, feature inventory).
 *
 * The subscriptions (`routes/Podcasts.svelte` ported): a grid of the shows this
 * server holds, a search field that waits 300 ms and then asks the provider's
 * catalogue once, a card that subscribes on a tap, and a pasted Deezer URL that
 * subscribes on Enter. Adding is ONE request, and the show it answers is opened
 * straight away — the person's tap lands on what they added.
 *
 * The search field only exists for an administrator (`$isAdmin && CAN_KEEP` in
 * the SPA): subscribing is a server-side decision, and a member sees the field
 * without a power to use it would be a lie. The card hides the grid while a
 * search is on, exactly as the SPA does.
 */
@Composable
fun PodcastsEntry(
    nav: NzNavigator,
    modifier: Modifier = Modifier,
    /** The seam a JVM test and a render use; the app passes nothing. */
    service: PodcastService? = AppPodcasts.service(),
    isAdmin: Boolean = AppPodcasts.isAdmin(),
) {
    val scope = rememberCoroutineScope()
    val model = remember(service, scope) {
        service?.let { PodcastsModel(it, scope) }
    }
    LaunchedEffect(model) { model?.start() }
    if (model == null) {
        CoreUnavailable(modifier = modifier)
        return
    }
    val state by model.state.collectAsState()

    // A successful subscribe opens the show it answered, and a failure is a
    // toast with the cause — never a silent button.
    LaunchedEffect(state.openShow) {
        state.openShow?.let { id ->
            model.consumeOpen()
            nav.go(AppLink.Podcast(id))
        }
    }

    PodcastsScreen(
        state = state,
        actions = PodcastsActions(
            retry = model::retry,
            setQuery = model::setQuery,
            retrySearch = model::retrySearch,
            submitLink = model::submitLink,
            subscribe = model::subscribeCard,
            openShow = { id -> nav.go(AppLink.Podcast(id)) },
            consumeMessage = model::consumeMessage,
        ),
        isAdmin = isAdmin,
        modifier = modifier,
    )
}

/** What the screen may ask of its model — one function per gesture. */
data class PodcastsActions(
    val retry: () -> Unit,
    val setQuery: (String) -> Unit,
    val retrySearch: () -> Unit,
    val submitLink: () -> Unit,
    val subscribe: (ShowData) -> Unit,
    val openShow: (String) -> Unit,
    val consumeMessage: () -> Unit,
)

/**
 * The screen itself: a pure function of the model's state, so a screenshot and a
 * JVM test render exactly what the app draws.
 */
@Composable
fun PodcastsScreen(
    state: PodcastsModel.State,
    actions: PodcastsActions,
    isAdmin: Boolean,
    modifier: Modifier = Modifier,
    toast: @Composable (String) -> Unit = {},
) {
    val colors = Sillon.colors
    Box(
        modifier
            .fillMaxSize()
            .semantics { contentDescription = PODCASTS_MARKER }
            .background(colors.bg),
    ) {
        Column(Modifier.fillMaxSize()) {
            if (isAdmin) {
                PodcastSearchField(
                    value = state.query,
                    onValueChange = actions.setQuery,
                    onSubmit = actions.submitLink,
                    onClear = { actions.setQuery("") },
                    modifier = Modifier.padding(
                        start = NzSpacing.pageX,
                        end = NzSpacing.pageX,
                        top = NzSpacing.pageY,
                    ),
                )
            }

            if (state.stale && state.shows.isNotEmpty()) {
                Text(
                    text = PodcastsCopy.STALE,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    modifier = Modifier.padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xs),
                )
            }

            // The main phase replaces itself in place (M7): skeleton → grid,
            // failure → retry, the search panel or the empty note. Four flags,
            // one phase (the plan's `(loading, failure!=null, shows.isEmpty(),
            // searchActive)`); a re-emit of the same phase animates nothing.
            NzSwap(
                targetState = podcastsPhase(state),
                modifier = Modifier.weight(1f),
                label = "podcasts",
            ) { phase ->
                // `AnimatedContent` stacks its content; a Column keeps the
                // `weight(1f)` of the grid and the search results.
                Column(Modifier.fillMaxSize()) {
                    when (phase) {
                        PODCASTS_LOADING -> Skeleton(
                            modifier = Modifier
                                .padding(NzSpacing.pageX)
                                .testTag(PODCASTS_LOADING_TAG),
                            kind = NzSkeletonKind.Shelf,
                            count = 6,
                        )
                        PODCASTS_FAILED -> LoadFailed(
                            cause = state.failure.orEmpty(),
                            onRetry = actions.retry,
                            modifier = Modifier.padding(NzSpacing.pageX),
                            title = "Les podcasts n'ont pas pu être chargés",
                        )
                        PODCASTS_SEARCH -> SearchResults(state, actions)
                        PODCASTS_EMPTY -> EmptyState(
                            title = "Aucun abonnement",
                            body = if (isAdmin) {
                                "${PodcastsCopy.NO_SUBSCRIPTION} ${PodcastsCopy.NO_SUBSCRIPTION_HINT}"
                            } else {
                                PodcastsCopy.NO_SUBSCRIPTION
                            },
                            glyph = NzGlyph.Mic,
                        )
                        else -> ShowGrid(
                            shows = state.shows,
                            onOpen = actions.openShow,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
            }
        }
        state.message?.let { message ->
            toast(message)
            LaunchedEffect(message) { actions.consumeMessage() }
        }
    }
}

/** The results of a search, with its three states (`runSearch`'s outcomes). */
@Composable
private fun SearchResults(state: PodcastsModel.State, actions: PodcastsActions) {
    val colors = Sillon.colors
    Column(Modifier.fillMaxSize()) {
        // The search's own phase replaces in place, exactly like the list's
        // (M7): the skeleton, the failure, the empty note and the grid.
        NzSwap(
            targetState = podcastsSearchPhase(state),
            modifier = Modifier.weight(1f),
            label = "podcasts-recherche",
        ) { phase ->
            Column(Modifier.fillMaxSize()) {
                when (phase) {
                    PODCASTS_LOADING -> Skeleton(
                        modifier = Modifier.padding(NzSpacing.pageX),
                        kind = NzSkeletonKind.List,
                        count = 4,
                    )
                    PODCASTS_FAILED -> LoadFailed(
                        cause = state.searchFailure.orEmpty(),
                        onRetry = actions.retrySearch,
                        modifier = Modifier.padding(NzSpacing.pageX),
                        title = "La recherche a échoué",
                    )
                    PODCASTS_EMPTY -> Text(
                        text = PodcastsCopy.noSearchResult(state.trimmedQuery),
                        style = Sillon.type.body,
                        color = colors.dim,
                        modifier = Modifier.padding(
                            start = NzSpacing.pageX,
                            end = NzSpacing.pageX,
                            top = NzSpacing.l,
                        ),
                    )
                    else -> {
                        Text(
                            text = "Résultats",
                            style = Sillon.type.titleSmall,
                            color = colors.tx,
                            modifier = Modifier.padding(
                                start = NzSpacing.pageX,
                                end = NzSpacing.pageX,
                                top = NzSpacing.m,
                                bottom = NzSpacing.s,
                            ),
                        )
                        ShowGrid(
                            shows = state.results,
                            onOpen = actions.openShow,
                            onSubscribe = actions.subscribe,
                            subscribing = state.subscribing,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
            }
        }
    }
}

/** The subscriptions' four phases, as the [NzSwap] target (the plan's flags). */
internal fun podcastsPhase(state: PodcastsModel.State): String = when {
    state.loading && state.shows.isEmpty() && !state.searchActive -> PODCASTS_LOADING
    state.failure != null && state.shows.isEmpty() -> PODCASTS_FAILED
    state.searchActive -> PODCASTS_SEARCH
    state.shows.isEmpty() -> PODCASTS_EMPTY
    else -> PODCASTS_GRID
}

/** A search's four outcomes, as the inner [NzSwap] target. */
private fun podcastsSearchPhase(state: PodcastsModel.State): String = when {
    state.searching && state.results.isEmpty() -> PODCASTS_LOADING
    state.searchFailure != null -> PODCASTS_FAILED
    state.results.isEmpty() -> PODCASTS_EMPTY
    else -> PODCASTS_GRID
}

internal const val PODCASTS_LOADING = "chargement"
internal const val PODCASTS_FAILED = "echec"
private const val PODCASTS_SEARCH = "recherche"
private const val PODCASTS_EMPTY = "vide"
private const val PODCASTS_GRID = "grille"

/** The subscriptions' loading skeleton, for the phase test. */
internal const val PODCASTS_LOADING_TAG = "podcasts-chargement"

/** The grid of show cards (`Podcasts.svelte`'s `.grid`, 160 px cells). */
@Composable
private fun ShowGrid(
    shows: List<ShowData>,
    onOpen: (String) -> Unit,
    modifier: Modifier = Modifier,
    onSubscribe: ((ShowData) -> Unit)? = null,
    subscribing: String? = null,
) {
    LazyVerticalGrid(
        columns = GridCells.Adaptive(minSize = 148.dp),
        modifier = modifier.fillMaxWidth(),
        contentPadding = PaddingValues(
            start = NzSpacing.pageX,
            end = NzSpacing.pageX,
            top = NzSpacing.pageY,
            bottom = NzSpacing.pageBottom,
        ),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        items(shows, key = { it.id.ifEmpty { it.deezerId ?: it.title } }) { show ->
            ShowCard(
                show = show,
                onOpen = { onOpen(show.id.ifEmpty { show.deezerId ?: show.id }) },
                onSubscribe = onSubscribe?.let { { it(show) } },
                busy = subscribing != null && subscribing == (show.deezerId ?: show.id),
            )
        }
    }
}

/** One card: the sleeve, the name, the count, and « Abonné » or « S'abonner ». */
@Composable
private fun ShowCard(
    show: ShowData,
    onOpen: () -> Unit,
    onSubscribe: (() -> Unit)?,
    busy: Boolean,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.card)
    NzCard(
        modifier = Modifier.fillMaxWidth(),
        onClick = if (onSubscribe != null) onSubscribe else onOpen,
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            CoverImage(
                url = show.cover,
                sizeDp = 120.dp,
                seed = show.id.ifEmpty { show.deezerId ?: show.title },
                modifier = Modifier.align(Alignment.CenterHorizontally),
            )
            Text(
                text = show.title,
                style = Sillon.type.label,
                color = colors.tx,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                Text(
                    text = PodcastsCopy.episodeCount(show.episodeCount.toInt()),
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (show.autoArchive) {
                    // « auto », as the SPA's own badge says (`Podcasts.svelte`).
                    Icon(glyph = NzGlyph.Archive, size = 12.dp, tint = colors.btn)
                    Text(text = "auto", style = Sillon.type.labelMono, color = colors.btn)
                }
            }
            if (onSubscribe != null) {
                // A catalogue card subscribes; « Abonné » already is one.
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(NzMinTouchTarget)
                        .clickable(role = Role.Button, onClick = onSubscribe)
                        .semantics {
                            contentDescription = "S'abonner à ${show.title}"
                        },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                ) {
                    Icon(
                        glyph = if (busy) NzGlyph.Refresh else NzGlyph.Plus,
                        size = 16.dp,
                        tint = colors.btn,
                    )
                    Text(
                        text = if (busy) "Ajout…" else "S'abonner",
                        style = Sillon.type.label,
                        color = colors.btn,
                    )
                }
            }
        }
    }
}

/** The search field: a pill, Sillon's colours, 48 dp, and Enter submits a link. */
@Composable
private fun PodcastSearchField(
    value: String,
    onValueChange: (String) -> Unit,
    onSubmit: () -> Unit,
    onClear: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.chip)
    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        singleLine = true,
        textStyle = Sillon.type.body.copy(color = colors.tx),
        cursorBrush = SolidColor(colors.acText),
        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
            imeAction = androidx.compose.ui.text.input.ImeAction.Search,
        ),
        keyboardActions = androidx.compose.foundation.text.KeyboardActions(
            onSearch = { onSubmit() },
        ),
        modifier = modifier
            .fillMaxWidth()
            .height(NzMinTouchTarget)
            .semantics { contentDescription = "Chercher un podcast" },
        decorationBox = { inner ->
            Row(
                modifier = Modifier
                    .fillMaxSize()
                    .background(colors.s1, shape)
                    .border(1.dp, colors.ln, shape)
                    .padding(horizontal = NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Icon(glyph = NzGlyph.Search, size = 16.dp, tint = colors.dim)
                Box(Modifier.weight(1f)) {
                    if (value.isEmpty()) {
                        Text(
                            text = "Nom du podcast ou lien Deezer…",
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
                        onClick = onClear,
                        size = 14.dp,
                    )
                }
            }
        },
    )
}

/** The build could not load the native core: the screen says so (rule 3). */
@Composable
internal fun CoreUnavailable(modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Column(
        modifier = modifier.fillMaxSize().padding(NzSpacing.pageX),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Spacer(Modifier.height(NzSpacing.m))
        Text(text = "Podcasts", style = Sillon.type.title, color = colors.tx)
        Text(
            text = "Le noyau de l'application n'a pas pu être chargé : les podcasts sont indisponibles.",
            style = Sillon.type.body,
            color = colors.dim,
        )
        Text(
            text = NzBridge.failure ?: "cause inconnue",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
    }
}

/**
 * Le marqueur de l'écran, pour la sonde de l'émulateur et le test de coquille : le
 * texte du stub MB0 (« Les abonnements, la recherche ») a disparu avec lui, et
 * aucune phrase de la grille n'est stable dans tous les états (la règle de
 * `HOME_MARKER` / `LIBRARY_MARKER`).
 */
const val PODCASTS_MARKER = "Podcasts Neezer"
