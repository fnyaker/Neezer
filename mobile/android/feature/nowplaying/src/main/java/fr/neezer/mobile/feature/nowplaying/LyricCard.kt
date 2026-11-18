package fr.neezer.mobile.feature.nowplaying

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.RadialGradient
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.Typeface
import android.net.Uri
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.NowPlaying

/**
 * The lyric card: a 4:5 poster made of one to three lines of the playing title's
 * words, shared as a PNG — `LyricCard.svelte` and `lib/lyriccard.js`, ported.
 *
 * # Where the split is, and why
 *
 * The RULES are Rust's (`nz_core::nowplaying::card`): which lines a card can
 * offer, which one it opens on, what a tap does at the limit, the order of the
 * words and the file's name. They are pure, they are held by Rust tests, and they
 * are what a person can feel.
 *
 * The PIXELS are drawn here, on Android's `Canvas` — the same port of the SPA's
 * `draw`/`paintGround`/`drawWords`/`drawMeta`, at the SPA's own geometry
 * (1080×1350, a 96 px margin, the words' block at y 250 and 760 tall, the sleeve
 * of the metadata 148 px at the bottom), because the layout needs the FONT's real
 * metrics and the mobile Rust workspace carries no text rasteriser (the lot's
 * section of `docs/agents/mobile.md` records the reason and the deviation it
 * costs). It opens with NO network: the sleeve is the one the cover cache already
 * holds (`screens.cover_path`), so a card made on a dead link is the card made on
 * Wi-Fi.
 *
 * # Sharing
 *
 * `ACTION_SEND` of an `image/png` through this module's `FileProvider`
 * (`fr.neezer.mobile.feature.nowplaying.cards`), with the file's name from Rust
 * (`neezer-<artist>-<title>.png`). A share that failed says why and offers
 * « Réessayer » — never a button that does nothing.
 */
class CardSheetState {
    var bitmap: Bitmap? by mutableStateOf(null)
    var file by mutableStateOf<File?>(null)
    var busy by mutableStateOf(false)
    var error: String? by mutableStateOf(null)

    fun reset() {
        bitmap = null
        file = null
        busy = false
        error = null
    }
}

