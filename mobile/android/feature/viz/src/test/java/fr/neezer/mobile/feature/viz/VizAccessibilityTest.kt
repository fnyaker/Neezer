package fr.neezer.mobile.feature.viz

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.test.assertIsDisplayed
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * What TalkBack finds on the Visualiseur (rule 5: a label on every control, and
 * the picture itself is decorative).
 *
 * Queries are SEMANTICS, never coordinates (the standard's own rule): the mode
 * choices, the palette choices, the eco switch with its sentence, the caption card
 * and its words. The canvas carries no semantics at all — a screen reader must
 * announce the track and the controls, not a waveform — and the warning's
 * « Activer » is disabled until its switch is on.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class VizAccessibilityTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    /** The service the composed screen talks to (one content set per test). */
    private lateinit var service: FakeVizService

    private fun show(
        settings: uniffi.nz_ffi.VizSettingsData = VizFixtures.settings(),
        frame: uniffi.nz_ffi.VizFrame = VizFixtures.barsFrame(),
        playing: Boolean = true,
    ) {
        service = FakeVizService(settings = settings, frameNow = frame)
        val model = VizModel(
            service = service,
            player = FakeVizPlayer(VizFixtures.snapshot(playing = playing)),
            covers = FakeVizCovers(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = {},
        )
        model.refresh()
        // The scene's loop waits on the frame clock: drive it by hand, as the
        // screenshot suite does (an EVENT, not a sleep).
        rule.mainClock.autoAdvance = false
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre) {
                // The LIVE state: a tap must repaint, which is exactly what a
                // snapshot would hide.
                val live by model.state.collectAsState()
                VizScreen(
                    state = live,
                    model = model,
                    modifier = androidx.compose.ui.Modifier,
                    service = service,
                )
            }
        }
        repeat(3) { rule.mainClock.advanceTimeByFrame() }
    }

    @Test
    fun everyControlIsAnnounced() {
        show()
        rule.onNodeWithText("Barres").assertIsDisplayed()
        rule.onNodeWithText("Oscilloscope").assertIsDisplayed()
        rule.onNodeWithText("Pochette").assertIsDisplayed()
        rule.onNodeWithText("Duo").assertIsDisplayed()
        rule.onNodeWithText("Monochrome").assertIsDisplayed()
        // The eco switch's sentence, not a glyph with no name.
        rule.onNodeWithContentDescription(VizCopy.ECO_LABEL).assertIsDisplayed()
        // The caption card is text, so a reader reads the track.
        rule.onNodeWithText("Déjà vu").assertIsDisplayed()
        rule.onNodeWithText("Beyoncé").assertIsDisplayed()
    }

    @Test
    fun theScopeShowsItsTwoKnobs() {
        show(settings = VizFixtures.settings(mode = "scope"), frame = VizFixtures.scopeFrame())
        rule.onNodeWithText("Horizontal").assertIsDisplayed()
        rule.onNodeWithText("Vertical").assertIsDisplayed()
        rule.onNodeWithText("Deux teintes").assertIsDisplayed()
    }

    @Test
    fun theBarsShowNoScopeKnobs() {
        // A control that does nothing is worse than no control: the scope's two
        // knobs only exist in scope mode (`VisualizerSettings.svelte`'s `{#if}`).
        show(settings = VizFixtures.settings(mode = "bars"))
        rule.onNodeWithText("Vertical").assertDoesNotExist()
    }

    @Test
    fun thePaletteTapWritesTheStore() {
        show()
        rule.onNodeWithText("Duo").performClick()
        // A recomposition needs a FRAME, and the clock is driven by hand.
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        org.junit.Assert.assertTrue("duo", service.calls.contains("palette"))
    }

    @Test
    fun theWarningCannotBeClickedThrough() {
        rule.mainClock.autoAdvance = false
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre) {
                FlashWarningSheet(visible = true, reduced = false, onAccept = {}, onDismiss = {})
            }
        }
        rule.mainClock.advanceTimeByFrame()
        // « Activer » exists, is DISABLED, and the agreement is off.
        rule.onNodeWithText(VizCopy.FLASH_ACCEPT).assertIsDisplayed().assertIsNotEnabled()
        val agree = rule.onNodeWithContentDescription(VizCopy.FLASH_AGREE)
        agree.assertIsOff()
        agree.performClick()
        // The agreement is Compose state: a frame makes it real (the clock is
        // driven by hand in this suite).
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        rule.onNodeWithText(VizCopy.FLASH_ACCEPT).assertIsEnabled()
        agree.assertIsOn()
    }

    @Test
    fun theEcoSwitchTakesTheLeafOffThePicture() {
        show()
        rule.onNodeWithContentDescription(VizCopy.ECO_LABEL).performClick()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        // Eco is ON in the store, and the screen now says so instead of a canvas.
        org.junit.Assert.assertTrue(service.calls.contains("eco"))
        rule.onNodeWithText(VizCopy.ECO_TITLE).assertIsDisplayed()
    }

}
