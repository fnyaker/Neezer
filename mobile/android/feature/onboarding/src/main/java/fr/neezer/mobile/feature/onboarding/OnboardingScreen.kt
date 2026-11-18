package fr.neezer.mobile.feature.onboarding

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.items as lazyItems
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzCard
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import fr.neezer.mobile.designsystem.NzSkeletonKind
import uniffi.nz_ffi.AccountOptions
import uniffi.nz_ffi.ArtistCard
import uniffi.nz_ffi.ImportJob
import uniffi.nz_ffi.ImportState
import uniffi.nz_ffi.OnboardingStep
import uniffi.nz_ffi.ProviderOptions

/**
 * Everything the onboarding can do, as one value.
 *
 * The screen is a pure function of [OnboardingModel.State] and these callbacks,
 * so the screenshot test draws the REAL screen from a fixture with no core and
 * no network.
 */
data class OnboardingActions(
    val toggle: (ArtistCard) -> Unit = { _ -> },
    val retryRelated: (String) -> Unit = { _ -> },
    val query: (String) -> Unit = { _ -> },
    val submitSearch: () -> Unit = {},
    val retrySearch: () -> Unit = {},
    val clearSearch: () -> Unit = {},
    /** The scroll reached the end of what is loaded: one page more. */
    val more: () -> Unit = {},
    /** A read that failed with nothing painted: one attempt more. */
    val retry: () -> Unit = {},
    val retryFeed: () -> Unit = {},
    val pass: () -> Unit = {},
    val finish: () -> Unit = {},
    val retryAction: () -> Unit = {},
    val loadOptions: () -> Unit = {},
    val open: (String) -> Unit = { _ -> },
    val closeForm: () -> Unit = {},
    val link: (String, String) -> Unit = { _, _ -> },
    val startImport: (String) -> Unit = { _ -> },
    val saveImport: () -> Unit = {},
    val retryImport: () -> Unit = {},
    val refreshImport: () -> Unit = {},
    val accountDone: () -> Unit = {},
    val skip: (String) -> Unit = { _ -> },
    val unskip: (String) -> Unit = { _ -> },
    val gotoArtists: () -> Unit = {},
    val dismissError: () -> Unit = {},
)

/** The screen's marker in every state: `ci-emulator.sh` waits for this node. */
const val ONBOARDING_MARKER = "Bienvenue sur Neezer"

/**
 * Bienvenue — lot MB5b: the artists step, then the account step, ported from
 * `routes/Onboarding.svelte` and `components/OnboardingAccount.svelte`.
 *
 * French copy, Sillon tokens, `Icon` glyphs, a 48 dp target on every control and
 * a TalkBack label on every icon button. The grid is a `LazyVerticalGrid`
 * (windowed by the platform), so the worst realistic feed never composes more
 * than the viewport.
 */
@Composable
fun OnboardingScreen(
    state: OnboardingModel.State,
    actions: OnboardingActions,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .fillMaxSize()
            .background(colors.bg)
            .semantics { contentDescription = ONBOARDING_MARKER },
    ) {
        Header(state = state, actions = actions)
        statusLine(state)
        if (!state.online) OfflineBanner(account = state.step == OnboardingStep.ACCOUNT)
        when {
            state.loading && state.tiles.isEmpty() -> Skeleton(
                modifier = Modifier.padding(NzSpacing.pageX),
                kind = NzSkeletonKind.List,
                count = 6,
            )
            state.failure != null -> LoadFailedBlock(cause = state.failure, onRetry = actions.retry)
            state.step == OnboardingStep.ACCOUNT -> AccountPane(state, actions)
            else -> ArtistsPane(state, actions)
        }
    }
}

// -- the bar ------------------------------------------------------------------

