// :playback — the broadcasts a media service must not miss
// (docs/agents/mobile.md, "MB3c": `ACTION_AUDIO_BECOMING_NOISY` → pause, screen
// on/off → `set_screen_on`; android.md bridge 8's "the screen being off").
package fr.neezer.mobile.playback

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioManager
import android.os.Build
import android.os.PowerManager

/**
 * The two dynamic receivers, registered while the service lives and removed in
 * [stop].
 *
 * Why dynamic and not a manifest receiver: `ACTION_SCREEN_ON`/`_OFF` are not on
 * Android's implicit-broadcast exception list (developer.android.com, *Implicit
 * broadcast exceptions*), so a manifest entry would never fire; and a receiver
 * that wakes the app when no music plays is a battery bug.
 *
 * [start] also reports the CURRENT screen state, because a service created while
 * the phone is already in a pocket would otherwise stay at "screen on" and keep
 * the 2 MB read-ahead rule until the person pressed a button.
 */
class PlaybackReceivers(
    private val context: Context,
    private val onScreen: (Boolean) -> Unit,
    private val onBecomingNoisy: () -> Unit,
) {
    private var screen: BroadcastReceiver? = null
    private var noisy: BroadcastReceiver? = null

    fun start() {
        if (screen == null) {
            val receiver =
                object : BroadcastReceiver() {
                    override fun onReceive(context: Context, intent: Intent) {
                        when (intent.action) {
                            Intent.ACTION_SCREEN_ON -> onScreen(true)
                            Intent.ACTION_SCREEN_OFF -> onScreen(false)
                            else -> Unit
                        }
                    }
                }
            val filter =
                IntentFilter().apply {
                    addAction(Intent.ACTION_SCREEN_ON)
                    addAction(Intent.ACTION_SCREEN_OFF)
                }
            register(receiver, filter)
            screen = receiver
            val power = context.getSystemService(PowerManager::class.java)
            onScreen(power?.isInteractive ?: true)
        }
        if (noisy == null) {
            val receiver =
                object : BroadcastReceiver() {
                    override fun onReceive(context: Context, intent: Intent) {
                        if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) onBecomingNoisy()
                    }
                }
            register(receiver, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY))
            noisy = receiver
        }
    }

    fun stop() {
        screen?.let { runCatching { context.unregisterReceiver(it) } }
        noisy?.let { runCatching { context.unregisterReceiver(it) } }
        screen = null
        noisy = null
    }

    /**
     * `RECEIVER_NOT_EXPORTED` from API 33 (an unexported system broadcast still
     * arrives); the plain call below, where the flag does not exist.
     */
    private fun register(receiver: BroadcastReceiver, filter: IntentFilter) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            context.registerReceiver(receiver, filter)
        }
    }
}
