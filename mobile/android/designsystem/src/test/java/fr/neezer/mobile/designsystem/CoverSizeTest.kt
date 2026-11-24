package fr.neezer.mobile.designsystem

import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * **The sleeve a loader is asked for is the size the screen DRAWS**, in pixels
 * (docs/agents/mobile.md, *Caches*: "downsampled to the view's size").
 *
 * The first version passed `sizeDp.value * 3`: a hard-coded density guess that
 * asked for 132 px for a 44 dp row on a 2× screen and 1 260 px for the player's
 * 420 dp stage — and would ask the same on a 1× tablet. The key includes that
 * number, so a wrong one is a second decode and a second cache entry for the
 * same sleeve.
 *
 * At xhdpi (density 2) the test reads the number a real loader would receive:
 * 44 dp = 88 px, 420 dp = 840 px. On the old code the first assertion sees 132.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h2080dp-xhdpi")
class CoverSizeTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private class RecordingLoader : NzCoverLoader {
        val asked = mutableListOf<Pair<String, Int>>()

        @Composable
        override fun Image(url: String, sizePx: Int) {
            asked += url to sizePx
        }
    }

    private fun drawnAt(sizeDp: Int): Int {
        val loader = RecordingLoader()
        rule.setContent {
            CompositionLocalProvider(LocalNzCoverLoader provides loader) {
                CoverImage(url = "https://example.test/cover", sizeDp = sizeDp.dp, seed = "album:1")
            }
        }
        rule.waitForIdle()
        assertEquals("the loader was asked once", 1, loader.asked.size)
        return loader.asked.single().second
    }

    @Test
    fun a44dpRowAsksForIts88Pixels() {
        assertEquals(88, drawnAt(44))
    }

    @Test
    fun thePlayers420dpStageAsksForIts840Pixels() {
        assertEquals(840, drawnAt(420))
    }
}
