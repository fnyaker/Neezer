// :playback — the CPU and Wi-Fi locks the media service holds
// (docs/agents/mobile.md, "MB3c": `PARTIAL_WAKE_LOCK` while Playing/Buffering, a
// `WifiLock` `WIFI_MODE_FULL_HIGH_PERF` only while `snapshot.streaming`; the
// rule itself lives in [LockPolicy]).
package fr.neezer.mobile.playback

import android.content.Context
import android.net.wifi.WifiManager
import android.os.PowerManager
import android.util.Log
import uniffi.nz_ffi.PlayState

/**
 * The two locks, acquired and released on state changes and never on a timer.
 *
 * Both are reference-counted OFF (`setReferenceCounted(false)`) and every call is
 * idempotent, because a mis-counted wake lock is either a phone that never sleeps
 * or music that stops behind a locked screen — the two failures this app exists
 * to avoid.
 */
class PlaybackLocks(context: Context) {
    private val power = context.getSystemService(PowerManager::class.java)
    private val wifi = context.applicationContext.getSystemService(WifiManager::class.java)

    private var wake: PowerManager.WakeLock? = null
    private var radio: WifiManager.WifiLock? = null

    /** The last state applied, so a repeated snapshot costs nothing. */
    private var lastWake = false
    private var lastRadio = false

    fun apply(state: PlayState, streaming: Boolean) {
        val wantWake = LockPolicy.wakeLock(state)
        // A null state (`STOPPED` on a player that never played) must not hold
        // the radio: `streaming` alone decides, exactly as `LockPolicy` says.
        val wantRadio = LockPolicy.wifiLock(streaming)
        if (wantWake != lastWake) {
            lastWake = wantWake
            if (wantWake) acquireWake() else releaseWake()
        }
        if (wantRadio != lastRadio) {
            lastRadio = wantRadio
            if (wantRadio) acquireRadio() else releaseRadio()
        }
    }

    fun release() {
        lastWake = false
        lastRadio = false
        releaseWake()
        releaseRadio()
    }

    private fun acquireWake() {
        if (wake != null) return
        val manager = power ?: return
        wake =
            runCatching {
                manager
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_TAG)
                    .apply {
                        setReferenceCounted(false)
                        acquire()
                    }
            }.onFailure { Log.w(TAG, "veille refusée : ${it.message}") }
                .getOrNull()
    }

    private fun releaseWake() {
        val lock = wake ?: return
        wake = null
        runCatching { if (lock.isHeld) lock.release() }
    }

    /**
     * `WIFI_MODE_FULL_HIGH_PERF` is what the note pins, and it is deprecated from
     * API 34 (the platform says the mode is no longer needed for low latency);
     * it still works and is the only mode that keeps the radio out of power
     * save for a stream, so the deprecation is suppressed here with its reason
     * rather than silently downgraded to `WIFI_MODE_FULL`.
     */
    @Suppress("DEPRECATION")
    private fun acquireRadio() {
        if (radio != null) return
        val manager = wifi ?: return
        radio =
            runCatching {
                manager
                    .createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, RADIO_TAG)
                    .apply {
                        setReferenceCounted(false)
                        acquire()
                    }
            }.onFailure { Log.w(TAG, "verrou Wi-Fi refusé : ${it.message}") }
                .getOrNull()
    }

    private fun releaseRadio() {
        val lock = radio ?: return
        radio = null
        runCatching { if (lock.isHeld) lock.release() }
    }

    /** What the locks really are, for the diagnostic screen and the tests. */
    fun held(): Pair<Boolean, Boolean> = (wake?.isHeld == true) to (radio?.isHeld == true)

    private companion object {
        const val TAG = "NzLocks"
        const val WAKE_TAG = "Neezer:lecture"
        const val RADIO_TAG = "Neezer:flux"
    }
}
