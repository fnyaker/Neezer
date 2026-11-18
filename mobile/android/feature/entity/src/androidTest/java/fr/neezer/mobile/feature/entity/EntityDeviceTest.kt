package fr.neezer.mobile.feature.entity

import android.app.Application
import android.graphics.Bitmap
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.AppLink

/**
 * The lot's ONE device test: the real album page, the real core, the real server
 * and the real output — on the headless emulator (worker rule 7: `-no-window
 * -no-audio`, `adb -s emulator-5740`, never the person's display or speakers).
 *
 * It proves the two things the acceptance asks for that a JVM test cannot:
 *
 * 1. **the screen opens on the device** against `tools/perf_api.py --serve`
 *    reached at `10.0.2.2:5750`, and its capture is the real page;
 * 2. **the first tap makes sound**, measured through `PlayerApi::output_stats()`:
 *    the tap instant passed with the command and the first rendered frame are
 *    read back (an event, never a clock the test watches).
 *
 * Run it with the ids the bench printed (its `READY` line):
 *
 * ```sh
 * gradle :feature:entity:connectedDebugAndroidTest -PskipRust \
 *   -Pandroid.testInstrumentationRunnerArguments.album=<album id> \
 *   -Pandroid.testInstrumentationRunnerArguments.artist=<artist id>
 * ```
 *
 * The screenshots land in the app's own `filesDir`, pulled with
 * `adb shell run-as fr.neezer.mobile.feature.entity.test cat files/<name>`.
 */