@Composable
private fun Header(state: OnboardingModel.State, actions: OnboardingActions) {
    val colors = Sillon.colors
    val account = state.step == OnboardingStep.ACCOUNT
    Column {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            if (account) {
                NzIconButton(
                    glyph = NzGlyph.ChevronLeft,
                    contentDescription = "Retour aux artistes",
                    onClick = actions.gotoArtists,
                )
            }
            Text(
                text = "Étape ${if (account) 2 else 1} sur 2",
                style = Sillon.type.labelSmall.copy(color = colors.dim),
                modifier = Modifier.weight(1f),
            )
            NzButton(
                text = if (state.finishing) {
                    "Enregistrement…"
                } else if (account) {
                    "Terminer"
                } else {
                    "Terminé"
                },
                onClick = if (account) actions.accountDone else actions.finish,
                glyph = if (state.finishing) NzGlyph.Refresh else NzGlyph.Check,
            )
        }
        StepRail(account = account)
    }
}

/** The step's progress: a transform over a hairline, 50 % then 100 %. */
@Composable
private fun StepRail(account: Boolean) {
    val colors = Sillon.colors
    Box(Modifier.fillMaxWidth().height(2.dp).background(colors.ln)) {
        Box(
            Modifier
                .fillMaxWidth(if (account) 1f else 0.5f)
                .height(2.dp)
                .background(colors.acText),
        )
    }
}

@Composable
private fun statusLine(state: OnboardingModel.State) {
    val colors = Sillon.colors
    val text = state.status.orEmpty().ifBlank { state.notice.orEmpty() }
    if (text.isEmpty()) return
    Text(
        text = text,
        style = Sillon.type.labelSmall.copy(color = colors.acText),
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xs)
            .semantics { liveRegion = LiveRegionMode.Polite },
    )
}

@Composable
private fun OfflineBanner(account: Boolean) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xs)
            .clip(RoundedCornerShape(NzRadius.field))
            .background(colors.s1)
            .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.field))
            .padding(NzSpacing.m)
            .semantics { liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(NzGlyph.CloudOff, size = 16.dp, tint = colors.dim)
        Text(
            text = if (account) {
                "Hors ligne : les liaisons et les imports attendent le retour du réseau."
            } else {
                "Hors ligne : vos choix seront enregistrés au retour du réseau."
            },
            style = Sillon.type.bodySmall.copy(color = colors.dim),
        )
    }
}

@Composable
private fun LoadFailedBlock(cause: String, onRetry: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(NzSpacing.pageX),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = "Impossible de charger cette étape.",
            style = Sillon.type.titleSmall.copy(color = Sillon.colors.tx),
            textAlign = TextAlign.Center,
        )
        LoadFailed(cause = cause, onRetry = onRetry)
        Text(
            text = "Le serveur n'a pas répondu. Votre compte est créé : rien n'est perdu.",
            style = Sillon.type.bodySmall.copy(color = Sillon.colors.dim),
            textAlign = TextAlign.Center,
        )
    }
}

// -- the artists step ---------------------------------------------------------

