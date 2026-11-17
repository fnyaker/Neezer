package fr.neezer.mobile.designsystem

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * The design system, rendered: three themes × two widths (360 dp, the phone, and
 * 600 dp, the tablet's narrow column), committed under
 * `docs/agents/screenshots/mobile/MB1/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). Paparazzi, which
 * the version catalog pins, cannot render this Compose (measured: its layoutlib
 * is API 34 and Compose 2026 calls API-35 framework methods; the 2.0 alpha
 * breaks AGP 8.13's resource merging). See the module's build file.
 */
abstract class ScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    protected fun capture(theme: NzThemeKind) {
        NzToasts.clear()
        NzToasts.show(text = "Ajouté à la file")
        rule.setContent {
            // The gallery is drawn at the width the screenshot claims, with the
            // theme's own tokens: no literal colour is involved.
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                NzComponentSheet(width = width)
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        val name = "mb1-%s-%ddp.png".format(theme.name.lowercase(), width)
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h2080dp-xhdpi")
class ScreenshotPhoneTest : ScreenshotBase(width = 360) {
    @Test fun encre() = capture(NzThemeKind.Encre)

    @Test fun papier() = capture(NzThemeKind.Papier)

    @Test fun pochette() = capture(NzThemeKind.Pochette)

    /** Pochette with a real sleeve colour, i.e. the theme actually doing its job. */
    @Test
    fun pochetteOnASleeve() {
        NzToasts.clear()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Pochette, coverAccent = androidx.compose.ui.graphics.Color(0xFF2E6F9E)) {
                NzComponentSheet(width = 360)
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        val file = File(outputDir, "mb1-pochette-sleeve-360dp.png")
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT ${file.name} ${file.length()} bytes")
    }
}

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h1980dp-xhdpi")
class ScreenshotTabletTest : ScreenshotBase(width = 600) {
    @Test fun encre() = capture(NzThemeKind.Encre)

    @Test fun papier() = capture(NzThemeKind.Papier)

    @Test fun pochette() = capture(NzThemeKind.Pochette)
}
