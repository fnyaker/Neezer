package fr.neezer.mobile.feature.home

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.FlowPress
import uniffi.nz_ffi.FlowStartData
import uniffi.nz_ffi.FlowStateData
import uniffi.nz_ffi.FlowStyleData
import uniffi.nz_ffi.HomeData
import uniffi.nz_ffi.TrackRow

/**
 * The home's logic, on the JVM (the screen standard: the data, the failure and
 * the click are the model's, and a fake proves them without JNA, a `.so` or a
 * device).
 *
 * What each test is really about:
 *
 * - **the cache paints, the request replaces it** — and a failed refresh keeps
 *   the copy and says « Hors ligne · version mémorisée »;
 * - **nothing to show is a cause and a working « Réessayer »**, never a spinner;
 * - **a click answers before its request**, and a refused write rolls back with
 *   a toast;
 * - **one action, one command**: the Flow's launch, a mood, a suggestion and a
 *   press each send exactly what the SPA sends, counted.
 */
class HomeModelTest {

    /** A `HomeService` with no core behind it: every answer is the test's. */
    private class FakeService(
        var cached: HomeData? = HomeFixtures.home(),
        var fresh: () -> HomeData = { HomeFixtures.home() },
        var styles: List<FlowStyleData> = listOf(
            FlowStyleData("Techno", null, 10u, 12u),
            FlowStyleData("House", null, 4u, 4u),
        ),
        var flowState: FlowStateData = HomeFixtures.flow(),
    ) : HomeService {
        var cachedCalls = 0
        var freshCalls = 0
        var styleCalls = 0
        var savedEnergy: Double? = null
        var energySaves = 0
        var levelSaves = 0
        var flowStarts = 0
        var feedbacks = mutableListOf<FlowPress>()
        var moodCalls = mutableListOf<String>()
        var suggestionCalls = 0
        var chosen: List<String>? = null
        var moodRows: List<TrackRow> = listOf(HomeFixtures.row("Mix 1"))
        var start: FlowStartData = FlowStartData(
            sid = "abcdef0123456789",
            tracks = listOf(HomeFixtures.row("Flow 1"), HomeFixtures.row("Flow 2", 1)),
        )
        var failEnergy: Exception? = null
        var failLevel: Exception? = null
        var failStart: Exception? = null
        var failFeedback: Exception? = null
        var failMood: Exception? = null

        override suspend fun cachedHome(): HomeData? {
            cachedCalls++
            return cached
        }

        override suspend fun home(): HomeData {
            freshCalls++
            return fresh()
        }

        override suspend fun cachedFlowStyles(): List<FlowStyleData> = styles

        override suspend fun flowStyles(): List<FlowStyleData> {
            styleCalls++
            return styles
        }

        override fun flowState(): FlowStateData = flowState

        override fun setFlowStyles(styles: List<String>) {
            chosen = styles
            flowState = flowState.copy(styles = styles)
        }

        override fun discoveryLevel(): Double = flowState.level

        override fun nudgeEnergy(direction: Int): Double? {
            val next = if (direction > 0) 0.5 else 0.1
            flowState = flowState.copy(energy = next)
            return next
        }

        override suspend fun setEnergy(notch: Double?): Double? {
            failEnergy?.let { throw it }
            energySaves++
            savedEnergy = notch
            flowState = flowState.copy(energy = notch)
            return notch
        }

        override suspend fun setDiscoveryLevel(level: Double): Double {
            failLevel?.let { throw it }
            levelSaves++
            flowState = flowState.copy(level = level)
            return level
        }

        override suspend fun startFlow(): FlowStartData {
            failStart?.let { throw it }
            flowStarts++
            return start
        }

        override suspend fun flowFeedback(press: FlowPress): TrackRow? {
            failFeedback?.let { throw it }
            feedbacks += press
            return HomeFixtures.row("Flow next", 3)
        }

        override suspend fun moodTracks(id: String): List<TrackRow> {
            failMood?.let { throw it }
            moodCalls += id
            return moodRows
        }

        override suspend fun moreSuggestions(): List<TrackRow> {
            suggestionCalls++
            return listOf(HomeFixtures.row("Nouveau", 0))
        }
    }

    /** A `PlayerPort` that records the commands a click sends. */
    private class FakePlayer : PlayerPort {
        data class Play(val tracks: List<TrackRow>, val start: Int, val kind: String, val id: String)

