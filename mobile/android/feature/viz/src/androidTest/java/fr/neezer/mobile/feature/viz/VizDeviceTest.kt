package fr.neezer.mobile.feature.viz

import android.graphics.Bitmap
import android.os.Handler
import android.os.Looper
import android.view.FrameMetrics
import android.view.Window
import androidx.activity.ComponentActivity
import androidx.compose.ui.platform.ComposeView
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.test.ext.junit.rules.ActivityScenarioRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import java.io.File
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.PI
import kotlin.math.sin
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.VizFrame
import uniffi.nz_ffi.VizSettingsData
import uniffi.nz_ffi.VizStats

/**
 * The visualiser on the DEVICE: the frame budget of its paint loop, and the
 * emulator's own screenshot of the real screen (lot MB21's acceptance).
 *
 * **Frame times come from `FrameMetrics`**, the platform's own per-frame record
 * (`TOTAL_DURATION`: the frame's whole cost, from its input to its present),
 * collected on the main looper while the real screen paints — never from a
 * stopwatch around the draw call, which would measure the test's own work.
 *
 * **The screen is composed by the ACTIVITY, not by a Compose test rule.** A
 * Compose rule's idling resource waits for "no frame is pending", and a
 * visualiser's loop has one pending forever by design: the first attempt timed out
 * with "Compose is busy due to pending recompositions". A plain
 * `ActivityScenario` + `setContent` measures the app's own frames with nothing
 * sitting between the device and the number.
 *
 * **Nothing waits on a clock.** The sample is a `CountDownLatch` the platform's
 * frame listener counts down: the test returns when the DEVICE has produced 120
 * frames — the event it needs — and a device that paints nothing fails with "no
 * frame" instead of a green test.
 */
@RunWith(AndroidJUnit4::class)
class VizDeviceTest {
    @get:Rule
    val activity = ActivityScenarioRule(ComponentActivity::class.java)

    /** 120 frames: two seconds at 60, enough for a p95 to mean something. */
    private val sampleFrames = 120

    private companion object {
        /** One frame at 60 Hz. */
        const val BUDGET_MS = 16.7
    }

    @Test
    fun theFullScreenBarsHoldTheFrameBudget() {
        val durations = measure(NzThemeKind.Encre, mode = "bars", frames = sampleFrames)
        report("barres plein écran, Encre", durations)
    }

    @Test
    fun theOscilloscopeHoldsTheFrameBudget() {
        val durations = measure(NzThemeKind.Papier, mode = "scope", frames = sampleFrames)
        report("oscilloscope plein écran, Papier", durations)
    }

    @Test
    fun theEmulatorCapture() {
        // The REAL screen, on the REAL device, captured by the device itself — the
        // acceptance's "headless-emulator screenshot", written where the lot pulls
        // it from (`adb pull`).
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        val latch = CountDownLatch(6)
        show(NzThemeKind.Encre, "bars") {
            latch.countDown()
        }
        assertTrue("the screen painted 6 frames", latch.await(20, TimeUnit.SECONDS))
        val shot: Bitmap = device.takeScreenshot() ?: error("the device produced no screenshot")
        val out = File(
            InstrumentationRegistry.getInstrumentation().targetContext.getExternalFilesDir(null),
            "mb21-emulateur-barres.png",
        )
        out.parentFile?.mkdirs()
        out.outputStream().use { shot.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("DEVICE-SCREENSHOT ${out.absolutePath} ${shot.width}x${shot.height} ${out.length()} bytes")
        assertTrue("a real screen", shot.width > 300 && shot.height > 300)
    }

    // -- the bench ------------------------------------------------------------

    /** Composes the real screen and calls `onFrame` for every frame the window paints. */
    private fun show(
        theme: NzThemeKind,
        mode: String,
        onFrame: (Long) -> Unit = {},
    ) {
        val service = DeviceVizService(mode)
        val model = VizModel(
            service = service,
            player = DevicePlayer,
            covers = null,
            scope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Main),
            io = kotlinx.coroutines.Dispatchers.IO,
        )
        model.refresh()
        activity.scenario.onActivity { host ->
            host.window.addOnFrameMetricsAvailableListener(
                { _, metrics, _ ->
                    onFrame(metrics.getMetric(FrameMetrics.TOTAL_DURATION))
                },
                Handler(Looper.getMainLooper()),
            )
            host.setContentView(
                ComposeView(host).apply {
                    setContent {
                        NzTheme(kind = theme) {
                            VizScreen(
                                state = model.state.value,
                                model = model,
                                
                                modifier = Modifier,
                                service = service,
                            )
                        }
                    }
                },
            )
        }
    }

    private fun measure(theme: NzThemeKind, mode: String, frames: Int): List<Long> {
        val collected: MutableList<Long> = Collections.synchronizedList(ArrayList())
        val gate = CountDownLatch(frames)
        // The first four frames of a window are the screen's own layout and the
        // surface's first paint: the sample starts after them (the warm-up the
        // desktop's `nz-viz` bench also skips).
        var seen = 0
        show(theme, mode) { duration ->
            seen++
            if (seen > 4) {
                collected.add(duration)
                gate.countDown()
            }
        }
        assertTrue("the device produced $frames frames (got ${collected.size})", gate.await(60, TimeUnit.SECONDS))
        return collected.toList()
    }

