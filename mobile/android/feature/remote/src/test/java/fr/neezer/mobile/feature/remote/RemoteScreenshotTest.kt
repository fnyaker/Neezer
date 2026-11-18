package fr.neezer.mobile.feature.remote

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
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzRowSource
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
import uniffi.nz_ffi.TrackRow
import java.io.File

/**
 * Télécommande, rendered: the sheet (new link + active links), the owner's chip
 * and the loan's control screen at 360 dp (the phone) and 600 dp (the tablet's
 * reading column), in Encre and Papier, plus the 200 % font scale the acceptance
 * asks about — committed under `docs/agents/screenshots/mobile/MB19/`.
 *
 * Robolectric's NATIVE graphics backend (Skia on the host): real layout, real text
 * with the three real fonts, real pixels, on the JVM — no emulator, no window on
 * the person's display (worker rule 7). The recipe MB1 measured and MB5/MB6/MB7/
 * MB9/MB16 reused.
 *
 * What is drawn is the REAL sheet and the REAL control screen over a fake
 * service: a screenshot of a mock would not have caught MB16's invisible button.
 */
abstract class RemoteScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    private fun capture(name: String, theme: NzThemeKind, fontScale: Float = 1f, content: @androidx.compose.runtime.Composable () -> Unit) {
        rule.setContent {
            NzTheme(kind = theme, accent = null, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        outputDir.mkdirs()
        File(outputDir, name).outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
    }

    /** A loan already claimed: the control screen's own state. */
    private fun loan(
        level: String = "full",
        owner: String = "Léa",
        playing: Boolean = true,
        position: Double = 84.0,
    ): RemoteModel {
        val service = FakeRemoteService()
        service.asLoan(FakeRemoteService.grant(level, owner))
        service.now = FakeRemoteService.nowOf(playing = playing, position = position)
        val model = model(service)
        model.claim("")
        service.push(
            FakeRemoteService.statusOf(
                role = "controller",
                grant = service.grant,
                phase = uniffi.nz_ffi.RemotePhase.LIVE,
                online = true,
            ),
        )
        return model
    }

    private fun model(service: FakeRemoteService): RemoteModel {
        val scope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Main)
        val model = RemoteModel(service, scope, kotlinx.coroutines.Dispatchers.Main)
        model.open()
        return model
    }

    private fun ring(): NzRowSource = object : NzRowSource {
        private val rows = (0 until 40).map { index ->
            TrackRow(
                key = "track-$index",
                id = "track-$index",
                title = "Titre ${index + 1}",
                artists = if (index % 3 == 0) "Artiste A, Artiste B" else "Artiste ${index + 1}",
                album = "Album ${index + 1}",
                albumId = null,
                durationS = (180 + index * 7).toUInt(),
                cover = null,
                explicit = index == 4,
                liked = false,
                disliked = false,
                unavailable = false,
                downloaded = false,
                source = "deezer",
                position = index.toUInt(),
            )
        }

        override fun len(): Int = rows.size

        override fun window(start: Int, count: Int): List<TrackRow> =
            rows.subList(start.coerceAtLeast(0), minOf(start + count, rows.size))
    }

    /** The owner's sheet, two links live, one controller on the first. */
    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun the_sheet_offers_a_link_and_lists_the_live_ones() {
        val service = FakeRemoteService(
            links = mutableListOf(
                FakeRemoteService.link(id = "a", level = "full", label = "Léa", controllers = 1),
                FakeRemoteService.link(id = "b", level = "read", label = "", expiry = "Sans limite"),
            ),
        )
        val model = model(service)
        capture("mb19-sheet-encre-${width}dp.png", NzThemeKind.Encre) {
            RemoteSheet(
                visible = true,
                model = model,
                onDismiss = {},
                onShare = {},
                onCopy = {},
                selfName = "Ce téléphone",
            )
        }
    }

    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun the_sheet_in_papier() {
        val service = FakeRemoteService(links = mutableListOf(FakeRemoteService.link(id = "a")))
        val model = model(service)
        capture("mb19-sheet-papier-${width}dp.png", NzThemeKind.Papier) {
            RemoteSheet(
                visible = true,
                model = model,
                onDismiss = {},
                onShare = {},
                onCopy = {},
                selfName = "Ce téléphone",
            )
        }
    }

    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun the_sheet_at_a_two_hundred_percent_font_scale() {
        val service = FakeRemoteService(links = mutableListOf(FakeRemoteService.link(id = "a")))
        val model = model(service)
        capture("mb19-sheet-encre-${width}dp-font200.png", NzThemeKind.Encre, fontScale = 2f) {
            RemoteSheet(
                visible = true,
                model = model,
                onDismiss = {},
                onShare = {},
                onCopy = {},
                selfName = "Ce téléphone",
            )
        }
    }

    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun the_sheet_scrolled_to_its_links() {
        val service = FakeRemoteService(
            links = mutableListOf(
                FakeRemoteService.link(id = "a", level = "full", label = "Léa", controllers = 1),
                FakeRemoteService.link(id = "b", level = "read", label = "", expiry = "Sans limite"),
            ),
        )
        val model = model(service)
        capture("mb19-sheet-liens-encre-${width}dp.png", NzThemeKind.Encre) {
            RemoteSheet(
                visible = true,
                model = model,
                onDismiss = {},
                onShare = {},
                onCopy = {},
                selfName = "Ce téléphone",
            )
        }
        rule.onNode(hasScrollAction()).performScrollToNode(hasText("Tout couper"))
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        outputDir.mkdirs()
        File(outputDir, "mb19-sheet-liens-encre-${width}dp.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
    }

    /** The loan's screen: the banner, the owner's player and its queue. */
    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun the_control_screen_draws_the_owner_player() {
        val model = loan()
        capture("mb19-control-encre-${width}dp.png", NzThemeKind.Encre) {
            ControlForTest(model = model, state = model.state.value, rows = RemoteRows(ring()))
        }
    }

    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun the_control_screen_wide() {
        val model = loan(level = "read", playing = false, position = 0.0)
        capture("mb19-control-encre-${width}dp.png", NzThemeKind.Encre) {
            ControlForTest(model = model, state = model.state.value, rows = RemoteRows(ring()))
        }
    }

    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun the_control_screen_in_papier() {
        val model = loan(level = "queue", playing = true, position = 12.0)
        capture("mb19-control-papier-${width}dp.png", NzThemeKind.Papier) {
            ControlForTest(model = model, state = model.state.value, rows = RemoteRows(ring()))
        }
    }

    /** The loan ended: the notice and the account that comes back. */
    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun the_ended_notice_names_the_account() {
        capture("mb19-ended-encre-${width}dp.png", NzThemeKind.Encre) {
            RemoteEnded(restored = "Camille", onBack = {})
        }
    }
}

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-xhdpi")
class RemoteScreenshotPhoneTest : RemoteScreenshotBase(360)

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "w600dp-h1000dp-xhdpi")
class RemoteScreenshotTabletTest : RemoteScreenshotBase(600)

/**
 * The control screen with a queue the JVM can build: the real `RemoteControl`
 * reads the model for everything, so this only swaps the rows the model would
 * have got from `/remote/queue`.
 */
@androidx.compose.runtime.Composable
private fun ControlForTest(model: RemoteModel, state: RemoteModel.State, rows: RemoteRows) {
    RemoteControlContent(model = model, state = state.copy(rows = rows), onLeave = {})
}
