package fr.neezer.mobile.feature.library

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.ListWindow
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzItemSpecs
import fr.neezer.mobile.designsystem.NzPaneEnter
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.SwipeRow
import fr.neezer.mobile.designsystem.asRowSource
import fr.neezer.mobile.designsystem.nzPaneEnter
import uniffi.nz_ffi.LibraryArtist
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.UploadUsageData

/**
 * What the library screen is called in the accessibility tree.
 *
 * A MARKER, not the visible title: `mobile/tools/ci-emulator.sh` proves the shell's
 * doors by a text only that screen draws, and the bottom bar already says
 * « Bibliothèque » on every tab — MB5 met the same collision on the home and
 * named its marker « Accueil Neezer ». Present in every state (the head, the
 * skeletons, a failure with no server), so the smoke can never mistake a
 * half-painted screen for an opened one.
 */
const val LIBRARY_MARKER = "Bibliothèque Neezer"

/** The same marker for the history route and the library's history leaf. */
const val HISTORY_MARKER = "Historique Neezer"

/** How many rows past the viewport are reported to the window (MB1's margin). */
internal const val WINDOW_OVERSCAN = 24

/** The shell's mini player sits over the last rows: leave room for it. */
internal val SHELL_CLEARANCE = 128.dp

/**
 * The window a list is drawn through.
 *
 * A real handle goes through the designsystem's own adapter and window
 * (`asRowSource` + `ListWindow`, the pair MB6 fixed in #321); a fake (a test, a
 * screenshot render) brings its own source. Nothing here re-implements either.
 */
@Composable
internal fun windowFor(rows: LibraryRows): ListWindow {
    val handle = rows.handle
    val source = if (handle != null) handle.asRowSource() else remember(rows) { FakeSource(rows) }
    return remember(source) { ListWindow(source) }
}

/** The pure source a fake list exposes: `len` and a page, nothing else. */
private class FakeSource(private val rows: LibraryRows) : fr.neezer.mobile.designsystem.NzRowSource {
    override fun len(): Int = rows.len()

    override fun window(start: Int, count: Int): List<TrackRow> = rows.window(start, count)
}

/**
 * Tells a [ListWindow] which rows the viewport shows — on every CHANGE of the lazy
 * state, never on a clock (`snapshotFlow`).
 *
 * [offset] is how many lazily-composed items sit above the window (the head, the
 * leaf strip, the tools): a screen with three such items reports
 * `firstVisibleItemIndex - 3` as its first row.
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
internal fun LibraryStaleNote(modifier: Modifier = Modifier) {
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

/**
 * The one search field of this area: a pill, the glyph, the placeholder, and the
 * keyboard's « chercher » action. `BasicTextField` and not a Material field —
 * Sillon's shapes and colours are the design, and a default Material outline is
 * exactly what rule 5 forbids.
 */
@Composable
internal fun NzSearchField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    label: String,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Row(
        modifier = modifier
            .heightIn(min = 48.dp)
            .clip(RoundedCornerShape(NzRadius.pill))
            .background(colors.s1)
            .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.pill))
            .padding(horizontal = NzSpacing.m),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = NzGlyph.Search, size = 16.dp, tint = colors.dim)
        Box(Modifier.weight(1f)) {
            if (value.isEmpty()) {
                Text(
                    text = placeholder,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            BasicTextField(
                value = value,
                onValueChange = onValueChange,
                singleLine = true,
                textStyle = Sillon.type.body.copy(color = colors.tx),
                cursorBrush = SolidColor(colors.acText),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                modifier = Modifier
                    .fillMaxWidth()
                    .semantics { contentDescription = label },
            )
        }
        if (value.isNotEmpty()) {
            NzIconButton(
                glyph = NzGlyph.Close,
                contentDescription = "Effacer la recherche",
                onClick = { onValueChange("") },
            )
        }
    }
}

/**
 * The upload quota (`Library.svelte`'s quota bar): a track, a fill and the two
 * numbers in bytes. Only shown when the person has a cap.
 */
@Composable
internal fun QuotaBar(usage: UploadUsageData, modifier: Modifier = Modifier) {
    if (usage.unlimited || usage.quota == 0uL) return
    val colors = Sillon.colors
    val fraction = (usage.percent / 100.0).coerceIn(0.0, 1.0).toFloat()
    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(6.dp)
                .clip(RoundedCornerShape(3.dp))
                .background(colors.s2),
        ) {
            Box(
                Modifier
                    .fillMaxWidth(fraction)
                    .height(6.dp)
                    .clip(RoundedCornerShape(3.dp))
                    .background(if (usage.percent > 90) colors.tx else colors.ac),
            )
        }
        Text(
            text = "${bytes(usage.used)} / ${bytes(usage.quota)} d'espace d'upload utilisé",
            style = Sillon.type.mono,
            color = colors.dim,
        )
    }
}

