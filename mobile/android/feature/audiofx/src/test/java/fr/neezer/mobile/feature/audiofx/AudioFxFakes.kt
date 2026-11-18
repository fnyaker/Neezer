package fr.neezer.mobile.feature.audiofx

import uniffi.nz_ffi.FxMeterFrame
import uniffi.nz_ffi.FxMeterStats
import uniffi.nz_ffi.FxPreset
import uniffi.nz_ffi.FxState
import uniffi.nz_ffi.FxVerdict
import uniffi.nz_ffi.FxWarning

/**
 * The screen's port, faked: every control writes to a plain state and answers it,
 * exactly as the player does (it applies at once and answers what it stored), so
 * the model's rules are testable on the JVM with no `.so` and no device.
 */
internal class FakeAudioFx(
    var state: FxState = FxFixtures.state(),
    var volume: Float = 1f,
    var muted: Boolean = false,
) : AudioFxBackend {
    /** Every setting the screen wrote, in order: the test's record of the taps. */
    val writes = mutableListOf<String>()

    /** Every frame the fake meter will push, once per [tick] call. */
    var frame: FxMeterFrame? = null
    var meterSubscribed = false
    var meterStopped = false
    var ticks = 0L
    var failure: Throwable? = null

    fun pushFrame(frame: FxMeterFrame) {
        this.frame = frame
        this.listener?.invoke(frame)
        ticks++
    }

    private var listener: ((FxMeterFrame) -> Unit)? = null

    private fun apply(tag: String, next: FxState): FxState {
        failure?.let { throw it }
        writes += tag
        // The store is the authority: the model draws what the call answered.
        state = next
        return next
    }

    override fun state(): FxState = state

    override fun setEqEnabled(on: Boolean) =
        apply("eq=$on", state.copy(eqEnabled = on))

    override fun setBand(index: Int, db: Float) =
        apply("band:$index=$db", state.copy(bands = state.bands.toMutableList().also { it[index] = db }))

    override fun setBass(lift: Float) = apply("bass=$lift", state.copy(bass = lift))

    override fun setNormalization(level: String) =
        apply("norm=$level", state.copy(normalization = level))

    override fun setMono(mono: Boolean) = apply("mono=$mono", state.copy(mono = mono))

    override fun setBalance(balance: Float) = apply("balance=$balance", state.copy(balance = balance))

    override fun resetBalance() = apply("balance=0", state.copy(balance = 0f))

    override fun setCrossfade(enabled: Boolean, seconds: Float) =
        apply("fade=$enabled/$seconds", state.copy(crossfadeEnabled = enabled, crossfadeS = seconds))

    override fun setSkipFade(enabled: Boolean, seconds: Float) = apply(
        "skip=$enabled/$seconds",
        state.copy(crossfadeOnSkip = enabled, crossfadeSkipS = seconds),
    )

    override fun setTrim(on: Boolean) = apply("trim=$on", state.copy(trim = on))

    override fun setOutput(mode: String) = apply("output=$mode", state.copy(output = mode))

    override fun savePreset(name: String): FxPreset? {
        failure?.let { throw it }
        writes += "save=$name"
        val preset = FxPreset(id = "p-new", name = name, builtin = false, spark = List(10) { 0.5f })
        state = state.copy(presets = state.presets + preset, activePreset = preset.id)
        return preset
    }

    override fun updatePreset(id: String): FxPreset? {
        failure?.let { throw it }
        writes += "update=$id"
        val preset = state.presets.firstOrNull { it.id == id } ?: return null
        state = state.copy(presets = state.presets.map { if (it.id == id) preset else it })
        return preset
    }

    override fun deletePreset(id: String): Boolean {
        failure?.let { throw it }
        writes += "delete=$id"
        state = state.copy(
            presets = state.presets.filterNot { it.id == id },
            activePreset = state.activePreset?.takeIf { it != id },
        )
        return true
    }

    override fun applyPreset(id: String): FxState? {
        failure?.let { throw it }
        writes += "apply=$id"
        val preset = state.presets.firstOrNull { it.id == id } ?: return null
        val next = state.copy(activePreset = preset.id, eqEnabled = true)
        state = next
        return next
    }

    override fun subscribeMeter(onFrame: (FxMeterFrame) -> Unit) {
        meterSubscribed = true
        meterStopped = false
        listener = onFrame
    }

    override fun unsubscribeMeter() {
        meterSubscribed = false
        meterStopped = true
        listener = null
    }

    override fun meterStats(): FxMeterStats =
        FxMeterStats(ticks = ticks.toULong(), frames = ticks.toULong(), running = meterSubscribed)

    override fun noteChange(key: String) {
        writes += "change=$key"
    }

    override fun retryNow() {
        writes += "retry"
    }

    override fun forgetGlitch(key: String) {
        writes += "forget=$key"
    }

    override fun volume(): Float = volume

    override fun isMuted(): Boolean = muted

    override fun setVolume(volume: Float) {
        failure?.let { throw it }
        writes += "volume=$volume"
        this.volume = volume
    }

    override fun toggleMute() {
        muted = !muted
        writes += "mute"
    }
}

