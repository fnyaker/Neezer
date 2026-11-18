package fr.neezer.mobile.feature.entity

import android.app.Application
import android.graphics.Bitmap
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import java.io.File
import java.io.OutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.AppLink

/**
 * The lot's ONE device test: the real album page, the real core, a real stream and
 * the real output — on the headless emulator (worker rule 7: `-no-window
 * -no-audio`, never the person's display or speakers).
 *
 * **It brings its own server.** The first version needed `tools/perf_api.py` on
 * the host and the bench's ids passed as instrumentation arguments — and CI's
 * emulator job, which runs every module's `connectedDebugAndroidTest`, failed on
 * a missing argument (measured: `IllegalStateException: pass … album=<id>`). A
 * device test that only runs on one machine is a test that is red for everybody
 * else. [Bench] is a loopback server inside the app process: the album and artist
 * pages, the session, and a WAV to play. No host, no argument, nothing skipped.
 *
 * It proves the two things the acceptance asks for that a JVM test cannot:
 *
 * 1. **the pages open on the device** and their band, tabs and rows are in the
 *    semantics tree;
 * 2. **the first tap makes sound**, read from `PlayerApi::output_stats()`: the tap
 *    instant travels with the command, the engine reports the title started, the
 *    clock moves and nothing glitches.
 *
 * The screenshot lands in the app's own `filesDir`; pull it with
 * `adb shell run-as fr.neezer.mobile.feature.entity.test cat files/<name>`.
 */
