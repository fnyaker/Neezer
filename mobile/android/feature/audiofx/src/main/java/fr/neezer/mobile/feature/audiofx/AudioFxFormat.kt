package fr.neezer.mobile.feature.audiofx

import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * The card's numbers, in French: a real minus sign, a comma, one decimal — the
 * same shapes `webapp/src/lib/eqpresets.js#fmtDb` prints, so a person who read
 * the web app reads the same values here.
 */
internal object FxFormat {
    private const val MINUS = "\u2212"

    /** `-12.04` → « −12,0 dB » ; `6.0` with [plus] → « +6,0 dB ». */
    fun db(value: Double, plus: Boolean = false): String {
        val shown = if (abs(value) < 0.05) 0.0 else value
        val body = String.format(java.util.Locale.ROOT, "%.1f", abs(shown)).replace('.', ',')
        val sign = when {
            shown < 0 -> MINUS
            plus && shown > 0 -> "+"
            else -> ""
        }
        return "$sign$body dB"
    }

    /** A fraction 0..1 as a percentage: « 35 % ». */
    fun percent(fraction: Float): String = "${(fraction.coerceIn(0f, 1f) * 100).roundToInt()} %"

    /** A gain in dB with the sign the fader shows: « +6 », « 0 », « −3 ». */
    fun gain(db: Float): String {
        val v = db.roundToInt()
        return when {
            v > 0 -> "+$v"
            v < 0 -> "$MINUS${-v}"
            else -> "0"
        }
    }

    /** A crossfade length: « 6,0 s », or « enchaîné direct » at zero. */
    fun fade(seconds: Float): String =
        if (seconds <= 0f) "enchaîné direct"
        else String.format(java.util.Locale.ROOT, "%.1f s", seconds).replace('.', ',')

    /** A manual skip's fade: « 1,5 s », or « coupe douce » at zero. */
    fun skipFade(seconds: Float): String =
        if (seconds <= 0f) "coupe douce"
        else String.format(java.util.Locale.ROOT, "%.2f", seconds).replace('.', ',').trimEnd('0').trimEnd(',') + " s"

    /** A band frequency: « 31 », « 62 », … « 16k ». */
    fun frequency(hz: Int): String = if (hz >= 1000) "${hz / 1000}k" else hz.toString()
}
