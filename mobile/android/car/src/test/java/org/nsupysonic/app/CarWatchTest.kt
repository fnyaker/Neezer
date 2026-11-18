package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The car mode's lifecycle, transition by transition (see [CarVisibility] and
 * [CarLifecycle]): the service exists only while a trigger wants it.
 */
class CarWatchTest {

    private fun ends(
        decided: Boolean = true,
        visible: Boolean = false,
        attached: Boolean = false,
        manual: Boolean = false,
        suppressed: Boolean = false,
    ) = CarLifecycle.endsWithTheDrive(decided, visible, attached, manual, suppressed)

    @Test
    fun aDetectedNavigationAppStartsTheMode() {
        // GPS trigger usable, Maps in front: the watch starts the service, the
        // card and its notification come with it.
        assertTrue(CarVisibility.wanted(true, true, true, true, false))
        // The Bluetooth trigger with the GPS trigger off, the radio connected.
        assertTrue(CarVisibility.wanted(false, true, false, false, true))
        // A Bluetooth connection alone, while GPS is watching, is not a start:
        // the service would stop a second later and flash a notification.
        assertFalse(CarVisibility.wanted(true, true, true, false, true))
    }

    @Test
    fun theServiceEndsWithTheDrive() {
        // The navigation app left: no card, no service, no notification.
        assertTrue(ends())
        // Still in front: the mode stays with its notification.
        assertFalse(ends(visible = true, attached = true))
        // The seed has not answered yet: nothing may be torn down on a guess.
        assertFalse(ends(decided = false))
        // The settings screen is on top: the overlay was detached on purpose.
        assertFalse(ends(attached = true))
        assertFalse(ends(suppressed = true))
    }

    @Test
    fun aRebootWithNothingInFrontStartsNothing() {
        // Both triggers on, the usage access granted, and at boot neither a
        // navigation app nor a radio: no service, no notification. (The
        // persisted job looks again every 15 minutes.)
        assertFalse(CarVisibility.wanted(true, true, true, false, false))
        assertFalse(CarVisibility.wanted(false, true, false, false, false))
    }

    @Test
    fun aDisabledTriggerNeverStartsAnything() {
        assertFalse(CarVisibility.wanted(false, false, false, false, false))
        assertFalse(CarVisibility.wanted(false, false, false, true, true))
        assertFalse(CarVisibility.wanted(false, false, true, true, true))
    }

    @Test
    fun aManualStartIsNotEndedByTheDriveRule() {
        // The tile/settings start: the person's own mode, stopped by them.
        assertFalse(ends(manual = true))
        // The card itself still goes when no navigation app is in front; only
        // the service (and its notification) stays.
        assertTrue(CarVisibility.shouldDetach(decided = true, visible = false, attached = true))
    }

    @Test
    fun theLadderIsBoundedAndOrdered() {
        assertEquals(listOf(10_000L, 60_000L, 300_000L), CarLifecycle.LADDER_MS.toList())
        assertEquals(10_000L, CarLifecycle.wakeAfter(0) ?: 0)
        assertEquals(300_000L, CarLifecycle.wakeAfter(2) ?: 0)
        assertNull(CarLifecycle.wakeAfter(-1))
        // Without the app playing in a charging car, the ladder ends after its
        // last rung: the 15-minute job is the only net then.
        assertEquals(1, CarLifecycle.nextRung(0, playing = false))
        assertEquals(2, CarLifecycle.nextRung(1, playing = false))
        assertEquals(-1, CarLifecycle.nextRung(2, playing = false))
        assertEquals(-1, CarLifecycle.nextRung(-1, playing = true))
        // With it, the sustain rung keeps looking once a minute — the process
        // is already alive for the player — and stops as soon as it does.
        assertEquals(CarLifecycle.LADDER_MS.size, CarLifecycle.nextRung(2, playing = true))
        assertEquals(CarLifecycle.LADDER_MS.size, CarLifecycle.nextRung(3, playing = true))
        assertEquals(CarLifecycle.SUSTAIN_MS, CarLifecycle.wakeAfter(3) ?: 0)
        assertEquals(-1, CarLifecycle.nextRung(3, playing = false))
    }

    @Test
    fun theOverlaysNeverCountAsAForegroundApp() {
        // The shade and permission dialogs are drawn OVER the map: they must
        // neither hide a card nor start the mode by themselves.
        assertTrue(CarForegroundScan.OVERLAYS.contains("com.android.systemui"))
        assertTrue(CarForegroundScan.OVERLAYS.contains("com.android.permissioncontroller"))
        assertFalse(CarForegroundScan.OVERLAYS.contains("com.google.android.apps.maps"))
        assertFalse(CarForegroundScan.OVERLAYS.contains("com.waze"))
    }
}
