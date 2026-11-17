package fr.neezer.mobile.designsystem

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.roundToInt

/**
 * The three Sillon themes: `encre` (dark, lemon accent — the default), `papier`
 * (warm light, ink accent) and `pochette` (dark, the accent IS the playing
 * sleeve's colour).
 *
 * The theme is not a prop a screen passes around: [NzTheme] provides it once and
 * every component reads [Sillon.colors]. A switch repaints, it does not rebuild
 * a screen (webapp.md, "A theme is an attribute on `<html>`").
 */
enum class NzThemeKind { Encre, Papier, Pochette }

// -- colour maths -------------------------------------------------------------
//
// The same rules `webapp/src/lib/sillon.js` and `lib/theme.js` run, ported so
// the glass tints, the readable accent and the contrast test all read ONE
// implementation. `webapp/src/app.css` mixes with CSS `color-mix(in srgb, …)`,
// which averages the sRGB channels as they are stored (not in linear light):
// [nzMix] does exactly that, so a tint computed here equals the browser's.

internal fun nzChannel(c: Float): Double = c.toDouble().coerceIn(0.0, 1.0)

/** WCAG 2.1 relative luminance of an opaque colour (the SPA's `luminance`). */
fun nzLuminance(color: Color): Double {
    fun lin(v: Float): Double {
        val c = nzChannel(v)
        return if (c <= 0.03928) c / 12.92 else ((c + 0.055) / 1.055).pow(2.4)
    }
    return 0.2126 * lin(color.red) + 0.7152 * lin(color.green) + 0.0722 * lin(color.blue)
}

/** WCAG 2.1 contrast ratio, 1…21 (the SPA's `contrast`). */
fun nzContrast(a: Color, b: Color): Double {
    val la = nzLuminance(a)
    val lb = nzLuminance(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)
}

/** `color-mix(in srgb, a weightOfA, b)`: channel-wise, alpha composited if any is translucent. */
fun nzMix(a: Color, b: Color, weightOfA: Float): Color {
    val w = weightOfA.coerceIn(0f, 1f)
    return Color(
        red = a.red * w + b.red * (1f - w),
        green = a.green * w + b.green * (1f - w),
        blue = a.blue * w + b.blue * (1f - w),
        alpha = a.alpha * w + b.alpha * (1f - w),
    )
}

/** [top] painted over an opaque [bottom] (`alpha·top + (1−alpha)·bottom`). */
fun nzOver(top: Color, bottom: Color): Color {
    val a = top.alpha
    return Color(
        red = top.red * a + bottom.red * (1f - a),
        green = top.green * a + bottom.green * (1f - a),
        blue = top.blue * a + bottom.blue * (1f - a),
        alpha = 1f,
    )
}

fun nzWithAlpha(color: Color, alpha: Float): Color = color.copy(alpha = alpha.coerceIn(0f, 1f))

/** The four channels as 0…255 integers — the form the SPA's maths works in. */
internal fun nzRgb(color: Color): IntArray =
    intArrayOf(
        (color.red * 255f).roundToInt().coerceIn(0, 255),
        (color.green * 255f).roundToInt().coerceIn(0, 255),
        (color.blue * 255f).roundToInt().coerceIn(0, 255),
    )

internal fun nzFromRgb(rgb: IntArray): Color =
    Color(rgb[0].coerceIn(0, 255), rgb[1].coerceIn(0, 255), rgb[2].coerceIn(0, 255), 255)

/**
 * Brighten [color] until it carries [ratio] against [on] — the SPA's
 * `readableAccent` (`lib/sillon.js`): a MULTIPLICATION of the channels (which
 * keeps the hue; blending toward white turned a deep sleeve blue into
 * grey-lilac), then a white blend for a colour with a dead channel.
 */
fun nzReadableInk(color: Color, on: Color, ratio: Double = 4.5): Color {
    val base = nzRgb(color)
    val bg = nzRgb(on)
    if (nzContrast(nzFromRgb(base), nzFromRgb(bg)) >= ratio) return nzFromRgb(base)
    var last = base
    var s = 1.05
    while (s <= 8.0001) {
        last = IntArray(3) { (base[it] * s).roundToInt().coerceIn(0, 255) }
        if (nzContrast(nzFromRgb(last), nzFromRgb(bg)) >= ratio) return nzFromRgb(last)
        s += 0.05
    }
    var k = 0.05
    while (k <= 1.0001) {
        val mixed = IntArray(3) { (last[it] + (255 - last[it]) * k).roundToInt().coerceIn(0, 255) }
        if (nzContrast(nzFromRgb(mixed), nzFromRgb(bg)) >= ratio) return nzFromRgb(mixed)
        k += 0.05
    }
    return Color.White
}

