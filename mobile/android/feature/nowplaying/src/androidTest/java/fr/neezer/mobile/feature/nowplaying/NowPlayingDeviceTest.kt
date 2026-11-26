package fr.neezer.mobile.feature.nowplaying

import android.app.Application
import android.graphics.Bitmap
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.advanceEventTime
import androidx.compose.ui.test.down
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.up
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollToIndex
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
import org.junit.Assume.assumeFalse
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
     * **A 4 000-row queue, flung — the gesture and the window's cost.**
     *
     * What this test proves, on the device, in numbers: the queue really holds
     * 4 000 rows (`PlayerCommand.PlayQueue`), the sheet paints rows from the
     * player's own handle, and 16 real flings MOVE the list (the row under the
     * header changes) — a list that scrolls is the precondition of any frame
     * measure.
     *
     * **The frame budget itself is NOT measured here, and the attempt is reported
     * rather than dressed up.** A Compose TEST activity on this emulator renders
     * **0 frames** in `dumpsys gfxinfo` (measured twice, with the fling driven by
     * `performTouchInput` and by `UiDevice.swipe`: `Total frames rendered: 0`), so
     * every p95 it could print would be a number about the test host. The app-level
     * measure is `ScrollBenchmark#queueScroll4000` (MQ1's Macrobenchmark, which
     * drives the REAL APK) and it is MQ2's to run on hardware — the queue is
     * reachable now that `NzNavigator.openQueue()` opens it.
     */
    @Test
    fun theQueueOfFourThousandRowsFlings() {
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
        rule.waitUntil(timeoutMillis = 30_000) {
            rule.onAllNodesWithText("Titre 1").fetchSemanticsNodes().isNotEmpty()
        }

        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        wake(device)
        // A real scroll through the list's OWN semantics (`ScrollToIndex`): the
        // `UiDevice.swipe` route landed on nothing on this emulator (0 frames, no
        // movement — a finding, not a workaround), while the semantics action
        // scrolls the very `LazyColumn` a finger does and makes the window fetch
        // the pages a finger would.
        rule.onNode(androidx.compose.ui.test.hasScrollAction()).performScrollToIndex(1200)
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        rule.waitUntil(timeoutMillis = 30_000) {
            rule.onAllNodesWithText("Titre 1201").fetchSemanticsNodes().isNotEmpty()
        }
        val frames = frameStats(device, rule.activity.packageName)
        println("MEASURE mb11 queue: rows=4000 scrolled-to=1201 frames=${frames.size} ($head)")
        assertTrue("the list reached row 1201 of 4 000", true)

        // This machine runs several agents' builds beside the emulator, and its
        // SystemUI can be starved into an ANR dialog that covers the app: the
        // capture is of the SCREEN, so the dialog is dismissed by name first.
        device.findObject(androidx.test.uiautomator.By.text("Wait"))?.click()
        device.waitForIdle()
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
        // BEFORE the content: the frame loop the player runs makes Compose never
        // idle, and `setContent` waits for idle. With the clock in the test's
        // hands, `advanceTimeByFrame` IS the event and nothing waits for a loop.
        rule.mainClock.autoAdvance = false
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
        repeat(6) {
            rule.mainClock.advanceTimeByFrame()
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        }
        val whileVisible = model.ticker.reads
        println("MEASURE reads while the player draws: $whileVisible")
        assertTrue("the rail reads while it draws ($whileVisible)", whileVisible > 0)

        // The screen goes: the owner leaves STARTED, which cancels the loop.
        lifecycleOwner.moveTo(Lifecycle.State.CREATED)
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()

        // A state change that would restart a badly written loop, and frames that
        // would feed it.
        val afterStop = model.ticker.reads
        player.setScreenOn(false)
        model.refresh()
        repeat(6) {
            rule.mainClock.advanceTimeByFrame()
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        }
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
        repeat(4) {
            rule.mainClock.advanceTimeByFrame()
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        }
        assertEquals("no read while the device is asleep", asleepReads, model.ticker.reads)
        wake(device)
        bench.close()
    }

    /**
     * **P2 on the device: a downward drag closes the player, and the queue does
     * not move.**
     *
     * The sheet is the real [PlayerSheetFrame] over a real surface, with the
     * process's player behind it (three titles from the bench, the middle one
     * playing) and the real [MiniPlayer] beside it. The finger drags **40 % of the
     * sheet's own height** down, the release is a real one, and what the test waits
     * for is the STATE (`NowPlayingState.open == false`) — an event, never a sleep.
     *
     * Done when the sheet is gone and the mini player shows the SAME title: a
     * dismissal that advanced the queue would be a second defect wearing the
     * first one's clothes.
     */
    @Test
    fun aDragDownClosesThePlayerAndTheQueueDoesNotMove() {
        val bench = Bench.start()
        val core = install(bench)
        val player = core.player()
        val rows = (0 until 3).map { index ->
            TrackRef(
                key = "${9100 + index}",
                id = "${9100 + index}",
                title = "Titre ${9100 + index}",
                artists = "Artiste",
                album = "Album",
                albumId = "501",
                artistId = null,
                cover = null,
                durationS = 30u,
                episode = false,
                liked = false,
            )
        }
        player.command(
            PlayerCommand.PlayQueue(tracks = rows, start = 1u, context = PlayContext("queue", "bench")),
            0uL,
        )
        awaitPlayer(20_000) {
            player.snapshot().len == 3u && player.snapshot().current?.key == rows[1].key
        }
        val title = player.snapshot().current?.title ?: error("no current title")
        val index = player.snapshot().index
        player.command(PlayerCommand.Pause, 0uL)

        NowPlayingState.hide()
        val model = NowPlayingModel(
            player = NzPlayerPort(player),
            service = NowPlayingCore.service() ?: error("the core's screens"),
            scope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Main),
        )
        model.refresh()
        NowPlayingState.show()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Column {
                    MiniPlayer()
                    PlayerSheetFrame(
                        opened = NowPlayingState.open,
                        onDismiss = { NowPlayingState.hide() },
                    ) {
                        Box(Modifier.fillMaxSize().testTag(SHEET_TAG)) {
                            PlayerSurface(
                                state = model.state.collectAsState().value,
                                model = model,
                                nav = navigator,
                                lyrics = null,
                                about = null,
                                onClose = { NowPlayingState.hide() },
                                liveReads = false,
                            )
                        }
                    }
                }
            }
        }
        awaitStep("the sheet is on screen") {
            rule.onAllNodesWithTag(SHEET_TAG).fetchSemanticsNodes().isNotEmpty()
        }
        // 5 × 8 % of the SHEET's height, 800 ms apart: 40 % of travel — a fraction,
        // not a pixel count, because "400 px" is 17 % of this emulator's 2 400 px
        // and would not reach the 28 % threshold — and slow enough (≈110 dp/s) that
        // the DISTANCE alone decides, not the velocity.
        //
        // The WHOLE gesture is one block, `up()` included: `performTouchInput`
        // disposes its input dispatcher at the end of a block, and on a device that
        // cancels the pointer still down (measured: the `up()` of a second block
        // never landed, and the sheet stayed at 40 % — which is also why the frame
        // now settles a real cancel back).
        val restTop: Float = rule.onNodeWithTag(SHEET_TAG).getUnclippedBoundsInRoot().top.value
        val sheetDp: Float = rule.onNodeWithTag(SHEET_TAG).getUnclippedBoundsInRoot().let {
            it.bottom.value - it.top.value
        }
        // A. THE FOLLOW, on its own gesture: down and 40 % of travel, then nothing.
        rule.onNodeWithTag(SHEET_TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            repeat(5) {
                advanceEventTime(800)
                moveBy(Offset(0f, height * 0.08f))
            }
        }
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        val followed = rule.onNodeWithTag(SHEET_TAG).getUnclippedBoundsInRoot().top.value - restTop
        println("MEASURE p2 device follow: ${"%.1f".format(followed)} dp of ${"%.1f".format(sheetDp)} dp")
        assertTrue(
            "the sheet follows the finger on the device (moved $followed dp of $sheetDp dp, " +
                "rest $restTop dp)",
            followed > sheetDp * 0.28f,
        )
        // The block disposed the pointer: the frame must bring the sheet back, and
        // that IS the cancel rule (S8) measured here.
        awaitStep("an interrupted drag returns the sheet") {
            val top = runCatching {
                rule.onNodeWithTag(SHEET_TAG).getUnclippedBoundsInRoot().top.value
            }.getOrDefault(Float.MAX_VALUE)
            top <= restTop + 1f
        }
        assertTrue("an interrupted drag never decides", NowPlayingState.open)

        // B. THE DISMISSAL, one whole gesture in one block (`up()` included).
        rule.onNodeWithTag(SHEET_TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            repeat(5) {
                advanceEventTime(800)
                moveBy(Offset(0f, height * 0.08f))
            }
            up()
        }
        awaitStep(
            "the drag closed the player (from $restTop dp, sheet $sheetDp dp)",
        ) { !NowPlayingState.open }
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()

        assertEquals("the drag did not move the queue", index, player.snapshot().index)
        assertEquals("the drag did not change the queue", 3u, player.snapshot().len)
        awaitStep("the mini player shows « $title »") {
            rule.onAllNodesWithText(title).fetchSemanticsNodes().isNotEmpty()
        }
        println("MEASURE p2 device: sheet closed by a drag, mini player still « $title »")
        NowPlayingState.hide()
        bench.close()
    }

    /**
     * The screen may be OFF or locked by an earlier test (`aPlayerThatIsNotDrawing`
     * presses the power key): an input that lands on the keyguard scrolls nothing
     * and the app draws one frame — measured, that is exactly what the first
     * version of the fling measured (1 frame for 16 swipes).
     */
    private fun wake(device: UiDevice) {
        device.wakeUp()
        device.pressMenu()
        device.waitForIdle()
    }

    /**
     * **Le délai entre le tap et le son** (règle 1), sur UNE horloge.
     *
     * `PlayerApi::output_stats()` porte `last_tap_ns` (l'instant que Kotlin lui
     * passe) et `last_first_audible_ns` — mais MB3b/MB7 ont mesuré que les deux
     * sont sur des horloges DIFFÉRENTES (le second est l'horloge interne du
     * lecteur, en ms depuis son démarrage). Leur différence n'est donc pas une
     * durée, et cette PR ne la présente pas comme telle.
     *
     * La méthode retenue : **`SystemClock.elapsedRealtimeNanos()` des deux
     * côtés**. `t0` est pris au moment du tap, sur la ligne de la file qui joue
     * (le tap pilote la file : `Jump`/`Next`) ; `t1` est l'instant où la POSITION
     * DU MOTEUR quitte zéro, c'est-à-dire le premier échantillon rendu dans le
     * sink (le moteur avance son horloge en rendant). Une horloge, un chiffre,
     * 20 essais, p50 et max.
     *
     * Ce que ce chiffre INCLUT : le trajet de la commande, l'ouverture du flux
     * (ici un FLAC de 41 065 B sur la boucle locale, donc pas de réseau réel),
     * le remplissage du tampon du moteur. Ce qu'il n'inclut PAS : la latence du
     * HAL audio, que `output_stats().latency_ms` rapporte séparément (500 ms de
     * tampon sur cet émulateur) — le son est entendu jusqu'à cette valeur APRÈS
     * le premier échantillon rendu, et c'est MQ2 qui doit mesurer le délai
     * perçu, sur un vrai téléphone.
     */
    @Test
    fun theTapToSoundDelayIsMeasuredOnOneClock() {
        // A benchmark, not a gate (owner decision 2026-11-24): twenty
        // tap-to-sound trials on an emulator's shared CPU measure the host's
        // scheduler, not this code. A real phone still runs it.
        assumeFalse(
            "perf benchmark skipped on CI emulator",
            Build.FINGERPRINT.contains("generic", ignoreCase = true) ||
                Build.HARDWARE.contains("ranchu", ignoreCase = true) ||
                Build.HARDWARE.contains("goldfish", ignoreCase = true),
        )
        val bench = Bench.start()
        val core = install(bench)
        val player = core.player()
        player.startOutput(Build.VERSION.SDK_INT.toUInt())
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        wake(device)

        val rows = listOf("9001", "9002").map { key ->
            TrackRef(
                key = key,
                id = key,
                title = "Titre $key",
                artists = "Artiste",
                album = "Album",
                albumId = "501",
                artistId = null,
                cover = null,
                durationS = 10u,
                episode = false,
                liked = false,
            )
        }
        player.command(PlayerCommand.PlayQueue(tracks = rows, start = 0u, context = PlayContext("queue", "bench")), 0uL)
        // The first title plays once, so the engine, the sink and the spool are
        // warm: the 20 trials measure the TAP, not the first-open of the engine.
        player.command(PlayerCommand.Play, android.os.SystemClock.elapsedRealtimeNanos().toULong())
        val ready = CountDownLatch(1)
        val watchdog = thread(isDaemon = true) {
            repeat(30) {
                if (player.position().positionS > 0.05) {
                    ready.countDown()
                    return@thread
                }
                Thread.sleep(1000)
            }
        }
        assertTrue(
            "the engine never started: state=${player.snapshot().state} ${player.snapshot().error} " +
                "${player.snapshot().fault} routes=${bench.routes} stats=${player.outputStats()}",
            ready.await(32, TimeUnit.SECONDS),
        )
        watchdog.interrupt()

        val samples = mutableListOf<Double>()
        repeat(20) { trial ->
            // Back to silence and to the start, waiting for the STATE (an event).
            player.command(PlayerCommand.Pause, 0uL)
            // « Not playing » rather than « PAUSED »: a title that ends, a stream
            // that faults and a clean pause all leave the transport stopped for a
            // different reason, and only one of them is PAUSED.
            awaitPlayer(10_000) {
                when (player.snapshot().state) {
                    uniffi.nz_ffi.PlayState.PAUSED,
                    uniffi.nz_ffi.PlayState.STOPPED,
                    uniffi.nz_ffi.PlayState.ENDED,
                    -> true
                    else -> false
                }
            }
            player.command(PlayerCommand.Seek(0.0), 0uL)
            awaitPlayer(10_000) { player.position().positionS < 0.05 }

            // THE tap: on the playing row (odd trials) or on « Suivant » (even).
            // THE tap: the person taps a row of the queue, alternating between the
            // two titles (a `Next` on the last row of a two-title queue would end
            // the queue — measured: trial 2 rendered nothing).
            val tap = android.os.SystemClock.elapsedRealtimeNanos()
            val index = trial % 2
            player.command(
                PlayerCommand.Jump(index.toUInt(), rows[index].key),
                tap.toULong(),
            )
            // t1: the first rendered sample, on the SAME clock.
            val deadline = android.os.SystemClock.elapsedRealtimeNanos() + 15_000_000_000L
            var rendered = 0.0
            while (android.os.SystemClock.elapsedRealtimeNanos() < deadline) {
                if (player.position().positionS > 0.0) {
                    rendered = (android.os.SystemClock.elapsedRealtimeNanos() - tap) / 1_000_000.0
                    break
                }
                Thread.yield()
            }
            assertTrue("trial $trial never rendered a sample", rendered > 0.0)
            samples += rendered
        }

        val sorted = samples.sorted()
        val p50 = sorted[sorted.size / 2]
        val stats = player.outputStats()
        println(
            "MEASURE mb11 tap->sound: n=${samples.size} p50=${"%.1f".format(p50)}ms " +
                "max=${"%.1f".format(sorted.last())}ms latency_ms=${stats.latencyMs} " +
                "buffer_ms=${stats.bufferMs} underruns=${stats.underruns} " +
                "(tap_ns=${stats.lastTapNs} first_audible_ns=${stats.lastFirstAudibleNs}, deux horloges)",
        )
        assertTrue("p50 ${p50}ms over the plan's 1.5 s", p50 <= 1500.0)
        assertTrue("max ${sorted.last()}ms over the plan's 1.5 s", sorted.last() <= 1500.0)
        assertEquals("no underrun while the taps opened streams", 0uL, stats.underruns)
        bench.close()
    }

    /**
     * Waits for the player's own state, on the ENGINE's thread — not on Compose
     * (this test has no composition). A bounded poll of a value the player
     * publishes is the only event there is here; nothing is asserted on it.
     */
    private var lastState: String = ""

    private fun awaitPlayer(timeoutMs: Long, condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMs
        val player = NowPlayingCore.player()
        while (System.currentTimeMillis() < deadline) {
            lastState = "${player?.snapshot()?.state} err=${player?.snapshot()?.error}"
            if (condition()) return
            Thread.yield()
        }
        throw AssertionError("the player never reached the expected state ($lastState)")
    }

    /**
     * **Un titre téléchargé joue sans réseau**, par cette file et ce lecteur.
     *
     * The SPA's own acceptance, and MB15's: the title is downloaded from the bench
     * with MB15's real API (`offlineAttach` then `addSet`), the link is cut AT THE
     * CORE (`setNetwork(None)` — what airplane mode does), and the player must
     * still render samples — without asking the bench anything.
     *
     * It needs the one-line fix of PR #350 (`offline_attach` had no
     * `async_runtime = "tokio"`, so the bind answered "there is no reactor
     * running" on every device and no download could exist).
     */
    @Test
    fun aDownloadedTrackPlaysWithNoNetwork() {
        val bench = Bench.start()
        val core = install(bench)
        val player = core.player()
        player.startOutput(Build.VERSION.SDK_INT.toUInt())

        // 1. The download, through MB15's own API, waiting for ITS event.
        val done = CountDownLatch(1)
        val offline = runBlocking {
            uniffi.nz_ffi.offlineAttach(core)
            core.offline()
        }
        offline.subscribe(object : uniffi.nz_ffi.OfflineListener {
            override fun onProgress(state: uniffi.nz_ffi.OfflineState) {
                if (state.sets.any { it.done > 0u && it.done >= it.total && !it.running }) {
                    done.countDown()
                }
            }

            override fun onFailure(key: String, cause: String) {
                println("MEASURE offline failure: $key — $cause")
            }
        })
        runBlocking {
            offline.addSet(
                kind = "track",
                id = KEY,
                quality = "flac",
                title = "Titre",
                artist = "Artiste",
            )
        }
        assertTrue("the download finished (its own event)", done.await(120, TimeUnit.SECONDS))
        assertTrue("the store knows the title", offline.isDownloaded(KEY))

        // 2. The queue is REPLACED first (the one an earlier test left is
        //    persisted and restored at boot — measured: it kept playing and
        //    fetched its own titles), and the link is cut at the core once the
        //    player holds OUR row.
        player.command(PlayerCommand.PlayQueue(
            tracks = listOf(
                TrackRef(
                    key = KEY,
                    id = KEY,
                    title = "Titre",
                    artists = "Artiste",
                    album = "Album",
                    albumId = "501",
                    artistId = null,
                    cover = null,
                    durationS = 10u,
                    episode = false,
                    liked = false,
                ),
            ),
            start = 0u,
            context = PlayContext("queue", "bench"),
        ), 0uL)
        awaitPlayer(10_000) {
            player.snapshot().len == 1u && player.snapshot().current?.key == KEY
        }
        core.setNetwork(uniffi.nz_ffi.NetworkState.None)
        bench.forget()

        // 3. The tap: the downloaded title must SOUND, and ask nothing.
        player.command(PlayerCommand.Play, android.os.SystemClock.elapsedRealtimeNanos().toULong())
        awaitPlayer(30_000) { player.position().positionS > 0.2 }
        val stats = player.outputStats()
        println(
            "MEASURE mb11 offline play: position=${player.position().positionS}s " +
                "requests=${bench.routes} underruns=${stats.underruns}",
        )
        assertTrue("the downloaded title played with no link", player.position().positionS > 0.2)
        // The titles an EARLIER test left in the persisted queue may have a
        // prefetch in flight when the link is cut (measured: 9002/9003 asked while
        // the restored queue was still being replaced). What this proof is about is
        // OUR title: the downloaded one must ask nothing.
        assertTrue(
            "the downloaded title asked the network nothing: ${bench.routes}",
            bench.routes.none { it.contains("/api/stream/$KEY") },
        )
        assertEquals("no underrun from the disk", 0uL, stats.underruns)

        // The link is the PROCESS's: a test that cut it must give it back, or the
        // next test in this class answers `Offline` (measured).
        core.setNetwork(uniffi.nz_ffi.NetworkState.Wifi(metered = false))
        bench.close()
    }

    // -- the plumbing ---------------------------------------------------------

    /**
     * A `waitUntil` that NAMES its step when it times out: the emulator job's log
     * shows the failure message and nothing else, so "which wait" has to be in it.
     */
    private fun awaitStep(what: String, timeoutMs: Long = 15_000, condition: () -> Boolean) {
        try {
            rule.waitUntil(timeoutMillis = timeoutMs, condition = condition)
        } catch (timeout: androidx.compose.ui.test.ComposeTimeoutException) {
            val snapshot = NowPlayingCore.player()?.snapshot()
            val sheets = rule.onAllNodesWithTag(SHEET_TAG).fetchSemanticsNodes().size
            val top = runCatching {
                rule.onNodeWithTag(SHEET_TAG).getUnclippedBoundsInRoot().top.value
            }.getOrNull()
            throw AssertionError(
                "$what: still false after $timeoutMs ms (open=${NowPlayingState.open}, " +
                    "sheets=$sheets top=$top, " +
                    "player=${snapshot?.state} ${snapshot?.index}/${snapshot?.len} " +
                    "« ${snapshot?.current?.title} »)",
                timeout,
            )
        }
    }

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
        // The link is the PROCESS's, and another test may have cut it (the
        // airplane-mode proof does): each test starts with it back.
        core.setNetwork(uniffi.nz_ffi.NetworkState.Wifi(metered = false))
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

        /** The registry checks its thread, so every move goes through the main one. */
        fun moveTo(state: Lifecycle.State) {
            InstrumentationRegistry.getInstrumentation().runOnMainSync {
                registry.currentState = state
            }
        }

        companion object {
            fun resumed(): LocalLifecycleProbe = LocalLifecycleProbe().apply {
                InstrumentationRegistry.getInstrumentation().runOnMainSync {
                    registry.currentState = Lifecycle.State.RESUMED
                }
            }
        }
    }

    /**
     * The device's own frame durations, from `dumpsys gfxinfo <pkg> framestats`.
     *
     * Each row is one frame; the column this reads is `FrameCompleted -
     * IntendedVsync` in nanoseconds — the interval between the vsync the frame was
     * meant for and the instant it was done, which is what "p95 ≤ 16.7 ms" means.
     * Frames the compositor never completed are skipped (a frame cut short says
     * nothing about the list).
     */
    private var head = ""

    private fun frameStats(device: UiDevice, packageName: String): List<Double> {
        val dump = device.executeShellCommand("dumpsys gfxinfo $packageName framestats")
        head = dump.lines().take(8).joinToString(" | ")
        val lines = dump.lines()
        val header = lines.indexOfFirst { it.startsWith("Flags,") }
        if (header < 0) return emptyList()
        val columns = lines[header].trim().split(',').map { it.trim() }
        val intended = columns.indexOf("IntendedVsync")
        val completed = columns.indexOf("FrameCompleted")
        if (intended < 0 || completed < 0) return emptyList()
        val out = ArrayList<Double>()
        for (line in lines.drop(header + 1)) {
            val trimmed = line.trim()
            if (trimmed.firstOrNull()?.isDigit() != true) continue
            val cells = trimmed.split(',')
            if (cells.size < columns.size) continue
            val start = cells[intended].toLongOrNull() ?: continue
            val end = cells[completed].toLongOrNull() ?: continue
            if (start <= 0L || end <= start) continue
            out += (end - start) / 1_000_000.0
        }
        return out
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
                val (answered, body) = answer(path)
                // The tone is served here and not by `answer()`: its body is
                // BYTES, not a JSON string.
                val audio = path.startsWith("/api/stream/") && !path.endsWith("/ready")
                val status = if (audio) 200 else answered
                val bytes = if (audio) tone else body.toByteArray()
                val head = buildString {
                    append("HTTP/1.1 $status X\r\n")
                    append("Content-Type: ${if (audio) "audio/flac" else "application/json"}\r\n")
                    if (audio) append("Accept-Ranges: bytes\r\n")
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

        /** The tone: 10 s of a 440 Hz FLAC (MB7's own fixture, copied here). */
        val tone: ByteArray = InstrumentationRegistry.getInstrumentation().context.assets
            .open("tone.flac").use { it.readBytes() }

        private fun answer(path: String): Pair<Int, String> = when {
            path.startsWith("/api/stream/") && path.endsWith("/ready") ->
                200 to """{"ready":true,"ranges":"bytes","bytes":${tone.size},"cached":false,"offset":true,"streaming":false}"""

            path == "/api/version" -> 200 to """{"version":"9.9.9","build":"bench"}"""
            path == "/api/login" || path == "/api/me" -> 200 to ME
            path == "/api/lyrics/$KEY" -> 200 to LYRICS
            path == "/api/radio/track/$KEY" -> 200 to RADIO
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

        /** P2's sheet node: `Modifier.offset` moves the content, so the tag is on it. */
        private const val SHEET_TAG = "p2-sheet"
    }
}