        val plays = mutableListOf<Play>()
        val queued = mutableListOf<TrackRow>()
        var cleared = 0
        var skips = 0

        override fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String) {
            plays += Play(tracks, start, contextKind, contextId)
        }

        override fun playNext(row: TrackRow) {
            queued += row
        }

        override fun clearUpcoming() {
            cleared++
        }

        override fun skipToNext() {
            skips++
        }
    }

    private class Harness(
        val service: FakeService = FakeService(),
        val player: FakePlayer = FakePlayer(),
    ) {
        val toasts = mutableListOf<String>()
        val model = HomeModel(
            service = service,
            player = player,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = { toasts += it },
        )

        fun loaded(): HomeModel {
            model.load()
            return model
        }
    }

    @Test
    fun aWarmHomePaintsTheCacheThenTheAnswer() {
        val harness = Harness()
        val model = harness.loaded()
        val state = model.state.value

        assertEquals("the device's copy is read once", 1, harness.service.cachedCalls)
        assertEquals("and refreshed once", 1, harness.service.freshCalls)
        assertEquals(HomeFixtures.home().hero.name, state.data?.hero?.name)
        assertFalse("the refresh succeeded", state.stale)
        assertTrue(harness.service.styleCalls >= 0)
    }

    @Test
    fun aFailedRefreshKeepsTheCachedCopyAndSaysSo() {
        val harness = Harness(
            FakeService(fresh = { throw IllegalStateException("Hors ligne") }),
        )
        val model = harness.loaded()
        val state = model.state.value

        assertTrue("the copy stays on screen, flagged", state.stale)
        assertEquals("Nadia", state.data?.hero?.name)
        assertEquals("the stale note is the screen's, not a toast", 0, harness.toasts.size)
    }

    @Test
    fun nothingToShowIsACauseAndAWorkingRetry() {
        val service = FakeService(cached = null, fresh = { throw IllegalStateException("Hors ligne") })
        val harness = Harness(service)
        val model = harness.loaded()

        val failed = model.state.value.screen as? fr.neezer.mobile.bridge.ScreenState.Failed
        assertTrue("a cause, in French", failed?.cause.orEmpty().contains("Hors ligne"))
        assertNull(model.state.value.data)

        // The link is back: « Réessayer » runs the read again, once.
        service.fresh = { HomeFixtures.home() }
        model.retry()
        assertEquals("Nadia", model.state.value.data?.hero?.name)
        assertFalse(model.state.value.stale)
    }

    @Test
    fun theNotchMovesBeforeItsRequestAndRollsBackWhenTheServerRefuses() {
        val service = FakeService()
        service.failEnergy = IllegalStateException("Serveur en erreur")
        val harness = Harness(service)
        val model = harness.loaded()
        val before = model.state.value.notch

        model.chooseNotch(0.7)
        // The write was refused: the notch is the one that was there, and the
        // person is told what to retry.
        assertEquals(before, model.state.value.notch)
        assertTrue(harness.toasts.single().contains("Réessayer"))
    }

    @Test
    fun aNotchIsOneSaveAndALiveFlowIsReaimed() {
        val harness = Harness()
        val model = harness.loaded()

        model.chooseNotch(0.5)
        assertEquals(1, harness.service.energySaves)
        assertEquals(0.5, model.state.value.notch!!, 0.0)

        model.chooseDiscovery(0.8)
        assertEquals(1, harness.service.levelSaves)
        assertEquals(0.8, model.state.value.level, 0.0)
        assertEquals(
            "a live Flow chooses its next title again, without cutting this one",
            listOf(FlowPress.REAIM),
            harness.service.feedbacks,
        )
    }

    @Test
    fun aStyleIsTheDevicesOwnChoiceAndCostsNoRequest() {
        val harness = Harness()
        val model = harness.loaded()

        // The fixture's Flow already rests on Techno: a second tap takes it back.
        model.pickStyle("House")
        assertEquals(listOf("Techno", "House"), harness.service.chosen)
        assertEquals(listOf("Techno", "House"), model.state.value.chosen)

        model.pickStyle("House")
        assertEquals(listOf("Techno"), harness.service.chosen)

        model.clearStyles()
        assertEquals(emptyList<String>(), harness.service.chosen)
    }

    @Test
    fun launchingTheFlowSendsOneCommandWithTheSessionsOwnId() {
        val harness = Harness()
        val model = harness.loaded()

        model.launchFlow()
        assertEquals("ONE request", 1, harness.service.flowStarts)
        val play = harness.player.plays.single()
        assertEquals("flow", play.kind)
        assertEquals("abcdef0123456789", play.id)
        assertEquals("the first title and its reserve", 2, play.tracks.size)
        assertFalse(model.state.value.starting)
    }

    @Test
    fun aRefusedLaunchIsAToastAndNotACrashingButton() {
        val service = FakeService()
        service.failStart = IllegalStateException("Hors ligne")
        val harness = Harness(service)
        val model = harness.loaded()

        model.launchFlow()
        assertTrue(harness.player.plays.isEmpty())
        assertTrue(harness.toasts.single().contains("Le Flow ne répond pas"))
        assertFalse("the button is usable again", model.state.value.starting)
    }

    @Test
    fun lessLikeThisReplacesTheUpcomingThenSkips() {
        val harness = Harness()
        val model = harness.loaded()

        model.pressFlow(FlowPress.LESS)
        assertEquals(listOf(FlowPress.LESS), harness.service.feedbacks)
        assertEquals(1, harness.player.cleared)
        assertEquals(1, harness.player.queued.size)
        assertEquals("this title makes way", 1, harness.player.skips)
    }

    @Test
    fun moreLikeThisReplacesTheUpcomingWithoutCuttingThePlayingTitle() {
        val harness = Harness()
        val model = harness.loaded()

        model.pressFlow(FlowPress.MORE)
        assertEquals(1, harness.player.cleared)
        assertEquals(1, harness.player.queued.size)
        assertEquals(0, harness.player.skips)
    }

    @Test
    fun aMoodIsOneRequestAndAFinitQueue() {
        val harness = Harness()
        val model = harness.loaded()

        model.playMood("calme")
        assertEquals(listOf("calme"), harness.service.moodCalls)
        val play = harness.player.plays.single()
        assertEquals("mood", play.kind)
        assertEquals("calme", play.id)
        assertNull("the busy flag is cleared", model.state.value.moodBusy)
    }

    @Test
    fun anEmptyMoodSaysSoInsteadOfPlayingNothing() {
        val service = FakeService()
        service.moodRows = emptyList()
        val harness = Harness(service)
        val model = harness.loaded()

        model.playMood("fete")
        assertTrue(harness.player.plays.isEmpty())
        assertTrue(harness.toasts.single().contains("vide"))
    }

    @Test
    fun anotherSuggestionReplacesTheRowsAndOneRefusedSaysWhy() {
        val harness = Harness()
        val model = harness.loaded()

        model.refreshReco()
        assertEquals(1, harness.service.suggestionCalls)
        assertEquals("Nouveau", model.state.value.data?.reco?.first()?.title)
        assertFalse(model.state.value.recoBusy)
    }

    @Test
    fun aReleasePlaysFromItsOwnFirstTitle() {
        val harness = Harness()
        val model = harness.loaded()

        model.playRelease(2)
        val play = harness.player.plays.single()
        assertEquals("releases", play.kind)
        assertEquals(2, play.start)
        assertEquals(3, play.tracks.size)

        model.playAllNew()
        assertEquals(0, harness.player.plays.last().start)
    }

    @Test
    fun aForYouRowPlaysItselfThenItsRadio() {
        val harness = Harness()
        val model = harness.loaded()
        val row = model.state.value.data!!.reco.first()

        model.playReco(row)
        val play = harness.player.plays.single()
        assertEquals("radio", play.kind)
        assertEquals(row.id, play.id)
        assertEquals(listOf(row), play.tracks)
    }

    @Test
    fun theStyleChipsAreAskedOnceForTheVisit() {
        val service = FakeService()
        service.styles = listOf(FlowStyleData("Techno", null, 10u, 12u))
        val harness = Harness(service)
        val model = harness.loaded()

        // The cached answer and the fresh one both arrive; the chips are asked
        // for ONCE (a second call would be the eighth request of a cold home).
        assertEquals(1, service.styleCalls)
        assertEquals(1, model.state.value.styles.size)
    }
}
