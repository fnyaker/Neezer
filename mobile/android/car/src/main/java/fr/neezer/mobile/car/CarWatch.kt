package fr.neezer.mobile.car

import android.app.AlarmManager
import android.app.PendingIntent
import android.app.job.JobInfo
import android.app.job.JobScheduler
import android.app.usage.UsageEvents
import android.app.usage.UsageStatsManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.os.BatteryManager
import android.os.PowerManager
import android.os.SystemClock
import java.util.concurrent.Executors

/**
 * What starts the car mode when nobody is looking, without a service of its own:
 * the Bluetooth and boot broadcasts, the wake-up ladder armed after a drive (or
 * from the app itself) and, while the app plays music in a charging car, one
 * look a minute in the process the player already keeps alive. The periodic job
 * is the only net when no event at all happens. Android has no third-party
 * callback for "an app came to the foreground" (`UsageStatsManager`'s observers
 * are system APIs), so the foreground app is read on those occasions only.
 */
object CarWatch {
    private const val JOB_ID = 4211
    private const val PERIOD_MS = 15 * 60 * 1000L

    private val worker = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "car-watch").apply { isDaemon = true }
    }

    fun watch(context: Context, done: (() -> Unit)? = null) = look(context, 0, done)

    fun evaluate(context: Context, done: (() -> Unit)? = null) = look(context, -1, done)

    fun step(context: Context, ladder: Int, done: (() -> Unit)? = null) {
        val app = context.applicationContext
        val next = CarLifecycle.nextRung(ladder, carMusic(app))
        look(context, next, done)
    }

    fun disarm(context: Context) {
        val app = context.applicationContext
        cancelWake(app)
        cancelJob(app)
    }

    fun rearm(context: Context) {
        val app = context.applicationContext
        if (!CarModeService.autoStartAllowed(app)) {
            cancelWake(app)
            cancelJob(app)
            return
        }
        ensureJob(app)
        if (interactive(app)) scheduleWake(app, CarLifecycle.wakeAfter(0) ?: 0L, 0)
    }

    private fun look(context: Context, ladder: Int, done: (() -> Unit)?) {
        val app = context.applicationContext
        worker.execute {
            try {
                if (!CarModeService.autoStartAllowed(app)) {
                    cancelWake(app)
                    cancelJob(app)
                    return@execute
                }
                ensureJob(app)
                if (CarModeService.isRunning()) {
                    CarModeService.refresh(app)
                    return@execute
                }
                if (activeNow(app)) {
                    CarModeService.detected(app)
                    return@execute
                }
                if (ladder < 0 || !interactive(app)) return@execute
                val after = CarLifecycle.wakeAfter(ladder) ?: return@execute
                scheduleWake(app, after, ladder)
            } finally {
                done?.invoke()
            }
        }
    }

    /**
     * A car with the app's music playing: the screen is on and the phone is
     * charging. That is the one case where the ladder keeps looking past its
     * last rung, because the next thing the person does is open the map.
     */
    private fun carMusic(context: Context): Boolean {
        if (!interactive(context)) return false
        val audio = context.getSystemService(AudioManager::class.java) ?: return false
        val playing = try {
            audio.isMusicActive
        } catch (_: Exception) {
            false
        }
        return playing && plugged(context)
    }

    private fun plugged(context: Context): Boolean {
        val battery = try {
            context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        } catch (_: Exception) {
            null
        } ?: return false
        return battery.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0
    }

    private fun activeNow(context: Context): Boolean {
        val prefs = CarPrefs(context)
        val usage = prefs.gpsWatching
        val gps = if (usage) gpsPackageInFront(context, prefs.gpsPackages) else null
        val sink = prefs.bluetoothTrigger && bluetoothSink(context)
        return CarVisibility.wanted(
            gpsTrigger = prefs.gpsTrigger,
            bluetoothTrigger = prefs.bluetoothTrigger,
            gpsUsable = usage,
            gpsInFront = gps != null,
            bluetoothSink = sink,
        )
    }

    @Suppress("DEPRECATION")
    private fun gpsPackageInFront(context: Context, packages: Set<String>): String? {
        val usage = context.getSystemService(Context.USAGE_STATS_SERVICE) as? UsageStatsManager
            ?: return null
        val now = System.currentTimeMillis()
        val events = try {
            usage.queryEvents(now - CarTrigger.SEED_MS, now)
        } catch (_: Exception) {
            null
        } ?: return null
        val event = UsageEvents.Event()
        var last: String? = null
        var at = 0L
        while (events.hasNextEvent()) {
            events.getNextEvent(event)
            if (event.eventType != UsageEvents.Event.MOVE_TO_FOREGROUND) continue
            if (event.timeStamp < at) continue
            val name = event.packageName ?: continue
            if (name in CarForegroundScan.OVERLAYS) continue
            at = event.timeStamp
            last = name
        }
        return last?.takeIf { it in packages }
    }

    private fun bluetoothSink(context: Context): Boolean {
        val audio = context.getSystemService(AudioManager::class.java) ?: return false
        val devices = try {
            audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
        } catch (_: Exception) {
            null
        } ?: return false
        return devices.any { CarTrigger.isBluetoothSink(it.type) }
    }

    private fun interactive(context: Context): Boolean =
        (context.getSystemService(Context.POWER_SERVICE) as? PowerManager)?.isInteractive != false

    private fun ensureJob(context: Context) {
        val scheduler = context.getSystemService(JobScheduler::class.java) ?: return
        try {
            if (scheduler.getPendingJob(JOB_ID) != null) return
            val job = JobInfo.Builder(
                JOB_ID,
                ComponentName(context, CarWatchJobService::class.java)
            )
                .setPeriodic(PERIOD_MS)
                .setPersisted(true)
                .build()
            scheduler.schedule(job)
        } catch (_: Exception) {
        }
    }

    private fun cancelJob(context: Context) {
        try {
            context.getSystemService(JobScheduler::class.java)?.cancel(JOB_ID)
        } catch (_: Exception) {
        }
    }

    private fun scheduleWake(context: Context, afterMs: Long, rung: Int) {
        val alarm = context.getSystemService(AlarmManager::class.java) ?: return
        try {
            alarm.set(
                AlarmManager.ELAPSED_REALTIME,
                SystemClock.elapsedRealtime() + afterMs,
                CarWakeReceiver.intent(context, rung),
            )
        } catch (_: Exception) {
        }
    }

    private fun cancelWake(context: Context) {
        val alarm = context.getSystemService(AlarmManager::class.java) ?: return
        try {
            alarm.cancel(CarWakeReceiver.intent(context, 0))
        } catch (_: Exception) {
        }
    }
}
