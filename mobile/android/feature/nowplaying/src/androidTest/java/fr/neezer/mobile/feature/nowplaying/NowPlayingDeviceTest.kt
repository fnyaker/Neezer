package fr.neezer.mobile.feature.nowplaying

import android.app.Application
import android.graphics.Bitmap
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.swipeUp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import java.io.File
import java.io.OutputStream
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.PlayContext
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.TrackRef

/**
 * MB11 on the device: the words cost one request on a cold open and none on a
 * warm one, a 4 000-row queue flings inside the frame budget, and **a player that
 * is not drawing reads the position ZERO times**.
 *
 * The three numbers the lot is held to, each read from an EVENT:
 *
 * - the requests are the loopback bench's own log (and `NzCore::stats`), never a
 *   proxy's guess;
 * - the frames are `Choreographer`'s own callbacks around a real fling (a fling is
 *   the gesture, the intervals are what the screen did), reported as p95/p99 —
 *   the plan's budget is p95 ≤ 16.7 ms, p99 ≤ 25 ms;
 * - the position reads are [PositionTicker]'s count, and the gate that stops them
 *   is the LIFECYCLE leaving STARTED — the same state a locked screen produces.
 *   The test proves it twice: by moving the activity to CREATED, and by putting
 *   the DEVICE to sleep and waiting for the observer's event.
 *
 * Nothing here waits on a clock to "let it settle": a latch, a lifecycle state or
 * a frame callback is the event.
 */
