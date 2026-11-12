package org.nsupysonic.app

import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import kotlin.math.roundToInt

/**
 * When the car mode should show itself, and whether the car is rolling.
 *
 * The prototype's "Déclenchement" settings, with no AccessibilityService and no
 * server call: a navigation app coming to the foreground is read from
 * UsageStatsManager (the user grants usage access in Settings), the car radio is
 * an A2DP/BLE output in AudioManager (no permission), and the speed comes from
 * [CarMotion]. ONE declaration of what `visible` means, used by the car service:
 *
 *     visible = (gpsTrigger && a navigation app is in front)
 *            || (bluetoothTrigger && a Bluetooth sink is connected)
 *
 * So the overlay disappears when the navigation app leaves, unless the person
 * asked Bluetooth to be a trigger too.
 *
 * Foreground-app detection: `queryEvents(now - 5 s, now)` every second, the
 * LAST `ACTIVITY_RESUMED` event wins — sticky across polls, because a navigation
 * app that has been in front for ten minutes emits no new event and must not
 * look gone. A longer one-off seed window at [start] reads the app already in
 * front when the service comes up (the common case: the service starts while
 * Maps is already guiding). `queryEvents` returns null while the device is
 * locked, and `PACKAGE_USAGE_STATS` may not have been granted: both keep the
 * previous state, and the option simply triggers nothing.
 *
 * Audio outputs are listed with `AudioManager.getDevices`; that needs no
 * permission (see [AudioRoute], which already does the same). The device
 * callback is immediate, so a radio connecting shows the overlay without
 * waiting for a poll.
 *
 * The listener is called on the main thread, only when `visible`, `moving` or
 * the whole km/h figure changed. [start]/[stop] are idempotent; values are
 * readable from any thread for the service's [CarHost].
 */
