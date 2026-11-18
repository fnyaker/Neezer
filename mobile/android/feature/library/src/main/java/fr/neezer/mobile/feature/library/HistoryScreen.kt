package fr.neezer.mobile.feature.library

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.Avatar
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import uniffi.nz_ffi.HistoryEventData
import uniffi.nz_ffi.HistorySummaryData
import uniffi.nz_ffi.TrackRow
import java.time.LocalDate
import java.time.ZoneId

/**
 * The font scale from which a row lays out VERTICALLY instead of squeezing: at
 * 200 % the three summary figures broke their labels mid-word (« écout/es ») and
 * an event's title and time were cut to « Ouv… » / « 21:0… » — measured on this
 * screen's own renders, so the scale is a layout decision, not a smaller box.
 */
private const val LARGE_TEXT_SCALE = 1.3f

/** What the history screen may ask. */
class HistoryActions(
    val setFilter: (HistoryFilter) -> Unit = {},
    val loadMore: () -> Unit = {},
    val retry: () -> Unit = {},
    val play: (HistoryEventData) -> Unit = {},
    val playTrack: (TrackRow) -> Unit = {},
    val more: (TrackRow) -> Unit = {},
    val openArtist: (String) -> Unit = {},
    val openRecap: () -> Unit = {},
    val openSearch: () -> Unit = {},
)

/**
 * Historique — lot MB8, ported from `routes/History.svelte`.
 *
 * - the last 30 days' card first (listens, skips, minutes, the two tops), which
 *   is hidden when there is nothing at all to tell;
 * - the three filters, then the days: each heading is « Aujourd'hui »,
 *   « Hier » or the French date, with the day's own counts, and each event states
 *   its verdict (« Écouté », « Passé », « Partiel ») and how long it played;
 * - « Charger plus » appends the next page (a keyset cursor), « Début de ton
 *   historique » ends it, and a later page's failure says so and offers its own
 *   retry;
 * - everything scrolls in ONE `LazyColumn` (day headings are 44 dp, events 72),
 *   so a year of listening costs only the rows on screen.
 *
 * [today] and [zone] are parameters so a render and a test can pin « Aujourd'hui »
 * instead of reading the clock.
 */
@Composable
fun HistoryScreen(
    state: HistoryModel.State,
    actions: HistoryActions,
    modifier: Modifier = Modifier,
    today: LocalDate = LocalDate.now(),
    zone: ZoneId = ZoneId.systemDefault(),
) {
    val colors = Sillon.colors
    val listState = rememberLazyListState()
    val largeText = LocalDensity.current.fontScale >= LARGE_TEXT_SCALE
    val days = remember(state.events, today, zone) { groupByDay(state.events, zone, today) }
    val rows = remember(days) { flattenDays(days) }
    val failure = state.failure

    Box(
        modifier
            .fillMaxSize()
            .background(colors.bg)
            .semantics { contentDescription = HISTORY_MARKER },
    ) {
        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(bottom = SHELL_CLEARANCE),
        ) {
            item(key = "head") {
                Column(
                    Modifier
                        .fillMaxWidth()
                        .padding(start = NzSpacing.pageX, end = NzSpacing.pageX, top = NzSpacing.l),
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                ) {
                    Text(
                        text = "Historique",
                        style = Sillon.type.displaySmall,
                        color = colors.tx,
                    )
                    if (state.stale) LibraryStaleNote()
                }
            }
            item(key = "summary") {
                val summary = state.summary
                when {
                    summary != null && !state.emptyHistory -> SummaryCard(summary, actions)
                    failure == null && !state.emptyHistory && state.screen is ScreenState.Loading ->
                        Skeleton(
                            kind = NzSkeletonKind.Header,
                            modifier = Modifier.padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
                        )
                    else -> Unit
                }
            }
            item(key = "filters") {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState())
                        .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                ) {
                    HistoryFilter.entries.forEach { filter ->
                        NzChip(
                            text = filter.label,
                            selected = state.filter == filter,
                            onClick = { actions.setFilter(filter) },
                        )
                    }
                }
            }
            if (failure != null) {
                item(key = "fail") {
                    LoadFailed(
                        cause = failure.cause,
                        onRetry = actions.retry,
                        title = "Historique indisponible",
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
                    )
                }
            } else if (state.screen is ScreenState.Loading && state.summary == null) {
                item(key = "skeleton") {
                    Skeleton(
                        kind = NzSkeletonKind.List,
                        count = 8,
                        modifier = Modifier.padding(horizontal = NzSpacing.pageX),
                    )
                }
            } else if (rows.isEmpty()) {
                item(key = "empty") {
                    EmptyState(
                        title = when (state.filter) {
                            HistoryFilter.Listen -> "Aucune écoute dans l'historique"
                            HistoryFilter.Skip -> "Aucun saut dans l'historique"
                            HistoryFilter.All -> "Aucune écoute pour l'instant"
                        },
                        body = if (state.filter == HistoryFilter.All) {
                            "Lance un titre : ce que tu écoutes et ce que tu passes apparaîtra ici, jour par jour."
                        } else {
                            "Change de filtre pour revoir tout ton historique."
                        },
                        glyph = NzGlyph.Timer,
                        actionLabel = if (state.filter == HistoryFilter.All) "Explorer" else "Voir tout l'historique",
                        onAction = if (state.filter == HistoryFilter.All) {
                            actions.openSearch
                        } else {
                            { actions.setFilter(HistoryFilter.All) }
                        },
                    )
                }
            } else {
                items(rows.size, key = { index -> rows[index].key }) { index ->
                    when (val row = rows[index]) {
                        is HistoryRowView.Day -> DayHeading(row.day)
                        is HistoryRowView.Event -> EventRow(
                            event = row.event,
                            today = today,
                            zone = zone,
                            onPlay = { actions.play(row.event) },
                            onMore = { actions.more(row.event.track) },
                        )
                    }
                }
                item(key = "more") {
                    MoreBlock(state, actions)
                }
            }
        }
    }
}

