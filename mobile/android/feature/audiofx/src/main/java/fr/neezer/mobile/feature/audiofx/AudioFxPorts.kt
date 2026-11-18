package fr.neezer.mobile.feature.audiofx

import android.content.Context
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.AudioFxListener
import uniffi.nz_ffi.AudioRoute
import uniffi.nz_ffi.FxMeterFrame
import uniffi.nz_ffi.FxMeterStats
import uniffi.nz_ffi.FxPreset
import uniffi.nz_ffi.FxState
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand

/**
 * What the effects screen asks of the device, as ONE interface.
 *
 * The model above it never touches the generated UniFFI types' runtime (JNA, the
 * `.so`, a device): a JVM test hands it [FakeAudioFx] and drives every control the
 * screen has. The real implementation is [PlayerAudioFx], three lines per method.
 */
internal interface AudioFxBackend {
    fun state(): FxState
    fun setEqEnabled(on: Boolean): FxState
    fun setBand(index: Int, db: Float): FxState
    fun setBass(lift: Float): FxState
    fun setNormalization(level: String): FxState
    fun setMono(mono: Boolean): FxState
    fun setBalance(balance: Float): FxState
    fun resetBalance(): FxState
    fun setCrossfade(enabled: Boolean, seconds: Float): FxState
    fun setSkipFade(enabled: Boolean, seconds: Float): FxState
    fun setTrim(on: Boolean): FxState
    fun setOutput(mode: String): FxState

    fun savePreset(name: String): FxPreset?
    fun updatePreset(id: String): FxPreset?
    fun deletePreset(id: String): Boolean
    fun applyPreset(id: String): FxState?

    /** The meter's lifetime: the card's own, never a timer. */
    fun subscribeMeter(onFrame: (FxMeterFrame) -> Unit)
    fun unsubscribeMeter()
    fun meterStats(): FxMeterStats

    /**
     * A deliberate change, so the warning's next episode starts from the reading
     * on screen (`AudioEffects.svelte#resetRiskHold`).
     */
    fun noteChange(key: String)

    fun retryNow()
    fun forgetGlitch(key: String)

    /** The player's own volume, for the warning's « Réduire le volume ». */
    fun volume(): Float
    fun isMuted(): Boolean
    fun setVolume(volume: Float)
    fun toggleMute()
}

/**
 * The real backend: MB12's own `PlayerApi` methods over the ONE Rust player
 * (`docs/agents/mobile.md`, MB3c). No request is made by any of them.
 */
internal class PlayerAudioFx(private val player: PlayerApi) : AudioFxBackend {
    override fun state(): FxState = player.fxState()
    override fun setEqEnabled(on: Boolean): FxState = player.fxSetEqEnabled(on)
    override fun setBand(index: Int, db: Float): FxState = player.fxSetBand(index.toUInt(), db)
    override fun setBass(lift: Float): FxState = player.fxSetBass(lift)
    override fun setNormalization(level: String): FxState = player.fxSetNormalization(level)
    override fun setMono(mono: Boolean): FxState = player.fxSetMono(mono)
    override fun setBalance(balance: Float): FxState = player.fxSetBalance(balance)
    override fun resetBalance(): FxState = player.fxResetBalance()
    override fun setCrossfade(enabled: Boolean, seconds: Float): FxState =
        player.fxSetCrossfade(enabled, seconds)
    override fun setSkipFade(enabled: Boolean, seconds: Float): FxState =
        player.fxSetSkipFade(enabled, seconds)
    override fun setTrim(on: Boolean): FxState = player.fxSetTrim(on)
    override fun setOutput(mode: String): FxState = player.fxSetOutput(mode)

    override fun savePreset(name: String): FxPreset? = player.fxSavePreset(name)
    override fun updatePreset(id: String): FxPreset? = player.fxUpdatePreset(id)
    override fun deletePreset(id: String): Boolean = player.fxDeletePreset(id)
    override fun applyPreset(id: String): FxState? = player.fxApplyPreset(id)

    override fun subscribeMeter(onFrame: (FxMeterFrame) -> Unit) {
        // A Kotlin interface, not a `fun interface`: the listener is an object.
        player.fxMeterSubscribe(
            object : AudioFxListener {
                override fun onMeter(frame: FxMeterFrame) = onFrame(frame)
            },
        )
    }

    override fun unsubscribeMeter() = player.fxMeterUnsubscribe()
    override fun meterStats(): FxMeterStats = player.fxMeterStats()
    override fun noteChange(key: String) = player.fxMeterNoteChange(key)
    override fun retryNow() = player.fxRetryNow()
    override fun forgetGlitch(key: String) = player.fxForgetGlitch(key)

