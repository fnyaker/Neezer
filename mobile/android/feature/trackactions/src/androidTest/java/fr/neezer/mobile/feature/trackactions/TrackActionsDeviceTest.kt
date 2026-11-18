package fr.neezer.mobile.feature.trackactions

import android.app.Application
import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import java.io.OutputStream
import java.net.InetAddress
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
import uniffi.nz_ffi.ActionsListener
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.NzCore
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.subscribeTrackActions

/**
 * The lot's device test: the real menu, the real core and the real network path
 * on the headless emulator (worker rule 7: `-no-window -no-audio`, never the
 * person's display or speakers).
 *
 * **It brings its own server.** A device test that only runs on one machine is a
 * test that is red for everybody else (MB7's own lesson: CI's emulator job runs
 * every module's `connectedDebugAndroidTest` and passed it no arguments), so
 * [Bench] is a loopback server inside the app process: the session, the picker's
 * playlists, the favourite and the replacement's candidates.
 *
 * It proves what a JVM test cannot:
 *
 * 1. **the menu is the core's** — the sheet that opens on the device carries the
 *    SPA's labels, because Rust built them (`Screens::track_menu`);
 * 2. **a like lands on EVERY open list in the same frame** — two `ListHandle`s
 *    are on screen, the tap goes through `like()`, the core patches both rows and
 *    the listener re-windows them, and the capture shows the two hearts filled
 *    beside the open menu's own "Retirer des favoris": what the acceptance asks
 *    for, in one image;
 * 3. **the picker's list is one request** and its target is writable.
 *
 * The screenshot lands in the app's own `filesDir`; pull it with
 * `adb -s emulator-<port> shell run-as fr.neezer.mobile.feature.trackactions.test cat files/<name>`.
 */
@RunWith(AndroidJUnit4::class)
class TrackActionsDeviceTest {

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

    /**
     * What a screen does when the core says a row changed: re-window.
     *
     * The hop to the main thread is the POINT: `on_rows_changed` is called on
     * nz-api's worker (the core never calls a listener from a lock), and a Compose
     * state written from there did NOT schedule a recomposition on the emulator —
     * measured here, where the menus and the rows kept their old glyphs while the
     * handles already said `liked = true`. A list that subscribes posts first.
     */
    private class CountingListener : ActionsListener {
        private val main = android.os.Handler(android.os.Looper.getMainLooper())

        var version by mutableStateOf(0uL)
            private set

        override fun onRowsChanged(keys: List<String>, version: ULong) {
            main.post { this.version = version }
        }
    }

