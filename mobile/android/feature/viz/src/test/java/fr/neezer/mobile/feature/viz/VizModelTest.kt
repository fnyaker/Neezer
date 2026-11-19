package fr.neezer.mobile.feature.viz

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The visualiser's screen state, without a device and without a `.so`.
 *
 * What is held here is the SPA's own behaviour: a tap is optimistic, a refused
 * write rolls back and says why, eco means NONE, the sleeve is decoded once per
 * track (and forgotten when the next track has none), and the card/notice follow
 * the player — `stores.js#persisted`, `VisualizerSettings.svelte#pickFlash` and
 * `Visualizer.svelte#loadCover`'s rules.
 */
class VizModelTest {
    private fun model(
        service: FakeVizService = FakeVizService(),
        player: FakeVizPlayer = FakeVizPlayer(),
        covers: FakeVizCovers = FakeVizCovers(),
        toasts: MutableList<String> = mutableListOf(),
    ): Triple<VizModel, FakeVizService, MutableList<String>> {
        val model = VizModel(
            service = service,
            player = player,
            covers = covers,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = { toasts.add(it) },
        )
        model.refresh()
        return Triple(model, service, toasts)
    }

    @Test
    fun theFirstPaintReadsTheStoreAndTheCard() {
        val (model, _, _) = model()
        val state = model.state.value
        assertEquals("bars", state.mode)
        assertEquals(60, state.fps)
        assertEquals("Déjà vu", state.title)
        assertEquals("Beyoncé", state.artists)
        assertTrue(state.draws)
        assertTrue("playing", state.playing)
        assertTrue("loaded", state.loaded)
    }

    @Test
    fun aTapIsOptimisticTheStateChangesBeforeTheStoreIsAsked() {
        val service = FakeVizService()
        val toasts = mutableListOf<String>()
        val model = VizModel(
            service = service,
            player = FakeVizPlayer(),
            covers = FakeVizCovers(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = { toasts.add(it) },
        )
        model.refresh()
        // The fake records the state at the instant the service is called: an
        // optimistic write has already painted the new choice by then.
        service.observer = { model.state.value.settings?.chosenMode ?: "?" }
        model.setMode("scope")
        assertEquals("the state moved BEFORE the call", "scope", service.stateAtCall)
        assertEquals("scope", model.state.value.settings?.chosenMode)
        assertTrue(toasts.isEmpty())
    }

    @Test
    fun aRefusedWriteRollsBackAndOffersTheWayBack() {
        val service = FakeVizService()
        val toasts = mutableListOf<String>()
        val (model, _, _) = model(service = service, toasts = toasts)
        service.refused = "viz.mode : mode inconnue"
        model.setMode("scope")
        val state = model.state.value
        assertEquals("rolled back to what the store holds", "bars", state.settings?.chosenMode)
        assertEquals("viz.mode", state.failure?.key)
        assertEquals(VizCopy.SETTING_FAILED, toasts.single())
        // « Réessayer » is a real action, not a label: with the store fixed it
        // writes for real.
        service.refused = null
        state.failure?.retry?.invoke()
        assertEquals("scope", model.state.value.settings?.chosenMode)
        assertNull(model.state.value.failure)
    }

    @Test
    fun ecoMeansNoneAndTheLeafSaysSo() {
        val toasts = mutableListOf<String>()
        val (model, service, _) = model(toasts = toasts)
        model.toggleEco()
        assertEquals("off", model.state.value.mode)
        assertTrue(model.state.value.eco)
        assertEquals(VizCopy.ECO_ON, toasts.last())
        assertTrue("the store was written", service.calls.contains("eco"))
        model.toggleEco()
        assertEquals("bars", model.state.value.mode)
        assertEquals(VizCopy.ECO_OFF, toasts.last())
    }

    @Test
    fun ecoCapsThePictureAtThirty() {
        // `settings::VizSettings#settle` owns the cap; the model shows what Rust
        // resolved, so the surface's loop reads the number from one place.
        val (model, _, _) = model(service = FakeVizService(settings = VizFixtures.settings(eco = true)))
        assertEquals(30, model.state.value.fps)
        assertEquals("off", model.state.value.mode)
    }

    @Test
    fun theSleeveIsAskedForOncePerTrackAndForgottenWhenThereIsNone() {
        val covers = FakeVizCovers(mapOf("cov-1" to "/nowhere/cov-1.jpg", "cov-2" to "/nowhere/cov-2.jpg"))
        val player = FakeVizPlayer()
        val (model, service, _) = model(player = player, covers = covers)
        // `refresh` already followed the track once.
        model.follow()
        model.follow()
        assertEquals("one ask per track, not per tick", listOf("cov-1"), covers.asked)
        assertEquals("the file does not exist: the row goes back to the walk", 1, service.cleared)

        // A second track asks for its own sleeve, once.
        player.snapshotNow = VizFixtures.snapshot(track = VizFixtures.track(title = "Autre", cover = "cov-2"))
        model.follow()
        model.follow()
        assertEquals(listOf("cov-1", "cov-2"), covers.asked.toList())
    }

    @Test
    fun aTrackWithNoArtworkClearsTheRow() {
        val covers = FakeVizCovers(mapOf("cov-1" to "/nowhere/cov-1.jpg"))
        val player = FakeVizPlayer()
        val (model, service, _) = model(player = player, covers = covers)
        player.snapshotNow = VizFixtures.snapshot(track = VizFixtures.track(cover = null))
        model.follow()
        assertNull("the card carries no sleeve", model.state.value.cover)
        assertEquals("asked, then cleared for a track with no sleeve", 2, service.cleared)
    }

    @Test
    fun thePauseStopsTheSceneAndTheCardStays() {
        val player = FakeVizPlayer(VizFixtures.snapshot(playing = false))
        val (model, _, _) = model(player = player)
        assertTrue(!model.state.value.playing)
        assertEquals("Déjà vu", model.state.value.title)
        player.snapshotNow = VizFixtures.snapshot(playing = true)
        model.follow()
        assertTrue(model.state.value.playing)
    }

    @Test
    fun nothingInThePlayerIsAnEmptyScreenNotAnError() {
        val player = FakeVizPlayer(VizFixtures.snapshot(playing = false, track = null))
        val (model, _, _) = model(player = player)
        assertTrue(!model.state.value.loaded)
        assertEquals("", model.state.value.title)
        assertTrue(!model.state.value.playing)
    }

    @Test
    fun theScopeKnobsAreWrittenToTheirOwnKeys() {
        val (model, service, _) = model()
        model.setScopeOrientation("vertical")
        model.setScopeColour("mono")
        model.setPalette("duo")
        model.setFps(30)
        assertEquals("vertical", model.state.value.orientation)
        assertEquals("mono", model.state.value.scopeColour)
        assertEquals("duo", model.state.value.palette)
        assertEquals(30, model.state.value.fps)
        assertTrue(service.calls.containsAll(listOf("orientation", "colour", "palette", "fps")))
    }
}
