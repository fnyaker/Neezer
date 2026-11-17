package fr.neezer.mobile.feature.offline

import android.app.job.JobParameters
import android.app.job.JobService

/**
 * The user-initiated data transfer job (API 34+) and, below it, the `dataSync`
 * worker's process holder (lot MB15).
 *
 * It keeps the PROCESS alive; the transfers themselves are the Rust offline
 * worker's, on the `Bulk` lane, yielding to the player's own hold. MB15 fills
 * `onStartJob` (schedule the worker, report the queue's progress and the CAUSE of
 * a failure) and stops it when the queue empties.
 */
class OfflineTransferJobService : JobService() {
    override fun onStartJob(params: JobParameters?): Boolean {
        // Nothing to transfer yet: the job ends at once rather than idling.
        jobFinished(params, false)
        return false
    }

    override fun onStopJob(params: JobParameters?): Boolean = false
}
