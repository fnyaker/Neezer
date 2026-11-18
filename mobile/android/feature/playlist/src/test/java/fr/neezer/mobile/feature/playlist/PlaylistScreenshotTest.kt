package fr.neezer.mobile.feature.playlist

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
 * The playlist, rendered: Encre and Papier at 360 dp (the phone) and 600 dp (the
 * tablet's reading column), plus the 200 % font scale the acceptance asks about,
 * committed under `docs/agents/screenshots/mobile/MB9/`.
 *
 * Robolectric's NATIVE graphics backend (Skia on the host): real layout, real text
 * with the three real fonts, real pixels, on the JVM — no emulator, no window on
 * the person's display (worker rule 7). The screen drawn is the REAL
 * [PlaylistScreen] over [PlaylistFixtures]'s 4 000-row playlist: what the captures
 * show is what a person sees.
 */
abstract class PlaylistScreenshotBase(private val width: Int) {
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
        state: PlaylistModel.State = populated(),
        suffix: String = "",
    ) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(
                    LocalDensity provides Density(density.density, fontScale),
                ) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        PlaylistScreen(
                            state = state,
                            actions = PlaylistActions(),
                            wide = width >= 600,
                        )
                    }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val name = "mb9-%s-%ddp%s%s.png".format(
            theme.name.lowercase(),
            width,
            if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "",
            suffix,
        )
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /** The window is taller than the page: the tail is the ground and nothing else. */
    private fun cropToContent(full: Bitmap, theme: NzThemeKind): Bitmap {
        val background = Sillon.theme(theme).bg.toArgb()
        var last = full.height - 1
        while (last > 0) {
            var uniform = true
            for (x in 0 until full.width step 8) {
                if (full.getPixel(x, last) != background) {
                    uniform = false
                    break
                }
            }
            if (!uniform) break
            last -= 1
        }
        val height = (last + 1).coerceAtLeast(1)
        return Bitmap.createBitmap(full, 0, 0, full.width, height)
    }
}

/** The phone: 360 dp. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h1400dp-xhdpi")
class PlaylistPhoneScreenshotTest : PlaylistScreenshotBase(360) {
    @Test
    fun encre() = capture(NzThemeKind.Encre)

    @Test
    fun papier() = capture(NzThemeKind.Papier)

    /** The acceptance's 200 % font scale: nothing clips, nothing overlaps. */
    @Test
    fun encre_at_two_hundred_percent() = capture(NzThemeKind.Encre, fontScale = 2f)

    /** Edit mode: the grips, the arrows, the remove buttons. */
    @Test
    fun editing() = capture(
        NzThemeKind.Encre,
        state = populated().copy(editing = true),
        suffix = "-edition",
    )

    /** The read-only band a viewer or a public link gets. */
    @Test
    fun lecture_seule() = capture(
        NzThemeKind.Papier,
        state = readOnly(),
        suffix = "-lecture-seule",
    )

    /** A failed read: the cause and « Réessayer », never a spinner. */
    @Test
    fun echec() = capture(
        NzThemeKind.Encre,
        state = PlaylistModel.State(id = "pl-1", failure = "Hors ligne"),
        suffix = "-echec",
    )
}

/** The tablet: 600 dp, the two-column reading shape. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w600dp-h1000dp-xhdpi")
class PlaylistWideScreenshotTest : PlaylistScreenshotBase(600) {
    @Test
    fun encre() = capture(NzThemeKind.Encre)

    @Test
    fun papier() = capture(NzThemeKind.Papier)
}

/** The import and the public page, at 360 dp: their own two screens. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h1400dp-xhdpi")
class PlaylistSheetsScreenshotTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set"),
        )

    /** One `setContent` per test: the compose rule refuses a second one. */
    private fun publicPage(theme: NzThemeKind) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    SharedPlaylistScreen(
                        state = SharedModel.State(
                            data = PlaylistFixtures.shared(),
                            token = "tok",
                        ),
                        onBack = {},
                        onRetry = {},
                        onPlay = {},
                    )
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        val name = "mb9-partagee-${theme.name.lowercase()}-360dp.png"
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath}")
    }

    @Test
    fun the_public_page_encre() = publicPage(NzThemeKind.Encre)

    @Test
    fun the_public_page_papier() = publicPage(NzThemeKind.Papier)
}

/** A page as the screen draws it: a 4 000-row playlist, editable, with suggestions. */
private fun populated(): PlaylistModel.State {
    val page = PlaylistFixtures.page(
        members = listOf(
            PlaylistFixtures.owner(),
            uniffi.nz_ffi.PlaylistMember("u2", "camille", uniffi.nz_ffi.PlaylistRole.EDITOR, null),
            uniffi.nz_ffi.PlaylistMember("u3", "june", uniffi.nz_ffi.PlaylistRole.VIEWER, null),
            uniffi.nz_ffi.PlaylistMember("u4", "sam", uniffi.nz_ffi.PlaylistRole.VIEWER, null),
        ),
    )
    return PlaylistModel.State(
        id = "pl-1",
        page = page,
        rows = page.rows,
        recs = RecsUi.Ready(listOf(PlaylistFixtures.track(9_000), PlaylistFixtures.track(9_001))),
    )
}

/** The band a viewer sees: read-only, the owner's name in the note. */
private fun readOnly(): PlaylistModel.State {
    val page = PlaylistFixtures.page(
        role = uniffi.nz_ffi.PlaylistRole.VIEWER,
        editable = false,
        canManage = false,
    )
    return PlaylistModel.State(id = "pl-1", page = page, rows = page.rows)
}