    override fun volume(): Float = player.snapshot().volume
    override fun isMuted(): Boolean = player.snapshot().muted
    override fun setVolume(volume: Float) {
        // No tap instant: this is not a "play" gesture, and the tap→sound latency
        // the journal keeps is measured on one.
        player.command(PlayerCommand.SetVolume(volume), 0uL)
    }

    override fun toggleMute() {
        player.command(PlayerCommand.ToggleMute, 0uL)
    }
}

/** The player behind the ONE bridge, or `null` before the core is built. */
internal fun bridgeAudioFx(): AudioFxBackend? =
    runCatching { NzBridge.core?.player()?.let(::PlayerAudioFx) }.getOrNull()

/**
 * Which output the sound is going to — `android/app/.../AudioRoute.kt`'s ranking,
 * ported: Media goes to the most recently connected external sink by Android's
 * own routing policy — Bluetooth, then USB, then wired, then HDMI, then the
 * speaker — and the ranking here is that order.
 *
 * Every change of the chosen device is a new CONNECTION (`conn`), unique across
 * process restarts: a verdict about a car radio is about one connection of it, so
 * the same radio reconnected is a fresh judgement (`output.js#normalizeRoute`).
 */
internal class AudioRouteWatcher(
    private val context: Context,
    private val onRoute: (AudioRoute) -> Unit,
) {
    private val manager: AudioManager? = context.getSystemService(AudioManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var started = false
    private var last = ""
    private val boot = System.currentTimeMillis()
    private var connections = 0
    private var deviceId = Int.MIN_VALUE

    private val callback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) = changed()
        override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) = changed()
    }

    fun start() {
        if (started) return
        started = true
        try {
            manager?.registerAudioDeviceCallback(callback, main)
        } catch (_: Exception) {
            started = false
        }
        changed()
    }

    fun stop() {
        if (!started) return
        started = false
        try {
            manager?.unregisterAudioDeviceCallback(callback)
        } catch (_: Exception) {
        }
    }

    /** The route as the card and the Rust verdicts read it. */
    fun current(): AudioRoute {
        val devices: Array<AudioDeviceInfo> = try {
            manager?.getDevices(AudioManager.GET_DEVICES_OUTPUTS) ?: emptyArray()
        } catch (_: Exception) {
            emptyArray()
        }
        var best: AudioDeviceInfo? = null
        var bestRank = Int.MAX_VALUE
        for (device in devices) {
            val rank = rank(device.type)
            if (rank < bestRank) {
                best = device
                bestRank = rank
            }
        }
        // A different device than last time is a new connection — and so is the
        // same radio gone and back, since the speaker was the output in between.
        val id = best?.id ?: -1
        if (id != deviceId) {
            deviceId = id
            connections++
        }
        return AudioRoute(
            kind = if (best == null) "unknown" else kindOf(best.type),
            name = clean(best?.productName?.toString()),
            conn = "$boot-$connections",
        )
    }

    private fun changed() {
        val route = current()
        val key = "${route.kind}|${route.name}|${route.conn}"
        if (key != last) {
            last = key
            onRoute(route)
        }
    }

    private fun rank(type: Int): Int = when (kindOf(type)) {
        "bluetooth" -> 0
        "usb" -> 1
        "wired" -> 2
        "hdmi" -> 3
        "speaker" -> 4
        else -> 9
    }

    private fun kindOf(type: Int): String = when (type) {
        AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
        AudioDeviceInfo.TYPE_BLE_HEADSET,
        AudioDeviceInfo.TYPE_BLE_SPEAKER,
        AudioDeviceInfo.TYPE_BLE_BROADCAST,
        AudioDeviceInfo.TYPE_HEARING_AID,
        -> "bluetooth"

        AudioDeviceInfo.TYPE_USB_HEADSET,
        AudioDeviceInfo.TYPE_USB_DEVICE,
        AudioDeviceInfo.TYPE_USB_ACCESSORY,
        AudioDeviceInfo.TYPE_DOCK,
        -> "usb"

        AudioDeviceInfo.TYPE_WIRED_HEADSET,
        AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
        AudioDeviceInfo.TYPE_LINE_ANALOG,
        AudioDeviceInfo.TYPE_LINE_DIGITAL,
        AudioDeviceInfo.TYPE_AUX_LINE,
        -> "wired"

        AudioDeviceInfo.TYPE_HDMI,
        AudioDeviceInfo.TYPE_HDMI_ARC,
        AudioDeviceInfo.TYPE_HDMI_EARC,
        -> "hdmi"

        AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "speaker"
        else -> "other"
    }

    /** A device names itself: keep it printable and short before it is shown. */
    private fun clean(name: String?): String =
        (name ?: "")
            .filter { it.isLetterOrDigit() || it in " -_.()'&+#/" }
            .trim()
            .take(60)
}
