package fr.neezer.mobile.designsystem

import androidx.compose.ui.graphics.Color
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Every text/surface pair Sillon draws, measured in all three themes.
 *
 * The thresholds are `webapp.md`'s: **4.5:1** for body text and for a foreground
 * accent (it is read as text), **3:1** for a large glyph or a mark (a heart, a
 * progress bar, an indicator — WCAG 1.4.11's non-text contrast).
 *
 * This is the test the MB1 acceptance names, and it is also the one that keeps
 * the two measured bugs from coming back:
 *
 *  * `#fff` on `--ac`/`--accent` — 1.16:1 on the lemon, which the old purple
 *    accent (3.9:1) hid for a year;
 *  * the lemon used as INK on Papier — 1.16:1 against the cream ground, which is
 *    why `--ac-text`/`--ac-mark` exist and why `Sillon.colors.acText` is the ink
 *    there.
 */
class ContrastTest {

    private data class Pair(val name: String, val fg: Color, val bg: Color, val min: Double)

    private fun pairs(c: NzColors): List<Pair> = listOf(
        // -- body text on the three opaque surfaces -------------------------
        Pair("tx on bg", c.tx, c.bg, AA),
        Pair("tx on s1", c.tx, c.s1, AA),
        Pair("tx on s2", c.tx, c.s2, AA),
        Pair("dim on bg", c.dim, c.bg, AA),
        Pair("dim on s1", c.dim, c.s1, AA),
        Pair("dim on s2", c.dim, c.s2, AA),
        // -- text ON an accent surface (the theme is a COLOUR PAIR) ---------
        Pair("onac on ac", c.onac, c.ac, AA),
        Pair("onbtn on btn", c.onbtn, c.btn, AA),
        // -- the accent used as INK / MARK ----------------------------------
        Pair("acText on bg", c.acText, c.bg, AA),
        Pair("acText on s1", c.acText, c.s1, AA),
        Pair("acText on s2", c.acText, c.s2, AA),
        Pair("acMark on s1", c.acMark, c.s1, NON_TEXT),
        Pair("acMark on s2", c.acMark, c.s2, NON_TEXT),
        Pair("hrt on s1", c.hrt, c.s1, NON_TEXT),
        Pair("hrt on s2", c.hrt, c.s2, NON_TEXT),
        // -- glass: the tint is composited over what is behind it ----------
        Pair("dim on glass 64 % over s1", c.dim, nzOver(c.glassBg, c.s1), AA),
        Pair("dim on glass 78 % over bg", c.dim, nzOver(c.glassBg2, c.bg), AA),
        Pair("tx on glass 78 % over bg", c.tx, nzOver(c.glassBg2, c.bg), AA),
        // The thick step sits on a PHOTOGRAPH (the phone player's queue and its
        // À propos panel): the worst case is a pure-white sleeve.
        Pair("dim on glass 93 % over white", c.dim, nzOver(c.glassBgStrong, Color.White), AA),
        Pair("dim on glass 93 % over black", c.dim, nzOver(c.glassBgStrong, Color.Black), AA),
        Pair("tx on glass 93 % over white", c.tx, nzOver(c.glassBgStrong, Color.White), AA),
        // A hairline is DECORATION: WCAG asks nothing of it, and Sillon's is 13 %
        // of the ink on purpose (a visible border reads as a box). It still has
        // to be seen at all, so the floor is asserted instead of being left to
        // taste: measured 1.27–1.45:1 on the three themes' grounds.
        Pair("ln on bg", c.ln, c.bg, 1.2),
        Pair("ln on s1", c.ln, c.s1, 1.2),
    )

    @Test
    fun everyPairIsLegible() {
        val failures = mutableListOf<String>()
        for (kind in NzThemeKind.entries) {
            for (accent in listOf(null, Color(0xFF7C5CFF), Color(0xFF6967D6))) {
                val c = Sillon.theme(kind, accent = accent)
                val label = "${kind}${if (accent != null) " +accent" else ""}"
                println("== $label ==")
                for (p in pairs(c)) {
                    val ratio = nzContrast(p.fg, p.bg)
                    println(String.format(java.util.Locale.ROOT, "  %-32s %5.2f:1 (min %.1f)", p.name, ratio, p.min))
                    if (ratio + 1e-9 < p.min) {
                        failures += String.format(java.util.Locale.ROOT, "%s — %s = %.2f:1, under %.1f:1", label, p.name, ratio, p.min)
                    }
                }
            }
        }
        assertTrue(
            "Contrast below the Sillon thresholds:\n" + failures.joinToString("\n"),
            failures.isEmpty(),
        )
    }

    /**
     * The two measured bugs, asserted as BUGS: white on the lemon is 1.16:1, and
     * the lemon on Papier's cream is 1.16:1. If a theme ever makes these legible
     * the rule can be revisited — until then using them is the mistake.
     */
    @Test
    fun theForbiddenPairsStayForbidden() {
        val encre = Sillon.theme(NzThemeKind.Encre)
        val papier = Sillon.theme(NzThemeKind.Papier)
        val pochette = Sillon.theme(NzThemeKind.Pochette)
        listOf(encre, papier, pochette).forEach { c ->
            val white = nzContrast(Color.White, c.ac)
            println(String.format(java.util.Locale.ROOT, "white on ${c.kind} ac = %.2f:1", white))
            assertTrue(String.format(java.util.Locale.ROOT, "white on ${c.kind}'s accent must stay under AA (was %.2f:1)", white), white < 4.5)
        }
        val lemonOnCream = nzContrast(Color(0xFFD2FF3C), papier.bg)
        println(String.format(java.util.Locale.ROOT, "lemon on papier bg = %.2f:1", lemonOnCream))
        assertTrue(String.format(java.util.Locale.ROOT, "the lemon must never be used as ink on Papier (%.2f:1)", lemonOnCream), lemonOnCream < 4.5)
        // …and the ink the theme gives instead is the one that passes.
        assertTrue(nzContrast(papier.acText, papier.bg) >= 4.5)
    }

    /**
     * A custom accent is lifted the way `lib/theme.js#accentVars` lifts it: the
     * surface is the exact chosen colour, the text on it is pure black or pure
     * white, and the ink is corrected against the demanding surface. Measured on
     * the SPA's own two examples (`#7C5CFF` and `#6967D6`).
     */
    @Test
    fun aCustomAccentKeepsItsPairs() {
        for (raw in listOf(Color(0xFF7C5CFF), Color(0xFFFFF176), Color(0xFF2F2F2F), Color(0xFF808080))) {
            for (kind in listOf(NzThemeKind.Encre, NzThemeKind.Papier)) {
                val pair = nzAccent(raw, kind)
                val on = nzContrast(pair.on, pair.surface)
                val ink = nzContrast(pair.ink, if (kind == NzThemeKind.Encre) Sillon.theme(kind).s2 else Sillon.theme(kind).s1)
                println(String.format(java.util.Locale.ROOT, "%s %s: on %.2f:1, ink %.2f:1", kind, raw, on, ink))
                assertTrue(String.format(java.util.Locale.ROOT, "text on a custom accent ($kind, $raw) = %.2f:1", on), on >= 4.5)
                assertTrue(String.format(java.util.Locale.ROOT, "the accent's ink on $kind = %.2f:1", ink), ink >= 4.5)
            }
        }
    }

    private companion object {
        const val AA = 4.5

        /** WCAG 1.4.11: a glyph, a mark, a border. */
        const val NON_TEXT = 3.0
    }
}
