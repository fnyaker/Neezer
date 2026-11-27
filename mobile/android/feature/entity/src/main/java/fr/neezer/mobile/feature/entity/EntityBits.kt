package fr.neezer.mobile.feature.entity

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.ListWindow
import fr.neezer.mobile.designsystem.NzAppearOnMount
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.SwipeRow
import fr.neezer.mobile.designsystem.nzDuration
import uniffi.nz_ffi.AlbumCardData
import uniffi.nz_ffi.TrackRow

/** How many rows past the viewport are reported to the window (MB1's own margin). */
internal const val WINDOW_OVERSCAN = 24

/**
 * Tells a [ListWindow] which rows the viewport is showing — on every CHANGE of
 * the lazy state, never on a clock (`snapshotFlow`: a state change is the event).
 *
 * [offset] is how many lazily-composed items sit above the window (the banner,
 * the actions, the tab bar): a screen with a header of two items reports
 * `firstVisibleItemIndex - 2` as its first row.
 */
@Composable
internal fun ReportWindow(state: LazyListState, window: ListWindow, offset: Int) {
    LaunchedEffect(state, window, offset) {
        snapshotFlow {
            Triple(
                state.firstVisibleItemIndex,
                state.layoutInfo.visibleItemsInfo.size,
                state.isScrollInProgress,
            )
        }.collect { (first, count, scrolling) ->
            val start = (first - offset).coerceAtLeast(0)
            window.onVisible(start, start + count.coerceAtLeast(1) + WINDOW_OVERSCAN, scrolling)
        }
    }
}

/** « Hors ligne · version mémorisée » — the copy is what is on screen (rule 3). */
@Composable
internal fun EntityStaleNote(modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.pill))
            .background(colors.s2)
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
            .semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = NzGlyph.CloudOff, size = 15.dp, tint = colors.dim)
        Text(
            text = "Hors ligne · version mémorisée",
            style = Sillon.type.labelSmall,
            color = colors.dim,
        )
    }
}

/** A section's title (`webapp.md` §6: `.section-title`). */
@Composable
internal fun SectionTitle(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        style = Sillon.type.title,
        color = Sillon.colors.tx,
        modifier = modifier.padding(top = NzSpacing.pageY, bottom = NzSpacing.s),
    )
}

/** A mono facts line: « Album · 1998 · 12 titres · 43:12 ». */
@Composable
internal fun FactsLine(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text.uppercase(),
        style = Sillon.type.labelMono,
        color = Sillon.colors.dim,
        modifier = modifier,
    )
}

/**
 * A dim sentence, for a section that has nothing to show.
 *
 * It ENTERS when it joins the tree (`NzAppearOnMount`, 140 ms + 4 dp at Full,
 * nothing at eco) instead of popping: the empty note of a list appears once its
 * phase says so, and every caller inherits the one definition (lot X4). A pane's
 * `nzPaneEnter` is NOT applied to an item that is only a note — two fades
 * multiplied would play a sharper 140 ms (the decision note's D2).
 */
@Composable
internal fun MutedNote(text: String, modifier: Modifier = Modifier) {
    NzAppearOnMount(modifier = modifier) {
        Text(
            text = text,
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
        )
    }
}

/**
 * One release: the sleeve, its title, and « Album · 1998 ». The whole tile is the
 * target (`Card.svelte`'s rule: never a number without a name).
 */
@Composable
internal fun AlbumTile(
    card: AlbumCardData,
    size: Dp,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .width(size)
            .clip(RoundedCornerShape(NzRadius.cover))
            .clickable(role = Role.Button, onClick = onClick)
            .padding(bottom = NzSpacing.xs)
            .semantics {
                contentDescription = "${card.title}, ${albumSubtitle(card)}"
            },
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        CoverImage(url = card.cover, sizeDp = size, seed = card.id)
        Text(
            text = card.title,
            style = Sillon.type.label,
            color = colors.tx,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = NzSpacing.xs),
        )
        Text(
            text = albumSubtitle(card),
            style = Sillon.type.labelSmall,
            color = colors.dim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/**
 * A grid of releases, [columns] across. Laid out as rows of tiles rather than a
 * `LazyVerticalGrid`: the grid is a section of a scrolling page, and nesting the
 * two scrolls is the trap `webapp.md` pays for every time.
 */
@Composable
internal fun AlbumGrid(
    cards: List<AlbumCardData>,
    columns: Int,
    tileSize: Dp,
    onOpen: (AlbumCardData) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        cards.chunked(columns.coerceAtLeast(1)).forEach { line ->
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                line.forEach { card ->
                    AlbumTile(card = card, size = tileSize, onClick = { onOpen(card) })
                }
                // Keep the last line's tiles their own size instead of stretching.
                repeat(columns - line.size) {
                    Box(Modifier.width(tileSize))
                }
            }
        }
    }
}

