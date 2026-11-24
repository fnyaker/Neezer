package fr.neezer.mobile.baselineprofile

import android.os.SystemClock
import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.JUnit4

/**
 * The locked-screen regression MQ2 found: the queue stopped after a few titles
 * because nothing ever made the media service a FOREGROUND service.
 *
 * The app's screens speak the Rust `PlayerApi` directly, so no `MediaController`
 * of its own ever bound to `NzMediaService`; Media3 only adds a session when a
 * controller binds, so `MediaNotificationManager` never saw ours — no
 * notification, no `startForeground`, and Android stopped the service as an
 * idle background service after a couple of minutes of PLAYING
 * (`Stopping service due to app idle`, `startForegroundCount=0`, measured on
 * emulator-5660). The fix is `addSession` in the service; this test is the
 * regression it earns.
 *
 * It needs the bench (`mobile/tools/perf-server.sh` and a signed-in account), so
 * a machine without one SKIPS with its cause instead of failing the emulator job
 * (MQ2's own rule).
 */
@RunWith(JUnit4::class)
class ScreenOffChainTest {

    private val scope = MacrobenchmarkScope(BenchTarget.PACKAGE_NAME, false)
    private val device: UiDevice get() = scope.device

    /** The service's own record, or "nothing". */
    private fun serviceDump(): String =
        device.executeShellCommand("dumpsys activity services ${BenchTarget.PACKAGE_NAME}")

    /**
     * The media session's `active item id`, which is the queue index: it moves
     * on every handover and never goes back, so a distinct set of them is a
     * count of rows played (MQ2 counted descriptions, which the seeded titles
     * repeat).
     */
    private fun activeItem(): Int? {
        val dump = device.executeShellCommand("dumpsys media_session")
        val block = dump.split(Regex("\n    (?=\\S)"))
            .firstOrNull { it.contains("package=${BenchTarget.PACKAGE_NAME}") }
            ?: return null
        return Regex("active item id=(-?\\d+)").find(block)?.groupValues?.get(1)?.toIntOrNull()
    }

    /** The app's PID, or nothing when the process is gone. */
    private fun appPid(): String =
        device.executeShellCommand("pidof ${BenchTarget.PACKAGE_NAME}")
            .trim()
            .split(" ")
            .firstOrNull()
            .orEmpty()

    private fun alive(pid: String): Boolean = appPid() == pid

