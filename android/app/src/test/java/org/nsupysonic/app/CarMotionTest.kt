package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The car motion thresholds: a red light must not unlock the keyboard, and a
 * GPS that says nothing must never unlock it either.
 *
 * The rules the car UI relies on (see [CarMotion]):
 * - 5 km/h and above is moving at once;
 * - below 5 km/h the answer stays "moving" for 5 s;
 * - an unknown speed (no fix, a fix without speed, no fix for 5 s) is moving.
 */
class CarMotionTest {

    private val state = CarMotionState()

    @Test
    fun speedIsReportedInKilometresPerHour() {
        assertEquals(54.0, kmhFromMps(15f), 1e-9)
        assertEquals(0.0, kmhFromMps(0f), 1e-9)
        assertEquals(3.6, kmhFromMps(1f), 1e-9)
    }

    @Test
    fun aboveFiveKilometresPerHourIsMovingAtOnce() {
        state.onFix(CarMotionState.MOVING_KMH + 0.1, 1_000)
        assertTrue(state.moving)
        assertEquals(5.1, state.speedKmh, 1e-9)
    }

    @Test
    fun aRedLightKeepsMovingForFiveSeconds() {
        state.onFix(2.0, 0)
        assertTrue(state.moving)
        state.onFix(2.0, 4_999)
        assertTrue(state.moving)      // 4.999 s below 5 km/h is not yet the grace
        state.onFix(2.0, 5_000)
        assertFalse(state.moving)     // 5.000 s: the red light is a stop
        assertEquals(2.0, state.speedKmh, 1e-9)
    }

    @Test
    fun leavingAgainDuringTheGraceRestartsIt() {
        state.onFix(2.0, 0)
        state.onFix(30.0, 1_000)
        assertTrue(state.moving)
        state.onFix(2.0, 2_000)
        state.onFix(2.0, 6_999)
        assertTrue(state.moving)      // only 4.999 s below since 2 s
        state.onFix(2.0, 7_000)
        assertFalse(state.moving)
    }

    @Test
    fun noFixForFiveSecondsIsUnknownAndThereforeMoving() {
        state.onFix(2.0, 0)
        state.onFix(2.0, 5_000)
        assertFalse(state.moving)
        state.silence(9_999)          // 4.999 s since the last fix
        assertFalse(state.moving)
        state.silence(10_000)         // 5 s: unknown, so moving
        assertTrue(state.moving)
        assertEquals(2.0, state.speedKmh, 1e-9)
    }

    @Test
    fun aFixWithoutASpeedIsMoving() {
        state.onFix(2.0, 0)
        state.onFix(2.0, 5_000)
        assertFalse(state.moving)
        state.onFix(null, 6_000)      // approximate/network fix: no speed
        assertTrue(state.moving)
        assertEquals(2.0, state.speedKmh, 1e-9)
    }

    @Test
    fun forgetIsTheSafeDefault() {
        state.onFix(2.0, 0)
        state.onFix(2.0, 5_000)
        assertFalse(state.moving)
        state.forget()
        assertTrue(state.moving)
        assertEquals(0.0, state.speedKmh, 1e-9)
        state.silence(1_000)          // nothing measured yet
        assertTrue(state.moving)
    }
}