/**
 * A colour made legible as INK on [background] (`lib/theme.js#inkFor`): darkened
 * on a light theme, lightened on a dark one. Returns the colour it had to use
 * and whether it moved, so Réglages can say « l'encre a été ajustée ».
 */
fun nzInkFor(color: Color, background: Color): Pair<Color, Boolean> {
    if (nzContrast(color, background) >= 4.5) return color to false
    val rgb = nzRgb(color)
    val bg = nzRgb(background)
    if (nzLuminance(background) > 0.5) {
        var last = rgb
        var s = 0.95
        while (s >= 0.0499) {
            last = IntArray(3) { (rgb[it] * s).roundToInt().coerceIn(0, 255) }
            if (nzContrast(nzFromRgb(last), nzFromRgb(bg)) >= 4.5) return nzFromRgb(last) to true
            s -= 0.05
        }
        var k = 0.05
        while (k <= 1.0001) {
            val mixed = IntArray(3) { (last[it] * (1 - k)).roundToInt().coerceIn(0, 255) }
            if (nzContrast(nzFromRgb(mixed), nzFromRgb(bg)) >= 4.5) return nzFromRgb(mixed) to true
            k += 0.05
        }
        return Color.Black to true
    }
    return nzReadableInk(color, background) to true
}

/**
 * The whole pair a custom accent needs on one theme (`lib/theme.js#accentVars`).
 *
 * [surface] is `--ac`/`--btn` and is a SURFACE: text on it is [on], pure black
 * or pure white, whichever wins — a near-black would miss 4.5:1 on a mid grey.
 * [ink] is `--ac-text`/`--ac-mark`/`--hrt`, a legible FOREGROUND, corrected
 * against the most demanding surface it is drawn on (Encre's `--s2`, Papier's
 * `--s1`) and never against the page's ground.
 */
@Immutable
data class NzAccent(
    val surface: Color,
    val on: Color,
    val ink: Color,
    val adjusted: Boolean,
)

/**
 * The surfaces an accent is drawn ON as ink: the page's ground, a card and a
 * chip. All three, because `--ac-text` is drawn on all three.
 *
 * MEASURED, and the reason this is a list and not the SPA's single `--s2`
 * (`lib/theme.js#INK_BG`): correcting `#7C5CFF`'s ink against one surface left
 * it at **4.18:1 on Papier's `--bg` and 3.74:1 on its `--s2`** — under AA on two
 * of the three surfaces it is painted on. [nzAccent] therefore walks every
 * surface; on a light theme the corrections only darken and on a dark one they
 * only lighten, so the walk is monotone and the last answer is the strictest.
 */
private val INK_SURFACES = mapOf(
    NzThemeKind.Encre to listOf(Color(0xFF0C0C0B), Color(0xFF151513), Color(0xFF1E1E1B)),
    NzThemeKind.Papier to listOf(Color(0xFFF3F0E8), Color(0xFFFBF9F4), Color(0xFFE8E4D9)),
    NzThemeKind.Pochette to listOf(Color(0xFF0F0E10), Color(0xFF18171A), Color(0xFF222024)),
)

fun nzAccent(raw: Color, kind: NzThemeKind = NzThemeKind.Encre): NzAccent {
    val black = nzContrast(raw, Color.Black)
    val white = nzContrast(raw, Color.White)
    val on = if (black >= white) Color.Black else Color.White
    var ink = raw
    var adjusted = false
    for (surface in INK_SURFACES.getValue(kind)) {
        val (next, moved) = nzInkFor(ink, surface)
        ink = next
        adjusted = adjusted || moved
    }
    return NzAccent(surface = raw, on = on, ink = ink, adjusted = adjusted)
}

// -- the tokens ---------------------------------------------------------------

/**
 * Every colour a screen may draw with, resolved for one theme.
 *
 * `ac` is a SURFACE (text on it is [onac]); a foreground accent is [acText] or
 * [acMark]. On Papier the lemon is a highlighter and never ink — acText there is
 * the theme's own ink (webapp.md, "A theme is a COLOUR PAIR").
 */