@Composable
private fun ArtistsPane(state: OnboardingModel.State, actions: OnboardingActions) {
    val colors = Sillon.colors
    val grid = rememberLazyGridState()
    val items = state.items
    LaunchedEffect(grid, items.size) {
        snapshotFlow { grid.layoutInfo.visibleItemsInfo.maxOfOrNull { it.index } ?: 0 }
            .collect { last ->
                if (items.isNotEmpty() && last >= items.size - 3) actions.more()
            }
    }
    LazyVerticalGrid(
        columns = GridCells.Adaptive(minSize = 132.dp),
        state = grid,
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(
            start = NzSpacing.pageX,
            end = NzSpacing.pageX,
            bottom = NzSpacing.xxl,
        ),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        item(span = { GridItemSpan(maxLineSpan) }) {
            Column(
                modifier = Modifier.padding(top = NzSpacing.s),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Text(
                    text = "Quels artistes écoutez-vous ?",
                    style = Sillon.type.title.copy(color = colors.tx),
                    modifier = Modifier.semantics { heading() },
                )
                Text(
                    text = "Choisissez-en au moins un. Trois à cinq nous donnent un bon départ.",
                    style = Sillon.type.body.copy(color = colors.dim),
                )
                SearchField(
                    value = state.query,
                    onValue = actions.query,
                    onSubmit = actions.submitSearch,
                    onClear = actions.clearSearch,
                )
                ProgressRow(state)
            }
        }
        val actionError = state.actionError
        if (actionError != null) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                AlertLine(actionError.message, onRetry = {
                    actionError.retry?.invoke()
                    actions.dismissError()
                })
            }
        }
        val searchFailure = state.searchFailure
        if (searchFailure != null) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    Text(
                        "La recherche n'a pas abouti.",
                        style = Sillon.type.titleSmall.copy(color = colors.tx),
                    )
                    LoadFailed(cause = searchFailure, onRetry = actions.retrySearch)
                    NzButton("Effacer la recherche", actions.clearSearch, kind = NzButtonKind.Ghost)
                }
            }
        } else if (state.searchingNow && items.isEmpty()) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                Text(
                    "Aucun artiste ne porte ce nom.",
                    style = Sillon.type.body.copy(color = colors.dim),
                    modifier = Modifier.padding(vertical = NzSpacing.m),
                )
            }
        }
        if (state.searchNote != null) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                Text(
                    text = "${state.searchNote}. Résultats de votre bibliothèque.",
                    style = Sillon.type.labelSmall.copy(color = colors.dim),
                )
            }
        }
        if (state.searching && items.isEmpty()) {
            items(6) {
                Skeleton(
                    modifier = Modifier.padding(NzSpacing.xs),
                    kind = NzSkeletonKind.List,
                    count = 1,
                )
            }
        } else if (items.isEmpty() && !state.searchingNow) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                EmptyFeed(state, actions)
            }
        } else {
            items(items, key = { it.key.ifEmpty { it.name } }) { card ->
                ArtistTile(
                    card = card,
                    liked = state.likedIds.contains(card.key),
                    pending = state.pendingIds.contains(card.key),
                    onToggle = { actions.toggle(card) },
                )
            }
        }
        for ((parent, slot) in state.relatedSlots) {
            if (slot != OnboardingModel.RelatedSlot.Failed) continue
            item(span = { GridItemSpan(maxLineSpan) }, key = "related-failed-$parent") {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = NzSpacing.xs),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                ) {
                    Text(
                        "Les artistes proches n'ont pas pu être chargés.",
                        style = Sillon.type.labelSmall.copy(color = colors.dim),
                        modifier = Modifier.weight(1f),
                    )
                    NzButton(
                        text = "Réessayer",
                        onClick = { actions.retryRelated(parent) },
                        kind = NzButtonKind.Ghost,
                    )
                }
            }
        }
        if (state.loadingMore) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                Text(
                    "Chargement…",
                    style = Sillon.type.labelSmall.copy(color = colors.dim),
                    modifier = Modifier.fillMaxWidth().padding(NzSpacing.s),
                    textAlign = TextAlign.Center,
                )
            }
        }
        if (state.endOfList && items.size > 6) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                Text(
                    "Vous avez fait le tour des suggestions.",
                    style = Sillon.type.labelSmall.copy(color = colors.dim),
                    modifier = Modifier.fillMaxWidth().padding(NzSpacing.m),
                    textAlign = TextAlign.Center,
                )
            }
        }
    }
}

@Composable
private fun ProgressRow(state: OnboardingModel.State) {
    val colors = Sillon.colors
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                repeat(5) { index ->
                    Box(
                        Modifier
                            .size(7.dp)
                            .clip(CircleShape)
                            .background(if (index < state.liked) colors.acText else colors.ln),
                    )
                }
            }
            Spacer(Modifier.width(NzSpacing.m))
            Text(
                OnboardingCopy.artistCount(state.liked),
                style = Sillon.type.labelSmall.copy(color = colors.tx),
            )
        }
        Text(
            OnboardingCopy.progressHint(state.liked),
            style = Sillon.type.bodySmall.copy(color = colors.dim),
            modifier = Modifier.semantics {
                stateDescription = "${OnboardingCopy.artistCount(state.liked)}. " +
                    OnboardingCopy.progressHint(state.liked)
            },
        )
    }
}