    private fun awaitTrue(what: String, timeoutMs: Long, predicate: () -> Boolean): Boolean {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            if (predicate()) return true
            SystemClock.sleep(250)
        }
        return false
    }

    /**
     * Library → Playlists → the named playlist, opened with the FILTER but with
     * the section expanded AFTER the query.
     *
     * `Journey.openPlaylist` expands « Sans dossier » before typing; on this app
     * the section folds again when the query changes (measured: the filter held
     * the name and the tree showed « 60 playlists » with no card under it), and
     * its `awaitCard` then scrolls an empty list for 20 s. Expanding after the
     * query is the whole difference, and it does not touch MQ2's journey.
     */
    /**
     * A node re-found on every attempt: the screen recomposes under the node
     * between the find and the tap and the old one goes stale (the MQ2 note's
     * own lesson, met again here).
     */
    private fun tap(selector: androidx.test.uiautomator.BySelector, what: String) {
        repeat(4) {
            val node = device.wait(Until.findObject(selector), 20_000)
            check(node != null) { "no $what on this screen" }
            if (runCatching { node.click() }.isSuccess) {
                device.waitForIdle()
                return
            }
            device.waitForIdle()
        }
        error("the node $what kept going stale")
    }

    private fun openPlaylist(name: String) {
        Journey.ensureLibrary(device)
        Journey.tapText(device, Journey.LEAF_PLAYLISTS)
        tap(By.clazz("android.widget.EditText"), "filter field")
        val claimed = SystemClock.uptimeMillis() + 3_000
        while (SystemClock.uptimeMillis() < claimed) {
            device.executeShellCommand("input text ${name.replace(" ", "%s")}")
            SystemClock.sleep(300)
            // The node goes STALE when the screen recomposes under it (measured
            // on this filter): re-find it for the proof, never reuse it.
            val typed =
                runCatching {
                    device.findObject(By.clazz("android.widget.EditText"))?.text ?: ""
                }.getOrDefault("")
            if (typed.contains(name, ignoreCase = true)) break
        }
        device.pressBack()
        device.waitForIdle()
        if (device.findObject(By.textStartsWith("Playlist ")) == null) {
            val head = Journey.awaitText(device, "Sans dossier")
            val b = runCatching { head.visibleBounds }.getOrNull()
            if (b != null) {
                device.click((b.left - 60).coerceAtLeast(8), b.centerY())
                device.waitForIdle()
            }
        }
        var opened = false
        repeat(3) {
            if (!opened) {
                val card = Journey.awaitCard(device, name)
                opened = runCatching { card.click() }.isSuccess
                device.waitForIdle()
            }
        }
        check(opened) { "the playlist card $name never opened" }
        Journey.awaitText(device, "Lire", 30_000)
    }

    @Test
    fun theQueueChainsBehindALockedScreenBecauseTheServiceIsForeground() {
        Journey.restart(scope)
        openPlaylist(Journey.DOWNLOAD_PLAYLIST)
        Journey.tapText(device, "Lire")
        Journey.awaitDesc(device, "Pause", 30_000)

        // 1. While PLAYING, the media service must be a foreground service.
        //    Without `addSession` this never happens: no controller binds, so
        //    Media3 never posts a notification and never calls startForeground.
        val foreground =
            awaitTrue("the media service in the foreground", 10_000) {
                serviceDump().contains("isForeground=true")
            }
        assertTrue(
            "the media service never became a foreground service while playing:\n${serviceDump()}",
            foreground,
        )

        val pid = appPid()
        assertTrue("the app has no PID", pid.isNotEmpty())
        device.executeShellCommand("logcat -c")

        // 2. Lock the screen and idle the device: the service must survive and
        //    the queue must hand over at least three titles (three files of the
        //    bench, 45 s each on the server MQ2 pinned) — the defect froze on the
        //    first or second handover after the idle stop.
        device.executeShellCommand("dumpsys battery unplug")
        device.executeShellCommand("dumpsys deviceidle enable all")
        device.executeShellCommand("input keyevent KEYCODE_SLEEP")
        device.executeShellCommand("dumpsys deviceidle force-idle")
        assertTrue(
            "le Doze n'est pas forcé",
            awaitTrue("the deep idle", 15_000) {
                device.executeShellCommand("dumpsys deviceidle")
                    .lines()
                    .any { it.contains("mState=IDLE") }
            },
        )

        val before = activeItem()
        val seen = linkedSetOf<Int>()
        before?.let { seen.add(it) }
        val deadline = SystemClock.uptimeMillis() + 4 * 60_000L
        while (seen.size < 3 && SystemClock.uptimeMillis() < deadline) {
            activeItem()?.let { seen.add(it) }
            SystemClock.sleep(500)
        }
        val dump = serviceDump()
        val log = device.executeShellCommand("logcat -d -s ActivityManager")
        device.executeShellCommand("dumpsys deviceidle unforce")
        device.executeShellCommand("input keyevent KEYCODE_WAKEUP")

        println(
            "MEASURE écran éteint + Doze : ${seen.size} titres distincts ${seen.sorted()}, " +
                "pid $pid vivant=${alive(pid)}, service=${dump.contains("isForeground=true")}",
        )
        assertEquals(
            "the queue did not chain three titles behind the locked screen",
            3,
            seen.size,
        )
        assertTrue("the app's process died behind the locked screen", alive(pid))
        assertFalse(
            "Android stopped the media service as an idle background service:\n$log",
            log.contains("Stopping service due to app idle"),
        )
    }
}
