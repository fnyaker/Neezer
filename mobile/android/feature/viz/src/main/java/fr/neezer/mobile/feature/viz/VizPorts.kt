package fr.neezer.mobile.feature.viz

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzErrors
import java.io.File
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.VizApi
import uniffi.nz_ffi.VizFrame
import uniffi.nz_ffi.VizSettingsData
import uniffi.nz_ffi.VizStats

/**
 * The visualiser's FFI, behind an interface a JVM test can fake.
 *
 * A screen never touches `uniffi.nz_ffi.*` directly: the surface and the model
 * talk to [VizService], so a screenshot test renders real pixels from a
 * synthetic frame with no `.so` and no emulator (the recipe MB11 measured).
 *
 * Every call is synchronous and cheap on Rust's side: `setVisible` flips a flag,
 * `frame` clones a published `Arc`, a setter writes the device's own `kv` store.
 * Nothing here is `suspend`, so the paint loop never awaits a coroutine.
 */
interface VizService {
    fun settings(): VizSettingsData

    fun setMode(mode: String): VizSettingsData

    fun setPalette(palette: String): VizSettingsData

    fun setIntensity(value: Double): VizSettingsData

    fun setFps(fps: Int): VizSettingsData

    fun setEco(eco: Boolean): VizSettingsData

    fun setScopeOrientation(orientation: String): VizSettingsData

    fun setScopeColour(colour: String): VizSettingsData

    /**
     * The newest analysed frame, or the rest frame while nothing has been
     * analysed yet. `null` only when nothing is drawn at all (eco).
     */
    fun frame(): VizFrame?

    /** Turns the analysis on or off for ONE view (`bars`, `scope`, `eq`). */
    fun setVisible(view: String, visible: Boolean)

    /** The surface's shape: capsules and scope columns. Never per frame. */
    fun setShape(bars: Int, cols: Int)

    /** The sleeve, as already-decoded RGBA pixels (64 × 64). */
    fun setArtwork(rgba: ByteArray, width: Int, height: Int)

    fun clearArtwork()

    /** The counters the acceptance reads (ticks, subscriptions, bytes). */
    fun stats(): VizStats
}

/** The real service: `NzCore::viz()`, or the "not loaded" state. */
class NzVizService(private val api: VizApi) : VizService {
    override fun settings(): VizSettingsData = api.settings()
    override fun setMode(mode: String): VizSettingsData = api.setMode(mode)
    override fun setPalette(palette: String): VizSettingsData = api.setPalette(palette)
    override fun setIntensity(value: Double): VizSettingsData = api.setIntensity(value)
    override fun setFps(fps: Int): VizSettingsData = api.setFps(fps.toUInt())
    override fun setEco(eco: Boolean): VizSettingsData = api.setEco(eco)
    override fun setScopeOrientation(orientation: String): VizSettingsData =
        api.setScopeOrientation(orientation)

    override fun setScopeColour(colour: String): VizSettingsData = api.setScopeColour(colour)
    override fun frame(): VizFrame? = api.frame()
    override fun setVisible(view: String, visible: Boolean) = api.setVisible(view, visible)
    override fun setShape(bars: Int, cols: Int) = api.setShape(bars.toUInt(), cols.toUInt())
    override fun setArtwork(rgba: ByteArray, width: Int, height: Int) =
        api.setArtwork(rgba, width.toUInt(), height.toUInt())

    override fun clearArtwork() = api.clearArtwork()
    override fun stats(): VizStats = api.stats()
}

/**
 * The player, as the card and the "paused" notice read it: one snapshot, no
 * position tick (the visualiser draws the sound, not the clock).
 */
interface VizPlayer {
    fun snapshot(): PlayerSnapshot?
}

object NzVizPlayer : VizPlayer {
    override fun snapshot(): PlayerSnapshot? =
        runCatching { NzBridge.core?.player()?.snapshot() }.getOrNull()
}

/**
 * The sleeve: the cover cache's own file, decoded at `coverpalette.js`'s 64 px,
 * handed to Rust as RGBA.
 *
 * The DECODE is the platform's (`BitmapFactory`, `inSampleSize` chosen so the
 * bitmap is about 64 px — one small allocation for a 1 000 px sleeve) and the
 * ANALYSIS is Rust's (`nz-viz::palette::analyse`, `coverpalette.js`'s pure
 * half). The note's rule: decoding in Rust and copying RGBA across JNI would
 * cost more than the platform decoder, so the platform decodes and Rust reads
 * 16 kB once per track.
 *
 * The file comes from `Screens::cover_path` — the same cached, lane-ordered
 * fetch the rest of the app uses (visible-first, one request cold, none warm),
 * never a URL the server sent.
 */