@RunWith(AndroidJUnit4::class)
class NowPlayingDeviceTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val navigator = object : NzNavigator {
        override fun go(link: AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    /** The words: ONE request cold, ZERO from the copy. */
    @Test
    fun theWordsCostOneRequestAndTheCopyIsFree() {
        val bench = Bench.start()
        val core = install(bench)
        val screens = core.screens()
        bench.forget()

        val lyrics = runBlocking { screens.lyrics(KEY) }
        assertEquals("one request for the words", 1, bench.routes.size)
        assertTrue("the words arrived: ${bench.routes}", !lyrics.empty)
        assertEquals(3, lyrics.synced.size)
        val stats = core.stats()
        println("MEASURE device lyrics cold: routes=${bench.routes} stats=$stats")

        bench.forget()
        val copy = screens.cachedLyrics(KEY)
        assertTrue("the copy answers with no request", copy != null)
        assertEquals("a warm open asks nothing", 0, bench.routes.size)
        println("MEASURE device lyrics warm: routes=${bench.routes} copy=${copy?.plain?.length}")

        // « Titres similaires » is the other read this lot owns: one request.
        bench.forget()
        val similar = runBlocking { screens.similarTracks(KEY) }
        assertEquals(1, bench.routes.size)
        assertEquals(1u, similar.len())
        bench.close()
    }

    /**
     * A 4 000-row queue, flung: p95 ≤ 16.7 ms and p99 ≤ 25 ms of frame interval,
     * with zero frozen frames (a frame over 700 ms).
     *
     * The queue is the REAL player's (`PlayerCommand.PlayQueue` of 4 000 rows), so
     * the list windows a real `ListHandle`: 64-row pages, the design system's
     * `ListWindow`, `LazyColumn` keys per row.
     */
    @Test
    fun theQueueOfFourThousandFlingsWithinBudget() {
        val bench = Bench.start()
        val core = install(bench)
        val player = core.player()
        val rows = (0 until 4_000).map { index ->
            TrackRef(
                key = "${9000 + index}",
                id = "${9000 + index}",
                title = "Titre $index",
                artists = "Artiste ${index % 7}",
                album = "Album ${index % 11}",
                albumId = "${index % 11}",
                artistId = null,
                cover = null,
                durationS = (180 + index % 60).toUInt(),
                episode = false,
                liked = false,
            )
        }
        player.command(PlayerCommand.PlayQueue(tracks = rows, start = 0u, context = PlayContext("queue", "bench")), 0uL)
        player.command(PlayerCommand.Pause, 0uL)
        assertEquals("the queue holds 4 000 rows", 4000u, player.snapshot().len)

        val model = NowPlayingModel(
            player = NzPlayerPort(player),
            service = NowPlayingCore.service() ?: error("the core's screens"),
            scope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Main),
        )
        model.refresh()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                QueueSheet(model = model, nav = navigator, lyrics = null, onClose = {})
            }
        }
        rule.waitUntil(timeoutMillis = 30_000) {
            rule.onAllNodesWithText("File d'attente").fetchSemanticsNodes().isNotEmpty()
        }
        // One fling to be sure the list is there, then the recorded ones.
        rule.onNodeWithText("Titre 1").performTouchInput { swipeUp() }
        rule.waitForIdle()

        val recorder = FrameRecorder.start()
        repeat(8) {
            rule.onRoot().performTouchInput { swipeUp() }
        }
        repeat(8) {
            rule.onRoot().performTouchInput { swipeDown() }
        }
        rule.waitForIdle()
        val frames = recorder.stop()

        val sorted = frames.sorted()
        val p95 = sorted[(sorted.size * 95 / 100).coerceAtMost(sorted.lastIndex)]
        val p99 = sorted[(sorted.size * 99 / 100).coerceAtMost(sorted.lastIndex)]
        val frozen = frames.count { it > 700.0 }
        println(
            "MEASURE mb11 queue fling: frames=${frames.size} p95=${"%.1f".format(p95)}ms " +
                "p99=${"%.1f".format(p99)}ms max=${"%.1f".format(sorted.last())}ms frozen=$frozen",
        )
        assertTrue("enough frames were drawn: ${frames.size}", frames.size >= 30)
        assertTrue("p95 ${p95}ms over 16.7", p95 <= 16.7)
        assertTrue("p99 ${p99}ms over 25", p99 <= 25.0)
        assertEquals("frozen frames", 0, frozen)
        capture("mb11-device-queue-4000.png")
        bench.close()
    }

    /**
     * **The acceptance's "0 FFI calls with the screen off".**
     *
     * The player is composed at the top of the sheet with the REAL model: its
     * frame loop reads `position()` once per visible frame. Then the lifecycle
     * leaves STARTED (what a locked screen does), and a state change that would
     * restart the loop is forced: the count does not move.
     */
    @Test
    fun aPlayerThatIsNotDrawingReadsNoPosition() {
        val bench = Bench.start()
        val core = install(bench)
        val player = core.player()
        val model = NowPlayingModel(
            player = NzPlayerPort(player),
            service = NowPlayingCore.service() ?: error("the core's screens"),
            scope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Main),
        )
        model.refresh()
        val lifecycleOwner = LocalLifecycleProbe.resumed()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                CompositionLocalProvider(
                    androidx.lifecycle.compose.LocalLifecycleOwner provides lifecycleOwner,
                ) {
                    PlayerSurface(
                        state = model.state.collectAsState().value,
                        model = model,
                        nav = navigator,
                        lyrics = null,
                        about = null,
                        onClose = {},
                    )
                }
            }
        }
        rule.waitForIdle()
        rule.waitUntil(timeoutMillis = 30_000) { model.ticker.reads > 0 }
        val whileVisible = model.ticker.reads
        println("MEASURE reads while the player draws: $whileVisible")

        // The screen goes: the owner leaves STARTED, which cancels the loop.
        lifecycleOwner.moveTo(Lifecycle.State.CREATED)
        rule.waitForIdle()

        // A state change that would restart a badly written loop, and one pause to
        // let any scheduled frame land.
        val afterStop = model.ticker.reads
        player.setScreenOn(false)
        model.refresh()
        rule.waitForIdle()
        Thread.sleep(120)
        rule.waitForIdle()
        val afterChange = model.ticker.reads
        println("MEASURE reads with the player not drawing: ${afterChange - afterStop} more")
        assertEquals("0 FFI position reads once the screen is gone", afterStop, afterChange)

        // And on the DEVICE: the power key, the observer's event, the same count.
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        val asleep = CountDownLatch(1)
        rule.runOnUiThread {
            rule.activity.lifecycle.addObserver(object : LifecycleEventObserver {
                override fun onStateChanged(source: LifecycleOwner, event: Lifecycle.Event) {
                    if (event == Lifecycle.Event.ON_STOP) asleep.countDown()
                }
            })
        }
        device.sleep()
        assertTrue("the screen went off (an event, not a wait)", asleep.await(20, TimeUnit.SECONDS))
        val asleepReads = model.ticker.reads
        Thread.sleep(300)
        assertEquals("no read while the device is asleep", asleepReads, model.ticker.reads)
        device.wakeUp()
        bench.close()
    }

    // -- the plumbing ---------------------------------------------------------

    /** The core over the bench, once per process (a phone installs one). */
    private fun install(bench: Bench): uniffi.nz_ffi.NzCore {
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

    /** The whole screen as the device shows it, in the app's own files directory. */
    private fun capture(name: String) {
        rule.waitForIdle()
        val frame = CountDownLatch(1)
        rule.runOnUiThread {
            android.view.Choreographer.getInstance().postFrameCallback { frame.countDown() }
        }
        frame.await(2, TimeUnit.SECONDS)
        val screenshot: Bitmap = InstrumentationRegistry.getInstrumentation()
            .uiAutomation
            .takeScreenshot() ?: error("the device refused a screenshot")
        val file = File(rule.activity.filesDir, name)
        file.outputStream().use { screenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${screenshot.width}x${screenshot.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /** A lifecycle the test drives: what the platform does when the screen locks. */
    private class LocalLifecycleProbe : LifecycleOwner {
        private val registry = androidx.lifecycle.LifecycleRegistry(this)
        override val lifecycle: Lifecycle get() = registry

        fun moveTo(state: Lifecycle.State) {
            registry.currentState = state
        }

        companion object {
            fun resumed(): LocalLifecycleProbe = LocalLifecycleProbe().apply {
                moveTo(Lifecycle.State.RESUMED)
            }
        }
    }

    /**
     * The device's own frame cadence, from `Choreographer`'s callbacks: each
     * interval is one frame as the screen presented it. A fling that drops frames
     * shows up here as a longer interval, which is what p95/p99 mean.
     */
    private class FrameRecorder private constructor() : android.view.Choreographer.FrameCallback {
        private val running = AtomicBoolean(true)
        private val intervals = CopyOnWriteArrayList<Double>()
        private var last = 0L

        override fun doFrame(frameTimeNanos: Long) {
            if (!running.get()) return
            if (last != 0L) intervals += (frameTimeNanos - last) / 1_000_000.0
            last = frameTimeNanos
            android.view.Choreographer.getInstance().postFrameCallback(this)
        }

        fun stop(): List<Double> {
            running.set(false)
            return intervals.toList()
        }

        companion object {
            fun start(): FrameRecorder {
                val recorder = FrameRecorder()
                InstrumentationRegistry.getInstrumentation().runOnMainSync {
                    android.view.Choreographer.getInstance().postFrameCallback(recorder)
                }
                return recorder
            }
        }
    }

    /**
     * A loopback server INSIDE the app process: the session, the words and a
     * radio, and a log of every route it was offered.
     */
    class Bench private constructor(private val socket: ServerSocket) : AutoCloseable {
        fun base(): String = "http://127.0.0.1:${socket.localPort}"

        val routes = CopyOnWriteArrayList<String>()

        fun forget() = routes.clear()

        private val thread = thread(isDaemon = true, name = "mb11-bench") {
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
                val method = parts.getOrNull(0).orEmpty()
                val path = parts.getOrNull(1).orEmpty().substringBefore('?')
                routes += "$method $path"
                val (status, body) = answer(path)
                val bytes = body.toByteArray()
                val head = buildString {
                    append("HTTP/1.1 $status X\r\n")
                    append("Content-Type: application/json\r\n")
                    append("Content-Length: ${bytes.size}\r\n")
                    if (path == "/api/login") append("Set-Cookie: session=good; Path=/\r\n")
                    append("Connection: close\r\n\r\n")
                }
                val out: OutputStream = it.getOutputStream()
                out.write(head.toByteArray())
                out.write(bytes)
                out.flush()
            }
        }

        private fun answer(path: String): Pair<Int, String> = when (path) {
            "/api/version" -> 200 to """{"version":"9.9.9","build":"bench"}"""
            "/api/login", "/api/me" -> 200 to ME
            "/api/lyrics/$KEY" -> 200 to LYRICS
            "/api/radio/track/$KEY" -> 200 to RADIO
            else -> 404 to """{"error":"not found"}"""
        }

        override fun close() {
            try {
                socket.close()
            } catch (_: Exception) {
                // a closed listener is the point
            }
        }

        companion object {
            fun start(): Bench = Bench(ServerSocket(0))

            private const val ME =
                """{"user":{"id":"u-mb11","name":"Nadia","admin":false,"onboarding":"done","access":{},"sources":[]}}"""

            private const val LYRICS = """{"lyrics":{"text":"Il est cinq heures\n♪\nParis s'éveille",
 "synced":[
  {"time":0,"text":"Il est cinq heures"},
  {"time":12000,"text":"♪"},
  {"time":24000,"text":"Paris s'éveille"}],
 "source":"archive"}}"""

            private const val RADIO = """{"tracks":[
 {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"9001","title":"Le chant des sirènes",
  "duration":215,"artist":{"deezer_id":"42","name":"Fauve"},
  "album":{"deezer_id":"501","title":"Vieux frères"}}]}"""
        }
    }

    companion object {
        private const val KEY = "9001"
    }
}
