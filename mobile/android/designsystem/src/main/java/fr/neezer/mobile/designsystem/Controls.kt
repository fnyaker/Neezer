package fr.neezer.mobile.designsystem

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * A vertical fader — the ten of the equaliser, and any "0…1" control the app
 * draws by hand.
 *
 * It is measured the way the SPA's faders are: the TRACK is 6 dp, the thumb 20,
 * and the finger's box is 48 × [height] dp, so a control drawn slim still takes
 * a thumb. The fill sits inside the track and its top edge is the value — the
 * keyboard and the screen reader get the value through
 * [ProgressBarRangeInfo], not through the pixels.
 *
 * The level is not animated (M6): it follows the finger.
 */
@Composable
fun NzFader(
    value: Float,
    onValueChange: (Float) -> Unit,
    modifier: Modifier = Modifier,
    height: Dp = 140.dp,
    label: String? = null,
    enabled: Boolean = true,
) {
    val colors = Sillon.colors
    val current = rememberUpdatedState(value)
    val emit = rememberUpdatedState(onValueChange)
    val trackWidth = 6.dp
    val thumb = 20.dp
    fun valueAt(y: Float, h: Float): Float = (1f - (y / h)).coerceIn(0f, 1f)
    Box(
        modifier = modifier
            .sizeIn(minWidth = NzMinTouchTarget, minHeight = NzMinTouchTarget)
            .size(width = NzMinTouchTarget, height = height)
            .semantics {
                if (label != null) contentDescription = label
                progressBarRangeInfo = ProgressBarRangeInfo(value, 0f..1f)
            }
            .pointerInput(enabled) {
                if (!enabled) return@pointerInput
                detectTapGestures { offset ->
                    emit.value(valueAt(offset.y, size.height.toFloat()))
                }
            }
            .pointerInput(enabled) {
                if (!enabled) return@pointerInput
                detectVerticalDragGestures(
                    onDragStart = { offset -> emit.value(valueAt(offset.y, size.height.toFloat())) },
                ) { change, _ ->
                    emit.value(valueAt(change.position.y, size.height.toFloat()))
                    change.consume()
                }
            },
    ) {
        Canvas(Modifier.size(NzMinTouchTarget, height)) {
            val cx = size.width / 2f
            val trackW = trackWidth.toPx()
            val top = thumb.toPx() / 2f
            val bottom = size.height - thumb.toPx() / 2f
            val span = (bottom - top).coerceAtLeast(1f)
            val v = current.value.coerceIn(0f, 1f)
            val knobY = bottom - span * v
            drawRoundRect(
                color = colors.s2,
                topLeft = Offset(cx - trackW / 2f, top),
                size = Size(trackW, span),
                cornerRadius = CornerRadius(trackW / 2f),
            )
            drawRoundRect(
                color = colors.acMark,
                topLeft = Offset(cx - trackW / 2f, knobY),
                size = Size(trackW, bottom - knobY),
                cornerRadius = CornerRadius(trackW / 2f),
            )
            drawCircle(color = colors.tx, radius = thumb.toPx() / 2f, center = Offset(cx, knobY))
        }
    }
}
