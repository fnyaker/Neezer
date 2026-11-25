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
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Assert.assertEquals
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
        state: HomeModel.State = defaultState(),
        name: String = "mb5-%s-%ddp%s.png".format(
            theme.name.lowercase(),
            width,
            if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "",
        ),
    ) {
        render(theme, fontScale, state)
        write(outputDir, name, theme)
    }

    /** The page as S2's geometry test and its own screenshot draw it. */
    protected fun render(
        theme: NzThemeKind,
        fontScale: Float = 1f,
        state: HomeModel.State = defaultState(),
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
    }

    protected fun defaultState(): HomeModel.State = HomeModel.State(
        screen = ScreenState.Ready(HomeFixtures.home()),
        styles = HomeFixtures.styles(),
        chosen = listOf("Techno"),
        flow = HomeFixtures.flow(),
    )

    /** Crop the empty ground away and write the PNG under [dir]. */
    protected fun write(dir: File, name: String, theme: NzThemeKind) {
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val file = File(dir, name)
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

    /**
     * S2 : la marge de page (20 dp) et le rythme des sections (32 dp), lus dans
     * l'arbre de sémantique — la marge sur le héros, l'écart entre le héros et la
     * première section qui le suit.
     */
    @Test
    fun thePageMarginsAndSectionRhythm() {
        render(NzThemeKind.Encre)
        rule.waitForIdle()
        val hero = rule.onNodeWithTag(HOME_HERO_TAG).getUnclippedBoundsInRoot()
        val resume = rule.onNodeWithTag(HOME_RESUME_TAG).getUnclippedBoundsInRoot()
        println(
            "MEASURE s2 accueil: width=${width}dp hero.top=${hero.top.value}dp " +
                "hero.bottom=${hero.bottom.value}dp resume.top=${resume.top.value}dp",
        )
        assertEquals("la marge haute de la page est celle du SPA (20 dp)", 20f, hero.top.value, 1f)
        assertEquals("deux sections sont à 32 dp", 32f, resume.top.value - hero.bottom.value, 1f)
        if (width == 360) write(s2OutputDir, "s2-accueil-360dp.png", NzThemeKind.Encre)
    }

    /** S2's own render: `docs/agents/screenshots/mobile/S2/`. */
    private val s2OutputDir: File
        get() = File(
            File(
                System.getProperty("nz.repoRoot")
                    ?: error("nz.repoRoot is not set: the screenshot test needs the repository root"),
            ),
            "docs/agents/screenshots/mobile/S2",
        )
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
