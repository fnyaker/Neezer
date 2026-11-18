package fr.neezer.mobile.feature.library

import android.app.Application
import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.performScrollToIndex
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import java.io.File
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.AppLink

/**
 * The lot's ONE device test: the real library, the real core, a real
 * `ListHandle` — on the headless emulator (worker rule 7: `-no-window`, never
 * the person's display or speakers; this screen makes no sound).
 *
 * **It brings its own server** (MB7's lesson: a device test that needs a host
 * process is a test that is red for everybody else, and `mobile.yaml`'s emulator
 * job runs every module's `connectedDebugAndroidTest`). [Bench] is a loopback
 * server inside the app process: the session, the playlist tree and 8 000
 * favourites.
 *
 * What only the device can prove, and this test asserts:
 *
 * 1. **8 000 favourites open on the device in ONE request** and paint from a real
 *    `ListHandle` through the designsystem's adapter (the `StackOverflowError`
 *    of `asRowSource` that every JVM test passed straight through, MB6/PR #321);
 * 2. **the list is WINDOWED**: a scroll to the last row reaches it, so the pages
 *    really come from the handle and not from a list copied into Kotlin.
 *
 * The screenshot lands in the app's own `filesDir`; pull it with
 * `adb shell run-as fr.neezer.mobile.feature.library.test cat files/<name>`.
 */