@Composable
private fun EmptyFeed(state: OnboardingModel.State, actions: OnboardingActions) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier.fillMaxWidth().padding(vertical = NzSpacing.xl),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        if (state.feedUnavailable) {
            Text(
                "Aucun artiste n'est disponible pour l'instant.",
                style = Sillon.type.titleSmall.copy(color = colors.tx),
                textAlign = TextAlign.Center,
            )
            Text(
                OnboardingCopy.feedCause(NzOnboardingRules.feedCause(state.feed.error)),
                style = Sillon.type.bodySmall.copy(color = colors.dim),
                textAlign = TextAlign.Center,
            )
            NzButton(
                text = "Réessayer",
                onClick = actions.retryFeed,
                kind = NzButtonKind.Secondary,
                glyph = NzGlyph.Refresh,
            )
            if (state.canPass) {
                Text(
                    "En passant, vos recommandations resteront génériques au début. " +
                        "Vous pourrez suivre des artistes plus tard.",
                    style = Sillon.type.bodySmall.copy(color = colors.dim),
                    textAlign = TextAlign.Center,
                )
                NzButton("Passer pour l'instant", actions.pass, kind = NzButtonKind.Ghost)
            }
        } else {
            Text(
                "Nous préparons une sélection…",
                style = Sillon.type.body.copy(color = colors.dim),
            )
            Skeleton(modifier = Modifier.fillMaxWidth(), kind = NzSkeletonKind.List, count = 4)
        }
    }
}

@Composable
private fun ArtistTile(
    card: ArtistCard,
    liked: Boolean,
    pending: Boolean,
    onToggle: () -> Unit,
) {
    val colors = Sillon.colors
    val relationship = card.parent?.let { "Proche" }
    val label = buildString {
        append(if (liked) "Retirer " else "Ajouter ")
        append(card.name)
        append(" à mes artistes")
        if (relationship != null) append(", artiste proche")
    }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.card))
            .clickable(role = Role.Button, onClickLabel = label, onClick = onToggle)
            .padding(NzSpacing.xs)
            .semantics {
                contentDescription = label
                stateDescription = when {
                    pending -> "Enregistrement"
                    liked -> "Suivi"
                    else -> "Non suivi"
                }
            },
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val side = minOf(maxWidth, 176.dp)
            Avatar(
                name = card.name,
                sizeDp = side,
                url = card.picture ?: card.pictureSmall,
                modifier = Modifier.align(Alignment.Center),
            )
            Box(
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(NzSpacing.xs)
                    .size(30.dp)
                    .clip(CircleShape)
                    .background(if (liked) colors.ac else colors.s1)
                    .border(1.dp, if (liked) colors.acText else colors.ln, CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    glyph = if (liked) NzGlyph.HeartFilled else NzGlyph.Heart,
                    size = 16.dp,
                    tint = if (liked) colors.onac else colors.tx,
                )
            }
        }
        Text(
            text = card.name,
            style = Sillon.type.label.copy(color = colors.tx),
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.heightIn(min = 34.dp),
        )
        if (relationship != null) {
            Text(
                relationship.uppercase(),
                style = Sillon.type.labelMono.copy(color = colors.dim),
            )
        }
    }
}

@Composable
private fun AlertLine(message: String, onRetry: (() -> Unit)?) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NzRadius.field))
            .background(colors.s1)
            .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.field))
            .padding(NzSpacing.m)
            .semantics { liveRegion = LiveRegionMode.Assertive },
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(NzGlyph.Alert, size = 16.dp, tint = colors.acText)
        Text(message, style = Sillon.type.bodySmall.copy(color = colors.tx), modifier = Modifier.weight(1f))
        if (onRetry != null) NzButton("Réessayer", onRetry, kind = NzButtonKind.Ghost)
    }
}

