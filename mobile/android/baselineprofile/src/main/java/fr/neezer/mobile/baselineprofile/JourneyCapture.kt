package fr.neezer.mobile.baselineprofile

import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.JUnit4

/**
 * The journey's screenshots, on the headless emulator, for
 * `docs/agents/screenshots/mobile/MQ2/` (docs/agents/mobile.md, the screen
 * standard's rule 5: "one headless-emulator screenshot of the real screen").
 *
 * The device writes them in its own media directory
 * (`/data/local/tmp/mq2/`), which the shell can write and `adb pull` reads; the
 * `/sdcard/Android/media/<pkg>` directory is the APP's own and a different
 * process cannot create it (measured):
 *
 * ```sh
 * adb -s emulator-5660 pull /data/local/tmp/mq2 docs/agents/screenshots/mobile/MQ2
 * ```
 *
 * One capture per screen the journey crosses, at the emulator's own phone-sized
 * window (1080 × 2400, 420 dpi): what the person sees, not a mock.
 */
@RunWith(JUnit4::class)
class JourneyCapture {

    private val scope = MacrobenchmarkScope(BenchTarget.PACKAGE_NAME, false)

    /** The scope's own device: `UiDevice.getInstance()` alone is not initialised outside a rule. */
    private val device: UiDevice get() = scope.device
    private val dir = "/data/local/tmp/mq2"

    private fun shot(name: String) {
        device.executeShellCommand("mkdir -p $dir")
        device.executeShellCommand("screencap -p $dir/$name.png")
        println("MEASURE capture $dir/$name.png")
    }

    @Test
    fun captureTheJourney() {
        Journey.restart(scope)
        Journey.awaitDesc(device, Journey.HOME)
        shot("1-accueil")

        Journey.tapText(device, BenchTarget.TAB_SEARCH)
        Journey.awaitDesc(device, Journey.SEARCH)
        Journey.type(device, Journey.PLAY_ALBUM_QUERY)
        // The results, as the person sees them (the « Titres » section first).
        Journey.awaitText(device, "Titres", 30_000)
        shot("2-recherche")

        Journey.tapText(device, "Albums")
        Journey.tapFirstCardBelow(device, "Albums")
        Journey.awaitDesc(device, Journey.ALBUM)
        shot("3-album")

        val tapToSound = Journey.playAlbum(scope)
        Journey.record(device, "album.tap_vers_pause_ms", tapToSound.toString())
        Journey.flingFavorites(scope)
        shot("4-bibliotheque-titres")

        Journey.flingPlaylist(scope)
        shot("5-playlist-4000")

        Journey.openPlayer(scope)
        shot("6-lecteur")
        Journey.openQueue(scope)
        Journey.closePlayer(scope)

        Journey.tapText(device, BenchTarget.ACTION_SETTINGS)
        Journey.scrollToText(device, "Assistance").click()
        device.waitForIdle()
        Journey.awaitText(device, "Partager le rapport", 30_000)
        shot("7-assistance-diagnostic")
        device.pressBack()
        device.pressBack()

        Journey.openVisualizer(scope)
        // The scene's own surface, after the sheet is gone: the route itself.
        device.wait(androidx.test.uiautomator.Until.findObject(By.textContains("Palette")), 20_000)
        shot("8-visualiseur")

        Journey.openPlaylist(device, Journey.DOWNLOAD_PLAYLIST)
        shot("9-playlist-a-telecharger")

        // The session's own counters, left ON SCREEN: the report is the app's
        // counted requests/bytes per tag and its counted memory, and the host
        // reads it from the accessibility tree (`uiautomator dump`) — the same
        // numbers a person can share from Aide.
        Journey.tapText(device, BenchTarget.ACTION_SETTINGS)
        Journey.scrollToText(device, "Assistance").click()
        device.waitForIdle()
        Journey.awaitText(device, "Partager le rapport", 30_000)
        shot("10-rapport-diagnostic")
        val report = device.wait(Until.findObject(By.textContains("requêtes par écran")), 30_000)
        if (report?.text != null) {
            Journey.record(device, Journey.Diagnostic.parse(report.text!!), "parcours")
        }
    }
}
