package fr.neezer.mobile.feature.entity

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import uniffi.nz_ffi.ArtistData
import uniffi.nz_ffi.TrackRow

/**
 * The screen's marker, present in EVERY state — a skeleton, a failure, the page —
 * and independent of the clock: what `ci-emulator.sh` and the instrumented test
 * wait for instead of a sentence that only exists once the data arrived.
 */
const val ARTIST_MARKER = "Fiche artiste"

/** S2's geometry test: the band (it cancels the page's top margin) and the list. */
const val ARTIST_BAND_TAG = "artist-band"
const val ARTIST_LIST_TAG = "artist-list"

/** Everything the artist page can do, so the screen stays a pure function. */
data class ArtistActions(
    val retry: () -> Unit = {},
    /** A tab of the page (the list tabs load on their first open). */
    val tab: (ArtistTab) -> Unit = {},
    /** A row of the popular shelf: the title then its radio. */
    val playTop: (TrackRow) -> Unit = {},
    /** « Lire »: the popular titles. */
    val playTopAll: () -> Unit = {},
    /** « Radio »: one request, then the player's own refill. */
    val radio: () -> Unit = {},
    /** « Suivre » / « Ne plus suivre ». */
    val favorite: () -> Unit = {},
    val playAllTracks: () -> Unit = {},
    val playTrack: (TrackRow) -> Unit = {},
    val queueNext: (TrackRow) -> Unit = {},
    val more: (TrackRow) -> Unit = {},
    val retryTracks: () -> Unit = {},
    val retryCredits: () -> Unit = {},
    /** A role chip of the Crédits tab. */
    val creditsRole: (String) -> Unit = {},
    val playCredits: () -> Unit = {},
    val playFromCredits: (TrackRow) -> Unit = {},
    /** A sleeve: the album page. */
    val openAlbum: (String) -> Unit = {},
    /** A similar artist. */
    val openArtist: (String) -> Unit = {},
    /** A name in the credits: the person's own page. */
    val openPerson: (String) -> Unit = {},
)

/**
 * Artiste — lot MB7, `routes/Artist.svelte` ported to Compose.
 *
 * The order is the SPA's: the band (`--s2`, the round portrait, the facts above
 * the name), then « Lire / Radio / Suivre », then the tabs Aperçu · Titres ·
 * Crédits. Aperçu shows « Dernières sorties », « Titres populaires », the album
 * grid, what only Deezer has, the singles, « Apparaît dans », the similar artists
 * and the biography — in that order, and each section only when it has something.
 *
 * Everything scrolls in ONE `LazyColumn`: the band scrolls away (as on the SPA)
 * and the two tab lists are windowed inside it (`ListWindow`, pages of 64), so a
 * 300-title artist never crosses whole.
 */
@Composable
fun ArtistScreen(
    state: ArtistModel.State,
    actions: ArtistActions,
    modifier: Modifier = Modifier,
    /** The playing track's key, when the shell knows it (MB11 feeds it). */
    playingKey: String? = null,
) {
    val colors = Sillon.colors
    BoxWithConstraints(
        modifier
            .fillMaxSize()
            .semantics { contentDescription = ARTIST_MARKER },
    ) {
        val wide = maxWidth >= 600.dp
        val columns = if (wide) 5 else 3
        val tileSize: Dp = ((maxWidth - NzSpacing.pageX * 2 - NzSpacing.m * (columns - 1)) / columns)
            .coerceAtLeast(72.dp)

        val failed = state.screen as? ScreenState.Failed
        if (failed != null) {
            Column(Modifier.fillMaxWidth().padding(NzSpacing.pageX)) {
                LoadFailed(
                    cause = failed.cause,
                    onRetry = actions.retry,
                    title = "Cette fiche n'a pas pu se charger",
                )
            }
            return@BoxWithConstraints
        }

        val listState = rememberLazyListState()
        val tracksWindow = state.trackSource?.let { source ->
            remember(source, state.id) { windowFor(source) }
        }
        val creditsWindow = state.creditsSource?.let { source ->
            remember(source, state.id) { windowFor(source) }
        }

        // A tab is a new pane: land at its top instead of wherever the previous
        // one was scrolled to (a 300-row overview would otherwise drop the
        // person mid-list). The band scrolls away with it, like the SPA's page.
        LaunchedEffect(state.tab) { listState.scrollToItem(0) }

        val rowOffset = if (state.tab == ArtistTab.Overview) 0 else 2
        if (state.tab == ArtistTab.Tracks) {
            tracksWindow?.let { ReportWindow(listState, it, rowOffset) }
        }
        if (state.tab == ArtistTab.Credits) {
            creditsWindow?.let { ReportWindow(listState, it, rowOffset) }
        }

        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize().background(colors.bg).testTag(ARTIST_LIST_TAG),
            contentPadding = PaddingValues(bottom = NzSpacing.pageBottom),
        ) {
            item(key = "band") {
                val data = state.data
                if (data != null) ArtistBand(state, data, actions, wide)
                ArtistTabs(state, actions)
                if (state.stale) {
                    EntityStaleNote(Modifier.padding(horizontal = NzSpacing.pageX))
                }
            }
            if (state.data == null) {
                item(key = "skeleton-header") {
                    Skeleton(
                        kind = NzSkeletonKind.Header,
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
                    )
                }
                item(key = "skeleton-list") {
                    Skeleton(
                        kind = NzSkeletonKind.List,
                        count = 6,
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
                    )
                }
                return@LazyColumn
            }
            when (state.tab) {
                ArtistTab.Overview -> artistOverview(state, actions, columns, tileSize, playingKey)
                ArtistTab.Tracks -> artistTracks(state, actions, tracksWindow, playingKey)
                ArtistTab.Credits -> artistCredits(state, actions, creditsWindow, playingKey)
            }
        }
    }
}

