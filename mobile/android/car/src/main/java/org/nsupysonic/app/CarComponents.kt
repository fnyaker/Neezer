package org.nsupysonic.app

import android.app.Activity
import android.app.Service
import android.app.job.JobParameters
import android.app.job.JobService
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.IBinder
import android.service.quicksettings.TileService

/**
 * The car mode's components, under their OLD fully-qualified names (lot MB24).
 *
 * The names are the point of this file: the old app's quick-settings tile, its
 * notification action and its pinned `PendingIntent`s all name
 * `org.nsupysonic.app.*`, and an update that renamed them would leave a dead
 * tile on the person's phone. `mobile/android/car/src/main/AndroidManifest.xml`
 * declares every one of these, so the merged manifest and these classes are one
 * contract, checked by the build.
 *
 * MB0 makes them inert on purpose: nothing starts itself, nothing draws, nothing
 * binds — a skeleton must not put a window over somebody's navigation app. MB24
 * fills them from `android/` (the overlay, the triggers, the tile, the settings
 * screen), with its two seams changed as the note says: a Media3 `MediaController`
 * instead of `PlayerService.transport/state`, and `NzCore` calls instead of the
 * `__nsCarPlay`/`__nsCarFav` page hooks.
 */

/** The overlay's foreground service: draws the card, owns the trigger. */
class CarModeService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // MB24: `CarVisibility.wanted` decides; a refused background start falls
        // back to a notification with a "Démarrer" action, never a crash.
        return START_STICKY
    }
}

/** The quick-settings tile. */
class CarTileService : TileService()

/** The car mode's full-screen settings (never exported). */
class CarSettingsActivity : Activity()

/** Boot and package-update: ARM the watch, never start the service. */
class CarBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        // MB24: CarWatch.arm(context) then one look on a worker thread.
    }
}

/** One rung of the watch's bounded ladder. */
class CarWakeReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        // MB24: one look, and the next rung only if a trigger still wants one.
    }
}

/** The car radio connecting or leaving. */
class CarBluetoothReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        // MB24: a connect arms the ladder; a disconnect only looks.
    }
}

/** The 15-minute periodic look: no notification, Android owns the schedule. */
class CarWatchJobService : JobService() {
    override fun onStartJob(params: JobParameters?): Boolean {
        jobFinished(params, false)
        return false
    }

    override fun onStopJob(params: JobParameters?): Boolean = false
}
