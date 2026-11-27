package fr.neezer.mobile.feature.entity

import androidx.compose.foundation.background
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import fr.neezer.mobile.designsystem.nzPaneEnter
import fr.neezer.mobile.designsystem.rememberPaneEnter
import fr.neezer.mobile.feature.offline.DownloadButton
import uniffi.nz_ffi.TrackRow

/** The screen's marker, in every state (see [ARTIST_MARKER]). */
const val ALBUM_MARKER = "Fiche album"

/** S2's geometry test: the band (it cancels the page's top margin) and the list. */
const val ALBUM_BAND_TAG = "album-band"
const val ALBUM_LIST_TAG = "album-list"

/**
 * What the page is drawing: its data arrived, its read failed, or nothing yet —
 * the pane's key (D2), from the same fields the branches below test.
 */
private enum class AlbumPhase { Loading, Failed, Content }

private fun albumPhase(state: AlbumModel.State): AlbumPhase = when {
    state.screen is ScreenState.Failed -> AlbumPhase.Failed
    state.data == null -> AlbumPhase.Loading
    else -> AlbumPhase.Content
}

/** Everything the album page can do. */
data class AlbumActions(
    val retry: () -> Unit = {},
    /** « Lire »: the album, from its first title — no request. */
    val playAll: () -> Unit = {},
    /** « Mélanger »: the same queue, shuffled once. */
    val shuffle: () -> Unit = {},
    /** « Favori » / « Retirer des favoris ». */
    val favorite: () -> Unit = {},
    val playFrom: (TrackRow) -> Unit = {},
    /** A row of the provider's remainder: the title then its radio. */
    val playMissing: (TrackRow) -> Unit = {},
    val queueNext: (TrackRow) -> Unit = {},
    val more: (TrackRow) -> Unit = {},
    /** The artist's page. */
    val openArtist: (String) -> Unit = {},
)

/**
 * Album — lot MB7, `routes/Album.svelte` ported to Compose.
 *
 * The band is the SPA's `--s2` row: a 96 dp sleeve, the facts above the title
 * (« Album · 1998 · 12 titres · 43:12 ») and the artist under it as a link. Then
 * « Lire », « Mélanger », « Télécharger » (MB15's own button), the heart — and
 * the tracklist, windowed, followed by « Disponible sur Deezer » when the library
 * does not hold the whole release.
 */
