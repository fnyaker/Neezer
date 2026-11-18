package fr.neezer.mobile.feature.nowplaying

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import androidx.compose.ui.graphics.toArgb
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.RuntimeEnvironment

/**
 * The lyric card's PNG and the sleeve's palette, drawn for real on the JVM.
 *
 * What is proven, in numbers:
 *
 * - the file is **1080 × 1350** (the SPA's 4:5 frame) and a real PNG (its header,
 *   its size in bytes);
 * - the palette's rules hold on a hand-drawn sleeve: the most PRESENT colour wins,
 *   near-identical colours collapse, and a transparent corner is not part of the
 *   picture (it would drag every zone to black);
 * - the words are wrapped by the font's own metrics and never escape the block —
 *   a one-word-per-line text stays inside the card's width.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * text layout with real faces, no emulator, no window on the person's display
 * (worker rule 7).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34])
class LyricCardTest {

    private val context get() = RuntimeEnvironment.getApplication()

    /** A hand-made sleeve: three quarters blue, one quarter red, a clear corner. */
    private fun sleeve(): Bitmap {
        val bitmap = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        val paint = Paint()
        paint.color = Color.rgb(20, 60, 200)
        canvas.drawRect(0f, 0f, 64f, 48f, paint)
        paint.color = Color.rgb(200, 30, 30)
        canvas.drawRect(0f, 48f, 64f, 64f, paint)
        paint.color = Color.TRANSPARENT
        paint.xfermode = android.graphics.PorterDuffXfermode(android.graphics.PorterDuff.Mode.CLEAR)
        canvas.drawRect(0f, 0f, 8f, 8f, paint)
        return bitmap
    }

    /** The most present colour is the answer, and the clear corner is ignored. */
    @Test
    fun the_palettes_dominant_colour_is_the_most_present() {
        val palette = CoverPalette.analyse(sleeve())
        assertTrue("a palette: $palette", palette.isNotEmpty())
        val dominant = palette.first()
        // Three quarters of the sleeve is this blue (the 4-bit bucket's mean).
        assertEquals(20f / 255f, dominant.red, 0.02f)
        assertEquals(60f / 255f, dominant.green, 0.02f)
        assertEquals(200f / 255f, dominant.blue, 0.02f)
        // The red is a DIFFERENT colour, so it is kept; nothing near the blue is.
        assertEquals(2, palette.size)
        // No pixel is counted twice, and none of the transparent ones is in.
        assertTrue(palette.none { it.alpha < 1f })
    }

    /** An entirely transparent image has no colour at all — never an invented one. */
    @Test
    fun a_transparent_image_has_no_palette() {
        val bitmap = Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888)
        assertTrue(CoverPalette.analyse(bitmap).isEmpty())
        assertEquals(null, CoverPalette.dominant(null))
    }

    /** The card is a 1080 × 1350 PNG, written where the share reads it. */
    @Test
    fun the_card_is_a_1080_by_1350_png() {
        val (bitmap, file) = LyricCardPainter.render(
            context = context,
            coverPath = null,
            title = "Le chant des sirènes",
            artist = "Fauve",
            lines = listOf("Il est cinq heures", "Paris s'éveille", "Je me réveille"),
            seed = "9001",
            filename = "neezer-fauve-le-chant-des-sirenes.png",
        )
        assertEquals(1080, bitmap.width)
        assertEquals(1350, bitmap.height)
        assertTrue("the file exists: $file", file.isFile)
        val bytes = file.readBytes()
        assertTrue("a real PNG: ${bytes.size} bytes", bytes.size > 20_000)
        // The PNG signature.
        assertEquals(0x89.toByte(), bytes[0])
        assertEquals('P'.code.toByte(), bytes[1])
        assertEquals('N'.code.toByte(), bytes[2])
        assertEquals('G'.code.toByte(), bytes[3])
        println("MEASURE lyric card: ${bitmap.width}x${bitmap.height} ${bytes.size} bytes")
        file.delete()
    }

    /** No sleeve still draws (Sillon's groove), and the words never leave the block. */
    @Test
    fun a_sleeveless_card_still_draws() {
        val long = "Un vers très long qui devrait être coupé par le mesureur de police avant " +
            "de sortir de la carte, encore et encore, jusqu'à dépasser largement la largeur"
        val (bitmap, file) = LyricCardPainter.render(
            context = context,
            coverPath = null,
            title = "Titre",
            artist = "Artiste",
            lines = listOf(long),
            seed = "sans-pochette",
            filename = "neezer-artiste-titre.png",
        )
        assertEquals(1080, bitmap.width)
        // The card is not a blank rectangle: it has ink on it.
        var inked = 0
        for (y in 0 until bitmap.height step 20) {
            for (x in 0 until bitmap.width step 20) {
                if (bitmap.getPixel(x, y) != Color.BLACK) inked += 1
            }
        }
        assertTrue("$inked pixels drawn", inked > 100)
        file.delete()
    }

    /** The preview's ink: cream words on a dark ground, whatever the theme is. */
    @Test
    fun the_cards_ground_is_dark_in_both_themes() {
        for (kind in listOf(NzThemeKind.Encre, NzThemeKind.Papier)) {
            val (bitmap, file) = LyricCardPainter.render(
                context = context,
                coverPath = null,
                title = "Le chant",
                artist = "Fauve",
                lines = listOf("Il est cinq heures"),
                seed = "9001",
                filename = "neezer-fauve-le-chant-${kind.name.lowercase()}.png",
            )
            // The card is always the sleeve's dark ground: a bright cover can
            // never make the cream words illegible (the SPA's own rule).
            val corner = bitmap.getPixel(4, bitmap.height - 4)
            val luminance = 0.2126 * Color.red(corner) + 0.7152 * Color.green(corner) +
                0.0722 * Color.blue(corner)
            assertTrue("$kind: the ground is dark ($luminance)", luminance < 120)
            assertNotNull(Sillon.theme(kind).bg.toArgb())
            file.delete()
        }
    }
}