/** The artist search box: 48 dp, a glyph, a clear control (the page's own). */
@Composable
private fun SearchField(
    value: String,
    onValue: (String) -> Unit,
    onSubmit: () -> Unit,
    onClear: () -> Unit,
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.pill)
    BasicTextField(
        value = value,
        onValueChange = onValue,
        singleLine = true,
        textStyle = Sillon.type.body.copy(color = colors.tx),
        cursorBrush = SolidColor(colors.acText),
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        keyboardActions = KeyboardActions(onSearch = { onSubmit() }),
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = NzMinTouchTarget)
            .semantics { contentDescription = "Chercher un artiste" },
        decorationBox = { inner ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = NzMinTouchTarget)
                    .clip(shape)
                    .background(colors.s1)
                    .border(1.dp, colors.ln, shape)
                    .padding(horizontal = NzSpacing.m),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Icon(NzGlyph.Search, size = 17.dp, tint = colors.dim)
                Box(Modifier.weight(1f)) {
                    if (value.isEmpty()) {
                        Text(
                            "Chercher un artiste…",
                            style = Sillon.type.body.copy(color = colors.dim),
                        )
                    }
                    CompositionLocalProvider(LocalContentColor provides colors.tx) { inner() }
                }
                if (value.isNotEmpty()) {
                    NzIconButton(
                        glyph = NzGlyph.Close,
                        contentDescription = "Effacer la recherche",
                        onClick = onClear,
                        size = 16.dp,
                    )
                }
            }
        },
    )
}

// -- the account step ---------------------------------------------------------

@Composable
private fun AccountPane(state: OnboardingModel.State, actions: OnboardingActions) {
    val colors = Sillon.colors
    val list = rememberLazyListState()
    LazyColumn(
        state = list,
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(
            start = NzSpacing.pageX,
            end = NzSpacing.pageX,
            bottom = NzSpacing.xxl,
        ),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.l),
    ) {
        item {
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                Text(
                    "Reliez ce que vous écoutez ailleurs",
                    style = Sillon.type.title.copy(color = colors.tx),
                    modifier = Modifier.semantics { heading() },
                )
                Text(
                    "Chaque option est facultative. Rien n'est publié ni modifié sur vos comptes.",
                    style = Sillon.type.body.copy(color = colors.dim),
                )
            }
        }
        state.accountNotice?.let { notice ->
            item { AlertLine(notice, onRetry = null) }
        }
        state.actionError?.let { error ->
            item {
                AlertLine(error.message) {
                    error.retry?.invoke()
                    actions.dismissError()
                }
            }
        }
        val options = state.options
        if (state.optionsFailure != null) {
            item {
                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                    LoadFailed(cause = state.optionsFailure, onRetry = actions.loadOptions)
                }
            }
        } else if (options == null) {
            item { Skeleton(Modifier.fillMaxWidth(), NzSkeletonKind.List, count = 3) }
        } else {
            item {
                DeezerCard(options, state, actions)
            }
            item {
                YouTubeCard(options, state, actions)
            }
            item {
                PlaylistCard(options, state, actions)
            }
            if (state.importRunning) {
                item {
                    Text(
                        "Un import est déjà en cours. Attendez qu'il finisse pour en lancer un autre.",
                        style = Sillon.type.bodySmall.copy(color = colors.dim),
                    )
                }
            }
            item {
                Text(
                    "Vous pourrez relier ces comptes plus tard dans Réglages, puis Sources.",
                    style = Sillon.type.bodySmall.copy(color = colors.dim),
                )
            }
        }
    }
}

