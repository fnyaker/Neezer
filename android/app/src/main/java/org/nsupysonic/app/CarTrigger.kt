package org.nsupysonic.app

import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import kotlin.math.roundToInt

/**
 * How one foreground package bears on the GPS decision.
 *
 * SYSTEM (the notification shade, a permission dialog) is drawn OVER the
 * navigation app: it must not change the answer, because closing it emits no
 * new ACTIVITY_RESUMED and the card would stay hidden. SELF, LAUNCHER and OTHER
 * are real departures: they get [CarTrigger.GRACE_MS] before the card goes, so
 * a home animation or the settings activity opening does not make it blink.
 */
enum class ForegroundKind { GPS, OTHER, SYSTEM, SELF, LAUNCHER, PERMISSION_CONTROLLER }

/**
 * The sticky "is a navigation app in front" answer, with the transient grace.
 * No Android type here: `CarForegroundTest` drives it on the JVM.
 *
 * The GPS app stays in front across polls (it emits no new event for ten
 * minutes), so [gpsInFront] is sticky. A NON-GPS app schedules the hide at
 * `event + grace`; if the person comes back within the grace (the shade, a
 * permission dialog, a mis-tap), nothing changes. A SYSTEM event is ignored
 * outright. [settle] applies the deadline on the 1 s poll, which bounds the
 * hide at grace + 1 s.
 */
class CarForegroundTracker(
    private val graceMs: Long = CarTrigger.GRACE_MS,
) {
    /** A configured navigation app is the last meaningful foreground app. */
    var gpsInFront: Boolean = false
        private set

    /** That app's package, so its own saved position is restored. */
    var gpsPackage: String? = null
        private set

    /** A foreground answer exists (a query came back, even an empty seed). */
    var decided: Boolean = false
        private set

    /** `event + grace`; null while there is nothing to hide. */
    private var hideAt: Long? = null

    /** A navigation app took over (or is still there): visible at once. */
    fun gps(pkg: String): Boolean {
        decided = true
        hideAt = null
        if (gpsInFront && gpsPackage == pkg) return false
        gpsInFront = true
        gpsPackage = pkg
        return true
    }

    /** Another app took the foreground: hide after the grace, not before. */
    fun other(at: Long): Boolean {
        decided = true
        if (!gpsInFront) {
            hideAt = null
            return false
        }
        if (hideAt == null) hideAt = at + graceMs
        return false
    }

    /** The system chrome/dialog: never changes the answer. */
    fun system(): Boolean = false

    /** Our own screen, the launcher, a runtime permission dialog. */
    fun transient(at: Long): Boolean = other(at)

    /** The hide deadline, applied by the poll. */
    fun settle(now: Long): Boolean {
        val at = hideAt ?: return false
        if (now < at) return false
        hideAt = null
        if (!gpsInFront) return false
        gpsInFront = false
        gpsPackage = null
        return true
    }

    /** The GPS app is gone but the card must go now (returning from settings). */
    fun hideNow(): Boolean {
        decided = true
        hideAt = null
        if (!gpsInFront) return false
        gpsInFront = false
        gpsPackage = null
        return true
    }

    /**
     * A six-hour seed with no meaningful foreground event at all: there is no
     * navigation app to be in front, and a manual card must not wait forever.
     */
    fun unknown(): Boolean {
        decided = true
        hideAt = null
        val changed = gpsInFront || gpsPackage != null
        gpsInFront = false
        gpsPackage = null
        return changed
    }

    /** The usage access was revoked (or the GPS switch turned off). */
    fun reset() {
        decided = false
        hideAt = null
        gpsInFront = false
        gpsPackage = null
    }
}