@Immutable
data class NzColors(
    val kind: NzThemeKind,
    /** The page's ground. */
    val bg: Color,
    /** First surface: cards, the sidebar, the player bar. */
    val s1: Color,
    /** Second surface: hover, chips, wells. */
    val s2: Color,
    /** Hairlines. */
    val ln: Color,
    /** Body text. */
    val tx: Color,
    /** Secondary text and metadata. */
    val dim: Color,
    /** The accent as a SURFACE. */
    val ac: Color,
    /** Text drawn on [ac]. */
    val onac: Color,
    /** The accent as TEXT (focus ring, links, the current track). */
    val acText: Color,
    /** The accent as a MARK (heart, progress, indicator). */
    val acMark: Color,
    /** The action surface of a primary button/pill (--btn). */
    val btn: Color,
    /** Text on [btn]. */
    val onbtn: Color,
    /** The heart. */
    val hrt: Color,
    /** Glass tier 1's fill at 64 % (no blur). */
    val glassBg: Color,
    /** Glass tier 2's fill at 78 % (blur 14 dp on API ≥ 31). */
    val glassBg2: Color,
    /** The thick step, 93 %: the sticky bar, and a sheet over a photograph. */
    val glassBgStrong: Color,
    /** The glass hairline (13 % of the ink). */
    val glassBorder: Color,
    /** A hover over glass (9 % of the ink over tier 2). */
    val glassHover: Color,
    /** The specular hairline on the lit edge. */
    val glassHi: Color,
    /** The darker rim of the material's thickness. */
    val glassLo: Color,
    val glassShadow: Color,
    val glassShadow2: Color,
    /** The 60 % veil under a sheet. */
    val scrim: Color,
    /** `color-scheme` in CSS: whether the platform draws light-on-dark. */
    val isDark: Boolean,
    /** A custom accent whose ink had to be lifted or darkened. */
    val accentAdjusted: Boolean = false,
) {
    /** Glass tier 2's blur: a popover. */
    val glassBlur2: Dp get() = 14.dp

    /** Glass tier 3's blur: a sheet. */
    val glassBlur3: Dp get() = 22.dp
}

/** The exact tables of `webapp/src/app.css` (`:root`, `[data-theme]`). */
private data class NzBase(
    val bg: Color,
    val s1: Color,
    val s2: Color,
    val ln: Color,
    val tx: Color,
    val dim: Color,
    val ac: Color,
    val onac: Color,
    val acText: Color,
    val acMark: Color,
    val btn: Color,
    val onbtn: Color,
    val hrt: Color,
    val hi: Color,
    val lo: Color,
    val shadow1: Color,
    val shadow2: Color,
    val isDark: Boolean,
)

private val WHITE = Color.White
private val BLACK = Color.Black

private val ENCRE = NzBase(
    bg = Color(0xFF0C0C0B),
    s1 = Color(0xFF151513),
    s2 = Color(0xFF1E1E1B),
    ln = Color(0xFF2A2A26),
    tx = Color(0xFFF1EFE8),
    dim = Color(0xFFA09D92),
    ac = Color(0xFFD2FF3C),
    onac = Color(0xFF0C0C0B),
    acText = Color(0xFFD2FF3C),
    acMark = Color(0xFFD2FF3C),
    btn = Color(0xFFD2FF3C),
    onbtn = Color(0xFF0C0C0B),
    hrt = Color(0xFFD2FF3C),
    hi = Color(0x29FFFFFF),
    lo = Color(0x8C000000),
    shadow1 = Color(0x57000000),
    shadow2 = Color(0x75000000),
    isDark = true,
)

private val PAPIER = NzBase(
    bg = Color(0xFFF3F0E8),
    s1 = Color(0xFFFBF9F4),
    s2 = Color(0xFFE8E4D9),
    ln = Color(0xFFD6D1C3),
    tx = Color(0xFF14130F),
    dim = Color(0xFF5F5B4F),
    ac = Color(0xFFD2FF3C),
    onac = Color(0xFF14130F),
    acText = Color(0xFF14130F),
    acMark = Color(0xFF14130F),
    btn = Color(0xFF14130F),
    onbtn = Color(0xFFF3F0E8),
    hrt = Color(0xFF14130F),
    hi = Color(0xF2FFFFFF),
    lo = Color(0x1714130F),
    shadow1 = Color(0x2914130F),
    shadow2 = Color(0x3D14130F),
    isDark = false,
)