@Composable
private fun ProviderCard(
    key: String,
    title: String,
    description: String,
    glyph: NzGlyph,
    provider: ProviderOptions,
    summary: OnboardingCopy.Summary?,
    state: OnboardingModel.State,
    actions: OnboardingActions,
    secretLabel: String,
    secretPlaceholder: String,
    secretHint: String,
    submitLabel: String,
    busyLabel: String,
) {
    val colors = Sillon.colors
    val skipped = state.skipped.contains(key)
    NzCard(Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            if (skipped) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                ) {
                    Icon(glyph, size = 16.dp, tint = colors.dim)
                    Text(
                        "$title — Ignoré",
                        style = Sillon.type.label.copy(color = colors.dim),
                        modifier = Modifier.weight(1f),
                    )
                    NzButton(
                        "Relier quand même",
                        { actions.unskip(key) },
                        kind = NzButtonKind.Ghost,
                    )
                }
                return@Column
            }
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                Box(
                    Modifier
                        .size(44.dp)
                        .clip(RoundedCornerShape(NzRadius.field))
                        .background(colors.s2),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(glyph, size = 20.dp, tint = colors.tx)
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                    Text(title, style = Sillon.type.titleSmall.copy(color = colors.tx))
                    Text(description, style = Sillon.type.bodySmall.copy(color = colors.dim))
                }
            }
            if (summary != null) {
                SummaryLine(summary)
            }
            val account = provider.account
            if (provider.linked && account != null && provider.available) {
                Text(
                    if (account.running != null) {
                        "L'import de votre compte continue sur le serveur. Vous pouvez quitter cette page."
                    } else {
                        "Votre compte est relié : ses nouveaux éléments sont importés automatiquement."
                    },
                    style = Sillon.type.bodySmall.copy(color = colors.dim),
                )
            } else if (provider.canLink && state.open != key) {
                NzButton(
                    text = if (key == "youtube") "Relier YouTube Music" else "Relier mon compte",
                    onClick = { actions.open(key) },
                    enabled = !state.importRunning,
                )
            } else if (provider.canLink) {
                var secret by rememberSaveable(key) { mutableStateOf("") }
                NzField(
                    label = secretLabel,
                    placeholder = secretPlaceholder,
                    value = secret,
                    onValue = { secret = it },
                    hint = secretHint,
                    masked = true,
                )
                NzButton(
                    text = if (state.linkBusy == key) busyLabel else submitLabel,
                    onClick = { actions.link(key, secret) },
                    enabled = state.linkBusy.isEmpty(),
                )
            }
            state.linkError[key]?.let { AlertLine(it, onRetry = null) }
            NzButton(
                text = "Ignorer",
                onClick = { actions.skip(key) },
                kind = NzButtonKind.Ghost,
            )
        }
    }
}

@Composable
private fun SummaryLine(summary: OnboardingCopy.Summary) {
    val colors = Sillon.colors
    val (glyph, tint) = when (summary.tone) {
        OnboardingCopy.Tone.Ok -> NzGlyph.Check to colors.dim
        OnboardingCopy.Tone.Error -> NzGlyph.Alert to colors.acText
        OnboardingCopy.Tone.Busy -> NzGlyph.Refresh to colors.dim
        OnboardingCopy.Tone.Idle -> NzGlyph.Info to colors.dim
    }
    Row(
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(glyph, size = 14.dp, tint = tint)
        Text(summary.text, style = Sillon.type.bodySmall.copy(color = colors.dim))
    }
}

@Composable
private fun DeezerCard(
    options: AccountOptions,
    state: OnboardingModel.State,
    actions: OnboardingActions,
) {
    ProviderCard(
        key = "deezer",
        title = "Deezer",
        description = "Retrouvez vos artistes suivis, vos titres aimés et vos playlists. " +
            "Le compte est lu, jamais modifié.",
        glyph = NzGlyph.Music,
        provider = options.deezer,
        summary = OnboardingCopy.deezerState(options.deezer),
        state = state,
        actions = actions,
        secretLabel = "ARL Deezer",
        secretPlaceholder = "Collez votre ARL",
        secretHint = "Votre ARL vaut une session : il est chiffré sur le serveur et ne sert qu'à " +
            "lire vos favoris.",
        submitLabel = "Relier mon compte",
        busyLabel = "Connexion…",
    )
}

@Composable
private fun YouTubeCard(
    options: AccountOptions,
    state: OnboardingModel.State,
    actions: OnboardingActions,
) {
    ProviderCard(
        key = "youtube",
        title = "YouTube Music",
        description = "Importez vos playlists et vos favoris. La connexion se fait depuis un " +
            "ordinateur, avec les en-têtes d'une session ouverte.",
        glyph = NzGlyph.Youtube,
        provider = options.youtube,
        summary = OnboardingCopy.youtubeState(options.youtube),
        state = state,
        actions = actions,
        secretLabel = "En-têtes de la requête browse",
        secretPlaceholder = "cookie: …\nauthorization: …",
        secretHint = "Copiez les en-têtes d'une requête browse de music.youtube.com, connecté. " +
            "Ils valent une session : ils sont chiffrés sur le serveur.",
        submitLabel = "Relier YouTube Music",
        busyLabel = "Connexion…",
    )
}