/**
 * When the car mode should show itself, and whether the car is rolling.
 *
 * The prototype's "Déclenchement" settings, with no AccessibilityService and no
 * server call: a navigation app coming to the foreground is read from
 * UsageStatsManager (the user grants usage access in Settings), the car radio is
 * an A2DP/BLE output in AudioManager (no permission), and the speed comes from
 * [CarMotion]. [CarVisibility] owns what `visible` means; this class only
 * decides which app is in front and for how long after it leaves.
 *
 * Foreground-app detection: `queryEvents(now - 5 s, now)` every second, the
 * LAST `ACTIVITY_RESUMED` event wins — sticky across polls, because a navigation
 * app that has been in front for ten minutes emits no new event and must not
 * look gone. A longer one-off seed window at [start] reads the app already in
 * front when the service comes up (the common case: the service starts while
 * Maps is already guiding); the seed skips the system chrome, which is drawn
 * over the map rather than replacing it. `queryEvents` returns null while the
 * device is locked, and `PACKAGE_USAGE_STATS` may not have been granted: both
 * keep the previous state, and the option simply triggers nothing.
 *
 * Transient packages: SystemUI and permission controllers are ignored outright
 * (the shade opening must not hide the card); our own screens and the launcher
 * hide it after [GRACE_MS], so a home animation or returning from the car
 * settings does not blink. A real app (YouTube, Messages) hides it after the
 * same grace.
 *
 * Audio outputs are listed with `AudioManager.getDevices`; that needs no
 * permission (see [AudioRoute], which already does the same). The device
 * callback is immediate, so a radio connecting shows the overlay without
 * waiting for a poll.
 *
 * The listener is called on the main thread, only when the answer changed
 * ([DriveState] equality, speed compared as a whole km/h). [start]/[stop] are
 * idempotent; values are readable from any thread for the service's [CarHost].
 */
