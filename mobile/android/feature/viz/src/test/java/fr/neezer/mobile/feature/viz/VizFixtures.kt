package fr.neezer.mobile.feature.viz

import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlin.math.sin
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.NowPlaying
import uniffi.nz_ffi.VizFrame
import uniffi.nz_ffi.VizSettingsData
import uniffi.nz_ffi.VizStats

/**
 * The frames and the fakes the visualiser's JVM tests draw and assert on.
 *
 * The frames are ANALYTIC, not recorded: a spectrum with a real shape (bass end
 * loud, treble end tapering) and an oscilloscope carrying a 220 Hz sine on both
 * channels — the same shape `nz-viz`'s own fixture produces, so a screenshot shows
 * what the app draws and a model test can assert a number.
 */
internal object VizFixtures {
    /** A settings record, the SPA's defaults unless a test changes one. */
    fun settings(
        mode: String = "bars",
        palette: String = "cover",
        fps: UInt = 60u,
        eco: Boolean = false,
        intensity: Double = 0.7,
        orientation: String = "horizontal",
        colour: String = "duo",
    ) = VizSettingsData(
        mode = if (eco) "off" else mode,
        chosenMode = mode,
        palette = palette,
        intensity = intensity,
        fps = if (eco) 30u else fps,
        eco = eco,
        needsWave = !eco && mode == "scope",
        scopeOrientation = orientation,
        scopeColour = colour,
    )

    /**
     * The bars: 32 capsules, a descending spectrum with a bump in the low mids —
     * what music looks like, so the capture shows a row and not a comb.
     */
    fun barsFrame(bars: Int = 32): VizFrame {
        val levels = List(bars) { i ->
            val t = i.toFloat() / (bars - 1).coerceAtLeast(1)
            val shape = 0.88f * (1f - t).let { it * it } + 0.35f * kotlin.math.exp(-((t - 0.35f) * 6f).let { it * it })
            (shape * (0.55f + 0.45f * sin(i * 1.7f))).coerceIn(0.02f, 1f)
        }
        return VizFrame(
            bars = levels,
            peaks = levels.map { (it + 0.18f).coerceAtMost(1f) },
            scopeA = emptyList(),
            scopeB = emptyList(),
            hue = 288.0,
            sat = 0.9,
            light = 0.58,
            spread = 92.0,
            zones = zones(hue = 288.0, sat = 0.8f),
            hasZones = true,
            level = 0.62f,
            gain = 0f,
            locked = 0f,
            signal = false,
            beat = false,
            bpm = 0f,
            palette = "duo",
        )
    }

    /**
     * The oscilloscope: two traces of a 220 Hz sine at 48 kHz, 256 columns — the
     * SPA's own 42 ms timebase (`scope.js#WINDOW_MS`, 2 016 samples at 48 kHz is
     * about two periods of a 220 Hz tone, which is what a real scope shows).
     */
    fun scopeFrame(cols: Int = 256): VizFrame {
        val window = 2_016
        var phase = 0.0
        val a = ArrayList<Float>(cols * 2)
        val b = ArrayList<Float>(cols * 2)
        val perColumn = window / cols
        for (c in 0 until cols) {
            var lo = 1f
            var hi = -1f
            var loB = 1f
            var hiB = -1f
            for (k in 0 until perColumn) {
                val s = sin(phase).toFloat() * 0.62f
                // The right channel is the same tone, inverted and a little
                // quieter — what two probes on one signal show.
                val sB = -s * 0.82f
                if (s < lo) lo = s
                if (s > hi) hi = s
                if (sB < loB) loB = sB
                if (sB > hiB) hiB = sB
                phase += 2.0 * PI * 220.0 / 48_000.0
            }
            a.add(lo)
            a.add(hi)
            b.add(loB)
            b.add(hiB)
        }
        return VizFrame(
            bars = List(16) { 0.4f },
            peaks = List(16) { 0.5f },
            scopeA = a,
            scopeB = b,
            hue = 196.0,
            sat = 0.8,
            light = 0.6,
            spread = 58.0,
            zones = zones(hue = 196.0, sat = 0.7f),
            hasZones = false,
            level = 0.5f,
            gain = 1.1f,
            locked = 1f,
            signal = true,
            beat = false,
            bpm = 0f,
            palette = "cover",
        )
    }

    /** Seven HSL stops, the sleeve's walk (`palette.js#ZONE_COUNT`). */
    fun zones(hue: Double, sat: Float): List<Float> {
        val out = ArrayList<Float>(21)
        for (i in 0 until 7) {
            val t = i / 6.0
            out.add(((hue + 40.0 * (t - 0.5)) % 360.0).toFloat())
            out.add(sat)
            out.add((0.55 + 0.1 * t).toFloat())
        }
        return out
    }

    fun track(title: String = "Déjà vu", artists: String = "Beyoncé", cover: String? = "cov-1") =
        NowPlaying(
            key = "k-1",
            id = "501",
            title = title,
            artists = artists,
            album = "B'Day",
            albumId = "501",
            artistId = "9",
            cover = cover,
            durationS = 240.0,
            isEpisode = false,
            quality = "320",
            source = "deezer",
            liked = false,
        )