/** The card's own sheet: the preview, the line picker, and the share. */
@Composable
fun LyricCardSheet(
    state: LyricsModel.State?,
    model: LyricsModel?,
    card: CardSheetState,
    track: NowPlaying?,
    onClose: () -> Unit,
) {
    val colors = Sillon.colors
    val context = LocalContext.current
    val service = remember { NowPlayingCore.service() }
    val texts = state?.texts.orEmpty()
    val lyricsState = state

    // The card is drawn when it OPENS and whenever the selection changes: the
    // preview IS the file, so what the person sees is what is shared.
    LaunchedEffect(track?.key, lyricsState?.selected, lyricsState?.lyrics) {
        if (model == null || lyricsState == null || track == null) return@LaunchedEffect
        val chosen = model.cardTexts()
        if (chosen.isEmpty()) return@LaunchedEffect
        card.busy = true
        card.error = null
        val drawn = withContext(Dispatchers.Default) {
            runCatching {
                val coverPath = service?.let { svc ->
                    track.cover?.let { url -> runCatching { svc.coverPath(url, 640) }.getOrNull() }
                }
                val filename = service?.cardFilename(track.artists, track.title) ?: "neezer-titre.png"
                LyricCardPainter.render(
                    context = context,
                    coverPath = coverPath,
                    title = track.title,
                    artist = track.artists,
                    lines = chosen,
                    seed = track.id,
                    filename = filename,
                )
            }
        }
        drawn
            .onSuccess {
                card.bitmap = it.first
                card.file = it.second
            }
            .onFailure { card.error = NowPlayingCore.cause(it) }
        card.busy = false
    }

    NzSheet(visible = true, onDismiss = onClose) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = NzMinTouchTarget),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Carte de paroles",
                style = Sillon.type.titleSmall,
                color = colors.tx,
                modifier = Modifier.weight(1f),
            )
            NzIconButton(
                glyph = NzGlyph.Close,
                contentDescription = "Fermer la carte",
                onClick = onClose,
            )
        }

        val bitmap = card.bitmap
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(4f / 5f)
                .clip(RoundedCornerShape(NzRadius.card))
                .background(colors.s1)
                .semantics { contentDescription = previewLabel(texts) },
            contentAlignment = Alignment.Center,
        ) {
            if (bitmap != null) {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = null,
                    modifier = Modifier.fillMaxWidth().aspectRatio(4f / 5f),
                )
            } else {
                Text(
                    text = if (card.busy) "Composition de la carte…" else "Carte indisponible.",
                    style = Sillon.type.body,
                    color = colors.dim,
                )
            }
        }

        if (lyricsState != null && lyricsState.lines.isNotEmpty()) {
            Text(
                text = "Choisissez une à trois lignes (${lyricsState.selected.size}/3)",
                style = Sillon.type.labelMono,
                color = colors.dim,
            )
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 168.dp)) {
                itemsIndexed(lyricsState.lines) { index, line ->
                    val chosen = lyricsState.selected.contains(index)
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .heightIn(min = NzMinTouchTarget)
                            .clip(RoundedCornerShape(NzRadius.field))
                            .background(if (chosen) colors.s2 else androidx.compose.ui.graphics.Color.Transparent)
                            .clickable(role = Role.Button) { model?.toggleLine(index) }
                            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    ) {
                        Text(
                            text = line.text,
                            style = Sillon.type.bodySmall,
                            color = if (chosen) colors.acText else colors.tx,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        val stamp = line.timeMs
                        if (stamp != null) {
                            Text(
                                text = fr.neezer.mobile.designsystem.nzDuration((stamp.toLong() / 1000).toInt()),
                                style = Sillon.type.mono,
                                color = colors.dim,
                            )
                        }
                    }
                }
            }
            if (lyricsState.notice.isNotBlank()) {
                Text(lyricsState.notice, style = Sillon.type.bodySmall, color = colors.acText)
            }
            if (lyricsState.lyrics?.synced.isNullOrEmpty()) {
                Text(
                    text = "Paroles non synchronisées : la carte n'affiche pas d'horodatage.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
        }

        if (card.error != null) {
            Text(
                text = card.error ?: "",
                style = Sillon.type.bodySmall,
                color = colors.acText,
            )
        }

        NzButton(
            text = if (card.busy) "Composition…" else "Partager la carte",
            onClick = {
                val file = card.file
                if (file == null) {
                    card.error = "La carte n'a pas encore été dessinée."
                    return@NzButton
                }
                runCatching { shareCard(context, file) }
                    .onFailure { card.error = NowPlayingCore.cause(it) }
            },
            kind = NzButtonKind.Primary,
            enabled = card.file != null && !card.busy,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

/** The screen reader's own sentence for the preview (`aria-label` in the SPA). */
private fun previewLabel(texts: List<String>): String = if (texts.isEmpty()) {
    "Aperçu de la carte"
} else {
    "Aperçu de la carte : ${texts.joinToString(" · ")}"
}

/** Hands the PNG to the platform's share sheet, as an image. */
private fun shareCard(context: Context, file: File) {
    val uri: Uri = FileProvider.getUriForFile(
        context,
        "${context.packageName}.nowplaying.cards",
        file,
    )
    val send = Intent(Intent.ACTION_SEND).apply {
        type = "image/png"
        putExtra(Intent.EXTRA_STREAM, uri)
        putExtra(Intent.EXTRA_TEXT, "Une carte de paroles, depuis Neezer.")
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    context.startActivity(Intent.createChooser(send, "Partager la carte"))
}

/**
 * The card's pixels — `LyricCard.svelte`'s canvas, at its own numbers.
 *
 * 1080 × 1350 (4:5, Instagram's documented portrait frame: a square loses about
 * 12 % of each side in the 3:4 profile grid), a 96 px margin, the words' block at
 * y 250 and 760 px tall, the metadata 148 px at the bottom. The ground takes the
 * sleeve's dominant colour as light — a glow in the top-right corner and a rule
 * above the title — over a blurred, darkened sleeve, so the cream words always
 * carry their own contrast whatever the art is.
 */
object LyricCardPainter {
    const val CARD_W = 1080
    const val CARD_H = 1350
    private const val MARGIN = 96f
    private const val LYRIC_TOP = 250f
    private const val LYRIC_MAX_H = 760f
    private const val META_H = 148f
    private const val META_TOP = CARD_H - MARGIN - META_H
    private const val CREAM = "#F7F4EC"
    private const val INK = "#0C0C0B"
    private const val TITLE_SIZE = 42f
    private const val TITLE_LH = 48f
    private const val ARTIST_SIZE = 30f

    /** The largest first: `fitLyricLayout` walks down until the block fits. */
    private val LYRIC_SIZES = listOf(96f, 88f, 80f, 72f, 64f, 56f, 48f, 40f, 34f, 30f, 26f)

    /**
     * Draws the card and writes it beside the app's cache, returning the bitmap
     * for the preview and the file for the share.
     */
    fun render(
        context: Context,
        coverPath: String?,
        title: String,
        artist: String,
        lines: List<String>,
        seed: String,
        filename: String,
    ): Pair<Bitmap, File> {
        // The faces are resolved from the design system's resources, once per draw.
        attach(context)
        val bitmap = Bitmap.createBitmap(CARD_W, CARD_H, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        val cover = decodeCover(coverPath, 640)
        val accent = accentOf(cover, seed)
        paintGround(canvas, cover, accent, seed)
        drawTopLine(canvas, accent)
        drawWords(canvas, lines)
        drawMeta(canvas, cover, title, artist, accent)

        val dir = File(context.cacheDir, "cards").apply { mkdirs() }
        val file = File(dir, filename)
        file.outputStream().use { out -> bitmap.compress(Bitmap.CompressFormat.PNG, 100, out) }
        return bitmap to file
    }

    /** The sleeve, decoded at most `max` px on its longest side (or `null`). */
    private fun decodeCover(path: String?, max: Int): Bitmap? {
        if (path.isNullOrBlank()) return null
        val file = File(path)
        if (!file.isFile) return null
        val bounds = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
        android.graphics.BitmapFactory.decodeFile(file.absolutePath, bounds)
        if (bounds.outWidth <= 0) return null
        var sample = 1
        while (bounds.outWidth / (sample * 2) >= max && bounds.outHeight / (sample * 2) >= max) {
            sample *= 2
        }
        val options = android.graphics.BitmapFactory.Options().apply {
            inSampleSize = sample
            inPreferredConfig = Bitmap.Config.ARGB_8888
        }
        return android.graphics.BitmapFactory.decodeFile(file.absolutePath, options)
    }

    /** The sleeve's dominant colour, lifted until cream text reads on it. */
    private fun accentOf(cover: Bitmap?, seed: String): Int {
        val dominant = cover?.let { CoverPalette.analyse(it).firstOrNull() }
        val base = dominant?.let { Color.rgb((it.red * 255).toInt(), (it.green * 255).toInt(), (it.blue * 255).toInt()) }
            ?: groovePair(seed).second
        return lift(base, Color.parseColor(INK), 3.0)
    }

    /** `readableAccent(rgb, INK, 3)`: scale the channels until it reads. */
    private fun lift(base: Int, on: Int, ratio: Double): Int {
        fun contrast(a: Int, b: Int): Double {
            fun luminance(c: Int): Double {
                fun channel(v: Int): Double {
                    val s = v / 255.0
                    return if (s <= 0.03928) s / 12.92 else Math.pow((s + 0.055) / 1.055, 2.4)
                }
                return 0.2126 * channel(Color.red(c)) + 0.7152 * channel(Color.green(c)) +
                    0.0722 * channel(Color.blue(c))
            }
            val la = luminance(a)
            val lb = luminance(b)
            return (maxOf(la, lb) + 0.05) / (minOf(la, lb) + 0.05)
        }
        if (contrast(base, on) >= ratio) return base
        var last = base
        var scale = 1.05
        while (scale <= 8.0001) {
            last = Color.rgb(
                (Color.red(base) * scale).toInt().coerceIn(0, 255),
                (Color.green(base) * scale).toInt().coerceIn(0, 255),
                (Color.blue(base) * scale).toInt().coerceIn(0, 255),
            )
            if (contrast(last, on) >= ratio) return last
            scale += 0.05
        }
        var mix = 0.05
        while (mix <= 1.0001) {
            val white = Color.rgb(
                (Color.red(last) + (255 - Color.red(last)) * mix).toInt().coerceIn(0, 255),
                (Color.green(last) + (255 - Color.green(last)) * mix).toInt().coerceIn(0, 255),
                (Color.blue(last) + (255 - Color.blue(last)) * mix).toInt().coerceIn(0, 255),
            )
            if (contrast(white, on) >= ratio) return white
            mix += 0.05
        }
        return Color.WHITE
    }

    /** Sillon's record groove pair, from the seed (the missing-cover tile). */
    private fun groovePair(seed: String): Pair<Int, Int> {
        val grooves = listOf(
            "#1E1B19" to "#D2FF3C",
            "#101820" to "#6BB6FF",
            "#1B1020" to "#C08BFF",
            "#101C16" to "#7BE0A3",
            "#201410" to "#FF9C6B",
            "#141414" to "#F2E9D8",
            "#1A1020" to "#FF6B9A",
            "#0F1A1C" to "#6BE0D2",
        )
        var hash = 0xcbf29ce484222325uL
        for (byte in seed.toByteArray()) {
            hash = hash xor byte.toUByte().toULong()
            hash *= 0x100000001b3uL
        }
        val pair = grooves[(hash % grooves.size.toULong()).toInt()]
        return Color.parseColor(pair.first) to Color.parseColor(pair.second)
    }

    private fun paintGround(canvas: Canvas, cover: Bitmap?, accent: Int, seed: String) {
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        if (cover != null) {
            // The sleeve, full-bleed over a 90 px bleed (the SPA's drawCoverFill).
            val source = android.graphics.Rect(0, 0, cover.width, cover.height)
            val target = RectF(-90f, -90f, CARD_W + 90f, CARD_H + 90f)
            canvas.drawBitmap(cover, source, target, paint)
            paint.color = Color.argb(168, 8, 8, 10)
            canvas.drawRect(0f, 0f, CARD_W.toFloat(), CARD_H.toFloat(), paint)
        } else {
            val (a, b) = groovePair(seed)
            paint.color = a
            canvas.drawRect(0f, 0f, CARD_W.toFloat(), CARD_H.toFloat(), paint)
            paint.style = Paint.Style.STROKE
            paint.strokeWidth = 3f
            paint.color = b
            paint.alpha = 153
            val cx = CARD_W * 0.7f
            val cy = CARD_H * 0.29f
            var radius = 10f
            while (radius < 1900f) {
                canvas.drawCircle(cx, cy, radius, paint)
                radius += 18f
            }
            paint.style = Paint.Style.FILL
            paint.alpha = 255
            paint.color = Color.argb(66, 8, 8, 10)
            canvas.drawRect(0f, 0f, CARD_W.toFloat(), CARD_H.toFloat(), paint)
        }

        // The sleeve's colour as a glow in the top-right corner.
        paint.shader = RadialGradient(
            CARD_W * 0.84f,
            CARD_H * 0.12f,
            CARD_W * 1.05f,
            intArrayOf(
                Color.argb(133, Color.red(accent), Color.green(accent), Color.blue(accent)),
                Color.argb(38, Color.red(accent), Color.green(accent), Color.blue(accent)),
                Color.argb(0, Color.red(accent), Color.green(accent), Color.blue(accent)),
            ),
            floatArrayOf(0f, 0.55f, 1f),
            Shader.TileMode.CLAMP,
        )
        canvas.drawRect(0f, 0f, CARD_W.toFloat(), CARD_H.toFloat(), paint)
        paint.shader = null

        // The bottom veil: the title and the artist are always legible.
        paint.shader = LinearGradient(
            0f,
            CARD_H * 0.55f,
            0f,
            CARD_H.toFloat(),
            intArrayOf(Color.argb(0, 6, 6, 8), Color.argb(224, 6, 6, 8)),
            floatArrayOf(0f, 1f),
            Shader.TileMode.CLAMP,
        )
        canvas.drawRect(0f, CARD_H * 0.55f, CARD_W.toFloat(), CARD_H.toFloat(), paint)
        paint.shader = null

        // The top veil, for the label and the mark.
        paint.shader = LinearGradient(
            0f,
            0f,
            0f,
            190f,
            intArrayOf(Color.argb(140, 6, 6, 8), Color.argb(0, 6, 6, 8)),
            floatArrayOf(0f, 1f),
            Shader.TileMode.CLAMP,
        )
        canvas.drawRect(0f, 0f, CARD_W.toFloat(), 190f, paint)
        paint.shader = null

        // A soft band behind the words, wide enough for a bright chorus.
        paint.shader = LinearGradient(
            0f,
            LYRIC_TOP - 90f,
            0f,
            LYRIC_TOP + LYRIC_MAX_H + 80f,
            intArrayOf(
                Color.argb(0, 6, 6, 8),
                Color.argb(76, 6, 6, 8),
                Color.argb(76, 6, 6, 8),
                Color.argb(0, 6, 6, 8),
            ),
            floatArrayOf(0f, 0.16f, 0.84f, 1f),
            Shader.TileMode.CLAMP,
        )
        canvas.drawRect(0f, LYRIC_TOP - 90f, CARD_W.toFloat(), LYRIC_TOP + LYRIC_MAX_H + 90f, paint)
        paint.shader = null

        // The short mark of the sleeve's colour above the metadata.
        paint.color = Color.argb(217, Color.red(accent), Color.green(accent), Color.blue(accent))
        canvas.drawRect(MARGIN, META_TOP - 42f, MARGIN + 64f, META_TOP - 37f, paint)
    }

    private fun drawTopLine(canvas: Canvas, accent: Int) {
        val mono = typeface("jetbrains_mono") ?: Typeface.MONOSPACE
        val display = typeface("bricolage_grotesque") ?: Typeface.SANS_SERIF
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            textSize = 24f
            typeface = Typeface.create(mono, Typeface.NORMAL)
            color = Color.argb(168, 247, 244, 236)
            textAlign = Paint.Align.LEFT
        }
        canvas.drawText("PAROLES", MARGIN, 120f, paint)

        paint.textSize = 28f
        paint.typeface = Typeface.create(display, Typeface.BOLD)
        paint.color = Color.argb(204, 247, 244, 236)
        paint.textAlign = Paint.Align.RIGHT
        canvas.drawText("Neezer", CARD_W - MARGIN, 118f, paint)
        val width = paint.measureText("Neezer")
        drawBrandMark(canvas, CARD_W - MARGIN - width - 22f - 30f, 92f, 30f, accent, display)
    }

    private fun drawBrandMark(canvas: Canvas, x: Float, y: Float, size: Float, accent: Int, display: Typeface) {
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        val cx = x + size / 2f
        val cy = y + size / 2f
        val r = size / 2f
        paint.color = Color.argb(82, 12, 12, 11)
        canvas.drawCircle(cx, cy, r, paint)
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = maxOf(1f, size * 0.045f)
        paint.color = Color.argb(184, 247, 244, 236)
        canvas.drawCircle(cx, cy, r, paint)
        paint.strokeWidth = maxOf(1f, size * 0.03f)
        paint.color = Color.argb(102, 247, 244, 236)
        canvas.drawCircle(cx, cy, r * 0.62f, paint)
        paint.style = Paint.Style.FILL
        paint.color = accent
        canvas.drawCircle(cx, cy, r * 0.24f, paint)
        paint.color = Color.argb(217, 12, 12, 11)
        canvas.drawCircle(cx, cy, r * 0.07f, paint)
    }

    private fun drawWords(canvas: Canvas, lines: List<String>) {
        if (lines.isEmpty()) return
        val display = typeface("bricolage_grotesque") ?: Typeface.SANS_SERIF
        val measure = Paint(Paint.ANTI_ALIAS_FLAG).apply { typeface = Typeface.create(display, Typeface.BOLD) }
        val layout = fitLayout(measure, lines, CARD_W - MARGIN * 2, LYRIC_MAX_H)
        if (layout.blocks.isEmpty()) return
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            typeface = Typeface.create(display, Typeface.BOLD)
            textSize = layout.size
            color = Color.parseColor(CREAM)
            setShadowLayer(18f, 0f, 3f, Color.argb(107, 0, 0, 0))
            textAlign = Paint.Align.LEFT
        }
        val lineHeight = layout.size * 1.24f
        val gap = layout.size * 0.14f
        val rows = layout.blocks.sumOf { it.size }
        val total = rows * lineHeight + (layout.blocks.size - 1).coerceAtLeast(0) * gap
        var y = LYRIC_TOP + maxOf(0f, (LYRIC_MAX_H - total) / 2f) + layout.size
        for (block in layout.blocks) {
            for (row in block) {
                canvas.drawText(row, MARGIN, y, paint)
                y += lineHeight
            }
            y += gap
        }
    }

    private fun drawMeta(canvas: Canvas, cover: Bitmap?, title: String, artist: String, accent: Int) {
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        val display = typeface("bricolage_grotesque") ?: Typeface.SANS_SERIF
        val body = typeface("instrument_sans") ?: Typeface.SANS_SERIF
        val sleeve = RectF(MARGIN, META_TOP, MARGIN + META_H, META_TOP + META_H)
        if (cover != null) {
            canvas.save()
            val clip = android.graphics.Path().apply {
                addRoundRect(sleeve, 14f, 14f, android.graphics.Path.Direction.CW)
            }
            canvas.clipPath(clip)
            val source = android.graphics.Rect(0, 0, cover.width, cover.height)
            canvas.drawBitmap(cover, source, sleeve, paint)
            canvas.restore()
        } else {
            paint.color = Color.argb(255, 30, 27, 25)
            canvas.drawRoundRect(sleeve, 14f, 14f, paint)
        }
        val textLeft = sleeve.right + 28f
        paint.typeface = Typeface.create(display, Typeface.BOLD)
        paint.textSize = TITLE_SIZE
        paint.color = Color.parseColor(CREAM)
        paint.textAlign = Paint.Align.LEFT
        canvas.drawText(ellipsise(paint, title, CARD_W - MARGIN - textLeft), textLeft, META_TOP + 56f, paint)
        paint.typeface = Typeface.create(body, Typeface.NORMAL)
        paint.textSize = ARTIST_SIZE
        paint.color = Color.argb(214, 247, 244, 236)
        canvas.drawText(
            ellipsise(paint, artist, CARD_W - MARGIN - textLeft),
            textLeft,
            META_TOP + 56f + TITLE_LH,
            paint,
        )
        paint.color = Color.argb(217, Color.red(accent), Color.green(accent), Color.blue(accent))
        canvas.drawRect(textLeft, META_TOP + META_H - 6f, textLeft + 48f, META_TOP + META_H - 1f, paint)
    }

    /** `fitOneLine`: the text on one line, ellipsised when it does not fit. */
    private fun ellipsise(paint: Paint, text: String, maxWidth: Float): String {
        if (paint.measureText(text) <= maxWidth) return text
        var cut = text.length
        while (cut > 0 && paint.measureText(text.substring(0, cut).trimEnd() + "…") > maxWidth) cut--
        return text.substring(0, cut).trimEnd() + "…"
    }

    /** The largest size whose wrapped rows fit, each selected line kept WHOLE. */
    private fun fitLayout(
        paint: Paint,
        texts: List<String>,
        maxWidth: Float,
        maxHeight: Float,
    ): Layout {
        var fallback = Layout(LYRIC_SIZES.last(), emptyList())
        for (size in LYRIC_SIZES) {
            paint.textSize = size
            val blocks = texts.map { wrap(paint, it, maxWidth) }.filter { it.isNotEmpty() }
            val rows = blocks.sumOf { it.size }
            val height = rows * size * 1.24f + (blocks.size - 1).coerceAtLeast(0) * size * 0.14f
            val layout = Layout(size, blocks)
            if (height <= maxHeight) return layout
            fallback = layout
        }
        return fallback
    }

    /** `wrapText`: word wrap with `measureText`, a too-wide word cut by character. */
    private fun wrap(paint: Paint, text: String, maxWidth: Float): List<String> {
        val out = ArrayList<String>()
        for (paragraph in text.split('\n')) {
            val words = paragraph.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }.toMutableList()
            if (words.isEmpty()) continue
            var line = ""
            var i = 0
            while (i < words.size) {
                var word = words[i]
                while (paint.measureText(word) > maxWidth && word.length > 1) {
                    var cut = word.length
                    while (cut > 1 && paint.measureText(word.substring(0, cut)) > maxWidth) cut--
                    if (cut < 1) cut = 1
                    if (line.isNotEmpty()) {
                        out.add(line)
                        line = ""
                    }
                    out.add(word.substring(0, cut))
                    word = word.substring(cut)
                }
                if (word.isEmpty()) {
                    i++
                    continue
                }
                val next = if (line.isEmpty()) word else "$line $word"
                if (line.isEmpty() || paint.measureText(next) <= maxWidth) {
                    line = next
                } else {
                    out.add(line)
                    line = word
                }
                i++
            }
            if (line.isNotEmpty()) out.add(line)
        }
        return out
    }

    private data class Layout(val size: Float, val blocks: List<List<String>>)

    /**
     * The three Sillon faces, resolved by RESOURCE NAME.
     *
     * `:designsystem` declares them (`R.font.bricolage_grotesque` …) and no API of
     * it hands a `Typeface` out — it hands Compose `FontFamily`s — so the card asks
     * Android's resource table for the same files, and falls back to the platform
     * faces when they are not there (a stripped resource build): the card is then
     * in the system's sans, still the SPA's geometry and palette.
     */
    private var resources: android.content.Context? = null

    fun attach(context: android.content.Context) {
        // The APPLICATION context: a painter outlives a screen, and holding an
        // Activity here would keep the whole view tree alive (lint's
        // `StaticFieldLeak`, and the real leak behind it).
        this.resources = context.applicationContext
    }

    private fun typeface(name: String): Typeface? {
        val context = resources ?: return null
        // The merged resource table names the library's fonts under the APP's
        // package on a device, and under the library's own in a JVM bench: both
        // are asked, and the platform face is the last answer.
        val id = context.resources.getIdentifier(name, "font", context.packageName)
            .takeIf { it != 0 }
            ?: context.resources.getIdentifier(name, "font", "fr.neezer.mobile.designsystem")
        if (id == 0) return null
        return runCatching { androidx.core.content.res.ResourcesCompat.getFont(context, id) }.getOrNull()
    }
}
