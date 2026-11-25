package fr.neezer.mobile.bridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

/**
 * `NzCoverRequest` is a **data class on purpose** (P1): Coil compares models by
 * equality and restarts the request when `model != previousModel`, so two
 * requests for the same sleeve must be `==`, and two different sleeves must not.
 *
 * Run against the plain `class` this lot replaced, the first assertion fails
 * (`equals` is identity), which is exactly the re-decode the player paid on
 * every track change: three requests instead of one.
 */
class CoversTest {
    @Test
    fun equalSleevesAtTheSameSizeAreEqual() {
        val a = NzCoverRequest(url = "/api/cover/501", sizePx = 720)
        val b = NzCoverRequest(url = "/api/cover/501", sizePx = 720)
        assertEquals(a, b)
        assertEquals(a.hashCode(), b.hashCode())
    }

    @Test
    fun aDifferentSleeveIsNotTheSameRequest() {
        val a = NzCoverRequest(url = "/api/cover/501", sizePx = 720)
        assertNotEquals(a, NzCoverRequest(url = "/api/cover/502", sizePx = 720))
    }

    @Test
    fun theSameSleeveAtAnotherSizeIsNotTheSameRequest() {
        val a = NzCoverRequest(url = "/api/cover/501", sizePx = 720)
        assertNotEquals(a, NzCoverRequest(url = "/api/cover/501", sizePx = 132))
    }
}
