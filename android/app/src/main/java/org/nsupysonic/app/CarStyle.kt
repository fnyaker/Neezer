package org.nsupysonic.app

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.util.TypedValue
import android.view.Gravity
import android.widget.TextView

/** Sillon's car palette and the few Android factories every car view shares. */
object CarStyle {
    val BG = Color.parseColor("#0C0C0B")
    val S1 = Color.parseColor("#151513")
    val S2 = Color.parseColor("#1E1E1B")
    val LN = Color.parseColor("#2A2A26")
    val TX = Color.parseColor("#F1EFE8")
    val DIM = Color.parseColor("#A09D92")
    val AC = Color.parseColor("#D2FF3C")
    val ONAC = Color.parseColor("#0C0C0B")
    val WARN = Color.parseColor("#FF8A65")

    fun dp(context: Context, value: Float): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value, context.resources.displayMetrics).toInt()

    fun sp(value: Float): Float = value

    fun roundRect(context: Context, fill: Int, stroke: Int = Color.TRANSPARENT, radiusDp: Float = 20f, strokeDp: Float = 1f): GradientDrawable =
        GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = dp(context, radiusDp).toFloat()
            setColor(fill)
            if (stroke != Color.TRANSPARENT) setStroke(dp(context, strokeDp), stroke)
        }

    fun pill(context: Context, fill: Int, stroke: Int = Color.TRANSPARENT): GradientDrawable =
        GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = 999f
            setColor(fill)
            if (stroke != Color.TRANSPARENT) setStroke(dp(context, 1f), stroke)
        }

    fun text(context: Context, value: String, size: Float, color: Int = TX, display: Boolean = false): TextView =
        TextView(context).apply {
            text = value
            setTextColor(color)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, size)
            typeface = if (display) CarType.display(context) else CarType.body(context)
            gravity = Gravity.CENTER_VERTICAL
            includeFontPadding = false
        }

    fun monoText(context: Context, value: String, size: Float, color: Int = DIM): TextView =
        TextView(context).apply {
            text = value
            setTextColor(color)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, size)
            typeface = CarType.mono(context)
            includeFontPadding = false
        }
}