    fun snapshot(
        playing: Boolean = true,
        track: NowPlaying? = track(),
    ) = PlayerSnapshot(
        state = if (playing) PlayState.PLAYING else PlayState.PAUSED,
        current = track,
        index = if (track == null) null else 0u,
        len = if (track == null) 0u else 1u,
        shuffle = false,
        repeat = uniffi.nz_ffi.Repeat.OFF,
        volume = 1f,
        muted = false,
        streaming = false,
        sleep = null,
        error = null,
        fault = null,
        version = 1u,
    )

    fun stats(ticks: ULong = 0u) = VizStats(
        ticks = ticks,
        frames = ticks,
        subscribes = 0u,
        dropped = 0u,
        memoryBytes = 8_192u,
        live = ticks > 0u,
    )

    /** A bar's height rounded, for an assertion that reads like a number. */
    fun height(value: Float): Int = (value * 100).roundToInt()

    /** Whether two hues are within `tolerance` degrees, the short way round. */
    fun sameHue(a: Float, b: Float, tolerance: Float): Boolean {
        val d = abs(((b - a) % 360f + 540f) % 360f - 180f)
        return d <= tolerance
    }
}

/**
 * A service a JVM test drives: it answers the settings it was given, records
 * every call, and can REFUSE a key (the store's validation) or refuse to give a
 * frame (eco).
 */
internal class FakeVizService(
    var settings: VizSettingsData = VizFixtures.settings(),
    var frameNow: VizFrame? = VizFixtures.barsFrame(),
) : VizService {
    val calls = mutableListOf<String>()
    val values = mutableListOf<String>()
    var visible = mutableListOf<Pair<String, Boolean>>()
    var shape: Pair<Int, Int>? = null
    var refused: String? = null
    var artwork: ArtworkPixels? = null
    var cleared = 0

    /** What the model asked for, in order — the proof of "optimistic first". */
    var stateAtCall: String? = null
    var observer: (() -> String)? = null

    /** Records the call, proves the state had ALREADY moved, then may refuse. */
    private fun record(name: String) {
        calls.add(name)
        stateAtCall = observer?.invoke()
        refused?.let { throw IllegalStateException(it) }
    }

    override fun settings(): VizSettingsData = settings
    override fun setMode(mode: String): VizSettingsData {
        val next = settings.copy(mode = mode, chosenMode = mode)
        record("mode")
        values.add(mode)
        settings = next
        return next
    }

    override fun setPalette(palette: String): VizSettingsData {
        val next = settings.copy(palette = palette)
        record("palette")
        values.add(palette)
        settings = next
        return next
    }

    override fun setIntensity(value: Double): VizSettingsData {
        val next = settings.copy(intensity = value)
        record("intensity")
        settings = next
        return next
    }

    override fun setFps(fps: Int): VizSettingsData {
        val next = settings.copy(fps = fps.toUInt())
        record("fps")
        values.add(fps.toString())
        settings = next
        return next
    }

    override fun setEco(eco: Boolean): VizSettingsData {
        val next = settings.copy(eco = eco, mode = if (eco) "off" else settings.chosenMode)
        record("eco")
        values.add(eco.toString())
        settings = next
        return next
    }

    override fun setScopeOrientation(orientation: String): VizSettingsData {
        val next = settings.copy(scopeOrientation = orientation)
        record("orientation")
        values.add(orientation)
        settings = next
        return next
    }

    override fun setScopeColour(colour: String): VizSettingsData {
        val next = settings.copy(scopeColour = colour)
        record("colour")
        values.add(colour)
        settings = next
        return next
    }

    override fun frame(): VizFrame? {
        calls.add("frame")
        return if (settings.eco) null else frameNow
    }

    override fun setVisible(view: String, visible: Boolean) {
        this.visible.add(view to visible)
    }

    override fun setShape(bars: Int, cols: Int) {
        shape = bars to cols
    }

    override fun setArtwork(rgba: ByteArray, width: Int, height: Int) {
        artwork = ArtworkPixels(rgba, width, height)
    }

    override fun clearArtwork() {
        cleared++
        artwork = null
    }

    override fun stats(): VizStats = VizFixtures.stats()
}

/** The player a JVM test drives: one snapshot, read on demand. */
internal class FakeVizPlayer(var snapshotNow: PlayerSnapshot? = VizFixtures.snapshot()) : VizPlayer {
    var reads = 0
    override fun snapshot(): PlayerSnapshot? {
        reads++
        return snapshotNow
    }
}

/** The cover cache a JVM test drives: a path per cover id, and a count. */
internal class FakeVizCovers(
    private val paths: Map<String, String?> = emptyMap(),
) : VizCovers {
    val asked = mutableListOf<String>()
    override suspend fun path(url: String): String? {
        asked.add(url)
        return paths[url]
    }
}