@Composable
fun AlbumScreen(
    state: AlbumModel.State,
    actions: AlbumActions,
    modifier: Modifier = Modifier,
    /** The playing track's key, when the shell knows it (MB11 feeds it). */
    playingKey: String? = null,
) {
    val colors = Sillon.colors
    // The page's phases are one pane (the decision note's D2): skeleton → album →
    // failure. Its key is the phase, so the content enters while nothing else on
    // the screen exists to be moved. The band is INSIDE the pane here — unlike the
    // artist's, it is a header skeleton until the page arrives, so it is part of
    // what changes.
    val pane = rememberPaneEnter(albumPhase(state))
    // Read in the composition: `items` runs in the LazyColumn's scope, where a
    // `CompositionLocal` cannot be read.
    val specs = Sillon.motion.itemSpecs()
    val enter = Modifier.nzPaneEnter(pane)
    // No `BoxWithConstraints`: this page has no table width to compute (the
    // artist's grid does), and lint rightly refuses a scope nothing reads.
    Box(
        modifier
            .fillMaxSize()
            .semantics { contentDescription = ALBUM_MARKER },
    ) {
        val failed = state.screen as? ScreenState.Failed
        if (failed != null) {
            Column(enter.fillMaxWidth().padding(NzSpacing.pageX)) {
                LoadFailed(
                    cause = failed.cause,
                    onRetry = actions.retry,
                    title = "Cet album n'a pas pu se charger",
                )
            }
            return@Box
        }

        val page = state.data
        val listState = rememberLazyListState()
        val window = page?.tracks?.let { source -> remember(source, state.id) { windowFor(source) } }
        window?.let { ReportWindow(listState, it, offset = 1) }

        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize().background(colors.bg).testTag(ALBUM_LIST_TAG),
            contentPadding = PaddingValues(bottom = NzSpacing.pageBottom),
        ) {
            item(key = "band") {
                if (page != null) {
                    AlbumBand(page = page, state = state, actions = actions, modifier = enter)
                    if (state.stale) {
                        EntityStaleNote(Modifier.padding(horizontal = NzSpacing.pageX))
                    }
                } else {
                    Skeleton(
                        kind = NzSkeletonKind.Header,
                        modifier = enter.padding(horizontal = NzSpacing.pageX),
                    )
                }
            }
            if (page == null) {
                item(key = "skeleton-list") {
                    Skeleton(
                        kind = NzSkeletonKind.List,
                        count = 6,
                        modifier = enter.padding(horizontal = NzSpacing.pageX),
                    )
                }
                return@LazyColumn
            }
            val pad = Modifier.padding(horizontal = NzSpacing.pageX)
            // A row's key is the TRACK's identity, not its rank: an indexed key
            // makes every row below a removal a new item, so `animateItem` has
            // nothing to move (measured in X5). An album holds each title once, so
            // the identity is unique here; a page that has not landed yet draws a
            // skeleton under its own `pending-<index>` key.
            items(count = page.tracks.len(), key = { index -> window?.rowAt(index)?.key ?: "pending-$index" }) { index ->
                val row = window?.rowAt(index)
                if (row == null) {
                    // D4: the skeleton's key leaves as the row's arrives; nothing
                    // fades out over the row that replaces it.
                    Skeleton(
                        kind = NzSkeletonKind.List,
                        count = 1,
                        modifier = enter.animateItem(null, specs.placement, null).then(pad),
                    )
                } else {
                    EntityTrackRow(
                        row = row,
                        number = index + 1,
                        showCover = false,
                        playing = playingKey == row.key,
                        onClick = { actions.playFrom(row) },
                        onMore = { actions.more(row) },
                        onSwipeQueue = { actions.queueNext(row) },
                        modifier = enter.animateItem(specs.fadeIn, specs.placement, specs.fadeOut).then(pad),
                    )
                }
            }
            // « Disponible sur Deezer »: the rest of the tracklist, the SPA's own
            // section under the library's rows. It is ONE album's remainder, so it
            // crosses whole and is composed as ordinary (virtualised) items.
            if (page.library) {
                when {
                    state.onDeezerBusy && !state.onDeezerFetched -> item(key = "more-skeleton") {
                        Column(enter.then(pad)) {
                            SectionTitle("Disponible sur Deezer")
                            Skeleton(kind = NzSkeletonKind.List, count = 3)
                        }
                    }
                    state.onDeezer.isNotEmpty() -> {
                        item(key = "more-title") {
                            SectionTitle("Disponible sur Deezer", enter.then(pad).padding(top = NzSpacing.xl))
                        }
                        items(
                            count = state.onDeezer.size,
                            key = { index -> "missing-${state.onDeezer[index].key}-$index" },
                        ) { index ->
                            val row = state.onDeezer[index]
                            EntityTrackRow(
                                row = row,
                                playing = playingKey == row.key,
                                onClick = { actions.playMissing(row) },
                                onMore = { actions.more(row) },
                                onSwipeQueue = { actions.queueNext(row) },
                                modifier = enter.animateItem(specs.fadeIn, specs.placement, specs.fadeOut).then(pad),
                            )
                        }
                    }
                    state.deezerDown -> item(key = "more-down") {
                        MutedNote(
                            "Deezer est injoignable : la suite de l'album n'est pas affichée.",
                            pad.padding(top = NzSpacing.l),
                        )
                    }
                }
            }
        }
    }
}

/** The band: sleeve, facts, title, the artist link, and the actions. */
@Composable
private fun AlbumBand(
    page: AlbumView,
    state: AlbumModel.State,
    actions: AlbumActions,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(colors.s2)
            .testTag(ALBUM_BAND_TAG)
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        // The facts span the whole band: beside a 96 dp sleeve they wrapped after
        // the last separator at 360 dp (« … 12 titres · / 37:06 »), which read as
        // a broken line rather than as one fact.
        FactsLine(state.facts)
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            CoverImage(url = page.cover, sizeDp = 96.dp, seed = page.id)
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                Text(
                    text = page.title,
                    style = Sillon.type.displaySmall,
                    color = colors.tx,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
                if (page.artistName.isNotBlank()) {
                    Text(
                        text = page.artistName,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier
                            .then(
                                if (page.artistId.isBlank()) {
                                    Modifier
                                } else {
                                    Modifier
                                        .clickable(role = Role.Button) { actions.openArtist(page.artistId) }
                                        .semantics {
                                            contentDescription = "${page.artistName}, ouvrir sa fiche"
                                        }
                                },
                            ),
                    )
                }
            }
        }
        ActionRow {
            NzButton(text = "Lire", onClick = actions.playAll, glyph = NzGlyph.Play)
            NzButton(
                text = "Mélanger",
                onClick = actions.shuffle,
                glyph = NzGlyph.Shuffle,
                kind = NzButtonKind.Secondary,
            )
            // MB15's own button: the album is what a person downloads offline.
            DownloadButton(context = "album:${page.id}")
            HeartButton(favorite = state.isFavorite, followed = false, onClick = actions.favorite)
        }
    }
    Spacer(Modifier.fillMaxWidth().height(1.dp).background(colors.ln))
}
