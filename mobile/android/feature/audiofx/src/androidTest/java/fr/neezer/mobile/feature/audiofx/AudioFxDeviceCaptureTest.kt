package fr.neezer.mobile.feature.audiofx

import android.graphics.Bitmap
import android.graphics.Canvas
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * **The card on a real device**: the app's own Rust core, its own FFI, the real
 * `AudioFxSections()` and the emulator's real fonts and layout — rendered into a
 * PNG this test writes where `adb pull` can reach it, then looked at.
 *
 * The acceptance asks for one emulator capture of the real screen; this is the
 * honest version of it. It needs no server and no session, because the card makes
 * no request at all (that is MB12's own claim): `NzBridge.install` builds the
 * core over the app's directories, and the card reads it.
 */
@RunWith(AndroidJUnit4::class)
class AudioFxDeviceCaptureTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun theRealCardOnTheDevice() {
        val app = ApplicationProvider.getApplicationContext<android.app.Application>()
        NzBridge.install(app)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    // The REAL entry point, the one Réglages hosts.
                    AudioFxSections()
                }
            }
        }
        rule.waitForIdle()
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()

        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        val dir = app.getExternalFilesDir(null) ?: app.filesDir
        val file = File(dir, "mb12-emulator.png")
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("DEVICE CAPTURE ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
        // The core opened and the card read it: the meter is the only thing that
        // would have needed audio, and it says so instead of inventing a value.
        NzBridge.shutdown()
    }
}
