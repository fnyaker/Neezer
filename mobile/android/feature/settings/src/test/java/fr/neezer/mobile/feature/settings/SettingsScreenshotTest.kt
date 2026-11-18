package fr.neezer.mobile.feature.settings

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
 * Réglages, rendered: Encre, Papier and Pochette at 360 dp (the phone) and
 * 600 dp (the tablet's reading column), plus the 200 % font scale the acceptance
 * asks about, committed under `docs/agents/screenshots/mobile/MB16/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The recipe MB1
 * measured and MB5/MB6/MB7 reused.
 *
 * What is drawn is the REAL section dispatch ([SettingsSection]) over
 * [FakeSettingsService]: a screenshot of a mock would not have caught MB1's
 * invisible button.
 */
abstract class SettingsScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    protected fun capture(
        section: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        service: FakeSettingsService = FakeSettingsService(),
        name: String = "mb16-%s-%s-%ddp%s.png".format(
            section,
            theme.name.lowercase(),
            width,
            if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "",
        ),
    ) {
        val model = SettingsModel(service, kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Main))
        model.start()
        model.open(section)
        rule.setContent {
            NzTheme(
                kind = theme,
                accent = service.settings.accent?.let(::colourOf),
                motion = NzMotionLevel.Full,
            ) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        SettingsSection(section = section, model = model)
                    }
                }
            }
        }
        // The model's flows run on the main dispatcher Robolectric owns: let them
        // land (an EVENT, not a duration — `runToEndOfTasks` drains the queue).
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()

        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        outputDir.mkdirs()
        val file = File(outputDir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /**
     * The window is taller than the page: the tail is the ground and nothing
     * else, and cropping keeps the PNG a picture of the SCREEN rather than of an
     * empty display (and a tenth of the bytes).
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
    fun appearanceInEncre() = capture(
        section = "appearance",
        theme = NzThemeKind.Encre,
        service = FakeSettingsService(settings = SettingsFixtures.settings(theme = "encre")),
    )

    @Test
    fun appearanceInPapier() = capture(
        section = "appearance",
        theme = NzThemeKind.Papier,
        service = FakeSettingsService(settings = SettingsFixtures.settings(theme = "papier")),
    )

    @Test
    fun appearanceInPochette() = capture(
        section = "appearance",
        theme = NzThemeKind.Pochette,
        service = FakeSettingsService(settings = SettingsFixtures.settings(theme = "pochette")),
    )

    @Test
    fun appearanceAtTwoHundredPercent() = capture(
        section = "appearance",
        theme = NzThemeKind.Papier,
        fontScale = 2f,
        name = "mb16-appearance-papier-%ddp-font200.png".format(width),
    )

    @Test
    fun sources() = capture(
        section = "sources",
        theme = NzThemeKind.Encre,
        service = FakeSettingsService(
            sources = SettingsFixtures.sources(),
            accounts = SettingsFixtures.accounts(),
            access = SettingsFixtures.access(),
            server = SettingsFixtures.serverSettings(),
            deezer = SettingsFixtures.deezerStatus(),
        ),
    )

    @Test
    fun sourcesInPapier() = capture(
        section = "sources",
        theme = NzThemeKind.Papier,
        service = FakeSettingsService(
            sources = SettingsFixtures.sources(),
            accounts = SettingsFixtures.accounts(),
            access = SettingsFixtures.access(),
        ),
    )

    @Test
    fun animations() = capture("viz", NzThemeKind.Encre)

    @Test
    fun animationsInPapier() = capture("viz", NzThemeKind.Papier)

    @Test
    fun help() = capture("help", NzThemeKind.Encre)

    /** A card that cannot load: the cause and « Réessayer », never a spinner. */
    @Test
    fun aFailureNamesItsCause() = capture(
        section = "sources",
        theme = NzThemeKind.Papier,
        service = FakeSettingsService().apply {
            failures["sources"] = IllegalStateException("Hors ligne")
        },
        name = "mb16-failure-papier-%ddp.png".format(width),
    )

    @Test
    fun discovery() = capture("discovery", NzThemeKind.Encre)
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h2400dp-xhdpi")
class SettingsScreenshotPhoneTest : SettingsScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w600dp-h2400dp-xhdpi")
class SettingsScreenshotTabletTest : SettingsScreenshotBase(width = 600)
