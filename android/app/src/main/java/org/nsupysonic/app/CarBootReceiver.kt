package org.nsupysonic.app

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Brings the car mode back without anyone opening Neezer: after the phone
 * boots, and after this app is updated.
 *
 * The service is what watches for the navigation app (no AccessibilityService,
 * no polling receiver): once it is alive, a GPS app coming to the foreground
 * shows the overlay even when Neezer was never opened. The receiver itself does
 * one thing and returns — `startIfEnabled` re-checks the person's settings
 * (configured server, a trigger on, overlay access, notifications) and refuses
 * quietly when the mode is not wanted.
 *
 * Android 12+ allows a foreground service started from ACTION_BOOT_COMPLETED /
 * ACTION_MY_PACKAGE_REPLACED (developer.android.com, "Exemptions from background
 * start restrictions"), and this app additionally holds SYSTEM_ALERT_WINDOW,
 * which is another exemption. If an OEM refuses anyway, CarModeService catches
 * the refusal and leaves the one notification that starts it by hand; the
 * receiver must never throw into the system.
 */
class CarBootReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent?) {
        val action = intent?.action ?: return
        if (action != Intent.ACTION_BOOT_COMPLETED &&
            action != Intent.ACTION_MY_PACKAGE_REPLACED
        ) return
        try {
            CarModeService.startIfEnabled(context.applicationContext)
        } catch (_: Exception) {
            // A refused start is CarModeService's own problem (it posts the
            // fallback notification); a receiver crash would only be noise.
        }
    }
}