@Composable
private fun PlaylistCard(
    options: AccountOptions,
    state: OnboardingModel.State,
    actions: OnboardingActions,
) {
    val colors = Sillon.colors
    val key = "playlist"
    val skipped = state.skipped.contains(key)
    NzCard(Modifier.fillMaxWidth()) {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            if (skipped) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(NzGlyph.Link, size = 16.dp, tint = colors.dim)
                    Text(
                        "Playlist publique — Ignoré",
                        style = Sillon.type.label.copy(color = colors.dim),
                        modifier = Modifier.weight(1f),
                    )
                    NzButton("Importer quand même", { actions.unskip(key) }, kind = NzButtonKind.Ghost)
                }
                return@Column
            }
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                Box(
                    Modifier
                        .size(44.dp)
                        .clip(RoundedCornerShape(NzRadius.field))
                        .background(colors.s2),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(NzGlyph.Link, size = 20.dp, tint = colors.tx)
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                    Text("Playlist publique", style = Sillon.type.titleSmall.copy(color = colors.tx))
                    Text(
                        "Importez une playlist publique par son lien. Deezer, Spotify, Apple Music " +
                            "ou YouTube.",
                        style = Sillon.type.bodySmall.copy(color = colors.dim),
                    )
                }
            }
            val job = state.importJob
            if (job != null) {
                ImportBlock(job, state, actions)
            } else if (!OnboardingCopy.playlistAvailable(options)) {
                Text(
                    "Aucune playlist publique n'est disponible sur ce serveur.",
                    style = Sillon.type.bodySmall.copy(color = colors.dim),
                )
            } else if (state.open != key) {
                NzButton(
                    text = "Importer une playlist",
                    onClick = { actions.open(key) },
                    enabled = !state.importRunning,
                )
            } else {
                var url by rememberSaveable(key) { mutableStateOf("") }
                NzField(
                    label = "Lien de la playlist",
                    placeholder = "https://open.spotify.com/playlist/…",
                    value = url,
                    onValue = { url = it },
                    hint = "Les titres sont rapprochés de votre bibliothèque ; rien n'est " +
                        "enregistré avant votre accord.",
                    keyboardType = KeyboardType.Uri,
                )
                NzButton(
                    text = if (state.linkBusy == key) "Analyse…" else "Importer",
                    onClick = { actions.startImport(url) },
                    enabled = state.linkBusy.isEmpty(),
                )
            }
            state.linkError[key]?.let { AlertLine(it, onRetry = null) }
            NzButton("Ignorer", { actions.skip(key) }, kind = NzButtonKind.Ghost)
        }
    }
}

