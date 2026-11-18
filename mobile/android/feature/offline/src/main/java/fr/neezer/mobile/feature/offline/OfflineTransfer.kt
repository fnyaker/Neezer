package fr.neezer.mobile.feature.offline

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.os.Build
import androidx.work.CoroutineWorker
import androidx.work.ForegroundInfo
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout

/**
 * Keeps the PROCESS alive while the Rust download worker transfers (lot MB15).
 *
 * Android owns no transfer here: `nz-offline`'s `nz-bulk` thread does the
 * transfers on `Lane::Bulk`, and this class only stops the system from killing
 * the process — `docs/agents/mobile.md` *Background downloads*:
 *
 * - **API ≥ 34: a user-initiated data transfer job** (`setUserInitiated(true)`),
 *   the platform's own path for a download the person asked for (6 h, no
 *   `dataSync` cap), granted because the app is in the foreground when they press
 *   the button;
 * - **below: a WorkManager `dataSync` foreground worker**, kept only while the
 *   queue is non-empty.
 *
 * Both stop as soon as the queue empties, and neither is a source of truth: the
 * queue is SQLite rows, so a kill resumes from the store on the next start
 * (rule 2) — [TransferPolicy] is the two decisions a test holds.
 */
object OfflineTransfer {
    /** The job's id: one, so a second request replaces the first. */
    const val JOB_ID = 8_151

    /** The notification channel the `dataSync` worker uses below API 34. */
    const val CHANNEL_ID = "neezer-downloads"

    /** Schedule the transfer for whatever the queue holds now. */
    fun ensure(context: Context) {
        when (TransferPolicy.kind(Build.VERSION.SDK_INT)) {
            TransferKind.UserInitiated -> scheduleJob(context)
            TransferKind.ForegroundWorker -> scheduleWorker(context)
        }
    }

    /** Ask the platform to stop keeping us alive (the queue emptied, a pause). */
    fun stop(context: Context) {
        runCatching {
            context.getSystemService(JobScheduler::class.java)?.cancel(JOB_ID)
        }
        runCatching { WorkManager.getInstance(context).cancelUniqueWork(TransferPolicy.WORK_NAME) }
    }

    private fun scheduleJob(context: Context) {
        val scheduler = context.getSystemService(JobScheduler::class.java) ?: return
        val component = ComponentName(context, OfflineTransferJobService::class.java)
        val info = JobInfo.Builder(JOB_ID, component)
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
            // A14's user-initiated transfer: the person pressed « Télécharger »,
            // so the system grants the long window instead of the dataSync cap.
            .setUserInitiated(true)
            .build()
        runCatching { scheduler.schedule(info) }
    }

    private fun scheduleWorker(context: Context) {
        val request = OneTimeWorkRequestBuilder<OfflineTransferWorker>()
            .setInputData(workDataOf("reason" to "downloads"))
            .build()
        runCatching {
            WorkManager.getInstance(context).enqueueUniqueWork(
                TransferPolicy.WORK_NAME,
                androidx.work.ExistingWorkPolicy.REPLACE,
                request,
            )
        }
    }

    /** The notification a `dataSync` worker must show while it runs. */
    fun foregroundInfo(context: Context, done: Int, total: Int): ForegroundInfo {
        val manager = context.getSystemService(NotificationManager::class.java)
        if (manager?.getNotificationChannel(CHANNEL_ID) == null) {
            manager?.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    "Téléchargements",
                    // LOW: the person sees it in the shade when they look, and it
                    // makes no sound over the music (rule 1: nothing on the
                    // speakers the person did not ask for).
                    NotificationManager.IMPORTANCE_LOW,
                ),
            )
        }
        // `Notification.Builder` and not `NotificationCompat`: this module does
        // not depend on `androidx.core` (the version catalog is MB0's, one lot
        // owns it) and minSdk 26 has the channel constructor anyway. The glyph
        // is the platform's: the app's own `ic_stat_neezer` lives in `:app` (LG1),
        // and a library module cannot read `:app`'s resources (MB0's lesson).
        val notification = Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("Téléchargements hors ligne")
            .setContentText(if (total > 0) "$done / $total" else "En file")
            .setOngoing(true)
            .setProgress(total.coerceAtLeast(0), done.coerceAtLeast(0), total <= 0)
            .build()
        return ForegroundInfo(JOB_ID, notification)
    }
}

/** Which platform path keeps the process alive (`docs/agents/mobile.md`). */
enum class TransferKind {
    /** API ≥ 34: `JobService` + `setUserInitiated(true)`. */
    UserInitiated,

    /** Below: WorkManager with a `dataSync` foreground notification. */
    ForegroundWorker,
}

/** The two decisions, in one place a JVM test holds without an emulator. */
object TransferPolicy {
    /** The one work name: a second `ensure` replaces the first. */
    const val WORK_NAME = "neezer-downloads"

    /** API 34 introduced user-initiated data transfer jobs. */
    const val USER_INITIATED_API = 34

    fun kind(sdkInt: Int): TransferKind =
        if (sdkInt >= USER_INITIATED_API) TransferKind.UserInitiated else TransferKind.ForegroundWorker

    /**
     * Whether the process must be kept alive: ONLY while the queue holds
     * something to fetch. An empty queue with a permanent notification is the
     * bug rule 2 warns about.
     */
    fun keepAlive(pending: Int, active: Int): Boolean = pending + active > 0
}

/**
 * The API-34 job: start the worker, then watch the queue and finish when it
 * empties. `onStopJob` pauses the Rust worker (the files already published stay,
 * a half file stays `active` for the next start) and does not reschedule by
 * itself: the next launch or the next tap schedules it again.
 */
class OfflineTransferJobService : JobService() {
    private var scope: CoroutineScope? = null
    private var watcher: Job? = null

    override fun onStartJob(params: JobParameters?): Boolean {
        val model = OfflineHost.model()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        this.scope = scope
        watcher = scope.launch {
            model.start()
            // The queue's own state is the event: nothing here is a clock.
            val settled = runCatching {
                withTimeout(6 * 60 * 60 * 1000L) {
                    model.state.first { state -> !state.loading && state.sets.none { it.running } }
                }
            }
            // A settled queue ends the job and releases the notification; a
            // timeout with work left asks the system for another window.
            jobFinished(params, settled.isFailure && model.state.value.sets.any { it.running })
        }
        return true
    }

    override fun onStopJob(params: JobParameters?): Boolean {
        val model = OfflineHost.model()
        scope?.launch { runCatching { model.pause() } }
        scope?.cancel()
        scope = null
        watcher = null
        // Not rescheduled by the system: the queue is in SQLite, and the next
        // start (or the next tap) resumes it (rule 2).
        return false
    }
}

/** Below API 34: the same watch, in a `dataSync` foreground worker. */
class OfflineTransferWorker(
    context: Context,
    params: WorkerParameters,
) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val model = OfflineHost.model()
        runCatching {
            setForeground(OfflineTransfer.foregroundInfo(applicationContext, 0, 0))
        }
        model.start()
        // The state's own changes are the event; a settled queue ends the work.
        val settled = runCatching {
            withTimeout(6 * 60 * 60 * 1000L) {
                model.state.first { state ->
                    !state.loading && state.sets.none { it.running }
                }
            }
        }
        val failed = model.state.value.sets.any { it.failed > 0u }
        return when {
            settled.isFailure -> Result.retry()
            failed -> Result.success()
            else -> Result.success()
        }
    }
}
