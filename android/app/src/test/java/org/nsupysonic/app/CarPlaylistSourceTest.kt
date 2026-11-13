package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.SocketTimeoutException
import javax.net.ssl.SSLHandshakeException

/**
 * The catalogue URL and the pin matching the car settings and the overlay
 * depend on. Both were wrong in ways that looked like "server down": the SPA's
 * `/app` path was glued in front of `/api/me/playlists`, and a pin stored under
 * a Deezer id never matched the list's UUID.
 */
class CarPlaylistSourceTest {

    @Test
    fun thePlaylistEndpointNeverCarriesTheAppPath() {
        assertEquals(
            "https://music.example/api/me/playlists",
            CarPlaylistSource.endpointFor("https://music.example"),
        )
        assertEquals(
            "https://music.example/api/me/playlists",
            CarPlaylistSource.endpointFor("https://music.example/"),
        )
        assertEquals(
            "https://music.example/api/me/playlists",
            CarPlaylistSource.endpointFor("https://music.example/app"),
        )
        assertEquals(
            "https://music.example/api/me/playlists",
            CarPlaylistSource.endpointFor("https://music.example/app/"),
        )
        assertEquals(
            "https://music.example/api/me/playlists",
            CarPlaylistSource.endpointFor("https://music.example/APP/"),
        )
    }

    @Test
    fun aReverseProxiedPrefixIsKept() {
        // Someone running the app under https://host/musique (Prefs.baseUrl
        // carries the path since the port bug fix) still reaches /musique/api.
        assertEquals(
            "https://music.example/musique/api/me/playlists",
            CarPlaylistSource.endpointFor("https://music.example/musique/app/"),
        )
        assertEquals(
            "http://192.168.1.10:5722/api/me/playlists",
            CarPlaylistSource.endpointFor("http://192.168.1.10:5722"),
        )
    }

    @Test
    fun aNonHttpOrHostlessBaseIsNotAnEndpoint() {
        assertNull(CarPlaylistSource.endpointFor(""))
        assertNull(CarPlaylistSource.endpointFor("   "))
        assertNull(CarPlaylistSource.endpointFor("ftp://music.example"))
        assertNull(CarPlaylistSource.endpointFor("music.example"))
        assertNull(CarPlaylistSource.endpointFor("https://"))
        assertNull(CarPlaylistSource.endpointFor("https://music.example:bad"))
    }

    @Test
    fun aPinMatchesTheUuidOrTheDeezerId() {
        val uuid = "0b7d5f3a-9c2e-4b41-8d6f-1a2b3c4d5e6f"
        val items = listOf(
            CarPlaylistSource.Item(uuid, "Voiture", 42, "/api/cover/x", "908622995"),
            CarPlaylistSource.Item(
                "6c9a1e2d-0000-4000-8000-123456789abc",
                "Route",
                7,
                null,
                null,
            ),
        )
        assertEquals("Voiture", CarPlaylistSource.find(items, uuid)?.title)
        assertEquals("Voiture", CarPlaylistSource.find(items, "908622995")?.title)
        // UUIDs compare case-insensitively; a pin copied from a URL may be upper.
        assertEquals("Voiture", CarPlaylistSource.find(items, uuid.uppercase())?.title)
        assertNull(CarPlaylistSource.find(items, "123"))
        assertNull(CarPlaylistSource.find(items, "  "))
    }

    @Test
    fun theCausesAreKeptApart() {
        assertTrue(carListFailureOf(java.io.IOException("HTTP 401")) is CarListFailure.NoSession)
        assertTrue(carListFailureOf(java.io.IOException("HTTP 403")) is CarListFailure.Http)
        assertEquals(403, (carListFailureOf(java.io.IOException("HTTP 403")) as CarListFailure.Http).code)
        assertTrue(carListFailureOf(java.io.IOException("HTTP 502")) is CarListFailure.Http)
        assertTrue(carListFailureOf(SocketTimeoutException("timeout")) is CarListFailure.Timeout)
        assertTrue(carListFailureOf(SSLHandshakeException("bad pin")) is CarListFailure.Tls)
        assertTrue(carListFailureOf(java.io.IOException("connection reset")) is CarListFailure.Offline)
    }

