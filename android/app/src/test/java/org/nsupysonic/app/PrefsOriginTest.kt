package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.URL

/**
 * The origin rule the native fetches and the navigation guard share. The base
 * is canonicalised once (`scheme://host:port`, default port spelled out); the
 * cases below are the ones that used to differ between `android.net.Uri` and
 * `java.net.URL`: default ports, case, IPv6 brackets, subdomains and userinfo.
 */
class PrefsOriginTest {

    private val base = "https://music.example:443"

    /** What `Net` does: parse once with URL, then compare the parts. */
    private fun originOf(url: String): String? {
        val parsed = URL(url)
        return Prefs.originOf(parsed.protocol, parsed.host, parsed.port)
    }

    @Test
    fun theSameOriginWithOrWithoutItsDefaultPort() {
        assertEquals(base, originOf("https://music.example/api/cover/x"))
        assertEquals(base, originOf("https://music.example:443/api/cover/x"))
        assertEquals(base, Prefs.originOf("https://music.example"))
        assertEquals(base, Prefs.originOf("https://music.example:443"))
        assertEquals("http://music.example:80", originOf("http://music.example/x"))
        assertEquals("http://music.example:80", originOf("http://music.example:80/x"))
    }

    @Test
    fun anExplicitDifferentPortIsAnotherOrigin() {
        assertNotEquals(base, originOf("https://music.example:444/api/cover/x"))
        assertNotEquals(base, originOf("http://music.example:443/api/cover/x"))
    }

    @Test
    fun theHostCaseDoesNotChangeTheOrigin() {
        assertEquals(base, originOf("https://MUSIC.EXAMPLE/api/cover/x"))
        assertEquals(base, originOf("https://Music.Example:443/api/cover/x"))
        assertEquals(base, Prefs.originOf("HTTPS", "MUSIC.EXAMPLE", -1))
    }

    @Test
    fun aSubdomainIsAnotherOrigin() {
        assertNotEquals(base, originOf("https://a.music.example/api/cover/x"))
        assertNotEquals(base, originOf("https://music.example.evil.com/api/cover/x"))
        assertEquals("https://a.music.example:443", originOf("https://a.music.example/x"))
    }

    @Test
    fun userinfoIsNotPartOfTheOrigin() {
        // The connection still goes to the host after the '@'; the origin is
        // that host, exactly as URL reports it.
        assertEquals(base, originOf("https://user@music.example/x"))
        assertEquals(base, originOf("https://user:pass@music.example:443/x"))
        assertNotEquals(base, originOf("https://music.example@evil.example/x"))
        assertNotEquals(base, originOf("https://user@evil.example/x"))
    }

    @Test
    fun ipv6BracketsAndPortsAreNormalised() {
        assertEquals("https://[::1]:443", originOf("https://[::1]/x"))
        assertEquals("https://[::1]:8443", originOf("https://[::1]:8443/x"))
        assertEquals("http://[::1]:80", originOf("http://[::1]/x"))
        assertNotEquals("https://[::1]:443", originOf("https://[::2]/x"))
    }

    @Test
    fun aNonHttpOrMalformedUrlHasNoOrigin() {
        assertNull(Prefs.originOf(""))
        assertNull(Prefs.originOf(null))
        assertNull(Prefs.originOf("ftp://music.example/x"))
        assertNull(Prefs.originOf("file:///etc/passwd"))
        assertNull(Prefs.originOf("blob:https://music.example/x"))
        assertNull(Prefs.originOf("javascript:alert(1)"))
        assertNull(Prefs.originOf("https://music.example:bad/x"))
        assertNull(Prefs.originOf("https:///no-host"))
        assertNull(Prefs.originOf("https", "", -1))
        assertNull(Prefs.originOf("https", null, -1))
    }

    @Test
    fun thePartsAndTheStringAgree() {
        for (url in listOf(
            "https://music.example/x",
            "https://music.example:443/x",
            "https://music.example:444/x",
            "https://[::1]:8443/x",
            "https://user@music.example/x",
        )) {
            val parsed = URL(url)
            assertEquals(Prefs.originOf(url), Prefs.originOf(parsed.protocol, parsed.host, parsed.port))
        }
    }

    @Test
    fun theServerDecisionUsesTheSamePartsAsTheConnection() {
        val prefs = testPrefs("host" to "https://music.example", "verifySsl" to false)
        assertTrue(prefs.isServerOrigin("https", "music.example", -1))
        assertTrue(prefs.isServerOrigin("https", "MUSIC.EXAMPLE", 443))
        assertFalse(prefs.isServerOrigin("https", "music.example", 444))
        assertFalse(prefs.isServerOrigin("http", "music.example", -1))
        assertFalse(prefs.isServerOrigin("https", "a.music.example", -1))
        assertFalse(prefs.isServerOrigin("https", null, -1))
        assertFalse(prefs.isServerOrigin("ftp", "music.example", -1))
    }
}
