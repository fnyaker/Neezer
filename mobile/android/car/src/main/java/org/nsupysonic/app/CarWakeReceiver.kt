package org.nsupysonic.app

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** One rung of [CarWatch]'s ladder: look at the foreground app again. */
class CarWakeReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent?) {
        if (intent?.action != ACTION_WAKE) return
        val pending = goAsync()
        CarWatch.step(
            context.applicationContext,
            intent.getIntExtra(EXTRA_LADDER, 0),
        ) { pending.finish() }
    }

    companion object {
        const val ACTION_WAKE = "org.nsupysonic.app.action.CAR_WAKE"
        const val EXTRA_LADDER = "ladder"
        private const val REQUEST = 4212

        fun intent(context: Context, ladder: Int): PendingIntent = PendingIntent.getBroadcast(
            context.applicationContext,
            REQUEST,
            Intent(context.applicationContext, CarWakeReceiver::class.java)
                .setAction(ACTION_WAKE)
                .putExtra(EXTRA_LADDER, ladder),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }
}
