package fr.neezer.mobile.feature.lan

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File
import uniffi.nz_ffi.LanStateFfi

/**
 * The two LAN screens, rendered: Encre and Papier at 360 dp (the phone) and
 * 600 dp (the tablet's reading column), plus the 200 % font scale, committed
 * under `docs/agents/screenshots/lan/LN14/`.
 *
 * Robolectric's NATIVE graphics backend (Skia on the host): real layout, real
 * text with the three real fonts, real pixels — no emulator and no window on the
 * person's display (worker rule 7), the recipe MB1 measured and MB5/MB6/MB7/MB16
 * reused. What is drawn is the REAL section over a fake port, so a layout bug
 * (a squeezed switch, a cut chip) shows here before the emulator run.
 */
abstract class LanScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    /** A model already in the state the screen shows on a real device. */
    private fun model(
        snapshot: uniffi.nz_ffi.LanSnapshotFfi = sampleSnapshot(
            state = LanStateFfi.Connected(peers = 2u),
            peers = listOf(
                samplePeer(name = "Bureau", own = true, platform = "desktop", rttMs = 4),
                samplePeer(pub = "bb".repeat(32), name = "Salon", own = false, platform = "android", rttMs = 23),
            ),
            transfers = listOf(sampleTransfer()),
        ),
        settings: uniffi.nz_ffi.LanSettingsFfi = sampleSettings(),
        devices: List<uniffi.nz_ffi.LanDeviceFfi> = listOf(
            sampleDevice(id = "self", name = "Téléphone", current = true),
            sampleDevice(id = "pc", name = "Bureau", platform = "desktop", current = false),
        ),
    ): LanModel {
        val fake = FakeLanPort(
            snapshotValue = snapshot,
            settingsValue = settings,
            devicesValue = devices,
            friendsValue = sampleFriends(
                sharesTo = listOf(
                    sampleFriend(name = "Alice", all = true),
                    sampleFriend(
                        userId = "9b2a6f0c-0000-4000-8000-000000000002",
                        name = "Bastien",
                        all = false,
                        playlists = listOf(uniffi.nz_ffi.LanPlaylistFfi(id = "1", title = "Route")),
                    ),
                ),
            ),
        )
        val model = LanModel(
            port = fake,
            scope = CoroutineScope(Dispatchers.Main),
            playlists = FakePlaylists(),
            io = Dispatchers.Main,
        )
        model.start()
        return model
    }

    private fun capture(
        theme: NzThemeKind,
        fontScale: Float = 1f,
        name: String,
        content: @androidx.compose.runtime.Composable () -> Unit,
    ) {
        rule.setContent {
            NzTheme(kind = theme, accent = null, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg).padding(NzSpacing.pageX)) {
                        content()
                    }
                }
            }
        }
        // The model's flows run on the main dispatcher Robolectric owns: drain the
        // queue (an EVENT, never a duration).
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

    private fun suffix(theme: NzThemeKind, fontScale: Float) =
        "${theme.name.lowercase()}-${width}dp" + if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else ""

    @Test
    fun devicesInEncre() = capture(NzThemeKind.Encre, name = "ln14-appareils-${suffix(NzThemeKind.Encre, 1f)}.png") {
        LanSettingsSection(model = model())
    }

    @Test
    fun devicesInPapier() = capture(NzThemeKind.Papier, name = "ln14-appareils-${suffix(NzThemeKind.Papier, 1f)}.png") {
        LanSettingsSection(model = model())
    }

    @Test
    fun devicesAtTwoHundredPercent() = capture(
        NzThemeKind.Papier,
        fontScale = 2f,
        name = "ln14-appareils-${suffix(NzThemeKind.Papier, 2f)}.png",
    ) {
        LanSettingsSection(model = model())
    }

    /** A server without the feature: one calm sentence, no switch, no list. */
    @Test
    fun anOlderServerDrawsOneSentence() = capture(
        NzThemeKind.Encre,
        name = "ln14-appareils-serveur-ancien-${width}dp.png",
    ) {
        LanSettingsSection(model = model(snapshot = sampleSnapshot(state = LanStateFfi.Unsupported)))
    }

    @Test
    fun peerSetsInEncre() = capture(NzThemeKind.Encre, name = "ln14-sur-vos-appareils-${suffix(NzThemeKind.Encre, 1f)}.png") {
        PeerSetsSection(model = peerSetsModel())
    }

    @Test
    fun peerSetsInPapier() = capture(NzThemeKind.Papier, name = "ln14-sur-vos-appareils-${suffix(NzThemeKind.Papier, 1f)}.png") {
        PeerSetsSection(model = peerSetsModel())
    }

    private fun peerSetsModel(): PeerSetsModel {
        val fake = FakeLanPort(
            peerSetsValue = listOf(
                sampleSet(ident = "42", title = "Route", tracks = 50, bytes = 350_000_000),
                sampleSet(ident = "43", title = "Nuit", tracks = 213, local = 12, complete = false, bytes = 1_400_000_000),
                sampleSet(
                    peer = "bb".repeat(32),
                    peerName = "Salon",
                    own = false,
                    kind = "album",
                    ident = "7",
                    title = "Random Access Memories",
                    tracks = 13,
                    bytes = 120_000_000,
                ),
            ),
        )
        val model = PeerSetsModel(port = fake, scope = CoroutineScope(Dispatchers.Main), io = Dispatchers.Main)
        model.reload()
        model.toggle(PeerSetsModel.keyOf(sampleSet(ident = "42", title = "Route", tracks = 50)))
        return model
    }
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h2600dp-xhdpi")
class LanScreenshotPhoneTest : LanScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w600dp-h2600dp-xhdpi")
class LanScreenshotTabletTest : LanScreenshotBase(width = 600)
