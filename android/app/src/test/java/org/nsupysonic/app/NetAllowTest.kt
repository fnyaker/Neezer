package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import java.io.IOException
import java.util.concurrent.atomic.AtomicInteger

/**
 * [Net.get] must ask its `allow` predicate about the FIRST URL before it parses
 * anything or opens a socket; the redirect half is the same check at the top
 * of the loop (see [NetOriginTest] for the no-cookie behaviour).
 */
class NetAllowTest {

    @Test
    fun theFirstHopIsRefusedBeforeAnyParseOrConnection() {
        val calls = AtomicInteger()
        val refused = try {
            Net.get(
                prefs = testPrefs(),
                url = "https://evil.example/cover.jpg",
                cookie = null,
                connectTimeout = 1_000,
                readTimeout = 1_000,
                allow = { calls.incrementAndGet(); false },
            )
            null
        } catch (error: IOException) {
            error
        }
        assertNotNull(refused)
        assertEquals("host refused", refused!!.message)
        assertEquals(1, calls.get())
    }

    @Test
    fun thePredicateSeesTheExactUrlBeforeItIsParsed() {
        val seen = ArrayList<String>()
        try {
            Net.get(
                prefs = testPrefs(),
                url = "https://127.0.0.1/internal",
                cookie = null,
                connectTimeout = 1_000,
                readTimeout = 1_000,
                allow = { url ->
                    seen.add(url)
                    false
                },
            )
        } catch (_: IOException) {
        }
        assertEquals(listOf("https://127.0.0.1/internal"), seen)
    }
}
