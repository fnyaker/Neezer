package fr.neezer.mobile.feature.home

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Lot JH2, the two defects the judge saw on the phone home:
 *
 * 1. **The SPA's `TopBar` above the hero.** `HomeTopBar` is drawn in EVERY state
 *    — the loading skeletons, the answer, a failed load — because the bar belongs
 *    to the frame, not to the answer. Its pill opens Explorer (it never types:
 *    the real box is on that screen), its circle starts the Flow.
 * 2. **« Par style » on the bench.** The bench library holds no measured genre,
 *    so `/api/flow/styles` answers `{"styles": []}`; the SPA still draws the
 *    eleven `STYLE_FALLBACK` names there (`StylePicker.svelte`), and the port used
 *    to return early. The block must be drawn, with the fallback names, on a home
 *    whose answer carries no style row.
 *
 * The tests are the premise measured on the bench server (lot JH2's PR): one
 * request, `{"styles": []}`, and the section simply absent before this lot.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class HomeTopBarTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    private fun render(state: HomeModel.State, actions: HomeActions) {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                        HomeScreen(state = state, actions = actions)
                    }
                }
            }
        }
    }

    @Test
    fun theTopbarIsOnTheLoadingHomeAndItsTwoTargetsAnswerOnce() {
        var searches = 0
        var flows = 0
        render(
            HomeModel.State(),
            HomeActions(openSearch = { searches++ }, launchFlow = { flows++ }),
        )

        rule.composeRule.onNodeWithTag(HOME_TOPBAR_SEARCH_TAG).assertExists()
        rule.composeRule.onNodeWithTag(HOME_TOPBAR_FLOW_TAG).assertExists()
        rule.composeRule.onNodeWithText("Titres, artistes, albums, podcasts…").assertExists()

        rule.composeRule.onNodeWithTag(HOME_TOPBAR_SEARCH_TAG).performClick()
        rule.composeRule.onNodeWithTag(HOME_TOPBAR_FLOW_TAG).performClick()
        rule.composeRule.waitForIdle()

        assertEquals("la pastille ouvre Explorer une fois", 1, searches)
        assertEquals("le cercle lance le Flow une fois", 1, flows)
    }

    @Test
    fun theTopbarStaysAboveTheLoadedHero() {
        render(
            HomeModel.State(screen = ScreenState.Ready(HomeFixtures.home())),
            HomeActions(),
        )

        rule.composeRule.onNodeWithTag(HOME_TOPBAR_SEARCH_TAG).assertExists()
        rule.composeRule.onNodeWithTag(HOME_TOPBAR_FLOW_TAG).assertExists()
        rule.composeRule.onNodeWithTag(HOME_HERO_TAG).assertExists()
        val bar = rule.composeRule.onNodeWithTag(HOME_TOPBAR_SEARCH_TAG)
            .fetchSemanticsNode().boundsInRoot
        val hero = rule.composeRule.onNodeWithTag(HOME_HERO_TAG)
            .fetchSemanticsNode().boundsInRoot
        assertEquals("la barre est au-dessus du héros", true, bar.bottom <= hero.top)
    }

    /**
     * The bench's own answer: `HomeData` with no style row at all. The section
     * and its fallback chips are what the reference (`01-accueil.png`) shows.
     */
    @Test
    fun theBenchWithoutAMeasuredGenreDrawsTheSpasFallbackChips() {
        render(
            HomeModel.State(screen = ScreenState.Ready(HomeFixtures.home()), styles = emptyList()),
            HomeActions(),
        )

        rule.composeRule.onNodeWithText("Par style").assertExists()
        rule.composeRule.onNodeWithText("Tout").assertExists()
        for (name in STYLE_FALLBACK.take(3)) {
            rule.composeRule.onNodeWithText(name).assertExists()
        }
    }
}
