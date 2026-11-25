package fr.neezer.mobile.feature.settings

import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * La porte du mode voiture, dans Réglages (M13) — celle qui manquait.
 *
 * Audit §9 : sur le téléphone, le mode voiture n'avait AUCUNE entrée depuis
 * Réglages ; il fallait la tuile des réglages rapides ou le panneau lui-même.
 * Le test compose la VRAIE section « Lecture & son » (`AudioFxSections()`
 * compris, qui répond « moteur audio indisponible » sans `.so`) et cherche le
 * bouton que `CarModeService.openSettings` ouvre.
 *
 * Le clic n'est pas joué : `openSettings` démarre une Activity que Robolectric
 * ne sert pas ici, et ce que le lot devait prouver est la PORTE, pas son effet.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class CarModeDoorTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** Drains what a composition queued — an EVENT, never a duration (testing.md). */
    private fun settle() {
        rule.waitForIdle()
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
    }

    private fun render() {
        val service = FakeSettingsService()
        val model = SettingsModel(service, CoroutineScope(Dispatchers.Main))
        model.start()
        model.open("playback")
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                // The page scrolls in the app (`SettingsPane`'s column does), and
                // the car card is the LAST of the section: the test scrolls to it
                // rather than hoping an 800 dp window holds three cards.
                Column(
                    modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
                ) {
                    PlaybackSection(model, model.state)
                }
            }
        }
        settle()
    }

    /** « Mode voiture » est là, et son bouton ouvre les réglages du mode. */
    @Test
    fun theCarModeCardOffersItsDoor() {
        render()
        rule.onNodeWithText("Mode voiture").performScrollTo().assertIsDisplayed()
        rule.onNode(hasText("Ouvrir les réglages du mode voiture") and hasClickAction())
            .performScrollTo()
            .assertIsDisplayed()
    }
}