/** The band: the portrait, the facts, the name, and the three actions. */
@Composable
private fun ArtistBand(
    state: ArtistModel.State,
    data: ArtistData,
    actions: ArtistActions,
    wide: Boolean,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.s2)
            .testTag(ARTIST_BAND_TAG)
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Avatar(
                name = data.name,
                sizeDp = if (wide) 140.dp else 104.dp,
                url = data.picture,
            )
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                FactsLine(artistFacts(state, data))
                Text(
                    text = data.name,
                    style = if (wide) Sillon.type.display else Sillon.type.displaySmall,
                    color = colors.tx,
                )
            }
        }
        ActionRow {
            NzButton(
                text = "Lire",
                onClick = actions.playTopAll,
                glyph = NzGlyph.Play,
                enabled = data.top.isNotEmpty(),
            )
            NzButton(
                text = "Radio",
                onClick = actions.radio,
                glyph = NzGlyph.Radio,
                kind = NzButtonKind.Secondary,
                enabled = !state.radioBusy,
            )
            HeartButton(
                favorite = state.isFavorite,
                followed = true,
                onClick = actions.favorite,
            )
        }
    }
    Spacer(Modifier.fillMaxWidth().height(1.dp).background(colors.ln))
}

/**
 * The heart of both entity pages: `--hrt` when it is on (never the accent alone —
 * `webapp.md` §10: no information by colour only, so the label says the state).
 */
@Composable
internal fun HeartButton(favorite: Boolean, followed: Boolean, onClick: () -> Unit) {
    val colors = Sillon.colors
    val label = when {
        favorite && followed -> "Ne plus suivre"
        followed -> "Suivre"
        favorite -> "Retirer des favoris"
        else -> "Ajouter aux favoris"
    }
    NzIconButton(
        glyph = if (favorite) NzGlyph.HeartFilled else NzGlyph.Heart,
        contentDescription = label,
        onClick = onClick,
        selected = favorite,
        tint = if (favorite) colors.hrt else null,
    )
}

/** `Artist.svelte#facts`: « Artiste · 12 titres » once the tab is loaded, else the fans. */
private fun artistFacts(state: ArtistModel.State, data: ArtistData): String {
    val count = state.trackCount
    val second = when {
        count != null -> "$count titre" + if (count > 1) "s" else ""
        (data.fans ?: 0u) > 0u -> "${frenchCount((data.fans ?: 0u).toInt())} fans"
        else -> null
    }
    return listOfNotNull("Artiste", second).joinToString(" · ")
}

/**
 * Aperçu · Titres · Crédits — the third only when the artist is credited somewhere.
 *
 * A wrapping row, not a scrolling one: at 200 % « Crédits » slid past the right
 * edge, and a TAB nobody can see is a tab that does not exist (`Artist.svelte`
 * wraps its tabs too, `flex-wrap: wrap`).
 */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
private fun ArtistTabs(state: ArtistModel.State, actions: ArtistActions) {
    androidx.compose.foundation.layout.FlowRow(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.pageY),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        NzChip(
            text = "Aperçu",
            onClick = { actions.tab(ArtistTab.Overview) },
            selected = state.tab == ArtistTab.Overview,
        )
        NzChip(
            text = "Titres",
            onClick = { actions.tab(ArtistTab.Tracks) },
            selected = state.tab == ArtistTab.Tracks,
        )
        if (state.hasCredits) {
            NzChip(
                text = "Crédits",
                onClick = { actions.tab(ArtistTab.Credits) },
                selected = state.tab == ArtistTab.Credits,
            )
        }
    }
}

