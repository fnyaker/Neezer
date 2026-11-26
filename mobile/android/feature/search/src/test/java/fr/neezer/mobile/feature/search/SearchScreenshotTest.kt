package fr.neezer.mobile.feature.search

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.animation.core.TweenSpec
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.TrackRow

/**
 * The search results, rendered and VERIFIED: a long highlight at 360 and 412 dp
 * (plan T3's matrix).
 *
 * T3 replaced MB6's write-only captures with these goldens. The state is the one
 * that breaks a result row: the typed word sits inside a 60-character title, so
 * the highlight, its ellipsis and the row's height are frozen in the image.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class SearchGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun longHighlightAt360() = shot(360, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h900dp-xhdpi")
    fun longHighlightAt412() = shot(412, 1.0f)

    private fun shot(width: Int, scale: Float) {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    SearchScreen(state = longHighlightState(), actions = SearchActions())
                }
            }
        }
        rule.capture("search-resultats-${width}dp-f$scale.png")
    }

    /** The typed word `sirènes` inside a title long enough to ellipsize. */
    private fun longHighlightState(): SearchModel.State {
        val base = SearchModel.State(
            query = "sirènes",
            answer = SearchFixtures.answer(term = "sirènes", trackCount = 3),
        )
        val answer = base.answer ?: return base
        val rows = listOf(
            row("Le chant des sirènes qui se brisent sur les rochers de la baie", 0),
            row("Sirènes, dernière partie : le retour des voix perdues", 1),
            row("Titre 2", 2),
        )
        return base.copy(answer = answer.copy(tracks = SearchFixtures.FakeRows(rows), trackCount = rows.size))
    }

    private fun row(title: String, position: Int) = TrackRow(
        key = "row:$position",
        id = "id-$position",
        title = title,
        artists = "Fauve, Ascenseur, June, Nina Simone",
        album = "Vieux frères, partie 2 (édition deluxe)",
        albumId = "501",
        durationS = (215 + position).toUInt(),
        cover = "/api/cover/501",
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = position.toUInt(),
    )

    /**
     * X3: the page's loading skeleton becomes the answer in the SAME frame at
     * eco (M7 + M13), and re-emitting the same phase adds no node (M6). The
     * curve is X0's `inPlaceSpecs` (see `TransitionTest` for why a spec, not a
     * running animation, is what a Compose test can read).
     */
    @Test
    fun theLoadingPhaseReplacesInTheSameFrameAtEco() {
        var state by mutableStateOf(SearchModel.State(query = "sirènes"))
        var tick by mutableIntStateOf(0)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    tick
                    SearchScreen(state = state, actions = SearchActions())
                }
            }
        }
        rule.composeRule.onNodeWithTag(SEARCH_LOADING_TAG).assertExists()

        state = SearchModel.State(
            query = "sirènes",
            answer = SearchFixtures.answer(term = "sirènes", trackCount = 3),
        )
        rule.composeRule.waitForIdle()
        rule.composeRule.onNodeWithTag(SEARCH_LOADING_TAG).assertDoesNotExist()
        rule.composeRule.onNodeWithText("Titre 1").assertExists()

        tick++
        rule.composeRule.waitForIdle()
        rule.composeRule.onAllNodesWithTag(SEARCH_TABS_TAG).assertCountEquals(1)

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }

    /**
     * X3: a tab switch swaps the panel, not the row that drives it. The chips
     * row keeps its slot (one node before, one after) and the tracks tab draws
     * its rows. At eco so the old panel is gone in the same frame.
     */
    @Test
    fun aTabSwitchKeepsTheChipsRowAndSwapsThePanel() {
        var state by mutableStateOf(
            SearchModel.State(
                query = "sirènes",
                answer = SearchFixtures.answer(term = "sirènes", trackCount = 3),
            ),
        )
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    SearchScreen(state = state, actions = SearchActions())
                }
            }
        }
        rule.composeRule.onAllNodesWithTag(SEARCH_TABS_TAG).assertCountEquals(1)

        state = state.copy(tab = SearchTab.Tracks)
        rule.composeRule.waitForIdle()
        rule.composeRule.onAllNodesWithTag(SEARCH_TABS_TAG).assertCountEquals(1)
        rule.composeRule.onNodeWithText("Titre 0").assertExists()
    }
}