/** The card's data for the tests and the screenshots. */
internal object FxFixtures {
    fun presets() = listOf(
        FxPreset("flat", "Plat", true, List(10) { 0.5f }),
        FxPreset("bass", "Basses+", true, List(10) { 0.6f }),
        FxPreset("vocal", "Voix", true, List(10) { 0.45f }),
        FxPreset("treble", "Aigus+", true, List(10) { 0.55f }),
        FxPreset("loud", "Loudness", true, List(10) { 0.5f }),
        FxPreset("p-car", "Voiture", false, List(10) { 0.62f }),
        FxPreset("p-night", "Nuit", false, List(10) { 0.4f }),
    )

    fun state(
        eqEnabled: Boolean = true,
        bands: List<Float> = listOf(6f, 5f, 4f, 2f, 0f, 0f, 0f, 0f, 0f, 0f),
        bass: Float = 0.35f,
        mono: Boolean = false,
        balance: Float = -0.3f,
        crossfade: Boolean = true,
        trim: Boolean = true,
        activePreset: String? = "bass",
        verdicts: List<FxVerdict> = emptyList(),
        retryAt: Double? = null,
    ) = FxState(
        eqEnabled = eqEnabled,
        bands = bands,
        bass = bass,
        normalization = "medium",
        mono = mono,
        balance = balance,
        panText = if (balance == 0f) "Centre" else "30 % à gauche",
        crossfadeEnabled = crossfade,
        crossfadeS = 6f,
        crossfadeOnSkip = true,
        crossfadeSkipS = 1.5f,
        trim = trim,
        trimDb = -45,
        output = "auto",
        routeText = "Bluetooth · KMM-BT309",
        pathText = "Lecteur Android",
        whyText = "Bluetooth · KMM-BT309",
        retryAtMs = retryAt,
        presets = presets(),
        activePreset = activePreset,
        verdicts = verdicts,
    )

    fun measured(peak: Double = 7.3, reduction: Double = 6.4, warning: Boolean = true) = FxMeterFrame(
        available = true,
        measured = true,
        peakDb = peak,
        headroomDb = -peak,
        reductionDb = reduction,
        risk = if (reduction >= 6.0) "clip" else if (reduction >= 2.0) "warn" else "ok",
        bands = listOf(0.62f, 0.5f, 0.41f, 0.3f, 0.26f, 0.58f, 0.36f, 0.22f, 0.14f, 0.06f),
        bandsDb = listOf(-22.8f, -30f, -35.4f, -42f, -44.4f, -25.2f, -38.4f, -46.8f, -51.6f, -56.4f),
        warning = if (warning) {
            FxWarning(tone = "clip", reductionDb = reduction, cutDb = 6.5, fromChange = false)
        } else {
            null
        },
        reason = "",
    )

    fun idle() = FxMeterFrame(
        available = false,
        measured = false,
        peakDb = -96.0,
        headroomDb = 96.0,
        reductionDb = 0.0,
        risk = "ok",
        bands = List(10) { 0f },
        bandsDb = List(10) { -60f },
        warning = null,
        reason = "idle",
    )
}
