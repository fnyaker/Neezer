package org.nsupysonic.app

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.util.AttributeSet
import android.view.View
import kotlin.math.max
import kotlin.math.min

/**
 * A record sleeve for the car: the eight Sillon groove pairs, deterministic
 * from the id, exactly like the SPA's `Cover` fallback. The car does not fetch
 * a 500 px CDN image for a 48 dp tile; the grooves are the design's own answer
 * to a sleeve we do not hold, and they never wait for a network.
 */
class CarCoverView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val clip = Path()
    private val bounds = RectF()

    var seed: String = ""
        set(v) {
            field = v
            invalidate()
        }

    var sizeDp: Int = 72
        set(v) {
            field = v
            requestLayout()
            invalidate()
        }

    var round: Boolean = false
        set(v) {
            field = v
            invalidate()
        }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val px = CarStyle.dp(context, sizeDp.toFloat())
        setMeasuredDimension(px, px)
    }

    override fun onDraw(canvas: Canvas) {
        val w = width.toFloat()
        val h = height.toFloat()
        if (w <= 0f || h <= 0f) return
        val pair = GROOVES[(hash(seed) % GROOVES.size + GROOVES.size) % GROOVES.size]
        val radius = if (round) min(w, h) / 2f else CarStyle.dp(context, 12f).toFloat()
        bounds.set(0f, 0f, w, h)
        clip.reset()
        clip.addRoundRect(bounds, radius, radius, Path.Direction.CW)
        canvas.save()
        canvas.clipPath(clip)
        paint.style = Paint.Style.FILL
        paint.color = Color.parseColor(pair[0])
        canvas.drawRect(bounds, paint)

        paint.style = Paint.Style.STROKE
        paint.strokeWidth = max(1f, CarStyle.dp(context, 1f).toFloat())
        val ringStep = max(1f, CarStyle.dp(context, 5f).toFloat())
        paint.color = Color.parseColor(pair[1])
        val cx = w * 0.68f
        val cy = h * 0.32f
        val maxR = max(w, h) * 1.1f
        var r = ringStep
        while (r < maxR) {
            canvas.drawCircle(cx, cy, r, paint)
            r += ringStep
        }
        canvas.restore()
    }

    private fun hash(seed: String): Int {
        var h = -0x7ee3623b // 0x811c9dc5
        for (i in seed.indices) {
            h = h xor seed[i].code
            h *= 16777619
        }
        return h
    }

    companion object {
        private val GROOVES = arrayOf(
            arrayOf("#2b2b29", "#3b3b37"),
            arrayOf("#7a1d12", "#9b2a1b"),
            arrayOf("#14304a", "#1c4466"),
            arrayOf("#4a2a62", "#5e3a7b"),
            arrayOf("#1f4a3a", "#2a634e"),
            arrayOf("#6b5a14", "#8a7420"),
            arrayOf("#5a1f3a", "#7a2c50"),
            arrayOf("#3a3a33", "#55554a"),
        )
    }
}