/** The import's progress, its per-line causes, and what it can do next. */
@Composable
private fun ImportBlock(
    job: ImportJob,
    state: OnboardingModel.State,
    actions: OnboardingActions,
) {
    val colors = Sillon.colors
    val progress = OnboardingCopy.importProgress(job)
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        SummaryLine(
            OnboardingCopy.Summary(
                progress.label,
                when {
                    progress.failed -> OnboardingCopy.Tone.Error
                    progress.busy -> OnboardingCopy.Tone.Busy
                    else -> OnboardingCopy.Tone.Ok
                },
            ),
        )
        if (progress.busy) {
            if (progress.percent == null) {
                Box(
                    Modifier
                        .fillMaxWidth()
                        .height(3.dp)
                        .background(colors.ln),
                )
            } else {
                Box(
                    Modifier
                        .fillMaxWidth()
                        .height(3.dp)
                        .background(colors.ln)
                        .semantics {
                            stateDescription = "Progression de l'import : ${progress.percent} %"
                        },
                ) {
                    Box(
                        Modifier
                            .fillMaxWidth(progress.percent / 100f)
                            .height(3.dp)
                            .background(colors.acText),
                    )
                }
            }
            Text(
                "L'import continue sur le serveur si vous quittez cette page.",
                style = Sillon.type.bodySmall.copy(color = colors.dim),
            )
        }
        if (progress.done) {
            UnmatchedBlock(job)
            val note = OnboardingCopy.importNote(job)
            if (note.isNotEmpty()) {
                Text(note, style = Sillon.type.bodySmall.copy(color = colors.dim))
            }
            if (job.state == ImportState.DONE) {
                NzButton(
                    text = if (state.importBusy == "save") "Ajout…" else "Ajouter à ma bibliothèque",
                    onClick = actions.saveImport,
                    enabled = state.importBusy.isEmpty(),
                )
            }
        }
        if (progress.failed) {
            NzButton("Relancer", actions.retryImport, kind = NzButtonKind.Secondary, glyph = NzGlyph.Refresh)
        }
        if (state.importCapped && importRunning(job)) {
            NzButton("Actualiser", actions.refreshImport, kind = NzButtonKind.Ghost)
        }
    }
}

/** The lines nobody found: each one's cause is its own title and artists. */
@Composable
private fun UnmatchedBlock(job: ImportJob) {
    val colors = Sillon.colors
    val unmatched = job.items.filter { !it.matched }
    if (unmatched.isEmpty()) return
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Text(
            OnboardingCopy.unmatchedLabel(unmatched.size),
            style = Sillon.type.label.copy(color = colors.tx),
        )
        for (item in unmatched.take(40)) {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                Text(
                    item.title,
                    style = Sillon.type.bodySmall.copy(color = colors.dim),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (item.artists.isNotEmpty()) {
                    Text(
                        "— ${item.artists}",
                        style = Sillon.type.bodySmall.copy(color = colors.dim),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        if (unmatched.size > 40) {
            Text(
                "et ${unmatched.size - 40} autres.",
                style = Sillon.type.bodySmall.copy(color = colors.dim),
            )
        }
    }
}

/** A labelled field: 48 dp minimum, its hint under it, its error in the accent. */
@Composable
internal fun NzField(
    label: String,
    placeholder: String,
    value: String,
    onValue: (String) -> Unit,
    hint: String,
    masked: Boolean = false,
    keyboardType: KeyboardType = KeyboardType.Text,
    error: String = "",
) {
    val colors = Sillon.colors
    val shape = RoundedCornerShape(NzRadius.field)
    Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
        Text(label, style = Sillon.type.label.copy(color = colors.tx))
        BasicTextField(
            value = value,
            onValueChange = onValue,
            singleLine = true,
            textStyle = Sillon.type.body.copy(color = colors.tx),
            cursorBrush = SolidColor(colors.acText),
            visualTransformation = if (masked) PasswordVisualTransformation() else VisualTransformation.None,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next, keyboardType = keyboardType),
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = NzMinTouchTarget)
                .clip(shape)
                .background(colors.bg)
                .border(1.dp, if (error.isNotEmpty()) colors.acText else colors.ln, shape)
                .padding(horizontal = NzSpacing.m, vertical = NzSpacing.m)
                .semantics { contentDescription = label },
            decorationBox = { inner ->
                Box {
                    if (value.isEmpty()) {
                        Text(placeholder, style = Sillon.type.body.copy(color = colors.dim))
                    }
                    CompositionLocalProvider(LocalContentColor provides colors.tx) { inner() }
                }
            },
        )
        Text(hint, style = Sillon.type.bodySmall.copy(color = colors.dim))
        if (error.isNotEmpty()) {
            Row(
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            ) {
                Icon(NzGlyph.Alert, size = 14.dp, tint = colors.acText)
                Text(error, style = Sillon.type.bodySmall.copy(color = colors.acText))
            }
        }
    }
}

/** Whether a job still has work, without importing the model's own private rule. */
internal fun importRunning(job: ImportJob): Boolean =
    job.state == ImportState.READING || job.state == ImportState.MATCHING ||
        job.state == ImportState.SAVING
