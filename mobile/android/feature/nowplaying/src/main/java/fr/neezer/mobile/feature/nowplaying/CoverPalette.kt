package fr.neezer.mobile.feature.nowplaying

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.ui.graphics.Color
import java.io.File
import kotlin.math.abs

/**
 * The colours of a sleeve — `webapp/src/lib/coverpalette.js#analyse`, ported.
 *
 * The SPA reads a cover ONCE: it draws it on a 64×64 canvas and reads the pixels
 * back, because everything here is an average over a region and the dominant
 * colour of a 500 px sleeve is the dominant colour of its 64 px reduction. On the
 * phone the decode is `BitmapFactory`'s (hardware-free, `inSampleSize` chosen so
 * the bitmap is about 64 px), and the analysis is the SPA's own:
 *
 * - **4 bits a channel (4 096 buckets)**, each bucket reporting its own MEAN, so
 *   the quantisation decides the grouping and never the colour;
 * - a pixel under `ALPHA_MIN` is not part of the picture (a rounded PNG corner, a
 *   transparent sleeve): averaging it in drags every zone toward black;
 * - the five most present colours come back **most present first**, and a colour
 *   within `DISTINCT` of one already kept is dropped, so five near-identical darks
 *   never stand in for five different colours.
 *
 * What this lot uses it for: the lyric card's ground (`LyricCard.svelte` takes the
 * sleeve's dominant colour and lifts it against the card's own ink), and the
 * Pochette theme's `--cover-ac` — whose LIFT is `:designsystem`'s
 * (`Sillon.resolve` calls `nzReadableInk`), so what crosses is the colour the
 * sleeve really is, never a pre-corrected one.
 *
 * The SPA cached per URL and de-duplicated in flight; here the caller passes the
 * file the cover cache already holds (`screens.coverPath`), and a decode that
 * fails answers `null` — never a colour invented from nothing.
 */
object CoverPalette {
    /** The analysis' own grid (`PALETTE_SIZE`). */
    const val SIZE = 64

    /** Below this alpha a pixel is not part of the picture. */
    private const val ALPHA_MIN = 200

    /** How far apart two "dominant" colours must be to be reported as distinct. */
    private const val DISTINCT = 26.0

    /** The most colours a palette reports. */
    private const val MAX_COLORS = 5

    /** The sleeve's most PRESENT colour, or `null` when there is nothing to read. */
    fun dominant(path: String?): Color? = dominantOfFile(path)?.firstOrNull()

    /**
     * Every distinct colour of the sleeve, most present first (at most five) —
     * `analyse`'s `colors`, which is what the card's glow and the visualiser will
     * ask for.
     */
    fun palette(path: String?): List<Color> = dominantOfFile(path).orEmpty()

    /** The same, from bytes already in memory (the card's own decode). */
    fun paletteOf(bitmap: Bitmap?): List<Color> = bitmap?.let(::analyse).orEmpty()

    private fun dominantOfFile(path: String?): List<Color>? {
        if (path.isNullOrBlank()) return null
        val file = File(path)
        if (!file.isFile) return null
        val bitmap = runCatching { decode64(file) }.getOrNull() ?: return null
        return runCatching { analyse(bitmap) }.getOrNull()?.takeIf { it.isNotEmpty() }
    }

    /** A ~64 px, software bitmap: `inSampleSize` skips pixels instead of reading
     *  them, so a 1 000 px sleeve costs one 64 px allocation, not four megabytes. */
    private fun decode64(file: File): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.absolutePath, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        var sample = 1
        while (bounds.outWidth / (sample * 2) >= SIZE && bounds.outHeight / (sample * 2) >= SIZE) {
            sample *= 2
        }
        val options = BitmapFactory.Options().apply {
            inSampleSize = sample
            inPreferredConfig = Bitmap.Config.ARGB_8888
        }
        return BitmapFactory.decodeFile(file.absolutePath, options)
    }

    /**
     * `analyse(pixels, width, height)`: the pure half, on a bitmap.
     *
     * Kept free of the file system and of Compose so a JVM test draws a hand-made
     * picture and holds the rules (`LyricCardTest`).
     */
    fun analyse(bitmap: Bitmap): List<Color> {
        val w = bitmap.width.coerceAtLeast(1)
        val h = bitmap.height.coerceAtLeast(1)
        val pixels = IntArray(w * h)
        bitmap.getPixels(pixels, 0, w, 0, 0, w, h)

        val hist = LinkedHashMap<Int, LongArray>()
        var total = 0L
        for (index in pixels.indices) {
            val pixel = pixels[index]
            val alpha = (pixel ushr 24) and 0xFF
            if (alpha < ALPHA_MIN) continue
            val r = (pixel shr 16) and 0xFF
            val g = (pixel shr 8) and 0xFF
            val b = pixel and 0xFF
            val key = ((r shr 4) shl 8) or ((g shr 4) shl 4) or (b shr 4)
            val bin = hist.getOrPut(key) { LongArray(4) }
            bin[0] += r.toLong()
            bin[1] += g.toLong()
            bin[2] += b.toLong()
            bin[3] += 1
            total += 1
        }
        if (total == 0L) return emptyList()

        val bins = hist.values
            .filter { it[3] > 0 }
            .sortedByDescending { it[3] }
            .map { mean(it) }

        val colors = ArrayList<Color>(MAX_COLORS)
        for (colour in bins) {
            if (colors.any { distance(it, colour) < DISTINCT }) continue
            colors.add(colour)
            if (colors.size >= MAX_COLORS) break
        }
        return colors
    }

    private fun mean(bin: LongArray): Color {
        val n = bin[3].coerceAtLeast(1)
        return Color(
            red = (bin[0] / n).toInt().coerceIn(0, 255) / 255f,
            green = (bin[1] / n).toInt().coerceIn(0, 255) / 255f,
            blue = (bin[2] / n).toInt().coerceIn(0, 255) / 255f,
            alpha = 1f,
        )
    }

    private fun distance(a: Color, b: Color): Double {
        val dr = ((a.red - b.red) * 255f).toDouble()
        val dg = ((a.green - b.green) * 255f).toDouble()
        val db = ((a.blue - b.blue) * 255f).toDouble()
        return kotlin.math.sqrt(dr * dr + dg * dg + db * db)
    }

    /** `light`: the average reads as a light surface (the SPA's own weighting). */
    fun isLight(color: Color): Boolean {
        val r = color.red * 255f
        val g = color.green * 255f
        val b = color.blue * 255f
        return abs(0.2126 * r + 0.7152 * g + 0.0722 * b) > 140
    }
}
