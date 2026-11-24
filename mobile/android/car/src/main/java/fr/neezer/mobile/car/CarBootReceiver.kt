package fr.neezer.mobile.car

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Re-arms the car mode after the phone boots and after this app is updated:
 * one look at what is actually in front (a navigation app, the car radio),
 * never a service parked with its notification for the whole day.
 *
 * The service is started only when a trigger really wants it. Android 12+
 * allows a foreground service started from ACTION_BOOT_COMPLETED /
 * ACTION_MY_PACKAGE_REPLACED (developer.android.com, "Exemptions from background
 * start restrictions"), and this app additionally holds SYSTEM_ALERT_WINDOW,
 * which is another exemption. A refused start is swallowed — the periodic job
 * looks again — and the receiver never throws into the system.
 */
class CarBootReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent?) {
        val action = intent?.action ?: return
        if (action != Intent.ACTION_BOOT_COMPLETED &&
            action != Intent.ACTION_MY_PACKAGE_REPLACED
        ) return
        val app = context.applicationContext
        val pending = goAsync()
        CarWatch.evaluate(app) { pending.finish() }
    }
}
