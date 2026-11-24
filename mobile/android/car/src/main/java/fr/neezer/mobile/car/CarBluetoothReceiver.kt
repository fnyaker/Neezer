package fr.neezer.mobile.car

import android.bluetooth.BluetoothA2dp
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothHeadset
import android.bluetooth.BluetoothProfile
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * The car radio connecting or leaving. These are the Bluetooth broadcasts
 * Android still delivers to a manifest receiver (the implicit broadcast
 * exceptions), so the mode can start without anything of ours running.
 */
class CarBluetoothReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent?) {
        val action = intent?.action ?: return
        val connected = when (action) {
            BluetoothDevice.ACTION_ACL_CONNECTED -> true
            BluetoothDevice.ACTION_ACL_DISCONNECTED -> false
            BluetoothA2dp.ACTION_CONNECTION_STATE_CHANGED,
            BluetoothHeadset.ACTION_CONNECTION_STATE_CHANGED ->
                intent.getIntExtra(BluetoothProfile.EXTRA_STATE, -1) ==
                    BluetoothProfile.STATE_CONNECTED
            else -> return
        }
        val app = context.applicationContext
        val prefs = CarPrefs(app)
        if (!prefs.gpsTrigger && !prefs.bluetoothTrigger) return
        val pending = goAsync()
        if (connected) {
            CarWatch.watch(app) { pending.finish() }
        } else {
            CarWatch.evaluate(app) { pending.finish() }
        }
    }
}
