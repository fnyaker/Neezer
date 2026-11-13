package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test
import java.io.IOException

/**
 * The single-parse boundary: the same [java.net.URL] that decides "ours"
 * (cookie, pin, cleartext) is the one opened, and a redirect is parsed and
 * judged again. A loopback server observes the Cookie header and the hits; no
 * external network, no Android runtime.
 */
class NetOriginTest {

    private fun ok() = TestHttpServer.Response(200, body = "ok")

    @Test
    fun theConfiguredOriginIsOursAndGetsTheCookie() {
        val server = TestHttpServer { _, _ -> ok() }
        try {
            val prefs = testPrefs("host" to "http://127.0.0.1:${server.port}", "verifySsl" to false)
            val conn = Net.get(
                prefs, "http://127.0.0.1:${server.port}/cover", "session=abc", 2_000, 2_000,
            )
            conn.inputStream.use { it.readBytes() }
            conn.disconnect()
            assertEquals("session=abc", server.cookies.single())
            assertEquals(1, server.hits.get())
        } finally {
            server.stop()
        }
    }

    @Test
    fun anotherPortIsNotOurServerAndIsRefusedBeforeAConnection() {
        val ours = TestHttpServer { _, _ -> ok() }
        val other = TestHttpServer { _, _ -> ok() }
        try {
            val prefs = testPrefs(
                "host" to "http://127.0.0.1:${ours.port}",
                "verifySsl" to false,
            )
            val error = assertThrows(IOException::class.java) {
                Net.get(
                    prefs, "http://127.0.0.1:${other.port}/cover", "session=abc",
                    1_000, 1_000,
                )
            }
            assertEquals("cleartext refused off the server", error.message)
            assertEquals(0, other.hits.get())
        } finally {
            ours.stop()
            other.stop()
        }
    }

    @Test
    fun aRedirectOnTheServerKeepsTheCookie() {
        val server = TestHttpServer { path, _ ->
            if (path == "/redirect") {
                TestHttpServer.Response(302, location = "/next")
            } else {
                ok()
            }
        }
        try {
            val prefs = testPrefs("host" to "http://127.0.0.1:${server.port}", "verifySsl" to false)
            val conn = Net.get(
                prefs, "http://127.0.0.1:${server.port}/redirect", "session=abc", 2_000, 2_000,
            )
            conn.inputStream.use { it.readBytes() }
            conn.disconnect()
            assertEquals(listOf("session=abc", "session=abc"), server.cookies)
        } finally {
            server.stop()
        }
    }

    @Test
    fun aRedirectToAnotherHostIsRefusedAndNeverSeesTheCookie() {
        val other = TestHttpServer { _, _ -> ok() }
        val ours = TestHttpServer { _, _ ->
            TestHttpServer.Response(
                302,
                location = "http://127.0.0.1:${other.port}/stolen",
            )
        }
        try {
            val prefs = testPrefs(
                "host" to "http://127.0.0.1:${ours.port}",
                "verifySsl" to false,
            )
            val error = assertThrows(IOException::class.java) {
                Net.get(
                    prefs, "http://127.0.0.1:${ours.port}/redirect", "session=abc",
                    1_000, 1_000,
                )
            }
            assertEquals("cleartext refused off the server", error.message)
            assertEquals("session=abc", ours.cookies.single())
            assertEquals(0, other.hits.get())
        } finally {
            ours.stop()
            other.stop()
        }
    }
}
