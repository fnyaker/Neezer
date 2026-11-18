package fr.neezer.mobile.feature.library

import fr.neezer.mobile.bridge.ScreenState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.HistoryEventData
import uniffi.nz_ffi.HistoryPageData
import uniffi.nz_ffi.TrackRow
import java.time.LocalDate
import java.time.ZoneId

/**
 * The history's logic, on the JVM: the keyset cursor, the filters, the appended
 * pages, the two failures (the first page's and a later one's), and the day
 * grouping — which is a PURE function, so « Aujourd'hui » is pinned to a fixed
 * date instead of read from the clock.
 */
class HistoryModelTest {

    private class FakeHistory(
        var first: HistoryPageData = LibraryFixtures.page(),
        var cached: HistoryPageData? = LibraryFixtures.page(),
        var next: HistoryPageData? = LibraryFixtures.page(
            events = listOf(LibraryFixtures.event(100, "2026-11-16T09:12:00", 1, title = "Plus ancien")),
            next = null,
        ),
    ) : HistoryService {
        var firstCalls = 0
        var moreCalls = 0
        var cachedCalls = 0
        var lastBefore: String? = null
        var lastBeforeId: Long? = null
        var lastOutcome: String? = null
        var failFirst: Exception? = null
        var failMore: Exception? = null

        override suspend fun cached(outcome: String): HistoryPageData? {
            cachedCalls++
            return cached
        }

        override suspend fun page(
            outcome: String?,
            before: String?,
            beforeId: Long?,
            limit: UInt,
        ): HistoryPageData {
            lastOutcome = outcome
            if (before != null || beforeId != null) {
                moreCalls++
                lastBefore = before
                lastBeforeId = beforeId
                failMore?.let { throw it }
                return next ?: LibraryFixtures.emptyPage()
            }
            firstCalls++
            failFirst?.let { throw it }
            return first
        }
    }

    private class FakePlayer : LibraryPlayer {
        val plays = mutableListOf<Pair<List<String>, String>>()

        override fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String) {
            plays += tracks.map { it.key } to contextKind
        }

        override fun playShuffled(tracks: List<TrackRow>, contextKind: String, contextId: String) = Unit

