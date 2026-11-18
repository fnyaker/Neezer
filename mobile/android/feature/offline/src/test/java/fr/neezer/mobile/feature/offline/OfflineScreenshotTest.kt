package fr.neezer.mobile.feature.offline

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Réglages → Téléchargements and the five states of `DownloadButton`, rendered:
 * Encre and Papier at 360 dp and 600 dp, plus the 200 % font scale the
 * acceptance asks about — committed under
 * `docs/agents/screenshots/mobile/MB15/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The same recipe
 * MB1 measured and MB5/MB7/MB10 hold.
 */
abstract class OfflineScreenshotBase(private val widthDp: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    private fun install(state: uniffi.nz_ffi.OfflineState = offlineState()): DownloadsModel {
        val port = FakeOfflinePort()
        port.stateFlow.value = state
        val model = DownloadsModel(
            port = port,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            freeSpace = { 8L * 1024 * 1024 * 1024 },
            toast = { _, _, _ -> },
            causeText = { it.message ?: "erreur" },
        )
        OfflineHost.install(model)
        return model
    }

    @After
    fun forget() {
        OfflineHost.reset()
    }

    private fun capture(
        name: String,
        theme: NzThemeKind,
        state: uniffi.nz_ffi.OfflineState = offlineState(),
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) {
        val model = install(state)
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(
                    LocalDensity provides Density(density.density, fontScale),
                ) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg).clipToBounds()) {
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .verticalScroll(rememberScrollState())
                                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
                            verticalArrangement = Arrangement.spacedBy(NzSpacing.l),
                        ) {
                            content()
                        }
                    }
                }
            }
        }
        // The section is static once painted; advancing the clock past the motion
        // tokens makes the IMAGE the settled state (an event, not a sleep).
        rule.mainClock.advanceTimeBy(1_000)
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
        // The model is the one the screen uses; it must not be collected before
        // the frame is read.
        require(model.state.value.version >= 0uL)
    }

    /** The decor is taller than the content: crop the empty ground away. */
    private fun cropToContent(full: Bitmap, theme: NzThemeKind): Bitmap {
        val ground = Sillon.theme(theme).bg.toArgb()
        var last = full.height - 1
        for (y in full.height - 1 downTo 0) {
            var content = false
            for (x in 0 until full.width step 4) {
                if (full.getPixel(x, y) != ground) {
                    content = true
                    break
                }
            }
            if (content) {
                last = y
                break
            }
        }
        val height = (last + NzSpacing.l.value.toInt()).coerceIn(1, full.height)
        return Bitmap.createBitmap(full, 0, 0, full.width, height)
    }

    @Test
    fun settings_encre() = capture("downloads-encre-${widthDp}dp.png", NzThemeKind.Encre) {
        OfflineSettingsSection()
    }

    @Test
    fun settings_papier() = capture("downloads-papier-${widthDp}dp.png", NzThemeKind.Papier) {
        OfflineSettingsSection()
    }

    @Test
    fun settings_200_percent() = capture(
        "downloads-encre-${widthDp}dp-200.png",
        NzThemeKind.Encre,
        fontScale = 2f,
    ) {
        OfflineSettingsSection()
    }

    @Test
    fun button_states() = capture(
        "bouton-${widthDp}dp.png",
        NzThemeKind.Encre,
        state = galleryState(),
    ) {
        DownloadButtonStateGallery()
    }

    /**
     * **TalkBack**: every control of the section carries a French name, and the
     * two that say something the pixels cannot (the switch's sentence, a
     * failure's cause) carry it in their label. A control a screen reader cannot
     * name is a control half the people cannot use.
     */
    @Test
    fun every_control_is_named_for_a_screen_reader() {
        install()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Column(Modifier.fillMaxWidth().padding(NzSpacing.pageX)) {
                    OfflineSettingsSection()
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithContentDescription("Ne lire que les titres téléchargés").assertExists()
        rule.onNodeWithText("Pause").assertExists()
        rule.onNodeWithText("Tout réessayer").assertExists()
        rule.onNodeWithText("Titres téléchargés").assertExists()
        // The failures are read, not just counted: each sentence is a node.
        rule.onNodeWithText("Titre 7 — le serveur prépare ce titre").assertExists()
    }

    /** The four states the button has, as the worker would report them. */
    private fun galleryState(): uniffi.nz_ffi.OfflineState {
        val base = offlineState()
        val running = base.sets.first().copy(done = 12u, failed = 0u, running = true)
        val done = base.sets.first().copy(context = "album:404", id = "404", title = "Album complet",
            done = 200u, failed = 0u, running = false, failures = emptyList())
        val failed = base.sets.first().copy(context = "show:9", id = "9", title = "Podcast",
            done = 12u, failed = 2u, running = false)
        return base.copy(sets = listOf(running, done, failed))
    }

    /** The four states side by side, with their French labels. */
    @Composable
    private fun DownloadButtonStateGallery() {
        Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
            // Absent: nothing queued for this context.
            DownloadButton(context = "playlist:1")
            // Queued / running: the fake's own `playlist:42` row.
            DownloadButton(context = "playlist:42")
            // Done and failed, as the worker reports them.
            DownloadButton(context = "album:404")
            DownloadButton(context = "show:9")
        }
    }
}

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h1600dp-xhdpi")
class OfflineScreenshotPhone : OfflineScreenshotBase(360)

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w600dp-h1800dp-xhdpi")
class OfflineScreenshotWide : OfflineScreenshotBase(600)
