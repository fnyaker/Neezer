package org.nsupysonic.app

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.content.Intent
import android.graphics.drawable.Icon
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

/**
 * The quick-settings tile: one tap starts or stops the car mode. A first tap
 * without the overlay permission does not start a service that could not draw
 * anything — it opens the one system screen that grants it.
 */
class CarTileService : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        render(CarModeService.isRunning())
    }

    override fun onClick() {
        super.onClick()
        if (CarModeService.isRunning()) {
            CarModeService.stop(this)
            render(false)
            return
        }
        // The system may keep the shade over a locked device; the service and
        // the settings screen only make sense once it is unlocked.
        unlockAndRun {
            if (!CarPermissions.hasOverlay(this)) {
                openOverlaySettings()
            } else {
                CarModeService.start(this)
                render(true)
            }
        }
    }

    private fun render(active: Boolean) {
        val tile = qsTile ?: return
        tile.label = getString(R.string.car_tile)
        tile.icon = Icon.createWithResource(this, R.drawable.ic_car)
        tile.state = if (active) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.updateTile()
    }

    // The Intent overload is only used below API 34 and works there; on 34+
    // the PendingIntent overload is the one Android accepts.
    @SuppressLint("StartActivityAndCollapseDeprecated")
    private fun openOverlaySettings() {
        val intent = CarPermissions.overlayIntent(this)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            if (Build.VERSION.SDK_INT >= 34) {
                val pending = PendingIntent.getActivity(
                    this, 20, intent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
                )
                startActivityAndCollapse(pending)
            } else {
                @Suppress("DEPRECATION")
                startActivityAndCollapse(intent)
            }
        } catch (_: Exception) {
            // Some OEMs refuse the collapse path; the plain activity still
            // opens the same permission screen.
            try {
                startActivity(intent)
            } catch (_: Exception) {
            }
        }
    }
}
