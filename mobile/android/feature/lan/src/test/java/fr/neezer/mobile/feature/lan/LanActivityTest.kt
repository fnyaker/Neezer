package fr.neezer.mobile.feature.lan

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.LanStateFfi

/**
 * The battery rule of the LAN (lot LN14): discovery runs while the app is in
 * front, or playing, or while a download runs, and stops 60 s after the last of
 * those.
 *
 * The clock is the test's (`runTest`'s virtual time), never the wall's: `advance
 * by 59 999 ms and nothing happened, by 1 more and it did` is an assertion; a
 * `sleep` would be a hope.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LanActivityTest {
    @Test
    fun any_of_the_three_flags_starts_the_discovery_at_once() = runTest {
        val events = mutableListOf<String>()
        val policy = LanActivityScheduler(
            scope = backgroundScope,
            graceMs = 60_000,
            onActive = { events += "active" },
            onIdle = { events += "idle" },
        )

        policy.update(foreground = false, playing = false, downloading = false)
        assertTrue(events.isEmpty())

        policy.update(foreground = true, playing = false, downloading = false)
        assertEquals(listOf("active"), events)

        policy.update(foreground = false, playing = true, downloading = false)
        advanceTimeBy(120_000)
        runCurrent()
        // A track playing is "the person is here": nothing goes idle.
        assertEquals(listOf("active"), events)
    }

    @Test
    fun the_stop_comes_sixty_seconds_after_the_last_activity() = runTest {
        val events = mutableListOf<String>()
        val policy = LanActivityScheduler(
            scope = backgroundScope,
            graceMs = LanActivityScheduler.SLEEP_GRACE_MS,
            onActive = { events += "active" },
            onIdle = { events += "idle" },
        )
        policy.update(foreground = true, playing = false, downloading = false)
        policy.update(foreground = false, playing = false, downloading = false)

        advanceTimeBy(LanActivityScheduler.SLEEP_GRACE_MS - 1)
        runCurrent()
        assertEquals("still browsing at 59.999 s", listOf("active"), events)

        advanceTimeBy(1)
        runCurrent()
        assertEquals(listOf("active", "idle"), events)
        assertFalse(policy.active)
    }

    @Test
    fun a_person_who_comes_back_within_the_minute_never_loses_the_browse() = runTest {
        val events = mutableListOf<String>()
        val policy = LanActivityScheduler(
            scope = backgroundScope,
            graceMs = 60_000,
            onActive = { events += "active" },
            onIdle = { events += "idle" },
        )
        policy.update(foreground = true, playing = false, downloading = false)
        policy.update(foreground = false, playing = false, downloading = false)
        advanceTimeBy(30_000)
        runCurrent()

        policy.update(foreground = true, playing = false, downloading = false)
        advanceTimeBy(120_000)
        runCurrent()

        // One activation, no idle: the pending stop was cancelled, not re-run.
        assertEquals(listOf("active"), events)
    }

    @Test
    fun shutdown_reports_idle_once_and_twice_is_a_no_op() = runTest {
        val events = mutableListOf<String>()
        val policy = LanActivityScheduler(
            scope = backgroundScope,
            graceMs = 60_000,
            onActive = { events += "active" },
            onIdle = { events += "idle" },
        )
        policy.update(foreground = true, playing = true, downloading = true)

        policy.shutdown()
        policy.shutdown()

        assertEquals(listOf("active", "idle"), events)
    }

    @Test
    fun the_grace_is_the_notes_sixty_seconds() {
        // Spelled here, not read from the constant the code uses (LN7's rule: a
        // test must not take its boundary from what it tests).
        assertEquals(60_000L, LanActivityScheduler.SLEEP_GRACE_MS)
    }

    @Test
    fun two_state_reads_are_two_different_values() {
        // Guard against an enum accidentally aliasing two states (they are
        // objects, so this also documents that a `when` must be exhaustive).
        assertNotEquals(LanStateFfi.Searching, LanStateFfi.Unsupported)
    }
}
