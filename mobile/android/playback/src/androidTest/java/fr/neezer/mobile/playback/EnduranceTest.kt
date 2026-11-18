// MB3c's long measurement (docs/agents/mobile.md, "MB3c" acceptance): 30 minutes
// with the screen off and Doze forced, over a 40-title queue, every title change
// happening, 0 underruns, and the process's CPU time written down.
//
// It runs ONLY when asked (`-e endurance 1`): thirty minutes is a measurement, not
// a gate, and CI's emulator job must not pay for it. The gate is
// `NzMediaServiceTest`.
package fr.neezer.mobile.playback

import android.app.Application
import android.content.ComponentName
import android.content.Intent
import android.os.Looper
import android.os.SystemClock
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.NzBridge
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.TrackRef

@RunWith(AndroidJUnit4::class)
class EnduranceTest {

    private lateinit var app: Application
    private lateinit var api: PlayerApi

    @Before
    fun onlyWhenAsked() {
        assumeTrue(
            "mesure de 30 min : lancer avec -e endurance 1",
            InstrumentationRegistry.getArguments().getString("endurance") == "1",
        )
        app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        // `Application.onCreate` is a main-thread call (see NzMediaServiceTest).
        if (NzBridge.core == null) {
            InstrumentationRegistry.getInstrumentation().runOnMainSync { NzBridge.install(app) }
        }
        val core = NzBridge.core ?: error("le noyau Rust n'a pas démarré")
        api = core.player()
        runBlocking {
            core.setServer(
                InstrumentationRegistry.getArguments().getString("nzServer")
                    ?: "http://10.0.2.2:4711",
            )
        }
    }

    /**
     * 40 titles of 45 s = 30 minutes, screen off, Doze forced.
     *
     * Every assertion is a COUNT: titles played, underruns, rebuffers, CPU ticks.
     * Nothing waits for a duration to "settle": the loop ends on the 40th title
     * (or on its deadline, which then FAILS with the number reached).
     */
    @Test
    fun forty_five_second_titles_keep_their_time_with_the_screen_off() {
        val tracks = (1..40).map { index -> track(index, seconds = 45) }
        api.command(PlayerCommand.PlayQueue(tracks = tracks, start = 0u, context = null), 0u)

        val player = connect()
        onMain { player.play() }
        assertTrue(
            "la lecture n'a pas démarré",
            awaitTrue("Playing", 30_000) { api.snapshot().state.name == "PLAYING" },
        )

        val ui = InstrumentationRegistry.getInstrumentation().uiAutomation
        val cpuBefore = cpuMillis()
        val started = SystemClock.elapsedRealtime()

        // The person locks the phone and walks away; Android then idles it.
        //
        // Three platform facts, measured: a device on AC power cannot enter deep
        // idle, the emulator's image ships deep idle DISABLED ("Unable to go deep
        // idle; not enabled"), and the screen must be off first. Getting them in
        // the wrong order silently measures "screen off" only.
        ui.executeShellCommand("dumpsys battery unplug")
        ui.executeShellCommand("dumpsys deviceidle enable all")
        ui.executeShellCommand("input keyevent 223")
        assertTrue("l'écran ne s'est pas éteint", awaitTrue("l'écran éteint", 10_000) { screenAsleep(ui) })
        ui.executeShellCommand("dumpsys deviceidle force-idle")
        assertTrue(
            "le Doze n'est pas forcé : ${dozeState(ui)}",
            awaitTrue("l'idle profond", 15_000) { dozeState(ui).contains("mState=IDLE") },
        )

        val seen = linkedSetOf<String>()
        api.snapshot().current?.let { seen.add(it.key) }
        val deadline = SystemClock.elapsedRealtime() + 34 * 60_000L
        while (seen.size < tracks.size && SystemClock.elapsedRealtime() < deadline) {
            api.snapshot().current?.let { seen.add(it.key) }
            SystemClock.sleep(500)
        }

        val wallMs = SystemClock.elapsedRealtime() - started
        val cpuMs = cpuMillis() - cpuBefore
        val stats = api.outputStats()
        ui.executeShellCommand("dumpsys deviceidle unforce")
        ui.executeShellCommand("dumpsys battery reset")
        ui.executeShellCommand("input keyevent 224")

        // The numbers the PR carries, printed even when everything passes.
        println(
            "MB3C-ENDURANCE titles=${seen.size}/${tracks.size} wall_s=${wallMs / 1000} " +
                "cpu_ms=$cpuMs cpu_pct=${"%.3f".format(cpuMs * 100.0 / wallMs)} " +
                "underruns=${stats.underruns} rebuffers=${stats.rebuffers} " +
                "buffer_ms=${stats.bufferMs} rate=${stats.sampleRate}",
        )

        assertEquals("tous les changements de titre", tracks.size, seen.size)
        assertEquals("sous-alimentations", 0uL, stats.underruns)
        assertEquals("reprises de tampon", 0uL, stats.rebuffers)
        assertTrue("le temps CPU n'est pas mesurable", cpuMs > 0)
    }

    private fun cpuMillis(): Long {
        // The APP's own process (worker rule 4: never a shared test process).
        val stat = java.io.File("/proc/${android.os.Process.myPid()}/stat").readText()
        val fields = stat.substringAfterLast(") ").split(" ")
        val utime = fields[11].toLong()
        val stime = fields[12].toLong()
        return (utime + stime) * 10
    }

    /**
     * The device-idle state, filtered HERE: `executeShellCommand` runs the
     * command directly, with no shell, so a `| grep` in it is "Unknown command".
     */
    private fun dozeState(ui: android.app.UiAutomation): String =
        shellText(ui, "dumpsys deviceidle").lines().firstOrNull { it.contains("mState=") }?.trim()
            ?: "aucun mState"

    private fun screenAsleep(ui: android.app.UiAutomation): Boolean =
        shellText(ui, "dumpsys power").contains("mWakefulness=Asleep")

    private fun connect(): MediaController {
        val token = SessionToken(app, ComponentName(app, NzMediaService::class.java))
        return MediaController.Builder(app, token)
            .setApplicationLooper(Looper.getMainLooper())
            .buildAsync()
            .get(20, TimeUnit.SECONDS)
    }

    /** Every MediaController call comes from its own (main) thread. */
    private fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    /** One shell command's whole output (the descriptor must be closed). */
    private fun shellText(ui: android.app.UiAutomation, command: String): String =
        android.os.ParcelFileDescriptor.AutoCloseInputStream(ui.executeShellCommand(command))
            .use { it.readBytes().toString(Charsets.UTF_8) }

    private fun awaitTrue(what: String, timeoutMs: Long, predicate: () -> Boolean): Boolean {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            if (predicate()) return true
            SystemClock.sleep(25)
        }
        return false
    }

    private fun track(index: Int, seconds: Int): TrackRef =
        TrackRef(
            key = "tone$seconds-$index",
            id = "tone$seconds-$index",
            title = "Titre $index",
            artists = "Artiste $index",
            album = "Album",
            albumId = null,
            artistId = null,
            cover = null,
            durationS = seconds.toUInt(),
            episode = false,
            liked = false,
        )
}
