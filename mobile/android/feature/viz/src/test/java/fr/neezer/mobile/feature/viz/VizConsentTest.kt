package fr.neezer.mobile.feature.viz

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * M12's gate: the visualiser asks before it pulses, once per device.
 *
 * The sheet is the SPA's `FlashWarning.svelte`, but with THIS product's copy
 * (`VizCopy.FLASH_VIZ_*`): the `FLASH_*` sentences describe « Débridé »'s ten
 * flashes a second, which this screen does not have. What is measured here is the
 * rule the component owes: « Activer » is inert until the agreement is on, and a
 * click through cannot accept anything.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class VizConsentTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** The sheet with the visualiser's own words, and what a tap counted. */
    private fun show(accepted: () -> Unit) {
        rule.mainClock.autoAdvance = false
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre) {
                FlashWarningSheet(
                    visible = true,
                    subtitle = VizCopy.FLASH_VIZ_SUBTITLE,
                    body = VizCopy.FLASH_VIZ_BODY,
                    reduced = false,
                    onAccept = accepted,
                    onDismiss = {},
                )
            }
        }
        rule.mainClock.advanceTimeByFrame()
    }

    @Test
    fun theWarningCannotBeClickedThrough() {
        var accepted = 0
        show { accepted++ }
        // The visualiser's own subtitle, not « Débridé »'s: what the screen says
        // is what the phone does.
        rule.onNodeWithText(VizCopy.FLASH_VIZ_SUBTITLE).assertIsDisplayed()
        rule.onNodeWithText(VizCopy.FLASH_VIZ_BODY).assertIsDisplayed()
        // « Activer » exists and is DISABLED: no tap, no consent.
        rule.onNodeWithText(VizCopy.FLASH_ACCEPT).assertIsDisplayed().assertIsNotEnabled()
        assertEquals("un bouton inerte n'accepte rien", 0, accepted)
    }

    @Test
    fun acceptingNeedsTheSwitch() {
        var accepted = 0
        show { accepted++ }
        val agree = rule.onNodeWithContentDescription(VizCopy.FLASH_AGREE)
        agree.performClick()
        // The agreement is Compose state: drive the clock by hand (an EVENT, not a
        // sleep) and the button becomes real.
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        rule.onNodeWithText(VizCopy.FLASH_ACCEPT).assertIsEnabled().performClick()
        assertEquals("le consentement est parti une fois", 1, accepted)
    }
}
