package fr.neezer.mobile.feature.audiofx

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.runBlocking
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * **The card on a real device**: the app's own Rust core, its own FFI, the real
 * `AudioFxSections()` and the emulator's real fonts, layout and pixels —
 * captured from the CARD's own graphics layer, so the image contains what this
 * lot draws and nothing the test host's window adds (no status bar, no dialog, no
 * pointer dot).
 *
 * The acceptance asks for one emulator capture of the real screen; this is it.
 * It needs no server and no session, because the card makes no request at all
 * (that is MB12's own claim): `NzBridge.install` builds the core over the app's
 * directories, and the card reads it.
 */
@RunWith(AndroidJUnit4::class)
class AudioFxDeviceCaptureTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun theRealCardOnTheDevice() {
        val app = ApplicationProvider.getApplicationContext<android.app.Application>()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        // `NzBridge.install` registers a network callback and a lifecycle
        // observer: both demand the main thread (the app calls it from
        // `Application.onCreate`, which is the main thread).
        instrumentation.runOnMainSync { NzBridge.install(app) }

        val holder = LayerHolder()
        rule.setContent {
            val layer = rememberGraphicsLayer()
            androidx.compose.runtime.SideEffect { holder.layer = layer }
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    // The REAL entry point, the one Réglages hosts.
                    Box(Modifier.width(360.dp).captureInto(layer)) {
                        AudioFxSections()
                    }
                }
            }
        }
        rule.waitForIdle()

        val layer = holder.layer ?: error("no layer was composed")
        val bitmap = runBlocking { layer.toImageBitmap().asAndroidBitmap() }
        val dir = app.getExternalFilesDir(null) ?: app.filesDir
        val file = File(dir, "mb12-emulator.png")
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println(
            "DEVICE CAPTURE ${bitmap.width}x${bitmap.height} ${file.absolutePath} " +
                "${file.length()} bytes"
        )
        // The core opened and the card read it: the meter is the only thing that
        // would have needed audio, and it says so instead of inventing a value.
        instrumentation.runOnMainSync { NzBridge.shutdown() }
    }

}

/** The composed layer, handed to the test after the first frame. */
private class LayerHolder {
    var layer: androidx.compose.ui.graphics.layer.GraphicsLayer? = null
}

/** Draws `content` into `layer` so the test can read back exactly those pixels. */
private fun Modifier.captureInto(
    layer: androidx.compose.ui.graphics.layer.GraphicsLayer,
): Modifier = drawWithContent {
    layer.record { this@drawWithContent.drawContent() }
    drawLayer(layer)
}