    @Test
    fun aLikeLandsOnEveryOpenListBesideTheOpenMenu() {
        val bench = Bench.start()
        val core = install(bench)
        val nav = RecordingNav()
        val listener = CountingListener()
        subscribeTrackActions(listener)

        val rows = listOf(
            row("Ouverture", "9001"),
            row("Suite", "9002"),
        )
        // TWO open lists, the same two titles: a playlist and the queue, as far as
        // the core is concerned (`ListHandle`s registered in the same registry).
        val playlist: ListHandle = core.openTrackList(rows)
        val queue: ListHandle = core.openTrackList(rows)

        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Column(
                    Modifier
                        .fillMaxSize()
                        .background(Sillon.colors.bg)
                        .statusBarsPadding()
                        .navigationBarsPadding(),
                ) {
                    TwoLists(listener = listener, playlist = playlist, queue = queue)
                }
                TrackActionsHost()
            }
        }

        // The menu opens on the device: its marker is in the tree, then the SPA's
        // own first entry.
        rule.runOnUiThread { TrackActions.open(rows[0], nav) }
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Lire ensuite").fetchSemanticsNodes().isNotEmpty()
        }
        assertEquals("the menu carries the SPA's own entries", true, menuHasAll())
        // The credits sheet's door is the SHELL's (this module cannot depend on
        // MB7's: entity already depends on this one). No door, no entry — a button
        // that opens nothing is a bug (rule 5).
        assertTrue(
            "no credits entry without its door",
            rule.onAllNodesWithText("Voir les crédits").fetchSemanticsNodes().isEmpty(),
        )

        // The like: the entry's own label is the row's state, flipped by the core.
        rule.onNodeWithText("Ajouter aux favoris").performClick()
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            bench.routes.contains("POST /api/favorite")
        }
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) { listener.version > 0uL }
        settle()
        println(
            "MEASURE like device: routes=${bench.routes} version=${listener.version} " +
                "playlist.liked=${playlist.window(0u, 2u)[0].liked} " +
                "queue.liked=${queue.window(0u, 2u)[0].liked}",
        )
        // The SAME frame, both lists: the core patched every open handle.
        assertTrue("the playlist's row flipped", playlist.window(0u, 2u)[0].liked)
        assertTrue("the queue's row flipped in the same frame", queue.window(0u, 2u)[0].liked)
        // And the menu, reopened, says so.
        rule.runOnUiThread { TrackActions.open(playlist.window(0u, 1u)[0], nav) }
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Retirer des favoris").fetchSemanticsNodes().isNotEmpty()
        }
        settle()
        capture("mb10-device-like-every-list.png")
        bench.close()
    }

    @Test
    fun thePickerAddsToAPlaylistOnTheDevice() {
        val bench = Bench.start()
        val core = install(bench)
        val nav = RecordingNav()
        val listener = CountingListener()
        subscribeTrackActions(listener)
        val rows = listOf(row("Ouverture", "9001"))
        val playlist: ListHandle = core.openTrackList(rows)
        val queue: ListHandle = core.openTrackList(rows)

        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Column(
                    Modifier
                        .fillMaxSize()
                        .background(Sillon.colors.bg)
                        .statusBarsPadding()
                        .navigationBarsPadding(),
                ) {
                    TwoLists(listener = listener, playlist = playlist, queue = queue)
                }
                TrackActionsHost()
            }
        }

        rule.runOnUiThread { TrackActions.open(rows[0], nav) }
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Ajouter à une playlist…").fetchSemanticsNodes().isNotEmpty()
        }
        rule.onNodeWithText("Ajouter à une playlist…").performClick()
        // The picker's marker, then the writable list the bench answered.
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Route du soir").fetchSemanticsNodes().isNotEmpty()
        }
        println("MEASURE picker device: routes=${bench.routes}")
        assertTrue(
            "the picker asked its list once (${bench.routes})",
            bench.routes.count { it == "GET /api/me/playlists" } == 1,
        )
        // The capture while the picker is OPEN: what a person sees when they are
        // choosing, with an unwritable list absent and the writable one there.
        settle()
        capture("mb10-device-picker.png")

        rule.onNodeWithText("Route du soir").performClick()
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            bench.routes.any { it.startsWith("POST /api/playlist/") }
        }
        println("MEASURE picker add: routes=${bench.routes}")
        assertTrue(
            "the add is ONE append",
            bench.routes.count { it.startsWith("POST /api/playlist/") } == 1,
        )
        bench.close()
    }

    /** The two lists, each with its first row's heart: what a screen would draw. */
    @Composable
    private fun TwoLists(listener: CountingListener, playlist: ListHandle, queue: ListHandle) {
        // Reading the listener's version is what re-windows the rows: the core's
        // patch already happened, this is the screen noticing it.
        Column(verticalArrangement = Arrangement.spacedBy(Sillon.space)) {
            // The VERSION is a parameter, not a read the parent does on the
            // section's behalf: a section whose parameters are unchanged is
            // SKIPPED by Compose, and the rows kept their old glyphs while the
            // handle already said `liked = true` (measured on this emulator).
            ListSection("Liste de lecture", playlist, listener.version)
            ListSection("File d'attente", queue, listener.version)
        }
    }

    @Composable
    private fun ListSection(title: String, handle: ListHandle, version: ULong) {
        Column(modifier = Modifier.fillMaxWidth().padding(Sillon.space)) {
            Text(text = title, style = Sillon.type.labelMono, color = Sillon.colors.dim)
            @Suppress("UNUSED_EXPRESSION")
            version
            val page = handle.window(0u, 2u)
            page.forEach { row ->
                Row(
                    modifier = Modifier.fillMaxWidth().padding(vertical = Sillon.gap),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(Sillon.gap),
                ) {
                    Text(text = row.title, style = Sillon.type.body, color = Sillon.colors.tx)
                    Icon(
                        glyph = if (row.liked) NzGlyph.HeartFilled else NzGlyph.Heart,
                        tint = if (row.liked) Sillon.colors.hrt else Sillon.colors.dim,
                        contentDescription = if (row.liked) "Aimé" else "Pas aimé",
                    )
                }
            }
        }
    }

    private fun menuHasAll(): Boolean = listOf(
        "Lire ensuite",
        "Ajouter à la file",
        "Ajouter à une playlist…",
        "Partager…",
        "Lancer la radio",
        "Ajouter aux favoris",
        "Je n'aime pas",
        "Aller à l'album",
    ).all { label -> rule.onAllNodesWithText(label).fetchSemanticsNodes().isNotEmpty() }

    /** The core, over the bench, once per test PROCESS (a phone installs one). */
    private fun install(bench: Bench): NzCore {
        val app = ApplicationProvider.getApplicationContext<Application>()
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
     * Waits for the SURFACE, not only for the semantics tree: `takeScreenshot`
     * reads the compositor's last committed frame, and the tree runs ahead of it
     * (MB7's own lesson, measured on its first captures).
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

    private fun row(title: String, id: String): TrackRow = TrackRow(
        key = "deezer:$id",
        id = id,
        title = title,
        artists = "Ascenseur",
        album = "Le matin",
        albumId = "501",
        durationS = 181u,
        cover = null,
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = 0u,
    )

    companion object {
        const val TIMEOUT_MS = 30_000L
    }

    /**
     * A loopback server inside the app process: the session, the picker's
     * playlists, the favourite and the replacement's candidates.
     *
     * A thread per connection, like the Rust fake: a screen fires its reads
     * together, and a single-threaded accept loop would deadlock the TEST instead
     * of the code.
     */
    class Bench private constructor(private val socket: ServerSocket) : AutoCloseable {
        /** `http://127.0.0.1:<port>` — what `setServer` is given. */
        fun base(): String = "http://127.0.0.1:${socket.localPort}"

        val routes = CopyOnWriteArrayList<String>()

        private val thread = thread(isDaemon = true, name = "mb10-bench") {
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
                var length = 0
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
                val (status, type, body) = answer(method, path)
                val head = buildString {
                    append("HTTP/1.1 $status X\r\n")
                    append("Content-Type: $type\r\n")
                    append("Content-Length: ${body.size}\r\n")
                    if (status == 200 && path == "/api/login") append("Set-Cookie: session=good; Path=/\r\n")
                    append("Connection: close\r\n\r\n")
                }
                val out: OutputStream = it.getOutputStream()
                out.write(head.toByteArray())
                out.write(body)
                out.flush()
            }
        }

        private fun answer(method: String, path: String): Triple<Int, String, ByteArray> {
            val json = "application/json"
            fun ok(body: String) = Triple(200, json, body.toByteArray())
            return when {
                path == "/api/version" -> ok("""{"version":"9.9.9","build":"bench"}""")
                path == "/api/login" || path == "/api/me" -> ok(ME)
                path == "/api/me/playlists" -> ok(PLAYLISTS)
                method == "POST" && path == "/api/favorite" -> ok("""{"ok":true,"favorite":true}""")
                method == "POST" && path == "/api/dislike" -> ok("""{"ok":true}""")
                method == "POST" && path.endsWith("/tracks") ->
                    ok("""{"ok":true,"added":1,"version":3,"nb_tracks":5}""")
                method == "POST" && path == "/api/playlists" ->
                    ok("""{"id":"55555555-5555-4555-8555-555555555555","title":"Ma playlist 2","editable":true,"nb_tracks":1,"version":1}""")
                method == "GET" && path.startsWith("/api/replace/candidates/") -> ok(CANDIDATES)
                method == "POST" && path == "/api/replace" -> ok("""{"ok":true,"job":"job-1"}""")
                method == "GET" && path.startsWith("/api/replace/status/") ->
                    ok("""{"running":false,"ok":true,"playlists":1,"favorites":1}""")
                method == "DELETE" && path.startsWith("/api/local/") -> ok("""{"ok":true}""")
                else -> Triple(404, json, """{"error":"not found"}""".toByteArray())
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
            private const val ME =
                """{"user":{"id":"u-mb10","name":"Nadia","admin":true,"onboarding":"done","access":{},"sources":[]}}"""

            private const val PLAYLISTS = """{"folders":[],"playlists":[
 {"id":"11111111-1111-4111-8111-111111111111","title":"Route du soir","cover":null,
  "editable":true,"role":"owner","nb_tracks":4,"version":2},
 {"id":"33333333-3333-4333-8333-333333333333","title":"Celle de June","cover":null,
  "editable":false,"role":"viewer","owner":"June","nb_tracks":9,"version":5}]}"""

            private const val CANDIDATES = """{"query":"Ascenseur Ouverture","candidates":[
 {"deezer_id":"9002","title":"Ouverture (live)","duration":240,
  "artist":{"deezer_id":"42","name":"Ascenseur"},
  "album":{"deezer_id":"501","title":"En concert"}}],"fallback_auto":null}"""

            /** A bench on a free loopback port (`emu.sh`'s rule: our own port only). */
            fun start(): Bench = Bench(ServerSocket(0, 50, InetAddress.getByName("127.0.0.1")))
        }
    }
}
