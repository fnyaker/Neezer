@file:OptIn(androidx.compose.ui.text.ExperimentalTextApi::class)

package fr.neezer.mobile.designsystem

import androidx.compose.runtime.Immutable
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp

/**
 * The three self-hosted faces (webapp.md, "Typography is self-hosted"):
 * **Bricolage Grotesque** for titles (600–800, `-0.03em` on the big ones),
 * **Instrument Sans** for the interface (400–700) and **JetBrains Mono** for
 * metadata and the small capitals (`.label-mono`, `0.08em`).
 *
 * The files are the variable fonts the SPA ships (`android/app/src/main/res/font`
 * held them for the WebView app); a variable face is instantiated per weight
 * through [FontVariation.Settings], so `W700` really is 700 and not a synthetic
 * squash. Bricolage ships WITHOUT its optical-size axis — identical text width
 * from 14 to 44 px — which is why one file covers the whole scale.
 */
private fun variable(id: Int, weight: Int): Font = Font(
    resId = id,
    weight = FontWeight(weight),
    variationSettings = FontVariation.Settings(FontVariation.weight(weight)),
)

private val Bricolage = FontFamily(
    variable(R.font.bricolage_grotesque, 600),
    variable(R.font.bricolage_grotesque, 700),
    variable(R.font.bricolage_grotesque, 800),
)

private val Instrument = FontFamily(
    variable(R.font.instrument_sans, 400),
    variable(R.font.instrument_sans, 500),
    variable(R.font.instrument_sans, 600),
    variable(R.font.instrument_sans, 700),
)

private val JetBrainsMono = FontFamily(
    variable(R.font.jetbrains_mono, 400),
    variable(R.font.jetbrains_mono, 500),
)

/** The display face, exposed for a screen that draws its own oversized title. */
val NzDisplayFont: FontFamily get() = Bricolage

/** The interface face. */
val NzBodyFont: FontFamily get() = Instrument

/** The figures and small capitals. */
val NzMonoFont: FontFamily get() = JetBrainsMono

/**
 * The type scale, named by ROLE, not by size: a screen asks for `title`, never
 * for `24.sp`. Values are `app.css`'s (`.page-title` 44/700/-0.03em/1.06,
 * `.section-title` 24/600/-0.015em/1.15, body 16/1.45, `.label-mono`
 * 11/500/0.08em, `.mono` 11/0.02em).
 */
@Immutable
data class NzTypography(
    /** A page's title — 44 px on a wide screen, 32 on a phone. */
    val display: TextStyle = TextStyle(
        fontFamily = Bricolage,
        fontWeight = FontWeight.W700,
        fontSize = 44.sp,
        lineHeight = 46.6.sp,
        letterSpacing = (-0.03).em,
    ),
    /** The same title on a phone (`@media (max-width: 640px)` narrows it). */
    val displaySmall: TextStyle = TextStyle(
        fontFamily = Bricolage,
        fontWeight = FontWeight.W700,
        fontSize = 32.sp,
        lineHeight = 33.9.sp,
        letterSpacing = (-0.03).em,
    ),
    /** A section's title (`.section-title`). */
    val title: TextStyle = TextStyle(
        fontFamily = Bricolage,
        fontWeight = FontWeight.W600,
        fontSize = 24.sp,
        lineHeight = 27.6.sp,
        letterSpacing = (-0.015).em,
    ),
    /** A card's or a sheet's heading. */
    val titleSmall: TextStyle = TextStyle(
        fontFamily = Bricolage,
        fontWeight = FontWeight.W600,
        fontSize = 19.sp,
        lineHeight = 22.sp,
        letterSpacing = (-0.01).em,
    ),
    /** Body copy: 16 px, 1.45. */
    val body: TextStyle = TextStyle(
        fontFamily = Instrument,
        fontWeight = FontWeight.W400,
        fontSize = 16.sp,
        lineHeight = 23.2.sp,
    ),
    /** A row's subtitle, a caption. */
    val bodySmall: TextStyle = TextStyle(
        fontFamily = Instrument,
        fontWeight = FontWeight.W400,
        fontSize = 14.sp,
        lineHeight = 19.6.sp,
    ),
    /** A button's or a chip's label — 13.5 px, 600, tight. */
    val label: TextStyle = TextStyle(
        fontFamily = Instrument,
        fontWeight = FontWeight.W600,
        fontSize = 13.5.sp,
        lineHeight = 14.sp,
    ),
    /** A secondary label (a count, an album on a row). */
    val labelSmall: TextStyle = TextStyle(
        fontFamily = Instrument,
        fontWeight = FontWeight.W500,
        fontSize = 12.sp,
        lineHeight = 15.sp,
    ),
    /** `.mono`: figures — durations, counts, the quality badge. */
    val mono: TextStyle = TextStyle(
        fontFamily = JetBrainsMono,
        fontWeight = FontWeight.W400,
        fontSize = 11.sp,
        lineHeight = 11.sp,
        letterSpacing = 0.02.em,
    ),
    /** `.label-mono`: the small capitals, 0.08em — uppercased by the caller. */
    val labelMono: TextStyle = TextStyle(
        fontFamily = JetBrainsMono,
        fontWeight = FontWeight.W500,
        fontSize = 11.sp,
        lineHeight = 11.sp,
        letterSpacing = 0.08.em,
    ),
)

/** The one instance: the scale holds no theme-dependent value. */
internal val NzTypographyTable = NzTypography()
