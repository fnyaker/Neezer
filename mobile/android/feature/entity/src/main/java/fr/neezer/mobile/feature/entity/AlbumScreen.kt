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
import fr.neezer.mobile.feature.offline.DownloadButton
import uniffi.nz_ffi.TrackRow

/** The screen's marker, in every state (see [ARTIST_MARKER]). */
const val ALBUM_MARKER = "Fiche album"

/** S2's geometry test: the band (it cancels the page's top margin) and the list. */
const val ALBUM_BAND_TAG = "album-band"
const val ALBUM_LIST_TAG = "album-list"

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
    // No `BoxWithConstraints`: this page has no table width to compute (the
    // artist's grid does), and lint rightly refuses a scope nothing reads.
    Box(
        modifier
            .fillMaxSize()
            .semantics { contentDescription = ALBUM_MARKER },
    ) {
        val failed = state.screen as? ScreenState.Failed
        if (failed != null) {
            Column(Modifier.fillMaxWidth().padding(NzSpacing.pageX)) {
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
                    AlbumBand(page = page, state = state, actions = actions)
                    if (state.stale) {
                        EntityStaleNote(Modifier.padding(horizontal = NzSpacing.pageX))
                    }
                } else {
                    Skeleton(
                        kind = NzSkeletonKind.Header,
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
                    )
                }
            }
            if (page == null) {
                item(key = "skeleton-list") {
                    Skeleton(
                        kind = NzSkeletonKind.List,
                        count = 6,
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
                    )
                }
                return@LazyColumn
            }
            val pad = Modifier.padding(horizontal = NzSpacing.pageX)
            items(count = page.tracks.len(), key = { index -> "album-track-$index" }) { index ->
                val row = window?.rowAt(index)
                if (row == null) {
                    Skeleton(kind = NzSkeletonKind.List, count = 1, modifier = pad)
                } else {
                    EntityTrackRow(
                        row = row,
                        number = index + 1,
                        showCover = false,
                        playing = playingKey == row.key,
                        onClick = { actions.playFrom(row) },
                        onMore = { actions.more(row) },
                        onSwipeQueue = { actions.queueNext(row) },
                        modifier = pad,
                    )
                }
            }
            // « Disponible sur Deezer »: the rest of the tracklist, the SPA's own
            // section under the library's rows. It is ONE album's remainder, so it
            // crosses whole and is composed as ordinary (virtualised) items.
            if (page.library) {
                when {
                    state.onDeezerBusy && !state.onDeezerFetched -> item(key = "more-skeleton") {
                        Column(pad) {
                            SectionTitle("Disponible sur Deezer")
                            Skeleton(kind = NzSkeletonKind.List, count = 3)
                        }
                    }
                    state.onDeezer.isNotEmpty() -> {
                        item(key = "more-title") { SectionTitle("Disponible sur Deezer", pad.padding(top = NzSpacing.xl)) }
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
                                modifier = pad,
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
private fun AlbumBand(page: AlbumView, state: AlbumModel.State, actions: AlbumActions) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
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
