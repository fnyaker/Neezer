package fr.neezer.mobile.feature.viz

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The visualiser, rendered: the bars and the oscilloscope, the full screen with its
 * card and controls, the pause notice, the eco notice and the photosensitivity
 * warning, in Encre and Papier at 360 dp and 600 dp plus the 200 % font scale —
 * committed under `docs/agents/screenshots/mobile/MB21/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the design system's faces, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7).
 *
 * **A screenshot is a test, not a decoration** (MB1's lesson): these images are
 * what proves a capsule is visible on Papier (the SPA's full screen is always
 * dark; this one follows the theme), that a label does not clip at 200 %, and that
 * the controls fit a 360 dp phone in one row of wrapping pills.
 */
abstract class VizScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot suite writes the lot's images"),
        )

    protected val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    protected fun capture(
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) {
        // The scene's loop waits on `withFrameNanos`, so the composition is NEVER
        // idle while it runs: the frame clock is driven by hand (MB11's measured
        // recipe — `autoAdvance` before `setContent`, which itself waits for idle).
        rule.mainClock.autoAdvance = false
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
                }
            }
        }
        // Four frames: the first lays the surface out and fixes its shape, the next
        // ones paint with a frame in hand. An EVENT (`advanceTimeByFrame`), never a
        // sleep — and no `waitForIdle`, which cannot return while a frame loop runs.
        repeat(4) { rule.mainClock.advanceTimeByFrame() }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /** The window is taller than the page: crop the ground away (MB5's recipe). */
    private fun cropToContent(full: Bitmap, theme: NzThemeKind): Bitmap {
        val background = Sillon.theme(theme).bg.toArgb()
        var last = full.height - 1
        val row = IntArray(full.width)
        while (last > 0) {
            full.getPixels(row, 0, full.width, 0, last, full.width, 1)
            if (row.any { it != background }) break
            last--
        }
        val height = (last + 24).coerceIn(1, full.height)
        if (height <= 24) return full
        return Bitmap.createBitmap(full, 0, 0, full.width, height)
    }

    private fun suffix(fontScale: Float) = if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else ""

    protected fun name(theme: NzThemeKind, screen: String, fontScale: Float = 1f) =
        "mb21-%s-%ddp-%s%s.png".format(theme.name.lowercase(), width, screen, suffix(fontScale))

    // -- the fakes ------------------------------------------------------------

    protected fun model(
        settings: uniffi.nz_ffi.VizSettingsData = VizFixtures.settings(),
        frame: uniffi.nz_ffi.VizFrame = VizFixtures.barsFrame(),
        playing: Boolean = true,
        track: uniffi.nz_ffi.NowPlaying? = VizFixtures.track(),
    ): Pair<VizModel, VizService> {
        val service = FakeVizService(settings = settings, frameNow = frame)
        val model = VizModel(
            service = service,
            player = FakeVizPlayer(VizFixtures.snapshot(playing = playing, track = track)),
            covers = FakeVizCovers(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = {},
        )
        model.refresh()
        return model to service
    }

    // -- the images -----------------------------------------------------------

    protected fun bars(theme: NzThemeKind, fontScale: Float = 1f, playing: Boolean = true) {
        val (model, service) = model(playing = playing)
        val state = model.state.value
        capture(name(theme, if (playing) "barres" else "barres-pause", fontScale), theme, fontScale) {
            VizScreen(state = state, model = model, modifier = Modifier, service = service)
        }
    }

    @Test
    fun barsEncrePhone() = bars(NzThemeKind.Encre)

    @Test
    fun barsPapierPhone() = bars(NzThemeKind.Papier)

    @Test
    fun barsAtTwoHundredPercent() = bars(NzThemeKind.Encre, fontScale = 2f)

    @Test
    fun barsPaused() = bars(NzThemeKind.Papier, playing = false)

    @Test
    fun scopeEncrePhone() {
        val (model, service) = model(
            settings = VizFixtures.settings(mode = "scope"),
            frame = VizFixtures.scopeFrame(),
        )
        val state = model.state.value
        capture(name(NzThemeKind.Encre, "oscilloscope"), NzThemeKind.Encre) {
            VizScreen(state = state, model = model, modifier = Modifier, service = service)
        }
    }

    @Test
    fun scopePapierVertical() {
        val (model, service) = model(
            settings = VizFixtures.settings(mode = "scope", orientation = "vertical", colour = "mono"),
            frame = VizFixtures.scopeFrame(),
        )
        val state = model.state.value
        capture(name(NzThemeKind.Papier, "oscilloscope-vertical"), NzThemeKind.Papier) {
            VizScreen(state = state, model = model, modifier = Modifier, service = service)
        }
    }

    @Test
    fun ecoShowsTheNoticeNotACanvas() {
        val (model, service) = model(settings = VizFixtures.settings(eco = true))
        val state = model.state.value
        capture(name(NzThemeKind.Papier, "eco"), NzThemeKind.Papier) {
            VizScreen(state = state, model = model, modifier = Modifier, service = service)
        }
    }

    @Test
    fun nothingPlayingSaysSoQuietly() {
        val (model, service) = model(playing = false, track = null)
        val state = model.state.value
        capture(name(NzThemeKind.Encre, "rien"), NzThemeKind.Encre) {
            VizScreen(state = state, model = model, modifier = Modifier, service = service)
        }
    }

    @Test
    fun theFlashWarningInBothThemes() {
        val accepted = false
        capture(name(NzThemeKind.Encre, "avertissement-flash"), NzThemeKind.Encre) {
            FlashWarningSheet(visible = true, reduced = accepted, onAccept = {}, onDismiss = {})
        }
    }

    @Test
    fun theFlashWarningOnPapierWithReducedMotion() {
        capture(name(NzThemeKind.Papier, "avertissement-flash-reduit"), NzThemeKind.Papier) {
            FlashWarningSheet(visible = true, reduced = true, onAccept = {}, onDismiss = {})
        }
    }
}

/** 360 dp: the phone the plan's worst case names. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class VizPhoneScreenshotTest : VizScreenshotBase(360)

/** 600 dp: the phone column one step from the wide shell, and a small tablet. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
class VizWideScreenshotTest : VizScreenshotBase(600) {
    @Test
    fun barsEncreWide() = bars(NzThemeKind.Encre)

    @Test
    fun barsPapierWide() = bars(NzThemeKind.Papier)
}