class CarTrigger(
    context: Context,
    private val prefs: CarPrefs,
    private val listener: Listener,
) {

    fun interface Listener {
        /**
         * Main thread. [visible] asks for the overlay; [moving] is the safe
         * answer (unknown counts as moving); [speedKmh] is the last measured
         * speed, 0 until one was measured or when location is not granted.
         */
        fun onDrive(visible: Boolean, moving: Boolean, speedKmh: Double)
    }

    private val app = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val usage = app.getSystemService(Context.USAGE_STATS_SERVICE) as? UsageStatsManager
    private val audio = app.getSystemService(AudioManager::class.java)
    private val power = app.getSystemService(Context.POWER_SERVICE) as? PowerManager
    private val motion = CarMotion(app) { _, _ -> emit() }

    /** Whether a trigger currently wants the overlay on screen. */
    @Volatile
    var visible: Boolean = false
        private set

    /** The last app to come to the foreground is one of [CarPrefs.gpsPackages]. */
    @Volatile
    var gpsInFront: Boolean = false
        private set

    /** An A2DP/BLE output is connected (car radio, headset, hearing aid). */
    @Volatile
    var bluetoothSink: Boolean = false
        private set

    /** For the settings screen: the package that last came forward, ours included. */
    @Volatile
    var lastForegroundPackage: String? = null
        private set

    /** The safe "is the car rolling" answer (see [CarMotion]). */
    val moving: Boolean get() = motion.moving

    /** The last measured speed in km/h, 0 when none. */
    val speedKmh: Double get() = motion.speedKmh

    private var started = false
    private var generation = 0
    private var worker: ExecutorService? = null
    // Main-thread only: one usage query at a time, so a slow query cannot pile
    // up behind the 1 s poll.
    private var querying = false
    private var seeded = false
    private var gpsReady = false
    private var lastEventTs = 0L
    private var emittedOnce = false
    private var lastVisible = false
    private var lastMoving = true
    private var lastSpeed = 0.0

    private val tick = object : Runnable {
        override fun run() {
            if (!started) return
            poll()
            main.postDelayed(this, POLL_MS)
        }
    }

    /** Start watching; safe to call twice. Callbacks start on the main thread. */
    fun start() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { start() }
            return
        }
        if (started) return
        started = true
        generation++
        querying = false
        seeded = false
        gpsReady = false
        lastEventTs = 0L
        lastForegroundPackage = null
        gpsInFront = false
        visible = false
        emittedOnce = false
        lastVisible = false
        lastMoving = true
        lastSpeed = 0.0
        worker = Executors.newSingleThreadExecutor { runnable ->
            Thread(runnable, "car-trigger").apply { isDaemon = true }
        }
        try {
            audio?.registerAudioDeviceCallback(deviceCallback, main)
        } catch (_: Exception) {
            // A device manager that refuses the callback leaves only the poll:
            // the Bluetooth state is still read at every [refresh].
        }
        bluetoothSink = readBluetoothSink()
        motion.start()
        emit() // CarMotion emits too; this covers a start with nothing to say
        main.post(tick)
    }

    /** Stop watching; safe to call twice. No callback fires after this. */
    fun stop() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { stop() }
            return
        }
        if (!started) return
        started = false
        generation++
        querying = false
        main.removeCallbacksAndMessages(null)
        try {
            audio?.unregisterAudioDeviceCallback(deviceCallback)
        } catch (_: Exception) {
        }
        worker?.shutdownNow()
        worker = null
        motion.stop()
        visible = false
        gpsInFront = false
        bluetoothSink = false
    }

    /**
     * Re-read the settings' triggers (the settings screen changed one) and the
     * Bluetooth outputs right away, instead of waiting for the next fix.
     */
    fun refresh() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { refresh() }
            return
        }
        if (!started) return
        bluetoothSink = readBluetoothSink()
        // The settings screen may have added or removed the app that is in
        // front right now, and that app emits no new event for us.
        lastForegroundPackage?.let { pkg -> gpsInFront = prefs.gpsPackages.contains(pkg) }
        emit()
    }

    private fun poll() {
        val ready = prefs.gpsTrigger && CarPermissions.hasUsageStats(app)
        if (!ready) {
            // No option or no usage access: no detection, whatever the last
            // value was, and the next enabling starts from a fresh seed.
            gpsReady = false
            gpsInFront = false
        }
        // Option changes take effect within a second even when no location fix
        // or device event happens (the settings screen also calls refresh()).
        emit()
        if (!ready) return
        if (!gpsReady) {
            gpsReady = true
            lastEventTs = 0L
            seeded = false
        }
        // With the screen off no app can come to the foreground: keep the last
        // state instead of waking the usage service every second. The first
        // poll after the screen comes back catches up.
        if (power?.isInteractive == false) return
        if (querying) return

        val now = System.currentTimeMillis()
        val seed = !seeded
        val begin = if (seed) now - SEED_MS else maxOf(now - WINDOW_MS, minOf(lastEventTs, now))
        seeded = true
        querying = true
        val gen = generation
        val executor = worker ?: run {
            querying = false
            return
        }
        executor.execute {
            val result = try {
                queryGps(now, begin)
            } catch (_: Throwable) {
                null
            }
            main.post {
                if (!started || gen != generation) return@post
                querying = false
                if (result == null) {
                    // The seed is the only way to learn about an app already in
                    // front: retry it rather than settle for 5 s windows.
                    if (seed) seeded = false
                    return@post
                }
                if (!result.seen) return@post
                if (result.lastTs > lastEventTs) lastEventTs = result.lastTs
                lastForegroundPackage = result.pkg
                applyGps(result.found)
            }
        }
    }

    /**
     * The last foreground activity in the window. Runs on the polling worker:
     * `queryEvents` is a system-service call and must not be on the main thread
     * that draws the car overlay.
     */
    @Suppress("DEPRECATION")
    private fun queryGps(now: Long, begin: Long): GpsQuery? {
        val manager = usage ?: return null
        val events = try {
            manager.queryEvents(begin, now)
        } catch (_: Exception) {
            null
        } ?: return null
        val event = UsageEvents.Event()
        var ts = begin
        var pkg: String? = null
        var seen = false
        while (events.hasNextEvent()) {
            events.getNextEvent(event)
            // ACTIVITY_RESUMED (API 29) is MOVE_TO_FOREGROUND's new name and
            // the same value (1): the API-21 constant reads both event types
            // without requiring API 29 on a minSdk 26 app.
            if (event.eventType != UsageEvents.Event.MOVE_TO_FOREGROUND) continue
            if (event.timeStamp < ts) continue
            ts = event.timeStamp
            pkg = event.packageName
            seen = true
        }
        if (!seen) return GpsQuery(false, begin, null, false)
        val packages = prefs.gpsPackages
        val front = pkg
        return GpsQuery(front != null && packages.contains(front), ts, front, true)
    }

    private fun applyGps(found: Boolean) {
        if (gpsInFront == found) return
        gpsInFront = found
        emit()
    }

    /**
     * ONE definition of the overlay's visibility. Called on the main thread
     * only; [lastSpeed] is compared as a whole km/h because that is what the
     * car UI shows.
     */
    private fun emit() {
        if (!started) return
        val visible = (prefs.gpsTrigger && gpsInFront) ||
            (prefs.bluetoothTrigger && bluetoothSink)
        val moving = motion.moving
        val speed = motion.speedKmh
        this.visible = visible
        if (emittedOnce && visible == lastVisible && moving == lastMoving &&
            speed.roundToInt() == lastSpeed.roundToInt()
        ) return
        emittedOnce = true
        lastVisible = visible
        lastMoving = moving
        lastSpeed = speed
        listener.onDrive(visible, moving, speed)
    }

    private fun readBluetoothSink(): Boolean {
        val devices = try {
            audio?.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
        } catch (_: Exception) {
            null
        } ?: return false
        return devices.any { isBluetoothSink(it.type) }
    }

    /**
     * A2DP and BLE outputs are the car-radio case. A headset also counts — the
     * platform cannot tell a headset from a car stereo, and the person's
     * `bluetoothTrigger` switch is the answer for anyone who does not want it.
     */
    private fun isBluetoothSink(type: Int): Boolean = when (type) {
        AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
        AudioDeviceInfo.TYPE_BLE_HEADSET,
        AudioDeviceInfo.TYPE_BLE_SPEAKER,
        AudioDeviceInfo.TYPE_BLE_BROADCAST,
        AudioDeviceInfo.TYPE_HEARING_AID -> true
        else -> false
    }

    private val deviceCallback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) = devicesChanged()

        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) = devicesChanged()
    }

    private fun devicesChanged() {
        if (!started) return
        val sink = readBluetoothSink()
        if (sink == bluetoothSink) return
        bluetoothSink = sink
        emit()
    }

    private data class GpsQuery(
        /** The newest foreground app is a navigation app. */
        val found: Boolean,
        /** Timestamp of that event; the next window starts there or 5 s back. */
        val lastTs: Long,
        /** The newest foreground package, ours and the launcher included. */
        val pkg: String?,
        /** False when the window held no foreground event at all. */
        val seen: Boolean,
    )

    companion object {
        /** How often the foreground app is looked at. */
        const val POLL_MS = 1_000L

        /** The usage-events window of every poll (the map's ~5 s). */
        const val WINDOW_MS = 5_000L

        /**
         * One longer look at start: a navigation app already guiding when the
         * service comes up will emit no new event just for us.
         */
        const val SEED_MS = 6 * 60 * 60 * 1000L
    }
}
