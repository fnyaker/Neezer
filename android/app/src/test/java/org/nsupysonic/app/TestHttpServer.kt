package org.nsupysonic.app

import java.io.BufferedReader
import java.io.InputStreamReader
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Collections
import java.util.concurrent.atomic.AtomicInteger

/**
 * A one-request-at-a-time HTTP/1.1 server on 127.0.0.1 for the JVM tests.
 * It records the `Cookie` header of every request and answers from [handler].
 * Deliberately no `com.sun.net.httpserver` (not on the Android unit-test
 * compile classpath), no Android runtime, no external network.
 */
internal class TestHttpServer(
    private val handler: (path: String, cookie: String?) -> Response,
) {
    data class Response(
        val status: Int,
        val location: String? = null,
        val body: String = "",
    )

    private val socket = ServerSocket(0, 0, InetAddress.getByName("127.0.0.1"))
    private val thread = Thread({ acceptLoop() }, "test-http").apply { isDaemon = true }

    val port: Int get() = socket.localPort
    val hits = AtomicInteger()
    val cookies: MutableList<String?> = Collections.synchronizedList(mutableListOf())

    init {
        thread.start()
    }

    private fun acceptLoop() {
        while (!socket.isClosed) {
            val client = try {
                socket.accept()
            } catch (_: Exception) {
                return
            }
            try {
                client.use { serve(it) }
            } catch (_: Exception) {
            }
        }
    }

    private fun serve(client: Socket) {
        val input = BufferedReader(InputStreamReader(client.getInputStream(), Charsets.US_ASCII))
        val request = input.readLine() ?: return
        val path = request.split(' ').getOrNull(1) ?: "/"
        var cookie: String? = null
        while (true) {
            val line = input.readLine() ?: break
            if (line.isEmpty()) break
            val colon = line.indexOf(':')
            if (colon > 0 && line.substring(0, colon).equals("Cookie", ignoreCase = true)) {
                cookie = line.substring(colon + 1).trim()
            }
        }
        hits.incrementAndGet()
        cookies.add(cookie)
        val response = handler(path, cookie)
        val body = response.body.toByteArray(Charsets.UTF_8)
        val reason = if (response.status == 302) "Found" else "OK"
        val headers = buildString {
            append("HTTP/1.1 ${response.status} $reason\r\n")
            append("Content-Length: ${body.size}\r\n")
            append("Connection: close\r\n")
            response.location?.let { append("Location: $it\r\n") }
            append("\r\n")
        }
        val out = client.getOutputStream()
        out.write(headers.toByteArray(Charsets.US_ASCII))
        out.write(body)
        out.flush()
    }

    fun stop() {
        try {
            socket.close()
        } catch (_: Exception) {
        }
    }
}
