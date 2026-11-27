package fr.neezer.mobile.feature.settings

import android.os.Looper
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Réglages, rendered and VERIFIED: the eight categories with the last row after
 * one scroll, and « Lecture & son » with its normalisation and car-mode cards —
 * at 360 and 412 dp and at the 1.0 and 1.3 font scales (plan T3's matrix).
 *
 * T3 replaced MB16's write-only captures with goldens: the menu's tail is where
 * a category row clips, and the playback page is where a control squeezed by a
 * label shows. Robolectric's NATIVE graphics backend draws the real section
 * dispatch over [FakeSettingsService] on the JVM (worker rule 7).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi")
class SettingsGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    private val nav = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    @Test
    fun menuAt360() = shotMenu(360, 1.0f)

    @Test
    @Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi", fontScale = 1.3f)
    fun menuAt360At130Percent() = shotMenu(360, 1.3f)

    @Test
    @Config(sdk = [35], qualifiers = "w412dp-h900dp-xhdpi")
    fun menuAt412() = shotMenu(412, 1.0f)

    @Test
    @Config(sdk = [35], qualifiers = "w412dp-h900dp-xhdpi", fontScale = 1.3f)
    fun menuAt412At130Percent() = shotMenu(412, 1.3f)

    @Test
    fun playbackAt360() = shotPlayback(360, 1.0f)

    @Test
    @Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi", fontScale = 1.3f)
    fun playbackAt360At130Percent() = shotPlayback(360, 1.3f)

    @Test
    @Config(sdk = [35], qualifiers = "w412dp-h900dp-xhdpi")
    fun playbackAt412() = shotPlayback(412, 1.0f)

    @Test
    @Config(sdk = [35], qualifiers = "w412dp-h900dp-xhdpi", fontScale = 1.3f)
    fun playbackAt412At130Percent() = shotPlayback(412, 1.3f)

    private fun shotMenu(width: Int, scale: Float) {
        render(section = null, fontScale = scale)
        // Une ligne d'aide sous un libellé est du « texte inutile » (C5, décision
        // du propriétaire 2026-11-27). La garde nomme celles que l'index portait.
        for (line in MENU_HELP_LINES) {
            rule.composeRule.onNodeWithText(line).assertDoesNotExist()
        }
        // « la dernière ligne visible après un défilement » (the matrix's state):
        // the menu is taller than a phone, and its tail is what a golden checks.
        rule.composeRule.onNodeWithText("Assistance").performScrollTo()
        rule.composeRule.waitForIdle()
        rule.capture("settings-menu-${width}dp-f$scale.png")
    }

    private fun shotPlayback(width: Int, scale: Float) {
        render(section = "playback", fontScale = scale)
        for (line in PLAYBACK_HELP_LINES) {
            rule.composeRule.onNodeWithText(line).assertDoesNotExist()
        }
        rule.capture("settings-lecture-${width}dp-f$scale.png")
    }

    private fun render(section: String?, fontScale: Float = 1f) {
        rule.setContent(fontScale) {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    SettingsEntry(section = section, nav = nav, service = FakeSettingsService())
                }
            }
        }
        // The model's flows run on the main dispatcher Robolectric owns: let them
        // land (an EVENT, not a duration — `runToEndOfTasks` drains the queue).
        shadowOf(Looper.getMainLooper()).idle()
        rule.composeRule.waitForIdle()
    }

    private companion object {
        /** Les descriptions de l'index que la décision du 2026-11-27 a retirées. */
        val MENU_HELP_LINES = listOf(
            "Sortie audio, volume, égaliseur, transitions",
            "Thème Encre, Papier ou Pochette",
            "Ce qui s'anime pendant la lecture",
            "Qualité, hors-ligne, préchargement",
            "Flow, titres que vous n'aimez pas, nouveautés",
            "Échange entre vos appareils et vos amis sur ce Wi-Fi",
            "Ordre des sources, vos comptes, et qui peut s'en servir",
            "Votre compte, l'export vers d'autres services",
            "Recharger l'application, journal de diagnostic",
        )

        /** Celles de « Lecture & son », la page que le juge photographie. */
        val PLAYBACK_HELP_LINES = listOf(
            "S'applique au prochain titre ; le titre en cours continue.",
            "Le panneau par-dessus la navigation, les déclencheurs, le placement et les playlists épinglées.",
        )
    }
}