private val POCHETTE = NzBase(
    bg = Color(0xFF0F0E10),
    s1 = Color(0xFF18171A),
    s2 = Color(0xFF222024),
    ln = Color(0xFF2E2C31),
    tx = Color(0xFFF4F1F5),
    dim = Color(0xFFA8A3AD),
    ac = Color(0xFFFF6B4A),
    onac = Color(0xFF0C0C0B),
    acText = Color(0xFFF4F1F5),
    acMark = Color(0xFFFF6B4A),
    btn = Color(0xFFFF6B4A),
    onbtn = Color(0xFF0C0C0B),
    hrt = Color(0xFFFF6B4A),
    hi = Color(0x2EFFFFFF),
    lo = Color(0x8C000000),
    shadow1 = Color(0x5C000000),
    shadow2 = Color(0x7A000000),
    isDark = true,
)

/** `--cover-ac`'s designed fallback (Pochette with no sleeve yet). */
val NzCoverFallback: Color = Color(0xFFFF6B4A)

private fun glassFill(ac: Color, s1: Color): Color {
    // --glass-fill: 3 % of white over (4 % of the accent mixed into --s1).
    val tinted = nzMix(ac, s1, 0.04f)
    return nzMix(WHITE, tinted, 0.03f)
}

/** `--onac` on Pochette: the ink the sleeve's colour is lifted against. */
private val ON_ACCENT = Color(0xFF0C0C0B)

private fun resolve(kind: NzThemeKind, accent: Color?, coverAccent: Color?): NzColors {
    val base = when (kind) {
        NzThemeKind.Encre -> ENCRE
        NzThemeKind.Papier -> PAPIER
        NzThemeKind.Pochette -> POCHETTE
    }
    // Which accent pair this theme draws with:
    //  * Pochette's IS the sleeve (a custom accent never applies there — the
    //    field in Réglages does not touch `--cover-ac`), lifted until black text
    //    on it carries 4.5:1 (lib/sillon.js#readableAccent, above ON_ACCENT);
    //  * a custom accent on Encre/Papier (lib/theme.js#accentVars);
    //  * else the theme's own table — and on Papier `--btn` is the INK, never
    //    the lemon, so a primary button is not a highlighter.
    val pair: NzAccent = when {
        kind == NzThemeKind.Pochette -> {
            val sleeve = coverAccent ?: NzCoverFallback
            val lifted = nzReadableInk(sleeve, ON_ACCENT)
            NzAccent(lifted, ON_ACCENT, base.tx, lifted != sleeve)
        }
        accent != null -> nzAccent(accent, kind)
        else -> NzAccent(base.ac, base.onac, base.acText, false)
    }
    val custom = accent != null && kind != NzThemeKind.Pochette
    val sleeve = kind == NzThemeKind.Pochette
    val surfaceAccent = custom || sleeve
    val ac = pair.surface
    val fill = glassFill(ac, base.s1)
    return NzColors(
        kind = kind,
        bg = base.bg,
        s1 = base.s1,
        s2 = base.s2,
        ln = base.ln,
        tx = base.tx,
        dim = base.dim,
        ac = ac,
        onac = if (custom || sleeve) pair.on else base.onac,
        acText = if (custom || sleeve) pair.ink else base.acText,
        acMark = when {
            sleeve -> ac
            custom -> pair.ink
            else -> base.acMark
        },
        btn = if (surfaceAccent) ac else base.btn,
        onbtn = if (surfaceAccent) pair.on else base.onbtn,
        hrt = when {
            sleeve -> ac
            custom -> pair.ink
            else -> base.hrt
        },
        glassBg = nzWithAlpha(fill, 0.64f),
        glassBg2 = nzWithAlpha(fill, 0.78f),
        glassBgStrong = nzWithAlpha(fill, 0.93f),
        glassBorder = nzWithAlpha(base.tx, 0.13f),
        // `color-mix(in srgb, var(--tx) 9%, var(--glass-bg-2))`: a weighted
        // average INCLUDING alpha, which is why it is a mix and not a paint.
        glassHover = nzMix(nzWithAlpha(base.tx, 0.09f), nzWithAlpha(fill, 0.78f), 0.09f),
        glassHi = base.hi,
        glassLo = base.lo,
        glassShadow = base.shadow1,
        glassShadow2 = base.shadow2,
        scrim = nzWithAlpha(BLACK, 0.60f),
        isDark = base.isDark,
        accentAdjusted = pair.adjusted,
    )
}