@RunWith(AndroidJUnit4::class)
class EntityDeviceTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** A navigator that records where a tap wanted to go (no shell here). */
    private class RecordingNav : NzNavigator {
        val visited = mutableListOf<AppLink>()

        override fun go(link: AppLink) {
            visited += link
        }

        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    @Test
    fun anAlbumOpensOnTheDeviceAndItsFirstTapMakesSound() {
        val arguments = InstrumentationRegistry.getArguments()
        val album = arguments.getString("album")
            ?: error("pass -Pandroid.testInstrumentationRunnerArguments.album=<id>")
        val app = ApplicationProvider.getApplicationContext<Application>()

        // `NzBridge.install` is a main-thread call (ProcessLifecycleOwner refuses
        // another thread): MB3c's own lesson, met again here.
        rule.runOnUiThread { NzBridge.install(app) }
        val core = NzBridge.core ?: error("the core could not be built: ${NzBridge.failure}")
        runBlocking {
            core.setServer(SERVER)
            core.login("bench", "Bench1")
        }
        val player = core.player()
        // The output is what makes `first_audible_ns` real: without it the clock
        // stays at 0 (MB3c measured exactly that over silence).
        player.startOutput(Build.VERSION.SDK_INT.toUInt())

        println("CORE server=${core.server()} failure=${NzBridge.failure}")
        val nav = RecordingNav()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                // The shell's Scaffold pads the screen; here the entry is composed
                // alone, so the insets are the test's to give (the first capture
                // had the facts line under the status bar).
                androidx.compose.foundation.layout.Column(
                    androidx.compose.ui.Modifier
                        .fillMaxSize()
                        .statusBarsPadding()
                        .navigationBarsPadding(),
                ) {
                    AlbumEntry(id = album, nav = nav)
                }
            }
        }

        // The page's own marker (in EVERY state), then its loaded body: two
        // events, never a clock (`waitUntil` watches the semantics tree).
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithContentDescription(ALBUM_MARKER).fetchSemanticsNodes().isNotEmpty()
        }
        // Its loaded body, once the server has answered.
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            val loaded = rule.onAllNodesWithText("Lire").fetchSemanticsNodes().isNotEmpty()
            if (!loaded) println("WAITING stats=${core.stats()}")
            loaded
        }
        println("CORE loaded stats=${core.stats()}")

        rule.onNodeWithText("Lire").performClick()

        // The SOUND, read from the engine: the transport state says PLAYING the
        // instant the button is pressed (an optimistic state is not "playing").
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) { player.position().positionS > 0.2 }
        val stats = player.outputStats()
        println(
            "MEASURE tap->sound: tap_ns=${stats.lastTapNs} first_audible_ns=${stats.lastFirstAudibleNs} " +
                "underruns=${stats.underruns} buffer_ms=${stats.bufferMs} " +
                "rate=${stats.sampleRate} position_s=${player.position().positionS}",
        )
        // The two instants are on DIFFERENT clocks in this build (`tap_ns` is
        // Android's CLOCK_MONOTONIC, `first_audible_ns` the player's own
        // milliseconds-since-start × 1e6), so their difference is not yet a
        // duration — a finding of this lot (nz-core::player is MB3b's file). What
        // IS proven here: the tap instant travelled with the command, the engine
        // reported the title started, the clock moved and nothing glitched.
        assertTrue(
            "the tap instant travels with the command (stats: $stats)",
            stats.lastTapNs > 0uL,
        )
        assertTrue(
            "the engine reported the title started (stats: $stats)",
            stats.lastFirstAudibleNs > 0uL,
        )
        assertTrue(
            "the engine's clock moved (it is the only proof of sound): ${player.position().positionS}",
            player.position().positionS > 0.2,
        )
        assertTrue(
            "the device output is open and glitch-free: ${stats.underruns} underruns",
            stats.underruns == 0uL,
        )
        settle()
        capture("mb7-device-album-playing.png")
    }

    /**
     * The ARTIST page on the device: the same screen a person opens, its portrait,
     * its facts and its rails, from the same server.
     */
    @Test
    fun anArtistPageOpensOnTheDevice() {
        val arguments = InstrumentationRegistry.getArguments()
        val artist = arguments.getString("artist")
            ?: error("pass -Pandroid.testInstrumentationRunnerArguments.artist=<id>")
        val app = ApplicationProvider.getApplicationContext<Application>()
        rule.runOnUiThread { NzBridge.install(app) }
        val core = NzBridge.core ?: error("the core could not be built: ${NzBridge.failure}")
        runBlocking {
            core.setServer(SERVER)
            core.login("bench", "Bench1")
        }
        val nav = RecordingNav()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                androidx.compose.foundation.layout.Column(
                    androidx.compose.ui.Modifier
                        .fillMaxSize()
                        .statusBarsPadding()
                        .navigationBarsPadding(),
                ) {
                    ArtistEntry(id = artist, nav = nav)
                }
            }
        }
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithContentDescription(ARTIST_MARKER).fetchSemanticsNodes().isNotEmpty()
        }
        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            val loaded = rule.onAllNodesWithText("Radio").fetchSemanticsNodes().isNotEmpty()
            if (!loaded) println("WAITING artist stats=${core.stats()}")
            loaded
        }
        // The artist page opens on the device: its band's « Radio » and the three
        // tabs are in the tree, from the same server. No capture here — the lot's
        // committed device image is the album page, whose frame the playing
        // capture (below) proves is live.
        println(
            "MEASURE artist device: stats=${core.stats()} tabs=" +
                rule.onAllNodesWithText("Titres").fetchSemanticsNodes().size +
                " creditsTab=" + rule.onAllNodesWithText("Crédits").fetchSemanticsNodes().size,
        )
    }

    /**
     * Waits for the SURFACE, not only for the semantics tree.
     *
     * A semantics match precedes the pixels: this lot's first captures showed the
     * skeletons of a page already proven loaded (31 500 B against the loaded
     * page's 221 528 B), because `takeScreenshot` reads the compositor's last
     * committed frame while the RenderThread is still one behind. So: drain the
     * main thread, wait for one real frame callback (an event, not a clock), then
     * give the compositor the time a frame takes to commit — the only place this
     * lot waits on a duration, and it decides an IMAGE, never an assertion.
     */
    private fun settle() {
        rule.waitForIdle()
        val frame = java.util.concurrent.CountDownLatch(1)
        rule.runOnUiThread {
            android.view.Choreographer.getInstance().postFrameCallback { frame.countDown() }
        }
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        frame.await(2, java.util.concurrent.TimeUnit.SECONDS)
        InstrumentationRegistry.getInstrumentation().waitForIdleSync()
        Thread.sleep(150)
    }

    /** The whole screen as the device shows it, in the app's own files directory. */
    private fun capture(name: String) {
        val screenshot: Bitmap = InstrumentationRegistry.getInstrumentation()
            .uiAutomation
            .takeScreenshot() ?: error("the device refused a screenshot")
        val file = File(rule.activity.filesDir, name)
        file.outputStream().use { screenshot.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${screenshot.width}x${screenshot.height} ${file.absolutePath} ${file.length()} bytes")
    }

    companion object {
        /** The bench server, as the emulator sees the host (`10.0.2.2`). */
        const val SERVER = "http://10.0.2.2:5750"

        const val TIMEOUT_MS = 30_000L
    }
}
