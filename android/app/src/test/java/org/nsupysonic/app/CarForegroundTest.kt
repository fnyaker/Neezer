package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The sticky foreground decision and its transient grace (see
 * [CarForegroundTracker]).
 *
 * Measured consequences: a real app hides the card at grace + at most one 1 s
 * poll (2.0–3.0 s); the notification shade never hides it; a navigation app
 * that went quiet for an hour is still in front (no new event is emitted while
 * it guides); switching Maps → Waze hands the new package to the service.
 */
class CarForegroundTest {

    private val tracker = CarForegroundTracker(graceMs = 2_000)

    @Test
    fun aQuietNavigationAppStaysInFront() {
        assertTrue(tracker.gps("com.google.android.apps.maps"))
        assertTrue(tracker.gpsInFront)
        assertEquals("com.google.android.apps.maps", tracker.gpsPackage)
        assertTrue(tracker.decided)
        // Ten minutes later it still guides and emits nothing: the decision is
        // sticky, the poll must not call it gone.
        assertFalse(tracker.gps("com.google.android.apps.maps"))
        assertFalse(tracker.settle(600_000))
        assertTrue(tracker.gpsInFront)
    }

    @Test
    fun anotherAppHidesTheCardAfterTwoSeconds() {
        tracker.gps("com.google.android.apps.maps")
        tracker.other(10_000)
        assertFalse(tracker.settle(11_999))       // 1.999 s: still the grace
        assertTrue(tracker.gpsInFront)
        assertTrue(tracker.settle(12_000))        // 2.000 s: gone
        assertFalse(tracker.gpsInFront)
        assertNull(tracker.gpsPackage)
    }

    @Test
    fun comingBackInsideTheGraceCancelsTheHide() {
        tracker.gps("com.google.android.apps.maps")
        tracker.other(10_000)
        assertFalse(tracker.settle(11_500))
        // Back to the map before the deadline (notification shade pulled down
        // and closed, a mis-tap on another app): nothing blinks.
        assertFalse(tracker.gps("com.google.android.apps.maps"))
        assertFalse(tracker.settle(60_000))
        assertTrue(tracker.gpsInFront)
    }

    @Test
    fun aSecondOtherAppDoesNotExtendTheGrace() {
        tracker.gps("com.google.android.apps.maps")
        tracker.other(10_000)
        tracker.other(11_000)                     // YouTube then Messages
        assertTrue(tracker.settle(12_000))        // deadline stays 10 s + grace
    }

    @Test
    fun theNotificationShadeNeverHidesTheCard() {
        tracker.gps("com.google.android.apps.maps")
        tracker.system()                          // com.android.systemui
        assertFalse(tracker.settle(3_600_000))
        assertTrue(tracker.gpsInFront)
        assertEquals("com.google.android.apps.maps", tracker.gpsPackage)
    }

    @Test
    fun theShadeDoesNotCancelARealDepartureEither() {
        tracker.gps("com.google.android.apps.maps")
        tracker.other(10_000)
        tracker.system()
        assertTrue(tracker.settle(12_000))
    }

    @Test
    fun theLauncherHidesAfterTheGrace() {
        tracker.gps("com.waze")
        tracker.transient(10_000)                 // Home, or our own screen
        assertFalse(tracker.settle(11_000))
        assertTrue(tracker.settle(12_000))
        assertFalse(tracker.gpsInFront)
    }

    @Test
    fun aLauncherBeforeAnyNavigationAppIsNotNavigation() {
        tracker.transient(10_000)
        assertTrue(tracker.decided)
        assertFalse(tracker.settle(12_000))
        assertFalse(tracker.gpsInFront)
        assertNull(tracker.gpsPackage)
    }

    @Test
    fun switchingFromOneNavigationAppToAnotherUpdatesThePackage() {
        tracker.gps("com.google.android.apps.maps")
        // Maps → Waze: the service restores Waze's own saved position.
        assertTrue(tracker.gps("com.waze"))
        assertEquals("com.waze", tracker.gpsPackage)
        assertTrue(tracker.gpsInFront)
        assertFalse(tracker.settle(10_000))
    }

    @Test
    fun anEmptySeedDecidesThatNoNavigationAppIsInFront() {
        // Six hours with no meaningful foreground event: the card must not
        // wait forever for a seed that never comes.
        tracker.gps("com.google.android.apps.maps")
        assertTrue(tracker.unknown())
        assertTrue(tracker.decided)
        assertFalse(tracker.gpsInFront)
        assertNull(tracker.gpsPackage)
    }

    @Test
    fun returningFromSettingsHidesImmediately() {
        // CarSettingsActivity.onPause goes through refresh(): last foreground
        // package is ours, and a grace here would paint the card over the app
        // the person just opened.
        tracker.gps("com.google.android.apps.maps")
        assertTrue(tracker.hideNow())
        assertTrue(tracker.decided)
        assertFalse(tracker.gpsInFront)
        assertNull(tracker.gpsPackage)
        assertFalse(tracker.settle(60_000))
    }

    @Test
    fun resetForgetsTheAnswerUntilTheNextSeed() {
        tracker.gps("com.google.android.apps.maps")
        tracker.reset()
        assertFalse(tracker.gpsInFront)
        assertFalse(tracker.decided)
        assertNull(tracker.gpsPackage)
        tracker.system()
        assertFalse(tracker.decided)
    }
}
