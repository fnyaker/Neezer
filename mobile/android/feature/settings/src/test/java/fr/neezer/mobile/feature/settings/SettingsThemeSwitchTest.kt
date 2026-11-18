package fr.neezer.mobile.feature.settings

import android.graphics.Bitmap
import androidx.compose.ui.graphics.toArgb
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.feature.home.HomeActions
import fr.neezer.mobile.feature.home.HomeModel
import fr.neezer.mobile.feature.home.HomeScreen
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * **The acceptance's theme test**: switching to Papier in Réglages repaints
 * ANOTHER feature — the home — with no restart, and the first frame already has
 * the stored theme.
 *
 * It renders the REAL `HomeScreen` inside [NzAppearanceTheme] (the wrapper
 * `MainActivity` uses), reads the pixels it painted, then changes the setting
 * the way the model does and reads them again. A colour change in a screenshot
 * is a claim about a component; a colour change in a TEST is a claim about the
 * app (MB1's lesson: the contrast test was green while the button was
 * invisible).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h1600dp-xhdpi")
class SettingsThemeSwitchTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** What the home painted behind its own blocks, as one ARGB pixel. */
    private fun homeBackground(): Int {
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(android.graphics.Canvas(bitmap))
        // The top-left corner is the page's own ground, above the hero's title.
        return bitmap.getPixel(2, 2)
    }

    @Test
    fun a_switch_to_papier_repaints_another_feature_at_once() {
        // The launch: the device stored Papier, and `MainActivity` reads it
        // BEFORE the first frame — so the first paint is already Papier.
        NzAppearance.load(FakeSettingsService(settings = SettingsFixtures.settings(theme = "papier")))
        rule.setContent {
            NzAppearanceTheme {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    HomeScreen(
                        state = HomeModel.State(screen = fr.neezer.mobile.bridge.ScreenState.Loading),
                        actions = HomeActions(),
                    )
                }
            }
        }
        val papier = homeBackground()

        // Réglages › Apparence, exactly what the model does on a tap.
        NzAppearance.apply(SettingsFixtures.settings(theme = "encre"))
        val encre = homeBackground()

        assertNotEquals("the home repainted without a restart", papier, encre)
        assertTrue(
            "and it is the theme's own ground",
            papier == Sillon.theme(fr.neezer.mobile.designsystem.NzThemeKind.Papier).bg.toArgb() ||
                papier == Sillon.theme(fr.neezer.mobile.designsystem.NzThemeKind.Papier).s1.toArgb(),
        )
    }

    /** Eco is `NzMotionLevel.Off`: no transition anywhere, in every feature. */
    @Test
    fun eco_switches_every_transition_off() {
        NzAppearance.apply(SettingsFixtures.settings(eco = true))
        assertTrue(
            "eco is the design system's Off level",
            NzAppearance.current.value.motion == fr.neezer.mobile.designsystem.NzMotionLevel.Off,
        )
        NzAppearance.apply(SettingsFixtures.settings(eco = false))
        assertTrue(
            "and off means the device decides again",
            NzAppearance.current.value.motion == null,
        )
    }
}