/** The Aperçu tab, section by section, in `Artist.svelte`'s order. */
private fun LazyListScope.artistOverview(
    state: ArtistModel.State,
    actions: ArtistActions,
    columns: Int,
    tileSize: Dp,
    playingKey: String?,
) {
    val data = state.data ?: return
    val pad = Modifier.padding(horizontal = NzSpacing.pageX)
    val latest = state.latest
    if (latest.isNotEmpty()) {
        item(key = "latest-title") { SectionTitle("Dernières sorties", pad) }
        item(key = "latest") {
            AlbumGrid(
                cards = latest,
                columns = columns,
                tileSize = tileSize,
                onOpen = { card -> actions.openAlbum(card.id) },
                modifier = pad,
            )
        }
    }
    if (data.top.isNotEmpty()) {
        item(key = "top-title") { SectionTitle("Titres populaires", pad) }
        items(
            count = data.top.size,
            key = { index -> "top-${data.top[index].key}-$index" },
        ) { index ->
            val row = data.top[index]
            EntityTrackRow(
                row = row,
                number = index + 1,
                playing = playingKey == row.key,
                onClick = { actions.playTop(row) },
                onMore = { actions.more(row) },
                onSwipeQueue = { actions.queueNext(row) },
                modifier = pad,
            )
        }
    }
    val albums = state.albums
    if (albums.isNotEmpty()) {
        item(key = "albums-title") { SectionTitle("Albums", pad) }
        item(key = "albums") {
            AlbumGrid(
                cards = albums,
                columns = columns,
                tileSize = tileSize,
                onOpen = { card -> actions.openAlbum(card.id) },
                modifier = pad,
            )
        }
    }
    if (data.library) {
        when {
            state.onDeezerBusy && !state.onDeezerFetched -> {
                item(key = "deezer-title") { SectionTitle("Disponible sur Deezer", pad) }
                item(key = "deezer-skeleton") {
                    Skeleton(kind = NzSkeletonKind.Shelf, count = 5, modifier = pad)
                }
            }
            state.onDeezer.isNotEmpty() -> {
                item(key = "deezer-title") { SectionTitle("Disponible sur Deezer", pad) }
                item(key = "deezer") {
                    AlbumGrid(
                        cards = state.onDeezer,
                        columns = columns,
                        tileSize = tileSize,
                        onOpen = { card -> actions.openAlbum(card.id) },
                        modifier = pad,
                    )
                }
            }
            state.deezerDown -> item(key = "deezer-down") {
                MutedNote(
                    "Deezer est injoignable : le reste du catalogue n'est pas affiché.",
                    pad.padding(vertical = NzSpacing.s),
                )
            }
        }
    }
    if (state.singles.isNotEmpty()) {
        item(key = "singles-title") { SectionTitle("Singles & EP", pad) }
        item(key = "singles") {
            AlbumGrid(
                cards = state.singles,
                columns = columns,
                tileSize = tileSize,
                onOpen = { card -> actions.openAlbum(card.id) },
                modifier = pad,
            )
        }
    }
    if (state.featured.isNotEmpty()) {
        item(key = "featured-title") { SectionTitle("Apparaît dans", pad) }
        item(key = "featured") {
            AlbumGrid(
                cards = state.featured,
                columns = columns,
                tileSize = tileSize,
                onOpen = { card -> actions.openAlbum(card.id) },
                modifier = pad,
            )
        }
    }
    if (data.related.isNotEmpty()) {
        item(key = "related-title") { SectionTitle("Artistes similaires", pad) }
        item(key = "related") {
            ArtistShelf(cards = data.related, onOpen = actions.openArtist, modifier = pad)
        }
    }
    data.bio?.let { bio ->
        item(key = "bio-title") { SectionTitle("Biographie", pad) }
        item(key = "bio") {
            Text(
                text = bio,
                style = Sillon.type.body,
                color = Sillon.colors.dim,
                modifier = pad.padding(bottom = NzSpacing.l),
            )
        }
    }
}