/**
 * The spacing scale. `Sillon.space` / `Sillon.gap` are kept as the two values
 * MB0's stubs already read (16 dp of padding, 8 dp between blocks); the rest is
 * the scale the components use.
 */
@Immutable
object NzSpacing {
    val xxs: Dp = 2.dp
    val xs: Dp = 4.dp
    val s: Dp = 8.dp
    val m: Dp = 12.dp
    val l: Dp = 16.dp
    val xl: Dp = 24.dp
    val xxl: Dp = 32.dp

    /** `--page-y`: the top/bottom margin of a page. */
    val pageY: Dp = 32.dp

    /** `--page-x`: the side margin, 20 dp on a phone (`≤ 640 px`). */
    val pageX: Dp = 20.dp

    /** A phone's tab bar and mini player, in dp (the SPA's 64 px of content). */
    val tabBar: Dp = 64.dp
    val miniPlayer: Dp = 64.dp
}

/**
 * The corner radii. `--r-card` 20, `--r-cover` 12, `--radius` 10 for the older
 * panels; controls are a pill.
 */
@Immutable
object NzRadius {
    val card: Dp = 20.dp
    val cover: Dp = 12.dp
    val field: Dp = 10.dp
    val chip: Dp = 999.dp
    val pill: Dp = 999.dp
}

/**
 * The minimum a finger may miss: 48 dp (`webapp.md` §10's 44 px rule, made 48 by
 * the MB1 acceptance). Every interactive component wraps its content in it.
 */
val NzMinTouchTarget: Dp = 48.dp

/**
 * The missing-cover grooves (`webapp/src/lib/sillon.js#GROOVES`): eight dark
 * pairs and the light ink drawn on them. They are TOKENS, not a decoration a
 * component invents — the record grain is Sillon §4, the same eight colours on
 * every device, chosen by FNV-1a over the seed so a placeholder never changes
 * colour on reload.
 */
@Immutable
object NzGrooves {
    val pairs: List<Pair<Color, Color>> = listOf(
        Color(0xFF2B2B29) to Color(0xFF3B3B37),
        Color(0xFF7A1D12) to Color(0xFF9B2A1B),
        Color(0xFF14304A) to Color(0xFF1C4466),
        Color(0xFF4A2A62) to Color(0xFF5E3A7B),
        Color(0xFF1F4A3A) to Color(0xFF2A634E),
        Color(0xFF6B5A14) to Color(0xFF8A7420),
        Color(0xFF5A1F3A) to Color(0xFF7A2C50),
        Color(0xFF3A3A33) to Color(0xFF55554A),
    )

    /** The initials and the glyph painted ON a groove: Avenir's rule, always light. */
    val ink: Color = Color(0xFFF1EFE8)
}

/**
 * Sillon's tokens, in ONE object: the values a screen reads. Colours depend on
 * the theme, so [colors] reads the composition (provided by [NzTheme]) and
 * [theme] resolves a table for previews and tests.
 */
object Sillon {
    /** The colours in force. Never a literal in a component. */
    val colors: NzColors
        @androidx.compose.runtime.Composable
        @androidx.compose.runtime.ReadOnlyComposable
        get() = LocalNzColors.current

    /** The type scale in force. */
    val type: NzTypography
        @androidx.compose.runtime.Composable
        @androidx.compose.runtime.ReadOnlyComposable
        get() = LocalNzTypography.current

    /** The motion level in force (eco, reduced or full). */
    val motion: NzMotion
        @androidx.compose.runtime.Composable
        @androidx.compose.runtime.ReadOnlyComposable
        get() = LocalNzMotion.current

    /** The table of one theme, for a preview, a test or the boot script. */
    fun theme(kind: NzThemeKind, accent: Color? = null, coverAccent: Color? = null): NzColors =
        resolve(kind, accent, coverAccent)

    /** MB0's two values, kept: a page's padding and the gap between blocks. */
    val space: Dp = NzSpacing.l
    val gap: Dp = NzSpacing.s

    val spacing: NzSpacing get() = NzSpacing
    val radius: NzRadius get() = NzRadius

    /** The type scale does not depend on the theme, only on the fonts. */
    val typography: NzTypography get() = NzTypographyTable
}
