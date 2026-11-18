package fr.neezer.mobile.feature.party

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.common.BitMatrix
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/**
 * The link as a QR code, the way `PartySheet.svelte` shows it for the phones in
 * the room: whoever cannot be handed the link by the share sheet scans it.
 *
 * The matrix comes from ZXing (the SPA's `qrcode-generator`'s role) at error
 * correction M, and it is drawn cell by cell on Compose's canvas on a WHITE
 * ground: a QR is a machine-readable mark, so it keeps the two colours every
 * reader expects — Encre or Papier behind it, white inside it (the same choice
 * `PartySheet.svelte` makes).
 */
@Composable
fun PartyQr(link: String, size: Dp, modifier: Modifier = Modifier) {
    val matrix = remember(link) { qrMatrix(link) }
    val dark = Color(0xFF0F0D13)
    val light = Color.White
    Box(
        modifier = modifier
            .size(size)
            .clip(RoundedCornerShape(12.dp))
            .background(light)
            .semantics { contentDescription = PartyCopy.SCAN },
    ) {
        if (matrix != null) {
            Canvas(Modifier.size(size)) {
                val cells = matrix.width
                val cell = this.size.minDimension / cells
                for (row in 0 until cells) {
                    for (column in 0 until cells) {
                        if (matrix.get(column, row)) {
                            drawRect(
                                color = dark,
                                topLeft = Offset(column * cell, row * cell),
                                size = Size(cell + 0.5f, cell + 0.5f),
                            )
                        }
                    }
                }
            }
        }
    }
}

/** The matrix, or `null` for a link ZXing refuses (never thrown at a screen). */
fun qrMatrix(link: String): BitMatrix? {
    if (link.isEmpty()) return null
    return try {
        val hints =
            mapOf(
                EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M,
                // The quiet zone is drawn by the white ground, not by the matrix.
                EncodeHintType.MARGIN to 0,
                EncodeHintType.CHARACTER_SET to "UTF-8",
            )
        QRCodeWriter().encode(link, BarcodeFormat.QR_CODE, 0, 0, hints)
    } catch (_: Exception) {
        null
    }
}
