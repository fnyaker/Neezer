package org.nsupysonic.app

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.util.AttributeSet
import android.view.View
import kotlin.math.min

/** The handful of line icons the car surface draws (no emoji, no font icon). */
class CarIconView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND
        strokeJoin = Paint.Join.ROUND
    }
    private val path = Path()
    private val oval = RectF()

    var name: String = "search"
        set(v) {
            field = v
            invalidate()
        }
    var sizeDp: Int = 24
        set(v) {
            field = v
            requestLayout()
            invalidate()
        }
    var color: Int = CarStyle.TX
        set(v) {
            field = v
            invalidate()
        }
    var filled: Boolean = false
        set(v) {
            field = v
            invalidate()
        }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val px = CarStyle.dp(context, sizeDp.toFloat())
        setMeasuredDimension(px, px)
    }

    override fun onDraw(canvas: Canvas) {
        val size = min(width, height).toFloat()
        if (size <= 0f) return
        val scale = size / 24f
        canvas.save()
        canvas.scale(scale, scale)
        paint.color = color
        paint.strokeWidth = 2f
        paint.style = if (filled) Paint.Style.FILL else Paint.Style.STROKE
        path.reset()

        when (name) {
            "search" -> {
                canvas.drawCircle(11f, 11f, 7f, paint)
                path.moveTo(16.5f, 16.5f)
                path.lineTo(21f, 21f)
                canvas.drawPath(path, paint)
            }
            "mic" -> {
                oval.set(9f, 3f, 15f, 15f)
                canvas.drawRoundRect(oval, 3f, 3f, paint)
                path.moveTo(5f, 11f)
                path.cubicTo(5f, 17f, 8f, 19f, 12f, 19f)
                path.cubicTo(16f, 19f, 19f, 17f, 19f, 11f)
                canvas.drawPath(path, paint)
                path.reset()
                path.moveTo(12f, 19f)
                path.lineTo(12f, 22f)
                canvas.drawPath(path, paint)
            }
            "heart" -> {
                path.moveTo(12f, 21f)
                path.cubicTo(4f, 15.8f, 4f, 9.5f, 8f, 7.2f)
                path.cubicTo(10f, 6f, 11.3f, 7f, 12f, 8f)
                path.cubicTo(12.7f, 7f, 14f, 6f, 16f, 7.2f)
                path.cubicTo(20f, 9.5f, 20f, 15.8f, 12f, 21f)
                path.close()
                paint.style = if (filled) Paint.Style.FILL else Paint.Style.STROKE
                canvas.drawPath(path, paint)
            }
            "pin" -> {
                path.moveTo(14f, 3f)
                path.lineTo(21f, 10f)
                path.lineTo(18f, 11f)
                path.lineTo(15f, 14f)
                path.lineTo(15.5f, 18.5f)
                path.lineTo(12f, 15f)
                path.lineTo(7f, 20f)
                path.lineTo(6f, 19f)
                path.lineTo(11f, 14f)
                path.lineTo(7f, 10.5f)
                path.lineTo(11.5f, 11f)
                path.lineTo(14.5f, 8f)
                path.close()
                paint.style = if (filled) Paint.Style.FILL else Paint.Style.STROKE
                canvas.drawPath(path, paint)
            }
            "playlist" -> {
                path.moveTo(3f, 6f); path.lineTo(16f, 6f)
                path.moveTo(3f, 12f); path.lineTo(16f, 12f)
                path.moveTo(3f, 18f); path.lineTo(11f, 18f)
                path.moveTo(19f, 14f); path.lineTo(19f, 21f)
                path.moveTo(16f, 18f); path.lineTo(19f, 21f); path.lineTo(22f, 18f)
                canvas.drawPath(path, paint)
            }
            "close" -> {
                path.moveTo(6f, 6f); path.lineTo(18f, 18f)
                path.moveTo(18f, 6f); path.lineTo(6f, 18f)
                canvas.drawPath(path, paint)
            }
            "chevronUp" -> {
                path.moveTo(6f, 15f); path.lineTo(12f, 9f); path.lineTo(18f, 15f)
                canvas.drawPath(path, paint)
            }
            "chevronDown" -> {
                path.moveTo(6f, 9f); path.lineTo(12f, 15f); path.lineTo(18f, 9f)
                canvas.drawPath(path, paint)
            }
            "chevronLeft" -> {
                path.moveTo(15f, 5f); path.lineTo(8f, 12f); path.lineTo(15f, 19f)
                canvas.drawPath(path, paint)
            }
            "chevronRight" -> {
                path.moveTo(9f, 5f); path.lineTo(16f, 12f); path.lineTo(9f, 19f)
                canvas.drawPath(path, paint)
            }
            "plus" -> {
                path.moveTo(12f, 5f); path.lineTo(12f, 19f)
                path.moveTo(5f, 12f); path.lineTo(19f, 12f)
                canvas.drawPath(path, paint)
            }
            "move" -> {
                // Four arrows from one centre: the drag handle of the car
                // card (no emoji, no font icon).
                path.moveTo(12f, 3f); path.lineTo(12f, 21f)
                path.moveTo(3f, 12f); path.lineTo(21f, 12f)
                path.moveTo(9f, 6f); path.lineTo(12f, 3f); path.lineTo(15f, 6f)
                path.moveTo(9f, 18f); path.lineTo(12f, 21f); path.lineTo(15f, 18f)
                path.moveTo(6f, 9f); path.lineTo(3f, 12f); path.lineTo(6f, 15f)
                path.moveTo(18f, 9f); path.lineTo(21f, 12f); path.lineTo(18f, 15f)
                canvas.drawPath(path, paint)
            }
            "more" -> {
                paint.style = Paint.Style.FILL
                canvas.drawCircle(5f, 12f, 1.8f, paint)
                canvas.drawCircle(12f, 12f, 1.8f, paint)
                canvas.drawCircle(19f, 12f, 1.8f, paint)
            }
            "pause" -> {
                paint.style = Paint.Style.FILL
                canvas.drawRoundRect(RectF(6f, 4f, 10f, 20f), 1f, 1f, paint)
                canvas.drawRoundRect(RectF(14f, 4f, 18f, 20f), 1f, 1f, paint)
            }
            else -> {
                path.moveTo(7f, 4f); path.lineTo(20f, 12f); path.lineTo(7f, 20f); path.close()
                paint.style = Paint.Style.FILL
                canvas.drawPath(path, paint)
            }
        }
        canvas.restore()
    }
}