/** `lib/format.js#bytes`: « 1,4 Go ». */
internal fun bytes(count: ULong): String {
    val units = listOf("o", "Ko", "Mo", "Go", "To")
    var value = count.toDouble()
    var unit = 0
    while (value >= 1024 && unit < units.lastIndex) {
        value /= 1024
        unit++
    }
    val text = if (value >= 10 || unit == 0) {
        "%.0f".format(value)
    } else {
        "%.1f".format(value)
    }
    return "$text ${units[unit]}".replace('.', ',')
}

/** A playlist card: its sleeve, its title, « Nadia · 12 titres », and its ⋯. */
@Composable
internal fun PlaylistCardTile(
    card: PlaylistCardView,
    onClick: () -> Unit,
    onMore: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.card))
            .clickable(role = Role.Button, onClick = onClick)
            .padding(NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        Box {
            CoverImage(url = card.cover, sizeDp = 148.dp, seed = card.id)
            if (onMore != null) {
                Box(modifier = Modifier.align(Alignment.TopEnd)) {
                    NzIconButton(
                        glyph = NzGlyph.More,
                        contentDescription = "Actions pour la playlist ${card.title}",
                        onClick = onMore,
                    )
                }
            }
        }
        Text(
            text = card.title,
            style = Sillon.type.body,
            color = colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = card.subtitle,
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** An artist's tile: the initialled disc or the portrait, the name, its count. */
@Composable
internal fun ArtistTile(
    artist: LibraryArtist,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.card))
            .clickable(role = Role.Button, onClick = onClick)
            .padding(NzSpacing.xs),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        fr.neezer.mobile.designsystem.Avatar(
            name = artist.name,
            sizeDp = 96.dp,
            url = artist.picture,
        )
        Text(
            text = artist.name,
            style = Sillon.type.body,
            color = colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = if (artist.tracks == 0u) "Aucun titre gardé" else plural(artist.tracks.toInt(), "titre", "titres"),
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** An album's card: the sleeve, the title, « Ascenseur · 2024 ». */
@Composable
internal fun AlbumTile(
    card: uniffi.nz_ffi.LibraryAlbum,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.card))
            .clickable(role = Role.Button, onClick = onClick)
            .padding(NzSpacing.xs),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
    ) {
        CoverImage(url = card.cover, sizeDp = 148.dp, seed = card.id)
        Text(
            text = card.title,
            style = Sillon.type.body,
            color = colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = listOfNotNull(card.artist.takeIf { it.isNotEmpty() }, card.year.takeIf { it.isNotEmpty() })
                .joinToString(" · "),
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** A section's heading: the chevron, the title, its count, and the folder's ⋯. */
@Composable
internal fun SectionHeading(
    section: SectionView,
    onToggle: () -> Unit,
    onMore: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Row(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .clickable(role = Role.Button, onClick = onToggle)
            .semantics {
                stateDescription = if (section.folded) "Replié" else "Déplié"
            }
            .padding(horizontal = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(
            glyph = if (section.folded) NzGlyph.ChevronRight else NzGlyph.ChevronDown,
            size = 18.dp,
            tint = colors.dim,
        )
        Text(
            text = section.title,
            style = Sillon.type.titleSmall,
            color = colors.tx,
            modifier = Modifier.weight(1f),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            text = plural(section.count, "playlist", "playlists"),
            style = Sillon.type.mono,
            color = colors.dim,
        )
        if (onMore != null) {
            NzIconButton(
                glyph = NzGlyph.More,
                contentDescription = "Actions pour le dossier ${section.title}",
                onClick = onMore,
            )
        }
    }
}

/** One row of the organiser: its name, its count, and the three moves. */
@Composable
internal fun OrganizeRowView(
    row: OrganizeRowView,
    first: Boolean,
    last: Boolean,
    busy: Boolean,
    onUp: () -> Unit,
    onDown: () -> Unit,
    onMove: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    Row(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .clip(RoundedCornerShape(NzRadius.chip))
            .background(colors.s1)
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xxs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
            Text(
                text = row.title,
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(text = row.detail, style = Sillon.type.mono, color = colors.dim)
        }
        NzIconButton(
            glyph = NzGlyph.ChevronUp,
            contentDescription = "Monter ${row.title}",
            onClick = onUp,
            enabled = !first && !busy,
        )
        NzIconButton(
            glyph = NzGlyph.ChevronDown,
            contentDescription = "Descendre ${row.title}",
            onClick = onDown,
            enabled = !last && !busy,
        )
        if (onMove != null) {
            NzIconButton(
                glyph = NzGlyph.FolderMove,
                contentDescription = "Déplacer ${row.title} vers un dossier",
                onClick = onMove,
                enabled = !busy,
            )
        }
    }
}

/** The mono line a count or a state is stated in (« 4 000 titres · hors-ligne »). */
@Composable
internal fun MonoLine(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        style = Sillon.type.mono,
        color = Sillon.colors.dim,
        modifier = modifier,
    )
}

/** A small vertical gap between two blocks of a leaf. */
@Composable
internal fun Gap(height: Int = 12) {
    Spacer(Modifier.height(height.dp))
}

/**
 * The track rows of a windowed list, as `LazyColumn` items.
 *
 * **The key is the track's own identity** (`<source>:<id>`, `lists.rs`'s
 * documented contract), not its index: an indexed key makes every row below a
 * removal a NEW item, so `animateItem` has nothing to move (measured in X5:
 * `276 → 220 dp` at frame 0). These three lists hold each track ONCE
 * (`/api/me/favorites`, `/api/me/local`, the unavailable list — one row per
 * track), so the identity is unique; a page that has not landed yet draws a
 * skeleton under its own `pending-<index>` key.
 *
 * A removal plays `itemSpecs()`: no mount fade (M4), a 200 ms placement at Full,
 * a 140 ms fade-out. A skeleton's replacement fades nothing out (`fadeOutSpec =
 * null`): the skeleton's key leaves as the row's arrives, and its exit would be
 * drawn over the row.
 *
 * @param pane the leaf's shared entrance (`rememberPaneEnter` in the screen), or
 *   `null` for a row outside a pane.
 * @param specs `Sillon.motion.itemSpecs()`, read by the caller: this extension
 *   runs in the `LazyColumn`'s scope, where a `CompositionLocal` cannot be read.
 */
internal fun LazyListScope.trackRows(
    window: ListWindow,
    playingKey: String?,
    onPlay: (TrackRow) -> Unit,
    onMore: (TrackRow) -> Unit,
    onQueue: (TrackRow) -> Unit,
    pane: NzPaneEnter?,
    specs: NzItemSpecs,
) {
    items(window.len, key = { index -> window.rowAt(index)?.key ?: "pending-$index" }) { index ->
        val row = window.rowAt(index)
        val enter = if (pane == null) Modifier else Modifier.nzPaneEnter(pane)
        if (row == null) {
            SkeletonRow(
                modifier = enter.animateItem(
                    fadeInSpec = null,
                    placementSpec = specs.placement,
                    fadeOutSpec = null,
                ),
            )
        } else {
            LibraryTrackRow(
                row = row,
                playing = playingKey != null && row.key == playingKey,
                onClick = { onPlay(row) },
                onMore = { onMore(row) },
                onSwipeQueue = { onQueue(row) },
                modifier = enter.animateItem(specs.fadeIn, specs.placement, specs.fadeOut),
            )
        }
    }
}

/**
 * One row of a library list: a 44 dp sleeve, the title, its artists, the
 * duration in mono, and the ⋯ control. The whole row is the target.
 *
 * It is this module's own rather than MB1's `TrackRowView` for one measured
 * reason: that one is a fixed 56 dp box, and at a 200 % font scale the artist's
 * line was cut in half (MB7 met the same row and drew `EntityTrackRow` the same
 * way). `heightIn(min = 56.dp)` and two one-line texts never clip.
 *
 * **Swipe right = « Lire ensuite »** (`webapp.md`): the design system's
 * [`SwipeRow`] (lot G1) tracks the finger, commits at 60 dp **or** 600 dp/s, and
 * returns on a spring seeded with the release velocity; a leftward drag goes to
 * the tab pager. The queue itself is the caller's.
 */
@Composable
internal fun LibraryTrackRow(
    row: TrackRow,
    playing: Boolean,
    onClick: () -> Unit,
    onMore: () -> Unit,
    onSwipeQueue: () -> Unit,
    modifier: Modifier = Modifier,
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
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
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
                text = fr.neezer.mobile.designsystem.nzDuration(row.durationS.toInt()),
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

/** What a page that has not landed yet draws: a row-shaped skeleton. */
@Composable
private fun SkeletonRow(modifier: Modifier = Modifier) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .height(56.dp)
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.s),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Box(
            Modifier
                .size(44.dp)
                .clip(RoundedCornerShape(NzRadius.cover))
                .background(Sillon.colors.s2),
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Box(
                Modifier
                    .fillMaxWidth(0.6f)
                    .height(12.dp)
                    .clip(RoundedCornerShape(6.dp))
                    .background(Sillon.colors.s2),
            )
            Box(
                Modifier
                    .fillMaxWidth(0.35f)
                    .height(10.dp)
                    .clip(RoundedCornerShape(5.dp))
                    .background(Sillon.colors.s2),
            )
        }
    }
}

/** A vertical hairline divider, used between the chips of a toolbar. */
@Composable
internal fun Hairline() {
    Box(
        Modifier
            .width(1.dp)
            .height(20.dp)
            .background(Sillon.colors.ln),
    )
}
