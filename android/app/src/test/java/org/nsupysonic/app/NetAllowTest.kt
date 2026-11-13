package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import java.io.IOException
import java.util.concurrent.atomic.AtomicInteger

/**
 * [Net.get] must ask its `allow` predicate about the FIRST URL before it opens
 * anything, and about every redirect target before the next hop. These tests
 * do not need a network: a refused hop throws `IOException("host refused")`
 * before `Prefs` is read or a socket is created.
 *
 * The redirect half is the same check inside the loop; it is not exercised
 * end to end here because [Net] reads [Prefs] (`android.net.Uri`) on the hop
 * that would be allowed first, and the JVM harness has no Android runtime.
 */
class NetAllowTest {

    @Test
    fun theFirstHopIsRefusedBeforeAnyConnectionOrPrefsRead() {
        val calls = AtomicInteger()
        val refused = try {
            Net.get(
                prefs = uninitializedPrefs(),
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
    fun thePredicateSeesTheExactUrlThatWouldBeOpened() {
        val seen = ArrayList<String>()
        try {
            Net.get(
                prefs = uninitializedPrefs(),
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

    /**
     * A [Prefs] that is never read: the refusal happens before its first
     * field. `Unsafe` avoids the constructor's Context/SharedPreferences.
     */
    private fun uninitializedPrefs(): Prefs {
        // Reflected, not imported: the Android unit-test compile classpath does
        // not expose sun.misc. The object is never read: the refusal happens
        // before Prefs' first field.
        val unsafeClass = Class.forName("sun.misc.Unsafe")
        val field = unsafeClass.getDeclaredField("theUnsafe")
        field.isAccessible = true
        val unsafe = field.get(null) ?: error("Unsafe unavailable")
        val allocate = unsafeClass.getMethod("allocateInstance", Class::class.java)
        return allocate.invoke(unsafe, Prefs::class.java) as Prefs
    }
}
