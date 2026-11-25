package fr.neezer.mobile.feature.search

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
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.Density
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
import uniffi.nz_ffi.SearchScope
import java.io.File

/**
 * The search, rendered: Encre and Papier at 360 dp (the phone) and 600 dp (the
 * tablet's reading column), plus the 200 % font scale the acceptance asks about,
 * committed under `docs/agents/screenshots/mobile/MB6/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The screen drawn
 * is the REAL [SearchScreen] over [SearchFixtures]: what the screenshots show is
 * what a person sees, not a mock.
 */
abstract class SearchScreenshotBase(private val width: Int) {
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
        state: SearchModel.State = resultsState(),
        name: String = "mb6-%s-%ddp%s.png".format(
            theme.name.lowercase(),
            width,
            if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "",
        ),
    ) {
        render(theme, fontScale, state)
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        write(outputDir, name, bitmap)
    }

    protected fun render(
        theme: NzThemeKind,
        fontScale: Float = 1f,
        state: SearchModel.State = resultsState(),
    ) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(
                    LocalDensity provides Density(density.density, fontScale),
                ) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        SearchScreen(state = state, actions = SearchActions())
                    }
                }
            }
        }
    }

    protected fun write(dir: File, name: String, bitmap: Bitmap) {
        val file = File(dir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /**
     * The window is taller than the page: the tail is the ground and nothing
     * else. Cropping keeps the PNG a picture of the SCREEN rather than of an
     * empty display (MB5's own crop, same reason).
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

    /** The empty page: « Récents », the lead, MB14's style tiles. */
    @Test
    fun encreEmpty() = capture(
        NzThemeKind.Encre,
        state = SearchModel.State(recent = listOf("daft punk", "Air", "Kavinsky")),
        name = "mb6-encre-%ddp-accueil.png".format(width),
    )

    /** The podcasts scope: the passages, with the typed word marked. */
    @Test
    fun encrePodcasts() = capture(
        NzThemeKind.Encre,
        state = SearchModel.State(
            query = "memoire",
            scope = SearchScope.PODCASTS,
            showsAvailable = true,
            answer = SearchFixtures.podcastsAnswer(),
        ),
        name = "mb6-encre-%ddp-podcasts.png".format(width),
    )

    /**
     * S2 : le premier bloc de texte (« Explorer ») est à 20 dp sous l'inset, et
     * la liste garde 40 dp sous sa dernière ligne.
     */
    @Test
    fun thePageStartsOnTheSpaMargin() {
        render(NzThemeKind.Encre)
        rule.waitForIdle()
        val title = rule.onNodeWithText("Explorer").getUnclippedBoundsInRoot()
        println("MEASURE s2 explorer: width=${width}dp title.top=${title.top.value}dp")
        assertEquals("la page commence à 20 dp", 20f, title.top.value, 1f)
        if (width != 360) return
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        write(s2OutputDir, "s2-rechercher-360dp.png", cropToContent(full, NzThemeKind.Encre))
    }

    private val s2OutputDir: File
        get() = File(
            File(
                System.getProperty("nz.repoRoot")
                    ?: error("nz.repoRoot is not set: the screenshot test needs the repository root"),
            ),
            "docs/agents/screenshots/mobile/S2",
        )

    companion object {
        /** A library answer with the fallback's row woven in, as a person sees it. */
        fun resultsState(): SearchModel.State = SearchModel.State(
            query = "daft punk",
            answer = SearchFixtures.answer(
                trackCount = 12,
                // The fallback's row inside the ten the "Tout" tab shows: its `YT`
                // mark is what the capture has to prove.
                youtubeAt = 2,
                radioOffered = true,
            ),
            youtube = SearchModel.YoutubeUi.Added(1),
        )
    }
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h2400dp-xhdpi")
class SearchScreenshotPhoneTest : SearchScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h2400dp-xhdpi")
class SearchScreenshotTabletTest : SearchScreenshotBase(width = 600)
