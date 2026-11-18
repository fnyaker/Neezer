package fr.neezer.mobile.feature.home

import android.graphics.Bitmap
import android.graphics.Canvas
import androidx.compose.ui.graphics.toArgb
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.bridge.ScreenState
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
 * The home, rendered: Encre and Papier at 360 dp (the phone) and 600 dp (the
 * tablet's reading column), plus the 200 % font scale the acceptance asks about,
 * committed under `docs/agents/screenshots/mobile/MB5/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The same recipe
 * MB1 measured (`:designsystem`'s build file records why Paparazzi cannot render
 * this tree's Compose).
 *
 * The screen drawn is the REAL [HomeScreen] over [HomeFixtures]: what the
 * screenshots show is what a person sees, not a mock.
 */
abstract class HomeScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    protected fun capture(
        theme: NzThemeKind,
        fontScale: Float = 1f,
        state: HomeModel.State = HomeModel.State(
            screen = ScreenState.Ready(HomeFixtures.home()),
            styles = HomeFixtures.styles(),
            chosen = listOf("Techno"),
            flow = HomeFixtures.flow(),
        ),
        name: String = "mb5-%s-%ddp%s.png".format(
            theme.name.lowercase(),
            width,
            if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "",
        ),
    ) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(
                    LocalDensity provides Density(density.density, fontScale),
                ) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                            HomeScreen(state = state, actions = HomeActions())
                        }
                    }
                }
            }
        }
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

    /**
     * The window is 2400 dp tall and the page is not: the tail is the ground and
     * nothing else. Cropping it keeps the PNG a picture of the SCREEN rather than
     * of an empty display (and a tenth of the bytes).
     */
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
    fun encre() = capture(NzThemeKind.Encre)

    @Test
    fun papier() = capture(NzThemeKind.Papier)

    /** 200 %: nothing clips, everything still reads (`webapp.md` §10). */
    @Test
    fun encreAtTwoHundredPercent() = capture(NzThemeKind.Encre, fontScale = 2f)
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h2400dp-xhdpi")
class HomeScreenshotPhoneTest : HomeScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h2400dp-xhdpi")
class HomeScreenshotTabletTest : HomeScreenshotBase(width = 600)
