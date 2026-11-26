package fr.neezer.mobile.designsystem

import androidx.compose.runtime.Composable
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The design system, rendered and VERIFIED: the three Sillon themes at 360 dp
 * (the phone) and 600 dp (the tablet's reading column), plus the 1.3 font scale.
 *
 * T3 replaced the write-only capture (25 `*ScreenshotTest.kt` that only ever
 * wrote a PNG) with this: the image is compared to the committed golden in
 * `mobile/android/screenshots/`, so a colour that falls back to a default
 * widget, a squeezed pill or a clipped label fails the suite instead of being
 * reviewed by hope. The gallery is the one Android Studio's preview shows
 * ([NzComponentSheet]): the MB1 lesson — the first version's « Lire » was ink on
 * ink on Papier — is exactly what a golden keeps caught.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). Paparazzi cannot
 * render this tree's Compose (`:designsystem`'s build file records the measured
 * failures).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h2400dp-xhdpi")
class DesignSystemGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun encreAt360() = shot(NzThemeKind.Encre, 360, 1.0f)

    @Test
    fun papierAt360() = shot(NzThemeKind.Papier, 360, 1.0f)

    /** Pochette lives on a sleeve: its accent comes from the cover, never a literal. */
    @Test
    fun pochetteOnItsSleeveAt360() = shot(NzThemeKind.Pochette, 360, 1.0f, sleeve = true)

    @Test
    @Config(sdk = [34], qualifiers = "w360dp-h2400dp-xhdpi", fontScale = 1.3f)
    fun encreAt360At130Percent() = shot(NzThemeKind.Encre, 360, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h1980dp-xhdpi")
    fun encreAt600() = shot(NzThemeKind.Encre, 600, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h1980dp-xhdpi")
    fun papierAt600() = shot(NzThemeKind.Papier, 600, 1.0f)

    private fun shot(
        theme: NzThemeKind,
        width: Int,
        scale: Float,
        sleeve: Boolean = false,
    ) {
        NzToasts.clear()
        NzToasts.show(text = "Ajouté à la file")
        rule.setContent(scale) { gallery(theme, width, sleeve) }
        rule.capture("designsystem-sillon-${theme.name.lowercase()}-${width}dp-f$scale.png")
    }

    @Composable
    private fun gallery(theme: NzThemeKind, width: Int, sleeve: Boolean) {
        if (sleeve) {
            NzTheme(
                kind = theme,
                coverAccent = androidx.compose.ui.graphics.Color(0xFF2E6F9E),
            ) {
                NzComponentSheet(width = width)
            }
        } else {
            NzTheme(kind = theme) {
                NzComponentSheet(width = width)
            }
        }
    }
}