class CarTrigger(
    context: Context,
    private val prefs: CarPrefs,
    private val listener: Listener,
) {

    fun interface Listener {
        /**
         * Main thread. [DriveState.visible] asks for the overlay;
         * [DriveState.moving] is the safe answer (unknown counts as moving);
         * [DriveState.gpsPackage] is the navigation app in front, so the
         * service can restore that app's position.
         */
        fun onDrive(state: DriveState)
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

    /** The configured navigation app currently in front, null when none. */
    @Volatile
    var gpsPackage: String? = null
        private set

    /** The last app to come to the foreground is one of [CarPrefs.gpsPackages]. */
    @Volatile
    var gpsInFront: Boolean = false
        private set

    /** An A2DP/BLE output is connected (car radio, headset, hearing aid). */
    @Volatile
    var bluetoothSink: Boolean = false
        private set

    /** For diagnostics/settings: the package that last came forward, ours included. */
    @Volatile
    var lastForegroundPackage: String? = null
        private set

    /** The safe "is the car rolling" answer (see [CarMotion]). */
    val moving: Boolean get() = motion.moving

    /** The last measured speed in km/h, 0 when none. */
    val speedKmh: Double get() = motion.speedKmh

    private val tracker = CarForegroundTracker()
    private var started = false
    private var generation = 0
    private var worker: ExecutorService? = null
    // Main-thread only: one usage query at a time, so a slow query cannot pile
    // up behind the 1 s poll.
    private var querying = false
    private var seeded = false
    private var gpsReady = false
    private var usageUsable = false
    private var lastEventTs = 0L
    private var lastState: DriveState? = null

    /**
     * The default launchers, plus every home activity the package manager
     * reports. The manifest declares the HOME intent query, so Android 11+
     * package visibility does not hide them. A custom launcher the person
     * installed is covered by the query; this list is the belt-and-braces one.
     */
    @Volatile
    private var homePackages: Set<String> = KNOWN_LAUNCHERS

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
        gpsPackage = null
        visible = false
        lastState = null
        tracker.reset()
        homePackages = readHomePackages()
        usageUsable = prefs.gpsTrigger && CarPermissions.hasUsageStats(app)
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
        gpsPackage = null
        bluetoothSink = false
        tracker.reset()
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
        usageUsable = prefs.gpsTrigger && CarPermissions.hasUsageStats(app)
        bluetoothSink = readBluetoothSink()
        // The settings screen may have added or removed the app that is in
        // front right now, and that app emits no new event for us. A GPS app
        // becomes visible at once; anything else (our own settings screen, the
        // launcher) hides now: the screen that was on top is going away, and a
        // grace here would paint the card over the app the person just opened.
        val pkg = lastForegroundPackage
        if (pkg != null) {
            when (kindOf(pkg)) {
                ForegroundKind.GPS -> tracker.gps(pkg)
                ForegroundKind.SYSTEM -> tracker.system()
                else -> tracker.hideNow()
            }
        }
        tracker.settle(System.currentTimeMillis())
        emit()
    }

    private fun poll() {
        if (!started) return
        usageUsable = prefs.gpsTrigger && CarPermissions.hasUsageStats(app)
        val ready = usageUsable
        if (!ready) {
            // No option or no usage access: no detection, whatever the last
            // value was, and the next enabling starts from a fresh seed.
            gpsReady = false
            tracker.reset()
        }
        tracker.settle(System.currentTimeMillis())
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
        val begin = if (seed) now - SEED_MS
        else maxOf(now - WINDOW_MS, minOf(lastEventTs, now))
        seeded = true
        querying = true
        val gen = generation
        val executor = worker ?: run {
            querying = false
            return
        }
        executor.execute {
            val result = try {
                queryForeground(now, begin)
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
                if (!result.seen) {
                    if (seed) {
                        // Six hours with no meaningful foreground event: there
                        // is no navigation app in front, and the service must
                        // not leave a manual card on whatever is.
                        lastForegroundPackage = null
                        tracker.unknown()
                        emit()
                    }
                    return@post
                }
                if (result.lastTs > lastEventTs) lastEventTs = result.lastTs
                lastForegroundPackage = result.pkg
                val pkg = result.pkg ?: return@post
                when (result.kind) {
                    ForegroundKind.GPS -> tracker.gps(pkg)
                    ForegroundKind.SYSTEM -> tracker.system()
                    else -> tracker.transient(result.at)
                }
                tracker.settle(System.currentTimeMillis())
                emit()
            }
        }
    }

    /**
     * The last MEANINGFUL foreground activity in the window, skipping the
     * system chrome (the notification shade is drawn over the map). Runs on the
     * polling worker: `queryEvents` is a system-service call and must not be on
     * the main thread that draws the car overlay.
     */
    @Suppress("DEPRECATION")
    private fun queryForeground(now: Long, begin: Long): GpsQuery? {
        val manager = usage ?: return null
        val from = begin.coerceAtMost(now - 1)
        val events = try {
            manager.queryEvents(from, now)
        } catch (_: Exception) {
            null
        } ?: return null
        val event = UsageEvents.Event()
        var ts = from
        var pkg: String? = null
        var kind = ForegroundKind.OTHER
        var lastTs = from
        var seen = false
        while (events.hasNextEvent()) {
            events.getNextEvent(event)
            if (event.timeStamp > lastTs) lastTs = event.timeStamp
            // ACTIVITY_RESUMED (API 29) is MOVE_TO_FOREGROUND's new name and
            // the same value (1): the API-21 constant reads both event types
            // without requiring API 29 on a minSdk 26 app.
            if (event.eventType != UsageEvents.Event.MOVE_TO_FOREGROUND) continue
            if (event.timeStamp < ts) continue
            val name = event.packageName ?: continue
            val what = kindOf(name)
            if (what == ForegroundKind.SYSTEM) continue
            ts = event.timeStamp
            pkg = name
            kind = what
            seen = true
        }
        if (!seen || pkg == null) return GpsQuery(null, kind, from, lastTs, false)
        return GpsQuery(pkg, kind, ts, lastTs, true)
    }

    /** Which kind of app [pkg] is, using the person's own GPS list first. */
    private fun kindOf(pkg: String): ForegroundKind {
        if (prefs.gpsPackages.contains(pkg)) return ForegroundKind.GPS
        if (pkg in SYSTEM_PACKAGES || pkg in PERMISSION_PACKAGES) {
            return ForegroundKind.SYSTEM
        }
        if (pkg == app.packageName) return ForegroundKind.SELF
        if (pkg in homePackages || pkg in KNOWN_LAUNCHERS) return ForegroundKind.LAUNCHER
        return ForegroundKind.OTHER
    }

    /**
     * ONE definition of the overlay's visibility, through [CarVisibility].
     * Called on the main thread only; the whole km/h is compared because that
     * is what the car UI shows.
     */
    private fun emit() {
        if (!started) return
        val speed = motion.speedKmh
        val decided = if (prefs.gpsTrigger && usageUsable) tracker.decided else true
        val visible = CarVisibility.wanted(
            gpsTrigger = prefs.gpsTrigger,
            bluetoothTrigger = prefs.bluetoothTrigger,
            gpsUsable = usageUsable,
            gpsInFront = tracker.gpsInFront,
            bluetoothSink = bluetoothSink,
        )
        this.visible = visible
        gpsInFront = tracker.gpsInFront
        gpsPackage = tracker.gpsPackage
        val state = DriveState(
            visible = visible,
            moving = motion.moving,
            speedKmh = speed,
            gpsPackage = tracker.gpsPackage,
            gpsInFront = tracker.gpsInFront,
            bluetoothSink = bluetoothSink,
            decided = decided,
        )
        val old = lastState
        if (old != null && old.copy(speedKmh = 0.0) == state.copy(speedKmh = 0.0) &&
            old.speedKmh.roundToInt() == speed.roundToInt()
        ) return
        lastState = state
        listener.onDrive(state)
    }

    /** The home activities, resolved once per trigger start. */
    private fun readHomePackages(): Set<String> {
        val out = linkedSetOf<String>()
        out += KNOWN_LAUNCHERS
        val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        try {
            val pm = app.packageManager
            val list = if (Build.VERSION.SDK_INT >= 33) {
                pm.queryIntentActivities(intent, PackageManager.ResolveInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                pm.queryIntentActivities(intent, 0)
            }
            for (info in list) {
                info.activityInfo?.packageName?.takeIf { it.isNotBlank() }?.let(out::add)
            }
        } catch (_: Exception) {
            // An OEM package manager that refuses the query leaves the known
            // names: the decision just treats an unknown launcher as another app.
        }
        return out
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
        /** The newest meaningful foreground package, null when none. */
        val pkg: String?,
        /** Its kind; ignored when [seen] is false. */
        val kind: ForegroundKind,
        /** Timestamp of that event (the grace starts there, not at now). */
        val at: Long,
        /** Newest event of any type in the window; the next window starts there. */
        val lastTs: Long,
        /** False when the window held no non-system foreground event. */
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

        /**
         * How long a non-GPS app must stay in front before the card goes. The
         * 1 s poll bounds the real hide at 2.0–3.0 s, which absorbs the home
         * animation, a notification-shade transition and the settings activity
         * opening without ever leaving the card over YouTube.
         */
        const val GRACE_MS = 2_000L

        /** Drawn over the navigation app, never a reason to hide it. */
        private val SYSTEM_PACKAGES = setOf(
            "com.android.systemui",
        )

        /** Runtime-permission dialogs: also drawn over the app they ask for. */
        private val PERMISSION_PACKAGES = setOf(
            "com.android.permissioncontroller",
            "com.google.android.permissioncontroller",
            "com.android.packageinstaller",
            "com.google.android.packageinstaller",
        )

        /** Launchers seen on the phones this app targets; the HOME query adds the rest. */
        private val KNOWN_LAUNCHERS = setOf(
            "com.android.launcher",
            "com.android.launcher3",
            "com.google.android.apps.nexuslauncher",
            "com.google.android.apps.launcher",
            "com.miui.home",
            "com.sec.android.app.launcher",
            "com.oneplus.launcher",
            "com.oppo.launcher",
            "com.vivo.launcher",
            "com.huawei.android.launcher",
            "com.teslacoilsw.launcher",
            "com.microsoft.launcher",
        )
    }
}
