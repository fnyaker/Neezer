package org.nsupysonic.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The car overlay's visibility rule, pure (see [CarVisibility]).
 *
 * The person's report: the card stayed on top of every app as soon as the car
 * radio was connected (`gps || bluetooth`), and a manual start never let it go.
 * GPS must come first; `manual` is not part of the formula at all.
 */
class CarVisibilityTest {

    @Test
    fun aNavigationAppInFrontShowsTheCard() {
        assertTrue(CarVisibility.wanted(true, true, true, true, false))
        assertTrue(CarVisibility.wanted(true, false, true, true, false))
        assertTrue(CarVisibility.wanted(true, false, true, true, true))
    }

    @Test
    fun bluetoothNeverKeepsTheCardOverAnotherApp() {
        // The old OR formula answered true here: YouTube with the car radio
        // connected is exactly the reported symptom.
        assertFalse(CarVisibility.wanted(true, true, true, false, true))
        assertFalse(CarVisibility.wanted(true, false, true, false, false))
    }

    @Test
    fun bluetoothAloneStillWorksWithTheGpsTriggerOff() {
        // The documented mode: the person switched the GPS trigger off and
        // asked for the radio to decide.
        assertTrue(CarVisibility.wanted(false, true, false, false, true))
        assertFalse(CarVisibility.wanted(false, true, false, false, false))
        assertFalse(CarVisibility.wanted(false, false, false, true, true))
    }

    @Test
    fun aGpsTriggerWithoutUsageAccessFallsBackToBluetooth() {
        // gpsTrigger = true, gpsUsable = false: the app cannot see the front
        // app, so only Bluetooth is left (and a GPS-only mode is refused at
        // auto-start by CarModeService).
        assertTrue(CarVisibility.wanted(true, true, false, false, true))
        assertFalse(CarVisibility.wanted(true, false, false, true, true))
    }

    @Test
    fun bothTriggersOffNeverShowThroughTheFormula() {
        assertFalse(CarVisibility.wanted(false, false, false, false, false))
        assertFalse(CarVisibility.wanted(false, false, false, true, true))
    }

    @Test
    fun aManualStartIsNotAVisibilitySource() {
        // The state a manual start sees before the seed: not visible, not
        // decided. The formula says no, and `manual` cannot override it.
        val fresh = DriveState(visible = false, decided = false)
        assertFalse(fresh.visible)
        assertFalse(CarVisibility.wanted(true, true, true, fresh.gpsInFront, true))
        assertFalse(CarVisibility.shouldAttach(fresh.visible, false, false, false))
    }

    @Test
    fun theOverlayAttachesExactlyOnce() {
        // ACTION_REFRESH on return from the car settings: the first call
        // attaches, the second (overlayAttached is now true) must not stack a
        // second window.
        assertTrue(CarVisibility.shouldAttach(true, false, false, false))
        assertFalse(CarVisibility.shouldAttach(true, false, false, true))
        // Suppressed by the settings screen, or deliberately closed.
        assertFalse(CarVisibility.shouldAttach(true, true, false, false))
        assertFalse(CarVisibility.shouldAttach(true, false, true, false))
        // Not wanted: no attach.
        assertFalse(CarVisibility.shouldAttach(false, false, false, false))
    }

    @Test
    fun detachWaitsForTheFirstAnswerOnly() {
        // A new trigger emits before its seed: do not tear a manual card down
        // while the six-hour query is still running.
        assertFalse(CarVisibility.shouldDetach(decided = false, visible = false, attached = true))
        // The answer is known and is "no navigation app": the card goes,
        // manual or not (the old `if (!manual)` is exactly what leaked).
        assertTrue(CarVisibility.shouldDetach(decided = true, visible = false, attached = true))
        // Visible, or nothing to detach.
        assertFalse(CarVisibility.shouldDetach(decided = true, visible = true, attached = true))
        assertFalse(CarVisibility.shouldDetach(decided = true, visible = false, attached = false))
    }
}
