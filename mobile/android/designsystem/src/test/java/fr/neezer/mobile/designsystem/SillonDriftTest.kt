package fr.neezer.mobile.designsystem

import androidx.compose.ui.graphics.Color
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The phone's tokens against the SPA's own file.
 *
 * `webapp/src/app.css` is the source of truth for Sillon, and `test/motion.test.mjs`
 * already fails the web app when `lib/motion.js` and `app.css` disagree. This is
 * the same guard across the stack boundary: the design system reads the CSS and
 * compares every Sillon token — thirteen colours per theme, the seven durations,
 * the two distances, the three scales, the two blur radii — so a colour adjusted
 * in the SPA and not here fails a JVM test instead of shipping two different
 * apps.
 *
 * A token that exists in the CSS but not here is listed too: the test knows the
 * names, so a NEW colour added to the SPA is a deliberate edit here, not a
 * silent omission.
 */
class SillonDriftTest {

    private val css: String by lazy {
        val root = System.getProperty("nz.repoRoot")
            ?: error("nz.repoRoot is not set: the drift test needs the repository (see build.gradle.kts)")
        File(root, "webapp/src/app.css").also {
            assertTrue("app.css not found at ${it.absolutePath}", it.exists())
        }.readText()
    }

    private fun block(selector: String): String {
        val at = css.indexOf(selector)
        assertTrue("app.css has no `$selector` block", at >= 0)
        val end = css.indexOf('}', at)
        return css.substring(at, end)
    }

    private fun token(source: String, name: String): String {
        val match = Regex("--${Regex.escape(name)}\\s*:\\s*([^;]+);").find(source)
            ?: error("app.css has no --$name")
        val value = match.groupValues[1].trim()
        // `--ac-text: var(--tx)` on Pochette: resolve the alias in the same block.
        val alias = Regex("^var\\(--([a-z0-9-]+)\\)$").find(value)
        return if (alias != null) token(source, alias.groupValues[1]) else value
    }

    /** A CSS colour: `#rrggbb`, or the fallback inside `var(--cover-ac, #ff6b4a)`. */
    private fun color(value: String): Color {
        val hex = Regex("#([0-9a-fA-F]{6})").find(value)?.groupValues?.get(1)
            ?: error("`$value` is not a hex colour")
        return Color(0xFF000000 or hex.toLong(16))
    }

    private fun assertToken(block: String, name: String, actual: Color, theme: String) {
        val expected = color(token(block, name))
        assertEquals("--$name on $theme", expected.red, actual.red, 1f / 255f)
        assertEquals("--$name on $theme (green)", expected.green, actual.green, 1f / 255f)
        assertEquals("--$name on $theme (blue)", expected.blue, actual.blue, 1f / 255f)
    }

    @Test
    fun theThirteenColoursAreTheSpas() {
        val blocks = mapOf(
            NzThemeKind.Encre to block("[data-theme=\"encre\"]"),
            NzThemeKind.Papier to block("[data-theme=\"papier\"]"),
            NzThemeKind.Pochette to block("[data-theme=\"pochette\"]"),
        )
        for ((kind, block) in blocks) {
            val c = Sillon.theme(kind)
            assertToken(block, "bg", c.bg, "$kind")
            assertToken(block, "s1", c.s1, "$kind")
            assertToken(block, "s2", c.s2, "$kind")
            assertToken(block, "ln", c.ln, "$kind")
            assertToken(block, "tx", c.tx, "$kind")
            assertToken(block, "dim", c.dim, "$kind")
            assertToken(block, "ac", c.ac, "$kind")
            assertToken(block, "onac", c.onac, "$kind")
            assertToken(block, "ac-text", c.acText, "$kind")
            assertToken(block, "ac-mark", c.acMark, "$kind")
            assertToken(block, "btn", c.btn, "$kind")
            assertToken(block, "onbtn", c.onbtn, "$kind")
            assertToken(block, "hrt", c.hrt, "$kind")
        }
    }