/** The « Artistes similaires » shelf: portraits, scrolled sideways. */
@Composable
internal fun ArtistShelf(
    cards: List<uniffi.nz_ffi.ArtistTileData>,
    onOpen: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        cards.forEach { card ->
            Column(
                modifier = Modifier
                    .width(96.dp)
                    .clip(RoundedCornerShape(NzRadius.card))
                    .clickable(role = Role.Button) { onOpen(card.id) }
                    .padding(NzSpacing.xs)
                    .semantics { contentDescription = card.name },
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
            ) {
                Avatar(name = card.name, sizeDp = 88.dp, url = card.picture)
                Text(
                    text = card.name,
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

/**
 * A band's actions, which WRAP (`Artist.svelte#.actions` / `Album.svelte#.actions`
 * at `max-width: 640px`).
 *
 * Three things were measured before this became a `FlowRow`:
 *
 * - a plain `Row` cut the album's heart off the right edge at 360 dp (four
 *   actions, 320 dp of content);
 * - a sideways-scrolling row kept the heart reachable but hidden behind a scroll
 *   nobody sees — a control that exists and cannot be found is not there;
 * - a `Row` that wrapped pushed « Télécharger » into two lines (« Téléchar /
 *   ger »), because a child of a wrapping row still gets the row's width.
 *
 * A `FlowRow` lays the children on as many lines as they need, each at its own
 * width, so every label stays whole and every control visible.
 */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun ActionRow(
    modifier: Modifier = Modifier,
    content: @Composable androidx.compose.foundation.layout.FlowRowScope.() -> Unit,
) {
    androidx.compose.foundation.layout.FlowRow(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        itemVerticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

/** A plaque that only exists to be read by TalkBack or a test: the screen's marker. */
@Composable
internal fun Marker(description: String) {
    Box(
        Modifier
            .size(0.dp)
            .clearAndSetSemantics { contentDescription = description },
    )
}

/**
 * One title of an entity page's list — the SPA's shared row, tuned for the pages
 * this lot owns.
 *
 * Why not MB1's `TrackRowView` everywhere: the SPA's album page draws its tracks
 * NUMBERED and WITHOUT the sleeve (the album's own cover is already the band's;
 * twelve copies of it is noise), and `TrackRowView` has neither switch. This row
 * also GROWS with the font scale (`heightIn(min = 56.dp)` rather than a fixed
 * height) — at 200 % the design system's fixed 56 dp clipped the artist's sub-line
 * in half, which this lot's first 200 % screenshot shows.
 *
 * The gesture is the design system's [`SwipeRow`] (lot G1), the app's ONE
 * implementation: swipe right = « Lire ensuite », the row follows the finger,
 * a release past 60 dp **or** 600 dp/s asks [onSwipeQueue], and the row returns
 * on a spring seeded with the release velocity. A leftward drag is left to the
 * tab pager.
 */
@Composable
internal fun EntityTrackRow(
    row: TrackRow,
    onClick: () -> Unit,
    onMore: () -> Unit,
    onSwipeQueue: () -> Unit,
    modifier: Modifier = Modifier,
    /** The track's position on its release, when the page numbers its rows. */
    number: Int? = null,
    showCover: Boolean = true,
    playing: Boolean = false,
) {
    val colors = Sillon.colors
    SwipeRow(onQueue = onSwipeQueue, modifier = modifier) { rowModifier ->
        Row(
            modifier = rowModifier
                .heightIn(min = 56.dp)
                .clickable(role = Role.Button, onClick = onClick)
                .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            if (number != null) {
                Box(Modifier.width(28.dp), contentAlignment = Alignment.Center) {
                    if (playing) {
                        Icon(
                            glyph = NzGlyph.Play,
                            size = 14.dp,
                            tint = colors.acText,
                            contentDescription = "En lecture",
                        )
                    } else {
                        Text(
                            text = number.toString(),
                            style = Sillon.type.mono,
                            color = colors.dim,
                        )
                    }
                }
            }
            if (showCover) {
                Box(contentAlignment = Alignment.Center) {
                    CoverImage(url = row.cover, sizeDp = 44.dp, seed = row.id)
                    if (playing) {
                        Box(
                            modifier = Modifier
                                .size(24.dp)
                                .clip(CircleShape)
                                .background(colors.bg.copy(alpha = 0.72f)),
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(
                                glyph = NzGlyph.Play,
                                size = 12.dp,
                                tint = colors.acText,
                                contentDescription = null,
                            )
                        }
                    }
                }
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                // One line each, ELLIPSISED rather than word-broken: at 200 % the
                // SPA's own rows truncate (« Ouvert… »), and Compose's default
                // breaks « Ouver / ture » when a word is wider than the cell,
                // which read as a clipped row (this lot's first 200 % shot).
                Text(
                    text = row.title,
                    style = Sillon.type.body,
                    color = if (playing) colors.acText else colors.tx,
                    maxLines = 1,
                    softWrap = false,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = row.artists,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    softWrap = false,
                    overflow = TextOverflow.Ellipsis,
                )
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
}
