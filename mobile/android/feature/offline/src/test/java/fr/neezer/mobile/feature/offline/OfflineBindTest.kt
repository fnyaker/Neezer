package fr.neezer.mobile.feature.offline

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The bind's two rules, held without a `.so`:
 *
 * 1. **an `attach` that fails PROPAGATES** — the screen shows the cause and offers
 *    « Réessayer » (rules 2 and 3); the version this lot found swallowed it with
 *    `runCatching { … }.isSuccess`, and on every device the attach answered
 *    `there is no reactor running` (`offline_attach` had no
 *    `async_runtime = "tokio"`), so nothing downloaded and nothing said why;
 * 2. **a failed bind is not marked done**, so the next call tries again instead of
 *    leaving the app permanently download-less.
 */
class OfflineBindTest {

    /** A bind that throws reaches the caller, with its own cause. */
    @Test
    fun a_failing_attach_surfaces_its_cause() = runTest {
        var attempts = 0
        val bound = java.util.concurrent.atomic.AtomicBoolean(false)
        val thrown = runCatching {
            if (bindOnce(bound.get()) {
                    attempts += 1
                    throw IllegalStateException("there is no reactor running")
                }
            ) {
                bound.set(true)
            }
        }.exceptionOrNull()
        assertEquals("the cause is NOT swallowed", "there is no reactor running", thrown?.message)
        assertEquals("one attempt", 1, attempts)
        assertFalse("and the bind is NOT marked done", bound.get())
    }

    /** The next call retries, and a bind that works is done once. */
    @Test
    fun a_retry_binds_and_a_good_bind_is_kept() = runTest {
        var attempts = 0
        val bound = java.util.concurrent.atomic.AtomicBoolean(false)
        // First: the device's failure (no reactor), swallowed nowhere.
        runCatching {
            bindOnce(bound.get()) {
                attempts += 1
                error("there is no reactor running")
            }
        }
        assertFalse(bound.get())
        // Then: the fixed export answers, and the bind is final.
        assertTrue(
            bindOnce(bound.get()) {
                attempts += 1
            },
        )
        bound.set(true)
        assertTrue(bindOnce(bound.get()) { attempts += 1 })
        assertEquals("two attempts, then no more", 2, attempts)
        assertTrue(bound.get())
    }
}
