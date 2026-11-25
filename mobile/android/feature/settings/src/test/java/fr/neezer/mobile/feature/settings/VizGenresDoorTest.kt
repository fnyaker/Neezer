package fr.neezer.mobile.feature.settings

import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * M7's door, on the real screen and the shared recording navigator
 * (`docs/agents/mobile.md`, section M7).
 *
 * Two facts, two tests: an administrator sees « Studio de genres » and
 * « Ouvrir » asks the shell for the studio exactly once; a member sees no card
 * at all. The door is drawn at the END of « Animations » (the SPA's order,
 * `VisualizerSettings.svelte:558-566`), below the fold on a 360×800 dp phone —
 * hence the scroll, which is what a thumb does before the tap.
 *
 * What a JVM test cannot hold is the route itself: `AppGraph.openGenres` needs a
 * real `NavController`, so the emulator's own probe (`ci-emulator.sh`,
 * `link=#/genres`) is what proves `Routes.Genres` still opens.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class VizGenresDoorTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** Drains what a click queued — an EVENT, never a duration (testing.md). */
    private fun settle() {
        rule.waitForIdle()
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
    }

    private fun render(nav: RecordingNavigator, isAdmin: Boolean) {
        // The fake is built OUTSIDE the composition: `SettingsEntry` remembers its
        // model by service, and a fresh fake per recomposition would rebuild the
        // model (and restart its effects) on every frame.
        val service = FakeSettingsService()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize()) {
                    SettingsEntry(
                        section = "viz",
                        nav = nav,
                        isAdmin = isAdmin,
                        service = service,
                    )
                }
            }
        }
        settle()
    }

    /** Un administrateur voit la carte, et « Ouvrir » ouvre le studio — une fois. */
    @Test
    fun anAdminSeesTheStudio() {
        val nav = RecordingNavigator()
        render(nav, isAdmin = true)
        // La carte est ajoutée APRÈS « Visualiseur » (l'ordre du SPA) : sous la
        // ligne de flottaison à 360×800 dp, on la fait venir comme un doigt.
        rule.onNode(hasScrollAction()).performScrollToNode(hasText("Studio de genres"))
        rule.onNodeWithText("Studio de genres").assertIsDisplayed()
        rule.onNode(hasScrollAction()).performScrollToNode(hasText("Ouvrir") and hasClickAction())
        rule.onNode(hasText("Ouvrir") and hasClickAction()).performClick()
        settle()
        assertEquals(1, nav.genres)
    }

    /** Un membre ne la voit pas : la carte est la seule différence. */
    @Test
    fun aMemberDoesNot() {
        render(RecordingNavigator(), isAdmin = false)
        rule.onNodeWithText("Studio de genres").assertDoesNotExist()
    }
}