/** The Titres tab: « Tout lire », the count, and the windowed list. */
private fun LazyListScope.artistTracks(
    state: ArtistModel.State,
    actions: ArtistActions,
    window: fr.neezer.mobile.designsystem.ListWindow?,
    playingKey: String?,
) {
    val pad = Modifier.padding(horizontal = NzSpacing.pageX)
    val failed = state.tracks as? ScreenState.Failed
    if (failed != null) {
        item(key = "tracks-failed") {
            LoadFailed(
                cause = failed.cause,
                onRetry = actions.retryTracks,
                title = "Les titres n'ont pas pu se charger",
                modifier = pad,
            )
        }
        return
    }
    val source = state.trackSource
    if (source == null) {
        item(key = "tracks-skeleton") {
            Skeleton(kind = NzSkeletonKind.List, count = 6, modifier = pad)
        }
        return
    }
    val count = source.len()
    if (count == 0) {
        item(key = "tracks-empty") {
            MutedNote("Aucun titre trouvé pour cet artiste.", pad.padding(vertical = NzSpacing.l))
        }
        return
    }
    item(key = "tracks-bar") {
        Row(
            modifier = pad.padding(vertical = NzSpacing.s),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            NzButton(
                text = "Tout lire",
                onClick = actions.playAllTracks,
                glyph = NzGlyph.Play,
            )
            Text(
                text = "$count titres",
                style = Sillon.type.labelMono,
                color = Sillon.colors.dim,
            )
        }
    }
    items(count = count, key = { index -> "track-$index" }) { index ->
        val row = window?.rowAt(index)
        if (row == null) {
            Skeleton(kind = NzSkeletonKind.List, count = 1, modifier = pad)
        } else {
            EntityTrackRow(
                row = row,
                number = index + 1,
                playing = playingKey == row.key,
                onClick = { actions.playTrack(row) },
                onMore = { actions.more(row) },
                onSwipeQueue = { actions.queueNext(row) },
                modifier = pad,
            )
        }
    }
}

/** The Crédits tab: the person's page, its chips, and the windowed list. */
private fun LazyListScope.artistCredits(
    state: ArtistModel.State,
    actions: ArtistActions,
    window: fr.neezer.mobile.designsystem.ListWindow?,
    playingKey: String?,
) {
    val pad = Modifier.padding(horizontal = NzSpacing.pageX)
    val failed = state.credits as? ScreenState.Failed
    if (failed != null) {
        item(key = "credits-failed") {
            LoadFailed(
                cause = failed.cause,
                onRetry = actions.retryCredits,
                title = "Les crédits n'ont pas pu se charger",
                modifier = pad,
            )
        }
        return
    }
    val page = state.creditsPage
    if (page == null) {
        item(key = "credits-skeleton") {
            Skeleton(kind = NzSkeletonKind.List, count = 6, modifier = pad)
        }
        return
    }
    item(key = "credits-bar") {
        PersonCreditsBar(
            page = page,
            role = state.creditsRole,
            count = state.creditsSource?.len() ?: 0,
            onRole = actions.creditsRole,
            onPlayAll = actions.playCredits,
            modifier = pad.padding(vertical = NzSpacing.s),
        )
    }
    val count = state.creditsSource?.len() ?: 0
    if (count == 0) {
        item(key = "credits-empty") {
            MutedNote("Aucun titre pour ce rôle.", pad.padding(vertical = NzSpacing.l))
        }
        return
    }
    items(count = count, key = { index -> "credited-$index" }) { index ->
        val row = window?.rowAt(index)
        if (row == null) {
            Skeleton(kind = NzSkeletonKind.List, count = 1, modifier = pad)
        } else {
            EntityTrackRow(
                row = row,
                playing = playingKey == row.key,
                onClick = { actions.playFromCredits(row) },
                onMore = { actions.more(row) },
                onSwipeQueue = { actions.queueNext(row) },
                modifier = pad,
            )
        }
    }
}

/** « Tout lire », the count, and one chip per role (`PersonCredits.svelte`). */
@Composable
internal fun PersonCreditsBar(
    page: PersonView,
    role: String,
    count: Int,
    onRole: (String) -> Unit,
    onPlayAll: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            NzButton(
                text = "Tout lire",
                onClick = onPlayAll,
                glyph = NzGlyph.Play,
                enabled = count > 0,
            )
            Text(text = "$count titres", style = Sillon.type.mono, color = colors.dim)
        }
        if (page.roles.size > 1) {
            Row(
                modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                RoleChip(
                    label = "Tout",
                    count = page.count,
                    selected = role.isEmpty(),
                    onClick = { onRole("") },
                )
                page.roles.forEach { entry ->
                    RoleChip(
                        label = entry.label,
                        count = entry.count.toInt(),
                        selected = role == entry.role,
                        onClick = { onRole(entry.role) },
                    )
                }
            }
        } else if (page.roles.size == 1) {
            MutedNote(page.roles[0].label)
        }
    }
}

/** The empty-state sentence of a person's page (`PersonCredits.svelte#empty`). */
internal fun emptyPersonSentence(name: String): String =
    "Aucun titre de la bibliothèque ne crédite ${name.ifBlank { "cette personne" }} pour l'instant."
