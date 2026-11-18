package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.SocketTimeoutException
import javax.net.ssl.SSLHandshakeException

/**
 * What the car's catalogue keeps from the old app now that the read itself is
 * the core's (MB24's second seam: `NzCore.car_catalogue`).
 *
 * The URL-building tests that used to live here (`endpointFor`,
 * `favoritesEndpointFor`, "the API never sits under `/app`") went WITH the
 * builder: an `nz-api` client is built from an `Origin`, and `Origin::parse`
 * refuses a path, so the property is structural in Rust and is held by
 * `nz-ffi`'s `car::tests::the_catalogue_never_carries_the_app_path` instead —
 * same assertion, on the side that now decides it. What stays here is everything
 * the car itself still owns: the pin matching, the star count, the failure
 * causes and the cover-path resolution.
 */
class CarPlaylistSourceTest {

    @Test
    fun theSnapshotOutcomeCarriesTheStarCount() {
        val ok = carSnapshotOutcome {
            CarPlaylistSource.Snapshot(listOf(CarPlaylistSource.Item("p", "T", 1)), 7)
        }
        assertTrue(ok is CarListOutcome.Ok)
        assertEquals(7, (ok as CarListOutcome.Ok).favorites)
        val unknown = carSnapshotOutcome { CarPlaylistSource.Snapshot(emptyList(), null) }
        assertNull((unknown as CarListOutcome.Ok).favorites)
        val offline = carSnapshotOutcome { throw java.io.IOException("unreachable") }
        assertTrue((offline as CarListOutcome.Fail).failure is CarListFailure.Offline)
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

/**
 * The loader's pure pieces that are still Kotlin's: one transfer per URL, shared
 * by every caller, and the format gate in front of the core. The disk cache and
 * its eviction are `nz-cache`'s now (300 MB, LRU, a hashed file name), so the
 * three tests that held them moved to `nz-ffi`'s own cover tests with the code.
 */
class CarCoverCacheTest {

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
