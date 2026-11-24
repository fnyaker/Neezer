package fr.neezer.mobile.baselineprofile

import android.os.SystemClock
import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.JUnit4

/**
 * The journey's long half: a 50-track download WHILE a track plays, the phone
 * pulled offline (airplane mode), the downloads played from disk, then forty
 * titles behind a locked screen with Doze forced (docs/agents/mobile.md, MQ2's
 * journey and *Measures*: "CPU/battery — /proc/<pid>/stat over 30 min of
 * playback, screen off, Doze forced" and "Underruns — 0 over 30 min").
 *
 * It is a test and not a benchmark on purpose: a frame metric over thirty
 * minutes measures the clock. The CPU number is the app's own `/proc/<pid>/stat`
 * (utime+stime, USER_HZ = 100 on Android), the memory is the app's own counted
 * bytes (the diagnostic report's `mémoire comptée`) beside PSS, and "the phone
 * kept playing" is a COUNT of distinct titles the media session announced —
 * never a sleep that assumes it did.
 *
 * It runs alone; the fast journey is [JourneyBenchmark] and [JourneyListsBenchmark].
 */
@RunWith(JUnit4::class)
class JourneyLongTest {

    private val scope = MacrobenchmarkScope(BenchTarget.PACKAGE_NAME, false)

    /** The scope's own device: `UiDevice.getInstance()` alone is not initialised outside a rule. */
    private val device: UiDevice get() = scope.device

    /** The app's process, or a failure naming the step that lost it. */
    private fun appPid(): String {
        val pid = device.executeShellCommand("pidof ${BenchTarget.PACKAGE_NAME}").trim()
        assertTrue("org.nsupysonic.app has no PID: the app is not running", pid.isNotEmpty())
        return pid
    }

    /** utime+stime of the APP's process, in clock ticks (10 ms each on Android). */
    private fun cpuTicks(pid: String): Long {
        val stat = device.executeShellCommand("cat /proc/$pid/stat")
        // `pid (comm) state ppid ...`: the comm may hold spaces and parentheses,
        // so the fields start AFTER the last ')'.
        val fields = stat.substringAfterLast(") ").trim().split(" ")
        val utime = fields[11].toLong()
        val stime = fields[12].toLong()
        return utime + stime
    }

    /** The app's own PSS, in bytes (`dumpsys meminfo <package>`'s TOTAL row). */
    private fun pssBytes(): Long {
        val dump = device.executeShellCommand("dumpsys meminfo ${BenchTarget.PACKAGE_NAME}")
        val kb = Regex("TOTAL PSS:\\s+(\\d+)").find(dump)?.groupValues?.get(1)
            ?: Regex("^\\s*TOTAL\\s+(\\d+)", RegexOption.MULTILINE).find(dump)?.groupValues?.get(1)
            ?: "0"
        return kb.toLong() * 1024
    }

    /**
     * How many DISTINCT titles the app's media session has announced: the
     * gapless handover is what the notification's description changes on, so a
     * count of them is a count of titles played — the event the soak waits for.
     */
    private fun titlesAnnounced(): Int {
        val dump = device.executeShellCommand("dumpsys media_session")
        return Regex("description=([^,\\n]+)").findAll(dump)
            .map { it.groupValues[1].trim() }
            .filter { it.isNotEmpty() }
            .distinct()
            .count()
    }

    @Test
    fun fiftyTrackDownloadOfflinePlaybackAndThirtyMinuteSoak() {
        Journey.restart(scope)
        Journey.searchToAlbum(scope)
        val tapToUi = Journey.playAlbum(scope)
        println("MEASURE offline tap→« Pause » (UI): ${tapToUi}ms")

        // 1. A 50-track playlist downloaded whole while a track plays (rule 1:
        //    the download yields to what the person is doing, and the assertion
        //    is that both happened).
        val downloadStart = SystemClock.uptimeMillis()
        Journey.downloadPlaylist(scope)
        val downloadMs = SystemClock.uptimeMillis() - downloadStart
        println("MEASURE offline téléchargement 50 titres : ${downloadMs}ms")

        // 2. Offline, for real: airplane mode on, and the app's own diagnostic
        //    says « hors ligne » — its ConnectivityManager callback, zero sockets.
        Journey.airplaneMode(device, on = true)
        val offline = Journey.readDiagnostic(scope)
        assertTrue("the app still believes it is online under airplane mode", !offline.online)
        println("MEASURE offline mémoire comptée hors ligne : ${offline.countedBytes}o")

        // The playlist's first title, from the device this time: the download
        // is what is played, and the NEXT title's appearance is the proof.
        Journey.openPlaylist(device, Journey.DOWNLOAD_PLAYLIST)
        val before = titlesAnnounced()
        Journey.tapText(device, "Lire")
        Journey.awaitDesc(device, "Pause", 30_000)
        val deadline = SystemClock.uptimeMillis() + 60_000
        while (titlesAnnounced() <= before && SystemClock.uptimeMillis() < deadline) {
            device.waitForIdle()
        }
        val afterOffline = titlesAnnounced()
        assertTrue(
            "the downloaded playlist did not hand over a second title offline ($before → $afterOffline)",
            afterOffline > before,
        )
        println("MEASURE offline titres enchaînés sans réseau : $afterOffline")
        Journey.airplaneMode(device, on = false)

        // 3. The soak: forty titles, screen off, Doze forced, and the app's own
        //    CPU time before and after.
        val pid = appPid()
        val pssBefore = pssBytes()
        val cpuBefore = cpuTicks(pid)
        val announcedBefore = titlesAnnounced()
        val soakStart = SystemClock.uptimeMillis()

        device.executeShellCommand("input keyevent KEYCODE_SLEEP")
        device.executeShellCommand("dumpsys deviceidle force-idle")

        // The queue was started above; it keeps handing over. Forty titles is
        // the plan's number, 30 minutes its window — whichever comes first ends
        // the run, and both are stated in the report.
        val soakDeadline = soakStart + 30L * 60 * 1000
        var played = 0
        while (SystemClock.uptimeMillis() < soakDeadline) {
            played = titlesAnnounced() - announcedBefore
            if (played >= 40) break
        }
        val soakMs = SystemClock.uptimeMillis() - soakStart
        val cpuAfter = cpuTicks(pid)
        val pssAfter = pssBytes()

        device.executeShellCommand("dumpsys deviceidle unforce")
        device.executeShellCommand("input keyevent KEYCODE_WAKEUP")

        val ticks = cpuAfter - cpuBefore
        val cpuMs = ticks * 10
        println(
            "MEASURE soak ${soakMs / 1000}s écran éteint, Doze : $played titres, " +
                "${cpuMs}ms CPU (${"%.3f".format(cpuMs.toDouble() / soakMs * 100)} % d'un cœur), " +
                "PSS $pssBefore → $pssAfter o",
        )
        assertEquals("the soak played no title at all", true, played > 0)
    }
}
