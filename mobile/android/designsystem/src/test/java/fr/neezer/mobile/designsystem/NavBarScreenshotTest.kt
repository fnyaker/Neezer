package fr.neezer.mobile.designsystem

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * L1's tab bar, VERIFIED: the four corners of its matrix (360/600 dp × Encre and
 * Papier), the 1.3 font scale and a detail shell that shows what the lot removes
 * (no band above the content, one back row).
 *
 * The images are goldens in `mobile/android/screenshots/` (T3 converted L1's
 * write-only captures the moment the UI plan landed), so a bar that grows, loses
 * its label or drifts a pixel fails here — the place the SPA's own `MobileNav`
 * keeps under its `test/ui` bench.
 *
 * The bounds half is T3's third reported misalignment, read from the semantics
 * tree: every tab is a ≥ 48 dp target, all the tabs share one baseline, and the
 * bar keeps its 64 dp content row. The plan's "≥ 80 dp at 1.3" counted the
 * phone's navigation-bar inset (64 dp row + 24 dp inset, `MobileNav.svelte`: "64
 * px EXACTLY, above the home indicator"); Robolectric's inset is 0, so the JVM
 * assertion is the 64 dp contract and the emulator carries the inset.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class NavBarGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun encreAt360() = shot(NzThemeKind.Encre, 360, 1.0f)

    @Test
    fun papierAt360() = shot(NzThemeKind.Papier, 360, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi", fontScale = 1.3f)
    fun encreAt360At130Percent() = shot(NzThemeKind.Encre, 360, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi", fontScale = 1.3f)
    fun papierAt360At130Percent() = shot(NzThemeKind.Papier, 360, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
    fun encreAt600() = shot(NzThemeKind.Encre, 600, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
    fun papierAt600() = shot(NzThemeKind.Papier, 600, 1.0f)

    /** What the band left: no bar above the content, one back row. */
    @Test
    fun aDetailRowAndTheBar() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Column(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Row(
                        modifier = Modifier.fillMaxWidth().height(56.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.Start,
                    ) {
                        NzIconButton(glyph = NzGlyph.ChevronLeft, contentDescription = "Retour", onClick = {})
                    }
                    Spacer(Modifier.weight(1f))
                    NzTabBar(items = TAB_ITEMS_FOR_TEST, selected = -1, onPick = {})
                }
            }
        }
        rule.capture("designsystem-barre-detail-360dp-f1.0.png")
    }

    /**
     * T3's bottom icon row: five ≥ 48 dp targets on one baseline, inside the bar's
     * 64 dp content row (the SPA's own contract, `MobileNav.svelte`).
     */
    @Test
    fun everyTabKeepsItsTargetAndOneBaseline() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Column(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Spacer(Modifier.weight(1f))
                    NzTabBar(items = TAB_ITEMS_FOR_TEST, selected = 1, onPick = {})
                }
            }
        }
        rule.composeRule.waitForIdle()
        val boxes = TAB_ITEMS_FOR_TEST.map { item ->
            val bounds = rule.composeRule
                .onNodeWithContentDescription(item.label)
                .getUnclippedBoundsInRoot()
            val width = bounds.right - bounds.left
            val height = bounds.bottom - bounds.top
            println(
                "MEASURE t3 barre « ${item.label} »: ${width}×${height} dp " +
                    "bottom=${bounds.bottom.value}dp",
            )
            assertTrue("« ${item.label} » is a 48 dp target wide ($width dp)", width.value >= 48f - 0.5f)
            assertTrue("« ${item.label} » is a 48 dp target tall ($height dp)", height.value >= 48f - 0.5f)
            bounds
        }
        val bottom = boxes.minOf { it.bottom.value }
        boxes.forEach {
            assertEquals("the tabs share one baseline", bottom, it.bottom.value, 1f)
        }
        val barHeight = boxes.minOf { (it.bottom - it.top).value }
        assertTrue(
            "the bar keeps its 64 dp content row (measured $barHeight dp; the plan's 80 dp " +
                "adds the phone's 24 dp navigation-bar inset, 0 on Robolectric)",
            barHeight >= 64f - 0.5f,
        )
    }

    private fun shot(theme: NzThemeKind, width: Int, scale: Float) {
        rule.setContent(scale) {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                Column(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Spacer(Modifier.weight(1f))
                    val density = LocalDensity.current
                    CompositionLocalProvider(
                        LocalDensity provides Density(density = density.density, fontScale = scale),
                    ) {
                        NzTabBar(items = TAB_ITEMS_FOR_TEST, selected = 1, onPick = {})
                    }
                }
            }
        }
        rule.capture("designsystem-barre-${theme.name.lowercase()}-${width}dp-f$scale.png")
    }
}

/**
 * The same five items the shell builds (`TAB_ITEMS` lives in `:app`, which the
 * design system cannot see): the bar's own picture must not depend on the shell.
 */
private val TAB_ITEMS_FOR_TEST = listOf(
    NzTabItem(NzGlyph.Home, "Accueil"),
    NzTabItem(NzGlyph.Compass, "Rechercher"),
    NzTabItem(NzGlyph.Library, "Biblio"),
    NzTabItem(NzGlyph.Mic, "Podcasts"),
    NzTabItem(NzGlyph.Settings, "Réglages"),
)
