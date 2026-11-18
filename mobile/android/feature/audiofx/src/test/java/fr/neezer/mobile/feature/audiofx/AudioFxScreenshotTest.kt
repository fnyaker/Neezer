package fr.neezer.mobile.feature.audiofx

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * The effects card, rendered: Encre and Papier at 360 dp (the phone) and 600 dp
 * (the tablet's reading column), the 200 % font scale the acceptance asks about,
 * and the two states that matter — a measured chain with its warning, and a card
 * whose meter has nothing to read. Committed under
 * `docs/agents/screenshots/mobile/MB12/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text, real pixels, on the JVM — no emulator, no window on the
 * person's display (worker rule 7). The recipe MB1 measured and MB5/MB6/MB7/MB16
 * reused.
 *
 * What is drawn is the REAL [AudioFxPanel] over [FakeAudioFx]: a screenshot of a
 * mock would not have caught what these caught (see the note's MB12 section).
 */
internal abstract class AudioFxScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    protected fun capture(
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        backend: FakeAudioFx = FakeAudioFx(),
        frame: uniffi.nz_ffi.FxMeterFrame? = FxFixtures.measured(),
    ) {
        val model = AudioFxModel(backend)
        model.start()
        model.openMeter()
        if (frame != null) backend.pushFrame(frame)
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        AudioFxPanel(model)
                    }
                }
            }
        }
        // The model's state landed synchronously; drain the queue so the frame is
        // measured before it is drawn (an EVENT, not a duration).
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()

        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        outputDir.mkdirs()
        val file = File(outputDir, "mb12-%s-%s-%ddp%s.png".format(
            name, theme.name.lowercase(), width,
            if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "",
        ))
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT ${file.name} ${bitmap.width}x${bitmap.height} ${file.length()} bytes")
    }

    /** The window is taller than the page: crop the empty ground away. */
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

    @Test
    fun measuredInEncre() = capture("effects", NzThemeKind.Encre)

    @Test
    fun measuredInPapier() = capture("effects", NzThemeKind.Papier)

    @Test
    fun atTwoHundredPercent() = capture("effects", NzThemeKind.Papier, fontScale = 2f)

    /** The equalizer off: the meter says it starts with the reading, and the
     *  faders are visibly inactive rather than mysteriously dead. */
    @Test
    fun withTheEqualizerOff() = capture(
        "off",
        NzThemeKind.Encre,
        backend = FakeAudioFx(FxFixtures.state(eqEnabled = false, activePreset = null)),
        frame = FxFixtures.idle(),
    )

    /** A verdict on this output: the retry row. */
    @Test
    fun withAnOutputVerdict() = capture(
        "verdict",
        NzThemeKind.Encre,
        backend = FakeAudioFx(FxFixtures.state(retryAt = 1_800_000_900_000.0)),
    )
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h2400dp-xhdpi")
internal class AudioFxScreenshotPhoneTest : AudioFxScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w600dp-h1600dp-xhdpi")
internal class AudioFxScreenshotTabletTest : AudioFxScreenshotBase(width = 600)
