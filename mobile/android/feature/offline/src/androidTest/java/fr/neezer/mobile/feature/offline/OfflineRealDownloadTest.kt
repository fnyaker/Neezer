package fr.neezer.mobile.feature.offline

import android.app.Application
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.NzBridge
import java.io.OutputStream
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.OfflineListener
import uniffi.nz_ffi.OfflineState
import uniffi.nz_ffi.offlineAttach

/**
 * **A real download, on the device, from the tap to the file** — the proof the
 * bind's fix exists for.
 *
 * The bug this test would have caught: `offline_attach` was exported WITHOUT
 * `async_runtime = "tokio"`, so on a phone it answered
 * `InternalException: there is no reactor running`; `OfflineCore.api()` swallowed
 * it, and every download button did nothing while every screen said
 * « les téléchargements ne sont pas reliés au noyau ». No JVM test could see it —
 * the host suites run inside their own runtime.
 *
 * What it does: a loopback bench in the app's own process serves the session and a
 * 10 s FLAC (MB7's fixture), the service is bound through the REAL `offlineAttach`,
 * one title is queued through the REAL `addSet`, and the test waits for the
 * worker's OWN event before asserting that the store knows the file and that its
 * bytes are on disk.
 */
@RunWith(AndroidJUnit4::class)
class OfflineRealDownloadTest {

    @Test
    fun aDownloadIsCreatedAndLandsOnDisk() {
        val bench = Bench.start()
        val app = ApplicationProvider.getApplicationContext<Application>()
        // `NzBridge.install` is a main-thread call (ProcessLifecycleOwner refuses
        // another thread): the acceptance's own run does it once per process.
        if (NzBridge.core == null) {
            InstrumentationRegistry.getInstrumentation().runOnMainSync {
                NzBridge.install(app)
            }
        }
        val core = NzBridge.core ?: error("the core could not be built: ${NzBridge.failure}")
        runBlocking {
            core.setServer(bench.base())
            core.login("bench", "Bench1")
            // THE bind, through the FFI the fix repaired. A failure here is the
            // regression this test exists for.
            offlineAttach(core)
        }
        val offline = core.offline()

        val done = CountDownLatch(1)
        offline.subscribe(object : OfflineListener {
            override fun onProgress(state: OfflineState) {
                if (state.sets.any { it.done > 0u && it.done >= it.total && !it.running }) {
                    done.countDown()
                }
            }

            override fun onFailure(key: String, cause: String) {
                println("MEASURE offline failure: $key — $cause")
            }
        })

        val progress = runBlocking {
            offline.addSet(
                kind = "track",
                id = KEY,
                quality = "flac",
                title = "Titre",
                artist = "Artiste",
            )
        }
        println("MEASURE offline add: context=${progress.context} total=${progress.total}")
        assertTrue("the download finished (the worker's own event)", done.await(120, TimeUnit.SECONDS))
        assertTrue("the store knows the title", offline.isDownloaded(KEY))
        val entry = offline.downloadFacts(listOf(KEY)).firstOrNull()
        println("MEASURE offline facts: ${entry?.bytes} bytes, ${entry?.quality}")
        assertTrue("its bytes are on disk: ${entry?.bytes}", (entry?.bytes ?: 0uL) > 0uL)
        assertEquals(
            "the stream was fetched from the bench",
            listOf("GET /api/stream/$KEY"),
            bench.routes.filter { it.startsWith("GET /api/stream/$KEY") }.distinct(),
        )
        bench.close()
    }

    /**
     * The bench: the session, one title's stream and its `ready` route. A thread
     * per connection, so the worker's two requests (the stream and its size) never
     * wait on each other.
     */
    class Bench private constructor(private val socket: ServerSocket) : AutoCloseable {
        fun base(): String = "http://127.0.0.1:${socket.localPort}"

        val routes = CopyOnWriteArrayList<String>()

        private val tone: ByteArray = InstrumentationRegistry.getInstrumentation().context.assets
            .open("tone.flac").use { it.readBytes() }

        private val thread = thread(isDaemon = true, name = "mb15-bench") {
            while (!socket.isClosed) {
                val stream = try {
                    socket.accept()
                } catch (_: Exception) {
                    break
                }
                thread(isDaemon = true) { serve(stream) }
            }
        }

        private fun serve(stream: Socket) {
            stream.use {
                val input = it.getInputStream().bufferedReader()
                val request = input.readLine() ?: return
                var line = input.readLine()
                while (line != null && line.isNotEmpty()) line = input.readLine()
                val parts = request.split(' ')
                val path = parts.getOrNull(1).orEmpty().substringBefore('?')
                routes += "${parts.getOrNull(0).orEmpty()} $path"
                val audio = path == "/api/stream/$KEY"
                val body = when {
                    path == "/api/version" -> """{"version":"9.9.9","build":"bench"}""".toByteArray()
                    path == "/api/login" || path == "/api/me" -> ME.toByteArray()
                    path == "/api/stream/$KEY/ready" ->
                        """{"ready":true,"ranges":"bytes","bytes":${tone.size},"cached":false,"offset":true,"streaming":false}"""
                            .toByteArray()
                    audio -> tone
                    else -> """{"error":"not found"}""".toByteArray()
                }
                val head = buildString {
                    append("HTTP/1.1 ${if (audio || body.isNotEmpty()) 200 else 404} X\r\n")
                    append("Content-Type: ${if (audio) "audio/flac" else "application/json"}\r\n")
                    append("Content-Length: ${body.size}\r\n")
                    if (audio) append("Accept-Ranges: bytes\r\n")
                    if (path == "/api/login") append("Set-Cookie: session=good; Path=/\r\n")
                    append("Connection: close\r\n\r\n")
                }
                val out: OutputStream = it.getOutputStream()
                out.write(head.toByteArray())
                out.write(body)
                out.flush()
            }
        }

        override fun close() {
            try {
                socket.close()
            } catch (_: Exception) {
                // a closed listener is the point
            }
        }

        companion object {
            fun start(): Bench = Bench(ServerSocket(0, 50, java.net.InetAddress.getByName("127.0.0.1")))

            private const val ME =
                """{"user":{"id":"u-mb15","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"""
        }
    }

    companion object {
        private const val KEY = "9001"
    }
}