@RunWith(AndroidJUnit4::class)
class LibraryDeviceTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** A navigator that records where a tap wanted to go (no shell here). */
    private class RecordingNav : NzNavigator {
        val visited = mutableListOf<AppLink>()

        override fun go(link: AppLink) {
            visited += link
        }

        override fun back() = Unit

        override fun openPlayer() = Unit

        override fun openQueue() = Unit
    }

    @Test
    fun eightThousandFavoritesOpenInOneRequestAndScrollOnTheDevice() {
        val bench = Bench.start()
        val core = install(bench)
        val nav = RecordingNav()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                // The shell pads the screen; the entry is composed alone here.
                Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
                    LibraryEntry(tab = "favorites", nav = nav)
                }
            }
        }

        // The screen paints…
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithContentDescription(LIBRARY_MARKER).fetchSemanticsNodes().isNotEmpty()
        }
        // …and so does its first rows, from the handle.
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Ouverture 0").fetchSemanticsNodes().isNotEmpty()
        }
        val stats = core.stats().filter { it.tag == "library" }
        println("MEASURE library device: routes=${bench.routes} stats=$stats")
        assertTrue("the list asked its route (${bench.routes})", bench.routes.any { it == "GET /api/me/favorites" })
        assertEquals(
            "8 000 favourites are ONE request",
            1uL,
            stats.fold(0uL) { total, row -> total + row.requests },
        )

        // The last row is reachable: the pages really come from the handle.
        rule.onNode(hasScrollAction(), useUnmergedTree = false).performScrollToIndex(Bench.ROWS - 1)
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Sinnerman ${Bench.ROWS - 1}").fetchSemanticsNodes().isNotEmpty()
        }
        println("MEASURE library device scrolled to row ${Bench.ROWS - 1}")

        settle()
        capture("mb8-device-library.png")
        bench.close()
    }

    /** The core, over the bench, once per test PROCESS (a phone installs one). */
    private fun install(bench: Bench): uniffi.nz_ffi.NzCore {
        val app = ApplicationProvider.getApplicationContext<Application>()
        // `NzBridge.install` is a main-thread call (ProcessLifecycleOwner refuses
        // another thread), and ONE core per process: installing twice leaves the
        // previous core's watchers alive (MB7 met this).
        if (NzBridge.core == null) {
            rule.runOnUiThread { NzBridge.install(app) }
        }
        val core = NzBridge.core ?: error("the core could not be built: ${NzBridge.failure}")
        runBlocking {
            core.setServer(bench.base())
            core.login("bench", "Bench1")
        }
        return core
    }

    /**
     * Waits for the SURFACE, not only the semantics tree (MB7's lesson: the tree
     * runs ahead of the compositor, so a capture taken too early shows skeletons).
     */
    private fun settle() {
        rule.waitForIdle()
        val frame = java.util.concurrent.CountDownLatch(1)
        rule.runOnUiThread {
            android.view.Choreographer.getInstance().postFrameCallback { frame.countDown() }
        }
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        frame.await(2, java.util.concurrent.TimeUnit.SECONDS)
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        Thread.sleep(150)
    }

    /** The whole screen as the device shows it, in the app's own files directory. */
    private fun capture(name: String) {
        val screenshot: Bitmap = InstrumentationRegistry.getInstrumentation()
            .uiAutomation
            .takeScreenshot() ?: error("the device refused a screenshot")
        val file = File(rule.activity.filesDir, name)
        file.outputStream().use { screenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${screenshot.width}x${screenshot.height} ${file.absolutePath} ${file.length()} bytes")
    }

    companion object {
        const val TIMEOUT_MS = 30_000L
    }

    /**
     * A loopback server inside the app process: the session, the playlist tree and
     * 8 000 favourites.
     *
     * A thread per connection, like the Rust fake: the screen fires its reads
     * together, and a single-threaded accept loop would deadlock the TEST instead
     * of the code.
     */
    class Bench private constructor(private val socket: ServerSocket) : AutoCloseable {
        /** `http://127.0.0.1:<port>` — what `setServer` is given. */
        fun base(): String = "http://127.0.0.1:${socket.localPort}"

        val routes = CopyOnWriteArrayList<String>()

        @Volatile
        private var running = true

        private val thread = thread(isDaemon = true, name = "mb8-bench") {
            while (running && !socket.isClosed) {
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
                var length = 0
                var line = input.readLine()
                while (line != null && line.isNotEmpty()) {
                    if (line.lowercase().startsWith("content-length:")) {
                        length = line.substringAfter(':').trim().toIntOrNull() ?: 0
                    }
                    line = input.readLine()
                }
                if (length > 0) {
                    val body = CharArray(length)
                    var read = 0
                    while (read < length) {
                        val n = input.read(body, read, length - read)
                        if (n <= 0) break
                        read += n
                    }
                }
                val parts = request.split(' ')
                val method = parts.getOrNull(0).orEmpty()
                val path = parts.getOrNull(1).orEmpty().substringBefore('?')
                routes += "$method $path"
                val body = answer(method, path) ?: errorBody()
                val head = "HTTP/1.1 ${body.first} X\r\nContent-Type: application/json\r\n" +
                    "Content-Length: ${body.second.toByteArray().size}\r\nConnection: close\r\n\r\n"
                it.getOutputStream().apply {
                    write(head.toByteArray())
                    write(body.second.toByteArray())
                    flush()
                }
            }
        }

        private fun answer(method: String, path: String): Pair<Int, String>? = when {
            method == "GET" && path == "/api/version" -> 200 to """{"version":"9.9.9","build":"bench"}"""
            method == "POST" && path == "/api/login" ->
                200 to """{"user":{"id":"u-bench","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"""
            method == "GET" && path == "/api/me" ->
                200 to """{"user":{"id":"u-bench","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"""
            method == "GET" && path == "/api/me/playlists" -> 200 to playlists()
            method == "GET" && path == "/api/me/favorites" -> 200 to favorites()
            method == "GET" && path == "/api/me/favorite-ids" -> 200 to """{"ids":[],"albums":[],"artists":[],"playlists":[],"disliked":[]}"""
            method == "OPTIONS" && path == "/api/playlist-folders" -> 200 to ""
            method == "GET" && path == "/api/me/artists" -> 200 to """{"artists":[]}"""
            method == "GET" && path == "/api/me/albums" -> 200 to """{"albums":[]}"""
            method == "GET" && path == "/api/me/local" -> 200 to """{"tracks":[]}"""
            method == "GET" && path == "/api/me/dislikes" -> 200 to """{"tracks":[]}"""
            method == "GET" && path == "/api/unavailable" -> 200 to """{"tracks":[]}"""
            method == "GET" && path == "/api/upload/usage" -> 200 to """{"used":0,"quota":0,"unlimited":true}"""
            method == "GET" && path == "/api/history" ->
                200 to """{"events":[],"next":null,"summary":{"days":30,"listens":0,"skips":0,"minutes":0,"top_artists":[],"top_tracks":[]}}"""
            else -> null
        }

        private fun errorBody(): Pair<Int, String> = 404 to """{"error":"not found"}"""

        private fun playlists(): String = """
            {"playlists":[
              {"id":"aaaaaaaa-1111-4111-8111-000000000001","title":"Route","cover":null,
               "nb_tracks":12,"role":"owner","owner":"Nadia","editable":true,"can_manage":true,
               "folder_id":null}],
             "folders":[{"id":"ffffffff-1111-4111-8111-000000000001","title":"Voyage","position":0,"nb_playlists":0}]}
        """.trimIndent()

        /** The 8 000 rows the acceptance names: one answer, one request. */
        private fun favorites(): String {
            val out = StringBuilder(ROWS * 190 + 32)
            out.append("""{"tracks":[""")
            for (index in 0 until ROWS) {
                if (index > 0) out.append(',')
                val title = if (index % 2 == 0) "Ouverture $index" else "Sinnerman $index"
                out.append(
                    """{"id":"11111111-1111-4111-8111-${"%012x".format(index)}","deezer_id":"${9_000_000 + index}",
                       "title":"$title","duration":${150 + index % 180},
                       "artist":{"deezer_id":"42","name":"Ascenseur"},
                       "album":{"deezer_id":"501","title":"Le matin","cover":null}}""",
                )
            }
            out.append("]}")
            return out.toString()
        }

        override fun close() {
            running = false
            runCatching { socket.close() }
        }

        companion object {
            /** The worst realistic library the plan names. */
            const val ROWS = 8_000

            fun start(): Bench {
                val socket = ServerSocket(0, 50, java.net.InetAddress.getByName("127.0.0.1"))
                return Bench(socket)
            }
        }
    }
}