/** The 30-day card: three figures, two tops, and the door to the recap. */
@Composable
private fun SummaryCard(summary: HistorySummaryData, actions: HistoryActions) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.pageX)
            .clip(RoundedCornerShape(NzRadius.card))
            .background(colors.s1)
            .padding(NzSpacing.m),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = "${summary.days} derniers jours",
                style = Sillon.type.titleSmall,
                color = colors.tx,
                modifier = Modifier.weight(1f),
            )
            NzButton(
                text = "Mon récap",
                onClick = actions.openRecap,
                kind = NzButtonKind.Ghost,
            )
        }
        val largeText = LocalDensity.current.fontScale >= LARGE_TEXT_SCALE
        if (largeText) {
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                Stat("${summary.listens}", if (summary.listens > 1u) "écoutes" else "écoute", Modifier.fillMaxWidth())
                Stat("${summary.skips}", if (summary.skips > 1u) "sauts" else "saut", Modifier.fillMaxWidth())
                Stat("${summary.minutes}", if (summary.minutes > 1u) "minutes" else "minute", Modifier.fillMaxWidth())
            }
        } else {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                Stat("${summary.listens}", if (summary.listens > 1u) "écoutes" else "écoute", Modifier.weight(1f))
                Stat("${summary.skips}", if (summary.skips > 1u) "sauts" else "saut", Modifier.weight(1f))
                Stat("${summary.minutes}", if (summary.minutes > 1u) "minutes" else "minute", Modifier.weight(1f))
            }
        }
        if (summary.topArtists.isNotEmpty()) {
            Text(text = "Artistes les plus écoutés", style = Sillon.type.mono, color = colors.dim)
            Row(
                modifier = Modifier.horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                summary.topArtists.forEach { artist ->
                    Column(
                        modifier = Modifier
                            .clip(RoundedCornerShape(NzRadius.card))
                            .semantics {
                                contentDescription =
                                    "Ouvrir ${artist.name}, ${plural(artist.count.toInt(), "écoute", "écoutes")}"
                            }
                            .padding(NzSpacing.xxs),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
                    ) {
                        Avatar(name = artist.name, sizeDp = 48.dp, url = artist.cover)
                        Text(
                            text = artist.name,
                            style = Sillon.type.bodySmall,
                            color = colors.tx,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            text = plural(artist.count.toInt(), "écoute", "écoutes"),
                            style = Sillon.type.mono,
                            color = colors.dim,
                        )
                    }
                }
            }
        }
        if (summary.topTracks.isNotEmpty()) {
            Text(text = "Titres les plus écoutés", style = Sillon.type.mono, color = colors.dim)
            summary.topTracks.forEach { item ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = 48.dp)
                        .clickable(role = Role.Button) { actions.playTrack(item.track) }
                        .semantics {
                            contentDescription = "Lire ${item.track.title}"
                        },
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                ) {
                    CoverImage(url = item.track.cover, sizeDp = 36.dp, seed = item.track.id)
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
                        Text(
                            text = item.track.title,
                            style = Sillon.type.body,
                            color = colors.tx,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            text = item.track.artists,
                            style = Sillon.type.bodySmall,
                            color = colors.dim,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    Text(
                        text = plural(item.count.toInt(), "écoute", "écoutes"),
                        style = Sillon.type.mono,
                        color = colors.dim,
                    )
                }
            }
        }
    }
}