    private fun report(label: String, durations: List<Long>) {
        val sorted = durations.map { it / 1_000_000.0 }.sorted()
        fun at(q: Double) = sorted[((sorted.size - 1) * q).toInt()]
        val mean = sorted.sum() / sorted.size
        println(
            "FRAMES %s: n=%d mean=%.2f ms p50=%.2f p95=%.2f p99=%.2f max=%.2f".format(
                label, sorted.size, mean, at(0.50), at(0.95), at(0.99), at(1.0),
            ),
        )
        // The standard's numbers: one frame at 60 Hz is 16.7 ms, and the plan's
        // p99 ceiling is 25 ms. A p95 above 16.7 means one frame in twenty is late.
        assertTrue("p95 ${"%.2f".format(at(0.95))} ms ≤ 16.7 ms", at(0.95) <= BUDGET_MS)
        assertTrue("p99 ${"%.2f".format(at(0.99))} ms ≤ 25 ms", at(0.99) <= 25.0)
    }
}

/** A player with one track playing, read once. */
private object DevicePlayer : VizPlayer {
    override fun snapshot() = uniffi.nz_ffi.PlayerSnapshot(
        state = uniffi.nz_ffi.PlayState.PLAYING,
        current = uniffi.nz_ffi.NowPlaying(
            key = "k",
            id = "1",
            title = "Déjà vu",
            artists = "Beyoncé",
            album = "B'Day",
            albumId = "1",
            artistId = "9",
            cover = null,
            durationS = 240.0,
            isEpisode = false,
            quality = "320",
            source = "deezer",
            liked = false,
        ),
        index = 0u,
        len = 1u,
        shuffle = false,
        repeat = uniffi.nz_ffi.Repeat.OFF,
        volume = 1f,
        muted = false,
        streaming = false,
        sleep = null,
        error = null,
        fault = null,
        version = 1u,
    )
}

/**
 * The analysed frame on the device: the same synthetic spectrum the JVM suite
 * uses, generated here so the instrumented APK carries no test fixture from the
 * unit-test source set (which it cannot see).
 */
/** The eco settings: mode `off`, 30 fps, and no frame to draw. */
private class ECoVizService : VizService by DeviceVizService("bars") {
    private val off = VizSettingsData(
        mode = "off",
        chosenMode = "bars",
        palette = "cover",
        intensity = 0.7,
        fps = 30u,
        eco = true,
        needsWave = false,
        scopeOrientation = "horizontal",
        scopeColour = "duo",
    )
    override fun settings() = off
    override fun frame(): VizFrame? = null
}

private class DeviceVizService(private val mode: String) : VizService {
    private val settings = VizSettingsData(
        mode = mode,
        chosenMode = mode,
        palette = "cover",
        intensity = 0.7,
        fps = 60u,
        eco = false,
        needsWave = mode == "scope",
        scopeOrientation = "horizontal",
        scopeColour = "duo",
    )
    private var tick = 0
    private val bars = 32
    private val cols = 256

    override fun settings() = settings
    override fun setMode(mode: String) = settings
    override fun setPalette(palette: String) = settings
    override fun setIntensity(value: Double) = settings
    override fun setFps(fps: Int) = settings
    override fun setEco(eco: Boolean) = settings
    override fun setScopeOrientation(orientation: String) = settings
    override fun setScopeColour(colour: String) = settings
    override fun setShape(bars: Int, cols: Int) = Unit
    override fun setArtwork(rgba: ByteArray, width: Int, height: Int) = Unit
    override fun clearArtwork() = Unit
    override fun setVisible(view: String, visible: Boolean) = Unit
    override fun stats() = VizStats(0u, 0u, 0u, 0u, 0u, false)

    override fun frame(): VizFrame {
        tick++
        val phase = tick * 0.15
        val levels = List(bars) { i ->
            val t = i.toFloat() / (bars - 1)
            ((0.85f * (1f - t) + 0.15f) * (0.55f + 0.45f * sin((i * 1.7 + phase).toDouble()).toFloat()))
                .coerceIn(0.02f, 1f)
        }
        val zone = List(21) { k ->
            when (k % 3) {
                0 -> ((288f + 40f * ((k / 3) - 3) / 6f) + 360f) % 360f
                1 -> 0.8f
                else -> 0.6f
            }
        }
        if (mode == "scope") {
            val a = ArrayList<Float>(cols * 2)
            val b = ArrayList<Float>(cols * 2)
            val per = 2_016 / cols
            var theta = phase.toDouble()
            for (c in 0 until cols) {
                var lo = 1f
                var hi = -1f
                for (k in 0 until per) {
                    val s = sin(theta).toFloat() * 0.62f
                    if (s < lo) lo = s
                    if (s > hi) hi = s
                    theta += 2.0 * PI * 220.0 / 48_000.0
                }
                a.add(lo)
                a.add(hi)
                b.add(-hi)
                b.add(-lo)
            }
            return VizFrame(
                bars = levels,
                peaks = levels.map { (it + 0.15f).coerceAtMost(1f) },
                scopeA = a,
                scopeB = b,
                hue = 196.0,
                sat = 0.8,
                light = 0.6,
                spread = 58.0,
                zones = zone,
                hasZones = false,
                level = 0.6f,
                gain = 1.05f,
                locked = 1f,
                signal = true,
                beat = false,
                bpm = 0f,
                palette = "cover",
            )
        }
        return VizFrame(
            bars = levels,
            peaks = levels.map { (it + 0.15f).coerceAtMost(1f) },
            scopeA = emptyList(),
            scopeB = emptyList(),
            hue = 288.0,
            sat = 0.9,
            light = 0.58,
            spread = 92.0,
            zones = zone,
            hasZones = true,
            level = 0.6f,
            gain = 0f,
            locked = 0f,
            signal = false,
            beat = false,
            bpm = 0f,
            palette = "duo",
        )
    }
}
