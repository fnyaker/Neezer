package org.nsupysonic.app

import android.content.Context
import android.graphics.Typeface
import androidx.core.content.res.ResourcesCompat

/**
 * Sillon's three faces, bundled (no Google Fonts request while driving).
 *
 * Bricolage Grotesque for titles, Instrument Sans for the interface, JetBrains
 * Mono for the small capitals and the numbers. Android cannot use the woff2
 * the web app ships; these are the same OFL families as TTF.
 */
object CarType {
    @Volatile private var displayFace: Typeface? = null
    @Volatile private var bodyFace: Typeface? = null
    @Volatile private var monoFace: Typeface? = null

    fun display(context: Context): Typeface =
        displayFace ?: (ResourcesCompat.getFont(context, R.font.bricolage_grotesque)
            ?: Typeface.DEFAULT_BOLD).also { displayFace = it }

    fun body(context: Context): Typeface =
        bodyFace ?: (ResourcesCompat.getFont(context, R.font.instrument_sans)
            ?: Typeface.DEFAULT).also { bodyFace = it }

    fun mono(context: Context): Typeface =
        monoFace ?: (ResourcesCompat.getFont(context, R.font.jetbrains_mono)
            ?: Typeface.MONOSPACE).also { monoFace = it }
}
