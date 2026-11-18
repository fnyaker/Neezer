package fr.neezer.mobile.feature.library

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.HistoryCursorData
import uniffi.nz_ffi.HistoryEventData
import uniffi.nz_ffi.HistoryPageData
import uniffi.nz_ffi.HistorySummaryData
import uniffi.nz_ffi.TrackRow
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

/** The three chips of `History.svelte#FILTERS`. */
enum class HistoryFilter(val id: String, val label: String) {
    All("", "Tout"),
    Listen("listen", "Écoutes"),
    Skip("skip", "Sauts"),
}

/** One day of the history, as the screen's sticky heading states it. */
data class HistoryDay(
    /** `yyyy-mm-dd`, the grouping key. */
    val key: String,
    /** « Aujourd'hui », « Hier », « Lundi 17 novembre ». */
    val label: String,
    val listens: Int,
    val skips: Int,
    val seconds: Int,
    val events: List<HistoryEventData>,
) {
    /** « 3 écoutes · 1 saut · 12 min » — the heading's own sentence. */
    val meta: String
        get() {
            val parts = mutableListOf<String>()
            if (listens > 0) parts += plural(listens, "écoute", "écoutes")
            if (skips > 0) parts += plural(skips, "saut", "sauts")
            if (seconds > 0) parts += "${seconds / 60} min"
            return parts.joinToString(" · ")
        }
}

/** One row of the flattened history: a day's heading, then its events. */
sealed interface HistoryRowView {
    val key: String

    data class Day(val day: HistoryDay) : HistoryRowView {
        override val key: String get() = "d:${day.key}"
    }

    data class Event(val event: HistoryEventData) : HistoryRowView {
        override val key: String get() = "e:${event.id}"
    }
}

/**
 * The history's state machine (lot MB8), ported from `routes/History.svelte`.
 *
 * - **A keyset cursor, never a page number** (`#loadFirst`/`#loadMore`): the
 *   server answers `next`, both halves are sent back together, and an event
 *   recorded while somebody reads page one can never push one off page two. The
 *   last page is kept in memory and « Charger plus » APPENDS to it.
 * - **The copy paints, the request replaces it** (`screenFlow`): the first page
 *   of a filter is kept on the device; a dead link keeps it with « Hors ligne ·
 *   version mémorisée ».
 * - **A failure says WHY** — the first page with `LoadFailed` + « Réessayer »,
 *   a later page with « Impossible de charger la suite » and its own retry
 *   (`moreError`).
 * - **The days are the DEVICE's**: the core hands the server's local ISO time and
 *   this model groups it (`dayLabel`), so « Aujourd'hui » is right in every
 *   timezone, and the grouping is a pure function a JVM test can pin.
 */
class HistoryModel(
    private val service: HistoryService,
    private val player: LibraryPlayer,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
    private val zone: ZoneId = ZoneId.systemDefault(),
    private val today: () -> LocalDate = { LocalDate.now(zone) },
) {
    data class State(
        /** The first page's state: `Loading`, the answer (with its cache flag), or
         * the cause and « Réessayer ». */
        val screen: ScreenState<HistoryPageData> = ScreenState.Loading,
        /** Every event read so far: the first page, then what « Charger plus » added. */
        val events: List<HistoryEventData> = emptyList(),
        val summary: HistorySummaryData? = null,
        val next: HistoryCursorData? = null,
        val filter: HistoryFilter = HistoryFilter.All,
        val loading: Boolean = false,
        /** A later page failed: « Impossible de charger la suite : … ». */
        val moreError: String? = null,
    ) {
        /** What was drawn came from the device and the refresh failed. */
        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true

        /** The failure of the first page, when there is one. */
        val failure: ScreenState.Failed? get() = screen as? ScreenState.Failed

        /**
         * `History.svelte#emptyHistory`: a history with nothing at all needs the
         * empty state, not a 30-day card full of zeros.
         */
        val emptyHistory: Boolean
            get() {
                val summary = summary ?: return false
                return events.isEmpty() &&
                    summary.listens.toInt() == 0 &&
                    summary.skips.toInt() == 0 &&
                    summary.minutes.toInt() == 0 &&
                    summary.topArtists.isEmpty() &&
                    summary.topTracks.isEmpty()
            }
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var firstJob: Job? = null
    private var moreJob: Job? = null

    /** Reads the first page of [filter] — the copy first, then the answer. */
    fun load(filter: HistoryFilter = _state.value.filter) {
        if (firstJob?.isActive == true) return
        if (_state.value.filter == filter && (_state.value.screen as? ScreenState.Ready)?.data != null) {
            return
        }
        _state.update {
            it.copy(filter = filter, screen = ScreenState.Loading, events = emptyList(), next = null, moreError = null)
        }
        firstJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cached(filter.id) } },
                fresh = { withContext(io) { service.page(filter.id.ifEmpty { null }, null, null, PAGE) } },
            ).collect { screen ->
                _state.update { state ->
                    if (screen is ScreenState.Ready) {
                        state.copy(
                            screen = screen,
                            events = screen.data.events,
                            summary = screen.data.summary,
                            next = screen.data.next,
                        )
                    } else {
                        state.copy(screen = screen)
                    }
                }
            }
        }
    }

    /** A chip: another filter starts again from the top (`setFilter`). */
    fun setFilter(filter: HistoryFilter) {
        if (filter == _state.value.filter) return
        firstJob?.cancel()
        moreJob?.cancel()
        load(filter)
    }

    /** « Charger plus »: the next page, appended (`loadMore`). */
    fun loadMore() {
        val cursor = _state.value.next ?: return
        if (_state.value.loading || moreJob?.isActive == true) return
        val filter = _state.value.filter
        _state.update { it.copy(loading = true, moreError = null) }
        moreJob = scope.launch {
            try {
                val page = withContext(io) {
                    service.page(
                        filter.id.ifEmpty { null },
                        cursor.before,
                        cursor.beforeId,
                        PAGE,
                    )
                }
                _state.update { state ->
                    // A cursor is exact, but the id check makes a duplicate
                    // impossible to show even if a page overlapped.
                    val known = state.events.map { it.id }.toSet()
                    val fresh = page.events.filter { it.id !in known }
                    state.copy(
                        events = state.events + fresh,
                        next = page.next,
                        loading = false,
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(loading = false, moreError = NzErrors.text(e)) }
            }
        }
    }

    /** « Réessayer »: the first page when there is none, else the next one. */
    fun retry() {
        val failed = _state.value.failure
        if (failed != null) {
            failed.retry()
            return
        }
        loadMore()
    }

    /** An event tapped: the title alone, then its radio (`playThenRadio`). */
    fun play(event: HistoryEventData) = playTrack(event.track)

    /** « Titres les plus écoutés »: the title alone, then its radio. */
    fun playTrack(row: TrackRow) {
        player.play(listOf(row), 0, CONTEXT_RADIO, row.id)
    }

    companion object {
        /** `History.svelte#PAGE`: fifty events a page. */
        const val PAGE: UInt = 50u

        const val CONTEXT_RADIO = "radio"
    }
}