    @Test
    fun theGlassLadderIsTheSpas() {
        // The two blur radii and the three alphas are the ladder's whole shape; a
        // component cannot invent a third value (the SPA's `test/glass.test.mjs`).
        val root = block(":root {")
        assertEquals("0.64", tokenF(css, "glass-bg"))
        assertEquals("0.78", tokenF(css, "glass-bg-2"))
        assertEquals("0.93", tokenF(css, "glass-bg-strong"))
        assertEquals(14f, Regex("--glass-blur-2\\s*:\\s*([0-9.]+)px").find(css)!!.groupValues[1].toFloat())
        assertEquals(22f, Regex("--glass-blur-3\\s*:\\s*([0-9.]+)px").find(css)!!.groupValues[1].toFloat())
        // …and the Compose side carries the same two numbers.
        assertEquals(14f, Sillon.theme(NzThemeKind.Encre).glassBlur2.value)
        assertEquals(22f, Sillon.theme(NzThemeKind.Encre).glassBlur3.value)
        assertTrue(root.isNotEmpty())
    }

    /** `color-mix(in srgb, var(--glass-fill) 64%, transparent)` → "0.64". */
    private fun tokenF(source: String, name: String): String {
        val raw = token(source, name)
        val percent = Regex("(\\d+)%").find(raw)?.groupValues?.get(1)
            ?: error("`$raw` carries no percentage")
        val value = percent.toInt() / 100.0
        return String.format(java.util.Locale.ROOT, "%.2f", value)
    }

    @Test
    fun theMotionTableIsTheSpas() {
        val durations = mapOf(
            "dur-instant" to NzDur.Instant,
            "dur-fast" to NzDur.Fast,
            "dur-base" to NzDur.Base,
            "dur-sheet" to NzDur.Sheet,
            "dur-sheet-out" to NzDur.SheetOut,
            "dur-screen" to NzDur.Screen,
            "dur-screen-out" to NzDur.ScreenOut,
        )
        for ((name, ms) in durations) {
            assertEquals("--$name", ms, token(css, name).removeSuffix("ms").toInt())
        }
        assertEquals("--move-s", NzMove.S, token(css, "move-s").removeSuffix("px").toInt())
        assertEquals("--move-m", NzMove.M, token(css, "move-m").removeSuffix("px").toInt())
        assertEquals("--scale-pop", NzScale.Pop, token(css, "scale-pop").toFloat(), 1e-6f)
        assertEquals("--scale-dialog", NzScale.Dialog, token(css, "scale-dialog").toFloat(), 1e-6f)
        assertEquals("--scale-screen", NzScale.Screen, token(css, "scale-screen").toFloat(), 1e-6f)
        // The curves, as the CSS writes them.
        assertEquals("cubic-bezier(0.05, 0.7, 0.1, 1)", token(css, "ease-enter"))
        assertEquals("cubic-bezier(0.3, 0, 0.8, 0.15)", token(css, "ease-exit"))
        assertEquals("cubic-bezier(0.2, 0, 0, 1)", token(css, "ease-standard"))
    }

    @Test
    fun theFontsAreTheSpas() {
        // The SPA self-hosts three faces; the app ships the same files, so the two
        // apps cannot drift into different typography.
        val root = System.getProperty("nz.repoRoot")!!
        val faces = File(root, "android/app/src/main/res/font")
        val shipped = File(root, "mobile/android/designsystem/src/main/res/font")
        assertTrue("the SPA's font directory moved", faces.isDirectory)
        val names = listOf("bricolage_grotesque.ttf", "instrument_sans.ttf", "jetbrains_mono.ttf")
        for (name in names) {
            val source = File(faces, name)
            val copy = File(shipped, name)
            assertTrue("$name is missing from the design system", copy.exists())
            assertEquals(
                "$name: the app must ship the SPA's own file (byte for byte)",
                source.readBytes().size,
                copy.readBytes().size,
            )
        }
    }
}
