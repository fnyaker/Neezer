package fr.neezer.mobile.app

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import fr.neezer.mobile.playback.NzMediaService
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The app starts its own player surface — the second half of the MB3c defect this
 * lot fixes.
 *
 * Before: `NzMediaService` was created only by its own tests and by the car. A
 * tap on « Lire » loaded the queue into Rust (`load … autoplay=true`) and opened
 * NO audio output and NO media session: the phone played silence (measured on
 * emulator-5660, 2026-11-23), and the notification, the lock screen, the headset
 * keys and Android Auto had nothing to talk to.
 *
 * `MainActivity` starts it now; this test launches the shell the way the
 * launcher does and reads the service's own flag. It fails on the old code (the
 * flag stays false), and it also holds the LAZY rule: creating the service must
 * NOT open the device — a browsing app holds no audio stream.
 */
@RunWith(AndroidJUnit4::class)
class PlaybackServiceTest {

    @Test
    fun openingTheAppStartsTheMediaServiceWithoutOpeningTheDevice() {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        assertNull("the device output is opened by a play, not by a launch", NzMediaService.outputHandle)

        // The launcher's own component (the old app's name is the alias).
        device.executeShellCommand("am start -W -n org.nsupysonic.app/org.nsupysonic.app.SetupActivity")

        val deadline = System.currentTimeMillis() + TIMEOUT_MS
        while (!NzMediaService.started && System.currentTimeMillis() < deadline) {
            device.waitForIdle()
        }
        assertTrue(
            "opening the app never started NzMediaService: no session, no notification, no sound",
            NzMediaService.started,
        )
    }

    private companion object {
        const val TIMEOUT_MS = 10_000L
    }
}
