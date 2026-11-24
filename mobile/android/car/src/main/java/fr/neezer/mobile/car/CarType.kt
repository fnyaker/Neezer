package fr.neezer.mobile.car

import android.content.Context
import android.graphics.Typeface
import androidx.core.content.res.ResourcesCompat
import fr.neezer.mobile.designsystem.R as SillonR

/**
 * Sillon's three faces, bundled (no Google Fonts request while driving).
 *
 * Bricolage Grotesque for titles, Instrument Sans for the interface, JetBrains
 * Mono for the small capitals and the numbers. Android cannot use the woff2
 * the web app ships; these are the same OFL families as TTF.
 *
 * The files live in `:designsystem` (MB1 bundled them once, for every screen);
 * the car reads them from there instead of carrying a second copy — two modules
 * declaring the same `font/bricolage_grotesque` would make AAPT2 refuse the
 * merge, and a second copy on disk is bytes the APK does not need.
 */
object CarType {
    @Volatile private var displayFace: Typeface? = null
    @Volatile private var bodyFace: Typeface? = null
    @Volatile private var monoFace: Typeface? = null

    fun display(context: Context): Typeface =
        displayFace ?: (ResourcesCompat.getFont(context, SillonR.font.bricolage_grotesque)
            ?: Typeface.DEFAULT_BOLD).also { displayFace = it }

    fun body(context: Context): Typeface =
        bodyFace ?: (ResourcesCompat.getFont(context, SillonR.font.instrument_sans)
            ?: Typeface.DEFAULT).also { bodyFace = it }

    fun mono(context: Context): Typeface =
        monoFace ?: (ResourcesCompat.getFont(context, SillonR.font.jetbrains_mono)
            ?: Typeface.MONOSPACE).also { monoFace = it }
}
