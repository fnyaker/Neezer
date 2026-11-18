package fr.neezer.mobile.feature.discovery

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzGrooveFor
import java.io.File

/**
 * The Récap's poster, drawn and shared (`components/RecapShare.svelte`).
 *
 * The SPA draws a 1080 × 1350 canvas and hands the PNG to `navigator.share`; on
 * Android the same image is drawn with `android.graphics` and handed to the
 * system's share sheet through the app's own `FileProvider`, so the person
 * shares a PICTURE — 4:5, the portrait size Instagram and the others fill a post
 * with, the size the SPA chose for that reason.
 *
 * The lines are the core's own `poster_lines` (the plan's copy, ellipsised to 38
 * characters); the four sleeves are the Sillon grooves, painted from the same
 * hash as a missing cover on screen. Drawing the encoded covers would mean
 * decoding four images to share one picture — the share sheet is opened once,
 * and a groove is what the app draws when a sleeve is not there.
 */
@Composable
fun RecapShareSheet(data: RecapDeck, onClose: () -> Unit) {
    val context = LocalContext.current
    val colors = Sillon.colors
    NzSheet(visible = true, onDismiss = onClose) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(NzSpacing.l),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Text("Ton récap, en une image", style = Sillon.type.titleSmall)
            Text(
                "PNG · 1080 × 1350",
                style = Sillon.type.labelMono,
                color = colors.dim,
            )
            Text(
                data.posterLines.joinToString(" · "),
                style = Sillon.type.bodySmall,
                color = colors.dim,
                modifier = Modifier.semantics {
                    contentDescription = "Aperçu de l'affiche : ${data.posterLines.joinToString(" · ")}"
                },
            )
            NzButton(
                text = "Partager l'image",
                glyph = NzGlyph.Share,
                onClick = { RecapPoster.share(context, data) },
                modifier = Modifier.fillMaxWidth(),
            )
            NzButton(
                text = "Fermer",
                glyph = NzGlyph.Close,
                kind = fr.neezer.mobile.designsystem.NzButtonKind.Ghost,
                onClick = onClose,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

/** Draws the poster and hands it to the platform, with a text fallback. */
object RecapPoster {
    const val WIDTH = 1080
    const val HEIGHT = 1350

    /** Where the app writes what another app will read (`file_paths.xml`). */
    fun path(context: Context, year: Int): File =
        File(File(context.cacheDir, "shared").apply { mkdirs() }, "recap-$year.png")

    /** Renders the poster to [path] and returns it. */
    fun write(context: Context, data: RecapDeck): File {
        val file = path(context, data.year)
        val bitmap = render(data)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
        return file
    }

    /** The 1080 × 1350 picture: the six lines, the year's grooves, the mark. */
    fun render(data: RecapDeck): Bitmap {
        val bitmap = Bitmap.createBitmap(WIDTH, HEIGHT, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        val background = Color.parseColor("#0F0F0E")
        canvas.drawColor(background)

        // The mosaic: up to four tiles from the year's own sleeves' hashes.
        val seeds = data.mosaic.ifEmpty { listOf("recap-${data.year}") }.take(4)
        val inset = 72f
        val tile = (WIDTH - inset * 2 - 8f) / 2f
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        seeds.forEachIndexed { index, seed ->
            val (top, bottom) = nzGrooveFor(seed).let {
                Color.parseColor("#" + Integer.toHexString(it.first.value.toInt()).takeLast(6)) to
                    Color.parseColor("#" + Integer.toHexString(it.second.value.toInt()).takeLast(6))
            }
            val x = inset + (index % 2) * (tile + 8f)
            val y = 120f + (index / 2) * (tile + 8f)
            paint.color = top
            canvas.drawRoundRect(RectF(x, y, x + tile, y + tile), 18f, 18f, paint)
            paint.color = bottom
            repeat(6) { ring ->
                val radius = 24f + ring * 26f
                canvas.drawCircle(x + tile * 0.68f, y + tile * 0.32f, radius, paint.apply {
                    style = Paint.Style.STROKE
                    strokeWidth = 3f
                })
            }
            paint.style = Paint.Style.FILL
        }

        var y = 120f + (if (seeds.size > 1) tile * 2 + 8f else tile) + 96f
        val kicker = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.parseColor("#B9B7AE")
            textSize = 34f
            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
            letterSpacing = 0.08f
        }
        canvas.drawText("RÉCAP", inset, y, kicker)
        y += 64f
        val title = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.WHITE
            textSize = 56f
            typeface = Typeface.create("sans-serif-black", Typeface.BOLD)
        }
        val body = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.parseColor("#E7E5DC")
            textSize = 44f
            typeface = Typeface.create("sans-serif", Typeface.NORMAL)
        }
        data.posterLines.forEachIndexed { index, line ->
            canvas.drawText(line, inset, y, if (index == 1) title else body)
            y += if (index == 1) 84f else 66f
        }
        val mark = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.parseColor("#B9B7AE")
            textSize = 32f
            letterSpacing = 0.12f
        }
        canvas.drawText("neezer", inset, HEIGHT - 72f, mark)
        return bitmap
    }

    /**
     * Hands the file to the system's share sheet. `FileProvider` because a bare
     * `file://` Uri throws `FileUriExposedException` since Android 7 — the
     * provider `:feature:setup` declares is the app's own, and `file_paths.xml`
     * already allows `cache/shared/`.
     */
    fun share(context: Context, data: RecapDeck): String {
        val file = runCatching { write(context, data) }.getOrNull()
            ?: return "L'affiche n'a pas pu être créée."
        val uri = runCatching {
            FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file)
        }.getOrNull()
        if (uri == null) {
            // No provider (a test, a build without `:feature:setup`): the words
            // still travel, which is the SPA's own text fallback.
            return shareText(context, data)
        }
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "image/png"
            putExtra(Intent.EXTRA_STREAM, uri)
            putExtra(Intent.EXTRA_TEXT, data.shareText)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        val chooser = Intent.createChooser(send, "Partager ton année").apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        return runCatching {
            context.startActivity(chooser)
            "Affiche prête à partager."
        }.getOrElse { shareText(context, data) }
    }

    private fun shareText(context: Context, data: RecapDeck): String {
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, data.shareText)
        }
        return runCatching {
            context.startActivity(Intent.createChooser(send, "Partager ton année").apply {
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            })
            "Partagé en texte."
        }.getOrElse { "Rien ne peut partager sur cet appareil." }
    }
}