/** One figure of the summary card. */
@Composable
private fun Stat(value: String, label: String, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.card))
            .background(Sillon.colors.s2)
            .padding(NzSpacing.s),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(text = value, style = Sillon.type.title, color = Sillon.colors.tx)
        Text(text = label, style = Sillon.type.labelSmall, color = Sillon.colors.dim)
    }
}

/** A day's heading: its name and its own counts. */
@Composable
private fun DayHeading(day: HistoryDay) {
    val colors = Sillon.colors
    val largeText = LocalDensity.current.fontScale >= LARGE_TEXT_SCALE
    val modifier = Modifier
        .fillMaxWidth()
        .heightIn(min = 44.dp)
        .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xxs)
    if (largeText) {
        Column(modifier, verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = day.label,
                style = Sillon.type.titleSmall,
                color = colors.tx,
            )
            if (day.meta.isNotEmpty()) {
                Text(text = day.meta, style = Sillon.type.mono, color = colors.dim)
            }
        }
        return
    }
    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(
            text = day.label,
            style = Sillon.type.titleSmall,
            color = colors.tx,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        if (day.meta.isNotEmpty()) {
            Text(text = day.meta, style = Sillon.type.mono, color = colors.dim)
        }
    }
}

/** One event: its sleeve, when it played, the title and its verdict. */
@Composable
private fun EventRow(
    event: HistoryEventData,
    today: LocalDate,
    zone: ZoneId,
    onPlay: () -> Unit,
    onMore: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 72.dp)
            .clickable(role = Role.Button, onClick = onPlay)
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xxs)
            .semantics { contentDescription = "Lire ${event.track.title}" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        CoverImage(url = event.track.cover, sizeDp = 56.dp, seed = event.track.id)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = event.track.title,
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = event.track.artists,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = "${whenOf(event, zone)} · ${listenedLine(event)}",
                style = Sillon.type.mono,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (LocalDensity.current.fontScale >= LARGE_TEXT_SCALE) {
                // At 200 % a pill on the right leaves the title « Ouv… »: the
                // verdict sits under the time, and the words stay whole.
                Verdict(verdictOf(event))
            }
        }
        if (LocalDensity.current.fontScale < LARGE_TEXT_SCALE) {
            Verdict(verdictOf(event))
        }
        NzIconButton(
            glyph = NzGlyph.More,
            contentDescription = "Plus d'actions pour ${event.track.title}",
            onClick = onMore,
        )
    }
}

/** The verdict pill: a glyph AND its word, never colour alone. */
@Composable
private fun Verdict(verdict: HistoryVerdict) {
    val colors = Sillon.colors
    val glyph = when (verdict) {
        HistoryVerdict.Listen -> NzGlyph.Check
        HistoryVerdict.Skip -> NzGlyph.Next
        HistoryVerdict.Partial -> NzGlyph.Minus
    }
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(NzRadius.pill))
            .background(colors.s2)
            .padding(horizontal = NzSpacing.s, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Icon(glyph = glyph, size = 13.dp, tint = colors.dim)
        Text(text = verdict.label, style = Sillon.type.labelSmall, color = colors.dim)
    }
}

/** « Charger plus », the end of the log, and a later page's own failure. */
@Composable
private fun MoreBlock(state: HistoryModel.State, actions: HistoryActions) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        if (state.next != null) {
            NzButton(
                text = if (state.loading) "Chargement…" else "Charger plus",
                onClick = actions.loadMore,
                glyph = if (state.loading) NzGlyph.Refresh else NzGlyph.Plus,
                kind = NzButtonKind.Ghost,
                enabled = !state.loading,
            )
        } else {
            Text(
                text = "Début de ton historique.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
        state.moreError?.let { cause ->
            Text(
                text = "Impossible de charger la suite : $cause.",
                style = Sillon.type.bodySmall,
                color = colors.acText,
            )
            NzButton(
                text = "Réessayer",
                onClick = actions.retry,
                glyph = NzGlyph.Refresh,
                kind = NzButtonKind.Ghost,
            )
        }
    }
}