    @Test
    fun anOutcomeNeverThrows() {
        val ok = carListOutcome { listOf(CarPlaylistSource.Item("p", "T", 1)) }
        assertTrue(ok is CarListOutcome.Ok)
        val offline = carListOutcome { throw java.io.IOException("unreachable") }
        assertTrue((offline as CarListOutcome.Fail).failure is CarListFailure.Offline)
        val unexpected = carListOutcome { throw CarListFailure.Unexpected }
        assertTrue((unexpected as CarListOutcome.Fail).failure is CarListFailure.Unexpected)
    }

    @Test
    fun aCoverPathResolvesAgainstTheServerRoot() {
        assertEquals(
            "https://music.example/musique/api/cover/x",
            resolveCoverUrl("https://music.example/musique/app/", "/api/cover/x"),
        )
        assertEquals(
            "https://cdn.example/cover.jpg",
            resolveCoverUrl("https://music.example", "https://cdn.example/cover.jpg"),
        )
        assertEquals("", resolveCoverUrl("https://music.example", ""))
        assertEquals("", resolveCoverUrl("https://music.example", "blob:http://x/y"))
        assertEquals("", resolveCoverUrl("https://music.example", "javascript:alert(1)"))
    }
}

/** The loader's pure pieces: disk naming, bound and in-flight coalescing. */
class CarCoverCacheTest {

    @Test
    fun aUrlBecomesAHashNotAPath() {
        val url = "https://music.example/api/cover/0b7d5f3a-9c2e"
        val key = coverCacheKey(url)
        assertEquals(64, key.length)
        assertTrue(key.all { it in "0123456789abcdef" })
        assertEquals(key, coverCacheKey(url))
        assertFalse(key == coverCacheKey("$url?size=250"))
        // Nothing of the URL (a host, a slash, a query) survives in the name.
        assertFalse(key.contains("music.example"))
        assertFalse(key.contains("/"))
        assertFalse(key.contains("?"))
    }

    @Test
    fun theDiskCacheDropsTheOldestUntilItFits() {
        val threeMb = 3L * 1024 * 1024
        val entries = (0 until 10).map {
            CoverDiskEntry("cover$it.img", threeMb, modified = it.toLong())
        }
        val evicted = coverEvictions(entries, budget = 24L * 1024 * 1024, maxFiles = 300)
        assertEquals(listOf("cover0.img", "cover1.img"), evicted)

        val files = (0 until 300).map { CoverDiskEntry("f$it.img", 1, it.toLong()) }
        val byCount = coverEvictions(files, budget = Long.MAX_VALUE, maxFiles = 250)
        assertEquals(50, byCount.size)
        assertEquals("f0.img", byCount.first())
        assertEquals("f49.img", byCount.last())
    }

    @Test
    fun aCacheThatFitsIsLeftAlone() {
        val entries = listOf(
            CoverDiskEntry("a.img", 1_000, 1),
            CoverDiskEntry("b.img", 1_000, 2),
        )
        assertTrue(coverEvictions(entries, budget = 2_000, maxFiles = 10).isEmpty())
    }

    @Test
    fun oneTransferServesEveryWaiterOfTheSameUrl() {
        val fanout = CoverFanout<String>()
        val seen = mutableListOf<String>()
        val owners = (0 until 10).count { i ->
            fanout.join("same") { seen.add("$i:$it") }
        }
        assertEquals(1, owners)
        val callbacks = fanout.finish("same")
        assertEquals(10, callbacks.size)
        for (callback in callbacks) callback("bitmap")
        assertEquals(10, seen.size)
        assertTrue(seen.all { it.endsWith(":bitmap") })
        assertTrue(fanout.finish("same").isEmpty())
    }

    @Test
    fun differentUrlsAreDifferentTransfers() {
        val fanout = CoverFanout<String>()
        assertEquals(10, (0 until 10).count { fanout.join("url$it") {} })
        for (i in 0 until 10) assertEquals(1, fanout.finish("url$i").size)
    }

    @Test
    fun onlyHttpUrlsAreFetched() {
        assertTrue(coverIsHttp("https://music.example/api/cover/x"))
        assertTrue(coverIsHttp("http://192.168.1.10:5722/api/cover/x"))
        assertFalse(coverIsHttp("blob:http://music.example/x"))
        assertFalse(coverIsHttp("javascript:alert(1)"))
        assertFalse(coverIsHttp("ftp://music.example/x"))
        assertFalse(coverIsHttp("/api/cover/x"))
        assertFalse(coverIsHttp("https://"))
        assertFalse(coverIsHttp("https://music.example/" + "x".repeat(3_000)))
    }
}