        override fun playNext(row: TrackRow) = Unit
    }

    private val zone: ZoneId = ZoneId.of("Europe/Paris")
    private val today = LocalDate.of(2026, 11, 17)

    private fun harness(
        service: FakeHistory = FakeHistory(),
        player: FakePlayer = FakePlayer(),
    ): Triple<HistoryModel, FakeHistory, FakePlayer> {
        val model = HistoryModel(
            service = service,
            player = player,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = {},
            zone = zone,
            today = { today },
        )
        model.load()
        return Triple(model, service, player)
    }

    /** The first page: ONE request, its events, its cursor and its summary. */
    @Test
    fun theFirstPageIsOneRequest() {
        val (model, service, _) = harness()
        assertEquals(1, service.firstCalls)
        assertEquals(1, service.cachedCalls)
        val state = model.state.value
        assertEquals(2, state.events.size)
        assertEquals(41, state.summary?.listens?.toInt())
        assertEquals("2026-11-17T20:58:00", state.next?.before.orEmpty())
        assertTrue(state.screen is ScreenState.Ready)
        assertNull("« Tout » asks no filter", service.lastOutcome)
    }

    /** « Charger plus » carries BOTH halves of the cursor and APPENDS. */
    @Test
    fun loadingMoreAppendsTheNextPage() {
        val (model, service, _) = harness()
        model.loadMore()

        assertEquals(1, service.moreCalls)
        assertEquals("2026-11-17T20:58:00", service.lastBefore.orEmpty())
        assertEquals(101L, service.lastBeforeId ?: -1L)
        assertEquals(3, model.state.value.events.size)
        assertNull("the last page ends the log", model.state.value.next)
        assertFalse(model.state.value.loading)
    }

    /** A page whose rows repeat is not shown twice (the id check). */
    @Test
    fun aPageThatRepeatsAnEventNeverShowsItTwice() {
        val service = FakeHistory(
            next = LibraryFixtures.page(events = listOf(LibraryFixtures.event(102, "2026-11-17T21:04:00", 1))),
        )
        val (model, _, _) = harness(service)
        model.loadMore()
        assertEquals("the duplicate was dropped", 2, model.state.value.events.size)
    }

    /** A filter is the server's `outcome`, and another filter starts again. */
    @Test
    fun aFilterAsksItsOwnOutcome() {
        val (model, service, _) = harness()
        model.setFilter(HistoryFilter.Skip)

        assertEquals("skip", service.lastOutcome.orEmpty())
        assertEquals("the new filter asks its own first page", 2, service.firstCalls)
        assertEquals(HistoryFilter.Skip, model.state.value.filter)
    }

    /** A later page that fails says WHY and offers its own retry. */
    @Test
    fun aFailedLaterPageNamesItsCauseAndRetries() {
        val service = FakeHistory()
        service.failMore = IllegalStateException("Hors ligne")
        val (model, _, _) = harness(service)

        model.loadMore()
        assertEquals("Hors ligne", model.state.value.moreError.orEmpty())
        assertEquals("what was read stays", 2, model.state.value.events.size)

        service.failMore = null
        model.retry()
        assertEquals(2, service.moreCalls)
        assertNull(model.state.value.moreError)
    }

    /** A first page that fails is the cause and « Réessayer », never a spinner. */
    @Test
    fun aFailedFirstPageIsTheCauseAndRetry() {
        val service = FakeHistory()
        service.failFirst = IllegalStateException("Hors ligne")
        service.cached = null
        val (model, _, _) = harness(service)

        val failure = model.state.value.failure
        assertTrue(failure != null)
        assertEquals("Hors ligne", failure!!.cause)

        service.failFirst = null
        model.retry()
        assertEquals(2, service.firstCalls)
        assertNull(model.state.value.failure)
    }

    /** An empty history is the empty state, not a card of zeros. */
    @Test
    fun anEmptyHistorySaysSo() {
        val service = FakeHistory(first = LibraryFixtures.emptyPage(), cached = null)
        val (model, _, _) = harness(service)
        assertTrue(model.state.value.events.isEmpty())
        assertTrue(model.state.value.emptyHistory)
    }

    /** A tap plays the title alone, then its radio (`playThenRadio`). */
    @Test
    fun aTapPlaysTheTitleThenItsRadio() {
        val (model, _, player) = harness()
        model.play(model.state.value.events.first())
        assertEquals(1, player.plays.size)
        assertEquals("radio", player.plays.single().second)
    }

    // -- the days (pure functions, a fixed today) ------------------------------

    @Test
    fun theDaysSayTodayYesterdayAndTheFrenchDate() {
        assertEquals("Aujourd'hui", dayLabel(LocalDate.of(2026, 11, 17), today))
        assertEquals("Hier", dayLabel(LocalDate.of(2026, 11, 16), today))
        assertEquals("Dimanche 15 novembre", dayLabel(LocalDate.of(2026, 11, 15), today))
        assertEquals(
            "capitalised, with the year when it is not this one: "
                .plus(dayLabel(LocalDate.of(2025, 3, 4), today)),
            "Mardi 4 mars 2025",
            dayLabel(LocalDate.of(2025, 3, 4), today),
        )
    }

    @Test
    fun aDayCountsItsOwnListensSkipsAndMinutes() {
        val events = listOf(
            LibraryFixtures.event(1, "2026-11-17T21:04:00", 1, listened = 187u),
            LibraryFixtures.event(2, "2026-11-17T20:58:00", -1, listened = 12u),
            LibraryFixtures.event(3, "2026-11-17T20:40:00", 0, listened = 61u),
            LibraryFixtures.event(4, "2026-11-16T09:12:00", 1, listened = 200u),
        )
        val days = groupByDay(events, zone, today)
        assertEquals(2, days.size)
        assertEquals("Aujourd'hui", days[0].label)
        assertEquals(1, days[0].listens)
        assertEquals(1, days[0].skips)
        assertEquals(260, days[0].seconds)
        assertEquals("1 écoute · 1 saut · 4 min", days[0].meta)
        assertEquals("Hier", days[1].label)
        assertEquals(1, days[1].listens)
    }

    @Test
    fun theRowsFlattenWithADayHeadingBeforeItsEvents() {
        val days = groupByDay(LibraryFixtures.page().events, zone, today)
        val rows = flattenDays(days)
        assertEquals(3, rows.size)
        assertTrue(rows[0] is HistoryRowView.Day)
        assertTrue(rows[1] is HistoryRowView.Event)
        assertEquals("d:2026-11-17", rows[0].key)
        assertEquals("e:102", rows[1].key)
    }

    @Test
    fun aRowsVerdictAndTimeAreTheSpas() {
        val listen = LibraryFixtures.event(1, "2026-11-17T21:04:00", 1, listened = 187u)
        val skip = LibraryFixtures.event(2, "2026-11-17T20:58:00", -1, listened = 12u)
        val partial = LibraryFixtures.event(3, "2026-11-17T20:40:00", 0, listened = 0u)

        assertEquals(HistoryVerdict.Listen, verdictOf(listen))
        assertEquals(HistoryVerdict.Skip, verdictOf(skip))
        assertEquals(HistoryVerdict.Partial, verdictOf(partial))
        assertEquals("Écouté", verdictOf(listen).label)
        assertEquals("21:04", whenOf(listen, zone))
        assertEquals("3:07 écoutés", listenedLine(listen))
        assertEquals("Pas écouté", listenedLine(partial))
    }

    @Test
    fun aTimeIsReadInTheDeviceZone() {
        val event: HistoryEventData = LibraryFixtures.event(9, "2026-11-17T21:04:00", 1)
        assertEquals(LocalDate.of(2026, 11, 17), dayOf(event, zone))
        assertEquals(LocalDate.of(2026, 11, 17), dayOf(event, ZoneId.of("UTC")))
        // A row that carries an offset is brought home before it is grouped.
        val zoned = event.copy(at = "2026-11-17T23:30:00Z")
        assertEquals(LocalDate.of(2026, 11, 18), dayOf(zoned, zone))
        assertNull(dayOf(event.copy(at = "pas une date"), zone))
    }
}