/** The day a row belongs to, in the DEVICE's zone (`groupByDay`). */
fun dayOf(event: HistoryEventData, zone: ZoneId): LocalDate? = parseAt(event.at, zone)?.toLocalDate()

/**
 * `History.svelte#parseAt`: the server's naive local ISO time; a fractional part,
 * if a row ever has one, is display-only noise.
 */
fun parseAt(value: String?, zone: ZoneId = ZoneId.systemDefault()): LocalDateTime? {
    if (value.isNullOrBlank()) return null
    val text = value.replace(Regex("\\.\\d+"), "").replace(' ', 'T')
    return runCatching { LocalDateTime.parse(text) }.getOrElse {
        // A row with a zone suffix: read it at that offset, then bring it home.
        runCatching {
            Instant.parse(text).atZone(zone).toLocalDateTime()
        }.getOrNull()
    }
}

/**
 * `History.svelte#dayLabel`: « Aujourd'hui », « Hier », or « Lundi 17 novembre »
 * (with the year when it is not this one). Capitalised in French.
 */
fun dayLabel(date: LocalDate, today: LocalDate): String {
    if (date == today) return "Aujourd'hui"
    if (date == today.minusDays(1)) return "Hier"
    val pattern = if (date.year != today.year) "EEEE d MMMM yyyy" else "EEEE d MMMM"
    val text = date.format(DateTimeFormatter.ofPattern(pattern, Locale.FRENCH))
    return text.replaceFirstChar { it.titlecase(Locale.FRENCH) }
}

/**
 * `History.svelte#groupByDay`: consecutive events of the same day, with that
 * day's listens, skips and seconds. Rows are kept in the server's order.
 */
fun groupByDay(
    events: List<HistoryEventData>,
    zone: ZoneId,
    today: LocalDate,
): List<HistoryDay> {
    val days = LinkedHashMap<LocalDate, MutableList<HistoryEventData>>()
    for (event in events) {
        val date = dayOf(event, zone) ?: continue
        days.getOrPut(date) { mutableListOf() } += event
    }
    return days.map { (date, rows) ->
        var listens = 0
        var skips = 0
        var seconds = 0
        for (row in rows) {
            if (row.outcome > 0) listens++ else if (row.outcome < 0) skips++
            seconds += row.listened.toInt().coerceAtLeast(0)
        }
        HistoryDay(
            key = date.toString(),
            label = dayLabel(date, today),
            listens = listens,
            skips = skips,
            seconds = seconds,
            events = rows,
        )
    }
}

/** `History.svelte#flatten`: the day headings and their events, in one list. */
fun flattenDays(days: List<HistoryDay>): List<HistoryRowView> {
    val rows = ArrayList<HistoryRowView>()
    for (day in days) {
        rows += HistoryRowView.Day(day)
        day.events.forEach { rows += HistoryRowView.Event(it) }
    }
    return rows
}

/** `History.svelte#when`: « 21:04 ». */
fun whenOf(event: HistoryEventData, zone: ZoneId = ZoneId.systemDefault()): String {
    val at = parseAt(event.at, zone) ?: return ""
    return at.format(DateTimeFormatter.ofPattern("HH:mm"))
}

/** `History.svelte#verdict`: which of the three outcomes a row states. */
enum class HistoryVerdict(val label: String) {
    Listen("Écouté"),
    Skip("Passé"),
    Partial("Partiel"),
}

fun verdictOf(event: HistoryEventData): HistoryVerdict = when {
    event.outcome > 0 -> HistoryVerdict.Listen
    event.outcome < 0 -> HistoryVerdict.Skip
    else -> HistoryVerdict.Partial
}

/** « 3:07 écoutés », or « Pas écouté » — the row's own mono line. */
fun listenedLine(event: HistoryEventData): String {
    val seconds = event.listened.toInt()
    if (seconds <= 0) return "Pas écouté"
    return "${seconds / 60}:${(seconds % 60).toString().padStart(2, '0')} écoutés"
}

/** A history row's track, for a tap that plays then radios. */
fun HistoryEventData.trackRow(): TrackRow = track