interface VizCovers {
    /** The cover's local path, or `null` when the track has no sleeve. */
    suspend fun path(url: String): String?
}

class NzVizCovers(private val screens: Screens) : VizCovers {
    override suspend fun path(url: String): String? =
        runCatching { screens.coverPath(url, PALETTE_PX.toUInt()) }.getOrNull()

    companion object {
        /** `coverpalette.js#PALETTE_SIZE`: 64 px is what the analysis reads. */
        const val PALETTE_PX = 64
    }
}

/** What one cover decode produced: RGBA bytes and their size. */
data class ArtworkPixels(val rgba: ByteArray, val width: Int, val height: Int) {
    // A byte array's identity is its contents here: two decodes of the same
    // sleeve are the same artwork (the model compares them to avoid a re-read).
    override fun equals(other: Any?): Boolean =
        other is ArtworkPixels &&
            width == other.width &&
            height == other.height &&
            rgba.contentEquals(other.rgba)

    override fun hashCode(): Int = 31 * (31 * rgba.contentHashCode() + width) + height
}

/**
 * The decode itself: a file in, [ArtworkPixels] out, or `null` when the sleeve
 * cannot be READ (a truncated file, an image the platform refuses) — a sleeve
 * that cannot be read goes back to the accent rather than keeping the last
 * track's colours (`Visualizer.svelte#loadCoverUrl`).
 */
object VizArtwork {
    fun decode(path: String?): ArtworkPixels? {
        if (path.isNullOrBlank()) return null
        val file = File(path)
        if (!file.isFile) return null
        return runCatching { decodeFile(file) }.getOrNull()
    }

    private fun decodeFile(file: File): ArtworkPixels? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.absolutePath, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        var sample = 1
        while (bounds.outWidth / (sample * 2) >= NzVizCovers.PALETTE_PX &&
            bounds.outHeight / (sample * 2) >= NzVizCovers.PALETTE_PX
        ) {
            sample *= 2
        }
        val options = BitmapFactory.Options().apply {
            inSampleSize = sample
            inPreferredConfig = Bitmap.Config.ARGB_8888
        }
        val decoded = BitmapFactory.decodeFile(file.absolutePath, options) ?: return null
        // One scale to exactly 64 × 64: the zones are averages over a third of
        // the artwork, and the SPA draws its sleeve on a 64 × 64 canvas for the
        // same reason (the dominant colour of 500 px is the dominant colour of
        // its 64 px reduction).
        val scaled = if (decoded.width == NzVizCovers.PALETTE_PX && decoded.height == NzVizCovers.PALETTE_PX) {
            decoded
        } else {
            Bitmap.createScaledBitmap(decoded, NzVizCovers.PALETTE_PX, NzVizCovers.PALETTE_PX, true)
        }
        val width = scaled.width
        val height = scaled.height
        val pixels = IntArray(width * height)
        scaled.getPixels(pixels, 0, width, 0, 0, width, height)
        val rgba = ByteArray(width * height * 4)
        for (i in pixels.indices) {
            val argb = pixels[i]
            rgba[i * 4] = ((argb shr 16) and 0xFF).toByte()
            rgba[i * 4 + 1] = ((argb shr 8) and 0xFF).toByte()
            rgba[i * 4 + 2] = (argb and 0xFF).toByte()
            rgba[i * 4 + 3] = ((argb ushr 24) and 0xFF).toByte()
        }
        if (scaled !== decoded) decoded.recycle()
        scaled.recycle()
        return ArtworkPixels(rgba, width, height)
    }
}

/** The core's visualiser, or `null` when the native library could not load. */
object VizCore {
    fun api(): VizApi? = runCatching { NzBridge.core?.viz() }.getOrNull()

    fun service(): VizService? = api()?.let(::NzVizService)

    fun player(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()

    fun covers(): VizCovers? = runCatching { NzBridge.core?.screens()?.let(::NzVizCovers) }.getOrNull()

    /** The French sentence for a failure: `nz_error_text`, never a code. */
    fun cause(e: Throwable): String = NzErrors.text(e)
}