@RunWith(AndroidJUnit4::class)
class EntityDeviceTest {

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
    fun anAlbumOpensOnTheDeviceAndItsFirstTapMakesSound() {
        val bench = Bench.start()
        val (core, player) = install(bench)
        val nav = RecordingNav()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                // The shell's Scaffold pads the screen; here the entry is composed
                // alone, so the insets are the test's to give.
                Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
                    AlbumEntry(id = Bench.ALBUM, nav = nav)
                }
            }
        }

        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithContentDescription(ALBUM_MARKER).fetchSemanticsNodes().isNotEmpty()
        }
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Lire").fetchSemanticsNodes().isNotEmpty()
        }
        println("MEASURE album device: requests=${bench.routes} stats=${core.stats()}")

        rule.onNodeWithText("Lire").performClick()

        // The SOUND, read from the engine: an optimistic transport state is not
        // "playing" (MB3b's own lesson), so the engine's clock is the proof.
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) { player.position().positionS > 0.2 }
        val stats = player.outputStats()
        println(
            "MEASURE tap->sound: tap_ns=${stats.lastTapNs} first_audible_ns=${stats.lastFirstAudibleNs} " +
                "underruns=${stats.underruns} buffer_ms=${stats.bufferMs} " +
                "rate=${stats.sampleRate} position_s=${player.position().positionS}",
        )
        // The two instants are on DIFFERENT clocks in this build (`tap_ns` is
        // Android's CLOCK_MONOTONIC, `first_audible_ns` the player's own
        // milliseconds-since-start × 1e6), so their difference is not yet a
        // duration — a finding of this lot (nz-core::player is MB3b's file). What
        // IS proven: the tap instant travelled with the command, the engine
        // reported the title started, the clock moved and nothing glitched.
        assertTrue("the tap instant travels with the command ($stats)", stats.lastTapNs > 0uL)
        assertTrue("the engine reported the title started ($stats)", stats.lastFirstAudibleNs > 0uL)
        assertTrue(
            "the engine's clock moved (the only proof of sound): ${player.position().positionS}",
            player.position().positionS > 0.2,
        )
        assertTrue("no underrun on the device (${stats.underruns})", stats.underruns == 0uL)
        assertTrue("the album page asked its routes (${bench.routes})", bench.routes.size >= 2)
        settle()
        capture("mb7-device-album-playing.png")
        bench.close()
    }

    @Test
    fun anArtistPageOpensOnTheDevice() {
        val bench = Bench.start()
        val (core, _) = install(bench)
        val nav = RecordingNav()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
                    ArtistEntry(id = Bench.ARTIST, nav = nav)
                }
            }
        }
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithContentDescription(ARTIST_MARKER).fetchSemanticsNodes().isNotEmpty()
        }
        // Its band's « Radio » and the two tabs, once the page answered.
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Radio").fetchSemanticsNodes().isNotEmpty()
        }
        println(
            "MEASURE artist device: routes=${bench.routes} stats=${core.stats()} " +
                "tracksTab=${rule.onAllNodesWithText("Titres").fetchSemanticsNodes().size} " +
                "creditsTab=${rule.onAllNodesWithText("Crédits").fetchSemanticsNodes().size}",
        )
        assertTrue("the artist page asked its routes (${bench.routes})", bench.routes.size >= 2)
        bench.close()
    }

    /** The core, over the bench, once per test PROCESS (a phone installs one). */
    private fun install(bench: Bench): Pair<uniffi.nz_ffi.NzCore, uniffi.nz_ffi.PlayerApi> {
        val app = ApplicationProvider.getApplicationContext<Application>()
        // `NzBridge.install` is a main-thread call (ProcessLifecycleOwner refuses
        // another thread): MB3c's own lesson, met again here. ONE core per process:
        // installing twice leaves the previous core's watchers and player alive.
        if (NzBridge.core == null) {
            rule.runOnUiThread { NzBridge.install(app) }
        }
        val core = NzBridge.core ?: error("the core could not be built: ${NzBridge.failure}")
        runBlocking {
            core.setServer(bench.base())
            core.login("bench", "Bench1")
        }
        val player = core.player()
        // The output is what makes the first audible frame real: without it the
        // clock stays at 0 (MB3c measured exactly that over silence).
        player.startOutput(Build.VERSION.SDK_INT.toUInt())
        return core to player
    }

    /**
     * Waits for the SURFACE, not only for the semantics tree.
     *
     * `takeScreenshot` reads the compositor's last committed frame, and the tree
     * runs ahead of it: this lot's first captures showed the skeletons of a page
     * already proven loaded (31 500 B against the loaded page's 221 528 B). So:
     * drain the main thread, wait for one real frame callback (an event), then
     * give the compositor the time one frame takes — the only place this lot waits
     * on a duration, and it decides an IMAGE, never an assertion.
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
     * A loopback server inside the app process: the session, one artist, one album
     * with two tracks, and a WAV to play.
     *
     * A thread per connection, like the Rust fake: the page fires its reads
     * together, and a single-threaded accept loop would deadlock the TEST instead
     * of the code.
     */
    class Bench private constructor(private val socket: ServerSocket, val audio: ByteArray) : AutoCloseable {
        /** `http://127.0.0.1:<port>` — what `setServer` is given. */
        fun base(): String = "http://127.0.0.1:${socket.localPort}"

        val routes = CopyOnWriteArrayList<String>()

        private val thread = thread(isDaemon = true, name = "mb7-bench") {
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
                var range: String? = null
                while (line != null && line.isNotEmpty()) {
                    if (line.lowercase().startsWith("range:")) range = line.substringAfter(':').trim()
                    line = input.readLine()
                }
                val parts = request.split(' ')
                val method = parts.getOrNull(0).orEmpty()
                val path = parts.getOrNull(1).orEmpty().substringBefore('?')
                routes += "$method $path"
                val (status, type, body) = answer(method, path, range)
                val head = buildString {
                    append("HTTP/1.1 $status X\r\n")
                    append("Content-Type: $type\r\n")
                    append("Content-Length: ${body.size}\r\n")
                    append("Accept-Ranges: bytes\r\n")
                    if (status == 200 && path == "/api/login") append("Set-Cookie: session=good; Path=/\r\n")
                    append("Connection: close\r\n\r\n")
                }
                val out: OutputStream = it.getOutputStream()
                out.write(head.toByteArray())
                out.write(body)
                out.flush()
            }
        }

        private fun answer(method: String, path: String, range: String?): Triple<Int, String, ByteArray> {
            val json = "application/json"
            fun ok(body: String) = Triple(200, json, body.toByteArray())
            return when {
                path == "/api/version" -> ok("""{"version":"9.9.9","build":"bench"}""")
                path == "/api/login" -> ok(ME)
                path == "/api/me" -> ok(ME)
                path == "/api/album/$ALBUM" -> ok(ALBUM_PAGE)
                path == "/api/album/$ALBUM/on-deezer" -> ok("""{"tracks":[]}""")
                path == "/api/artist/$ARTIST" -> ok(ARTIST_PAGE)
                path == "/api/artist/$ARTIST/discography" -> ok(DISCOGRAPHY)
                path == "/api/artist/$ARTIST/on-deezer" -> ok("""{"albums":[]}""")
                path == "/api/artist/$ARTIST/tracks" -> ok(ARTIST_TRACKS)
                path == "/api/credits/person" -> ok(PERSON)
                path.startsWith("/api/stream/") && path.endsWith("/ready") -> ok(
                    """{"ready":true,"ranges":"bytes","bytes":${audio.size},"cached":false,"offset":true,"streaming":false}""",
                )
                path.startsWith("/api/stream/") -> {
                    val slice = slice(range)
                    Triple(if (range == null) 200 else 206, "audio/flac", slice)
                }
                else -> Triple(404, json, """{"error":"not found"}""".toByteArray())
            }
        }

        /** The WAV, or the byte range a player asks for (`bytes=<from>-<to>`). */
        private fun slice(range: String?): ByteArray {
            if (range == null) return audio
            val spec = range.removePrefix("bytes=").substringBefore(',')
            val from = spec.substringBefore('-').toIntOrNull() ?: 0
            val to = spec.substringAfter('-').takeIf { it.isNotEmpty() }?.toIntOrNull()
                ?: (audio.size - 1)
            return audio.copyOfRange(from.coerceIn(0, audio.size), (to + 1).coerceIn(0, audio.size))
        }

        override fun close() {
            try {
                socket.close()
            } catch (_: Exception) {
                // a closed listener is the point
            }
        }

        companion object {
            const val ALBUM = "1"
            const val ARTIST = "1"
        private const val TRACK_ID = "11111111-1111-4111-8111-111111111111"

            private const val ME =
                """{"user":{"id":"u-mb7","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"""

            private const val ALBUM_PAGE = """{"album":{"id":"bbbbbbbb-1111-4111-8111-111111111111","deezer_id":"$ALBUM",
 "title":"Le matin","cover":null,"release_date":"2024-03-01","year":2024,
 "artist":{"id":"aaaaaaaa-1111-4111-8111-111111111111","deezer_id":"$ARTIST","name":"Ascenseur"},
 "nb_tracks":2,"is_favorite":false},
 "tracks":[
  {"id":"$TRACK_ID","deezer_id":"9001","title":"Ouverture","duration":60,
   "artist":{"deezer_id":"$ARTIST","name":"Ascenseur"},
   "album":{"deezer_id":"$ALBUM","title":"Le matin"}},
  {"id":"22222222-2222-4222-8222-222222222222","deezer_id":"9002","title":"Suite","duration":60,
   "artist":{"deezer_id":"$ARTIST","name":"Ascenseur"},
   "album":{"deezer_id":"$ALBUM","title":"Le matin"}}],
 "library":true}"""

            private const val ARTIST_PAGE = """{"artist":{"id":"aaaaaaaa-1111-4111-8111-111111111111","deezer_id":"$ARTIST",
 "name":"Ascenseur","picture":null,"nb_fan":null,"is_favorite":false},
 "bio":null,
 "top":[{"id":"$TRACK_ID","deezer_id":"9001","title":"Ouverture","duration":60,
   "artist":{"deezer_id":"$ARTIST","name":"Ascenseur"},
   "album":{"deezer_id":"$ALBUM","title":"Le matin"}}],
 "albums":[{"id":"bbbbbbbb-1111-4111-8111-111111111111","deezer_id":"$ALBUM","title":"Le matin",
   "year":2024,"record_type":"album","nb_tracks":2,
   "artist":{"deezer_id":"$ARTIST","name":"Ascenseur"}}],
 "related":[],"library":true,"credited":2}"""

            private const val DISCOGRAPHY = """{"discography":{"all":[
 {"deezer_id":"$ALBUM","title":"Le matin","year":"2024","record_type":"album","nb_tracks":2,
  "artist":{"deezer_id":"$ARTIST","name":"Ascenseur"}}],
 "album":[{"deezer_id":"$ALBUM","title":"Le matin","year":"2024","record_type":"album","nb_tracks":2,
  "artist":{"deezer_id":"$ARTIST","name":"Ascenseur"}}]}}"""

            private const val ARTIST_TRACKS = """{"tracks":[
 {"id":"$TRACK_ID","deezer_id":"9001","title":"Ouverture","duration":60,
  "artist":{"deezer_id":"$ARTIST","name":"Ascenseur"},
  "album":{"deezer_id":"$ALBUM","title":"Le matin"}},
 {"id":"22222222-2222-4222-8222-222222222222","deezer_id":"9002","title":"Suite","duration":60,
  "artist":{"deezer_id":"$ARTIST","name":"Ascenseur"},
  "album":{"deezer_id":"$ALBUM","title":"Le matin"}}]}"""

            private const val PERSON =
                """{"name":"Nina Simone","roles":[{"role":"composer","label":"Compositeur","count":1}],
 "labels":{"composer":"Compositeur"},
 "tracks":[{"id":"$TRACK_ID","deezer_id":"9001","title":"Sinnerman","duration":60,
  "credit_roles":["composer"],
  "artist":{"deezer_id":"$ARTIST","name":"Nina Simone"},
  "album":{"deezer_id":"$ALBUM","title":"Pastel Blues"}}]}"""

            /**
             * A bench on a free loopback port.
             *
             * The stream is a **FLAC** and not a WAV: the engine's `symphonia` is
             * built without the `wav` feature (MB3c's own notes), so a WAV answers
             * `no suitable format reader found` — measured here, where the first
             * self-contained version served a generated WAV and the clock never
             * moved. `assets/tone.flac` is 10 s of a 440 Hz tone at 8 kHz mono
             * (41 065 B), from ffmpeg, committed beside the test.
             */
            fun start(): Bench {
                val assets = InstrumentationRegistry.getInstrumentation().context.assets
                val wave = assets.open("tone.flac").use { it.readBytes() }
                val socket = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
                return Bench(socket, wave)
            }
        }
    }
}
