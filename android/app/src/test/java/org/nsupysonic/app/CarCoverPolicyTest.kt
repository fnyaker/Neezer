package org.nsupysonic.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The cover host policy: a URL named by the server may only make the device
 * talk to the configured origin or Deezer's image CDN, over HTTPS, with no
 * userinfo and no private/loopback/link-local IP literal. This is the rule
 * `CarCovers` hands to [Net] for the first hop and for every redirect.
 */
class CarCoverPolicyTest {

    private val origin = "https://music.example"

    /** A stand-in for Prefs.isServerUrl: exact origin, path ignored. */
    private val sameOrigin: (String) -> Boolean = { candidate ->
        candidate.startsWith("$origin/") || candidate == origin
    }

    private fun allowed(url: String): Boolean = coverAllowed(url, sameOrigin)

    @Test
    fun theConfiguredOriginIsAllowed() {
        assertTrue(allowed("$origin/api/cover/abc"))
        assertTrue(allowed("$origin/api/cover/abc?size=250"))
        assertTrue(allowed("$origin/app/api/cover/abc"))
    }

    @Test
    fun aConfiguredCleartextServerIsAllowedByItsOwnOriginRule() {
        // A home server may be http://192.168.1.10:5722. The predicate says
        // "same origin" and the device's Prefs.isServerUrl owns that answer;
        // what must never pass is another host over cleartext.
        assertTrue(coverAllowed("http://192.168.1.10:5722/api/cover/x") {
            it.startsWith("http://192.168.1.10:5722/")
        })
        assertFalse(allowed("http://192.168.1.10:5722/api/cover/x"))
    }

    @Test
    fun deezerCdnOverHttpsIsAllowed() {
        assertTrue(allowed("https://e-cdns-images.dzcdn.net/images/cover/x/500x500.jpg"))
        assertTrue(allowed("https://cdns-images.dzcdn.net/images/cover/x/1000x1000.jpg"))
        assertTrue(allowed("https://another-edge.dzcdn.net/x"))
        // Canonical case/trailing dot are the same DNS name.
        assertTrue(allowed("https://E-CDNS-IMAGES.DZCDN.NET/x"))
        assertTrue(allowed("https://e-cdns-images.dzcdn.net./x"))
    }

    @Test
    fun anyOtherHostIsRefused() {
        assertFalse(allowed("https://evil.example/x"))
        assertFalse(allowed("https://music.example.evil.com/x"))
        assertFalse(allowed("https://e-cdns-images.dzcdn.net.evil.com/x"))
        assertFalse(allowed("https://dzcdn.net/x")) // apex, not an image host
        assertFalse(allowed("https://evildzcdn.net/x"))
        assertFalse(allowed("https://evil.example/?u=https://e-cdns-images.dzcdn.net/x"))
        assertFalse(allowed("https://8.8.8.8/x")) // public, but not allowed
    }

    @Test
    fun cleartextOffTheConfiguredOriginIsRefused() {
        assertFalse(allowed("http://e-cdns-images.dzcdn.net/images/cover/x.jpg"))
        assertFalse(allowed("http://cdns-images.dzcdn.net/x"))
    }

    @Test
    fun userinfoIsRefusedEvenOnOurOwnHost() {
        assertFalse(allowed("https://notre-hote@evil.example/"))
        assertFalse(allowed("https://user:pass@music.example/api/cover/x"))
        // The same-origin test must not be reached first: a lambda that says
        // "yes" to everything still cannot smuggle a userinfo URL through.
        assertFalse(coverAllowed("https://user@music.example/x") { true })
    }

    @Test
    fun privateLoopbackAndLinkLocalLiteralsAreRefused() {
        val literals = listOf(
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.1.1",
            "169.254.1.1",
            "0.0.0.0",
            "100.64.0.1", // CGNAT
        )
        for (host in literals) {
            assertFalse("http://$host/cover.jpg must be refused", allowed("http://$host/cover.jpg"))
            assertFalse("https://$host/cover.jpg must be refused", allowed("https://$host/cover.jpg"))
        }
        assertFalse(allowed("https://[::1]/cover.jpg"))
        assertFalse(allowed("https://[fc00::1]/cover.jpg"))
        assertFalse(allowed("https://[fd12:3456::1]/cover.jpg"))
        assertFalse(allowed("https://[fe80::1]/cover.jpg"))
        assertFalse(allowed("https://[::ffff:127.0.0.1]/cover.jpg"))
        assertFalse(allowed("https://[::ffff:192.168.1.1]/cover.jpg"))
        assertFalse(allowed("https://localhost/cover.jpg"))
        assertFalse(allowed("https://player.localhost/cover.jpg"))
    }

    @Test
    fun aUrlThatIsNotAnHttpPageIsRefused() {
        assertFalse(allowed(""))
        assertFalse(allowed("/api/cover/x"))
        assertFalse(allowed("file:///etc/passwd"))
        assertFalse(allowed("ftp://e-cdns-images.dzcdn.net/x"))
        assertFalse(allowed("javascript:alert(1)"))
        assertFalse(allowed("https://"))
    }

    @Test
    fun aDifferentPortOrSchemeIsNotTheConfiguredOrigin() {
        assertFalse(allowed("https://music.example:444/api/cover/x"))
        assertFalse(allowed("http://music.example/api/cover/x"))
        assertFalse(allowed("https://music.example.evil.com/"))
    }

    @Test
    fun theRemoteRuleStandsAlone() {
        assertTrue(coverAllowedRemote("https://e-cdns-images.dzcdn.net/x"))
        assertFalse(coverAllowedRemote("$origin/api/cover/x"))
        assertFalse(coverAllowedRemote("https://127.0.0.1/x"))
        assertFalse(coverAllowedRemote("http://e-cdns-images.dzcdn.net/x"))
    }
}
