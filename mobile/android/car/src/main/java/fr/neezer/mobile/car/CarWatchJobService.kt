package fr.neezer.mobile.car

import android.app.job.JobParameters
import android.app.job.JobService

/** The periodic net behind [CarWatch]: no notification, one look every 15 minutes. */
class CarWatchJobService : JobService() {

    override fun onStartJob(params: JobParameters): Boolean {
        CarWatch.evaluate(this) { jobFinished(params, false) }
        return true
    }

    override fun onStopJob(params: JobParameters): Boolean = false
}
