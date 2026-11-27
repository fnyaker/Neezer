package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import fr.neezer.mobile.bridge.ScreenState
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
import uniffi.nz_ffi.ExplorerStylesData

/**
 * Découverte, rendered and VERIFIED: the releases page with one group open at
 * 360 dp (plan T3's matrix, one representative state).
 *
 * T3 replaced MB14's write-only captures with this golden. The real
 * [ReleasesScreen] is drawn over [DiscoveryFixtures] (worker rule 7: Robolectric's
 * NATIVE backend, no emulator, no window); a shelf header or a row that drifts
 * fails the comparison.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class DiscoveryGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun releasesAt360() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    // No outer scroll: the screen scrolls itself (a LazyColumn
                    // cannot be measured inside one).
                    Column(Modifier.fillMaxSize()) {
                        ReleasesScreen(
                            state = ReleasesModel.State(
                                screen = ScreenState.Ready(DiscoveryFixtures.releases()),
                                showAll = true,
                                opened = setOf("501"),
                                groupRows = mapOf("501" to DiscoveryFixtures.rows().take(2)),
                            ),
                            actions = ReleasesActions(),
                        )
                    }
                }
            }
        }
        rule.capture("discovery-nouveautes-360dp-f1.0.png")
    }

    /**
     * X3: the feed's phase replaced in place. At eco the skeleton is out of the
     * tree in the SAME frame the shelves are drawn (M7 + M13); a re-emit of the
     * same phase adds no node (M6). The curve is X0's `inPlaceSpecs` (its exact
     * spec; `TransitionTest` explains why a running animation is unreadable).
     */
    @Test
    fun theReleasesPhaseReplacesInTheSameFrameAtEco() {
        var state by mutableStateOf(ReleasesModel.State())
        var tick by mutableIntStateOf(0)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Column(Modifier.fillMaxSize()) {
                        tick
                        ReleasesScreen(state = state, actions = ReleasesActions())
                    }
                }
            }
        }
        rule.composeRule.onNodeWithTag(DISCOVERY_LOADING_TAG).assertExists()
        rule.composeRule.onNodeWithText("Tes artistes").assertDoesNotExist()

        state = ReleasesModel.State(
            screen = ScreenState.Ready(DiscoveryFixtures.releases()),
            showAll = true,
            opened = setOf("501"),
            groupRows = mapOf("501" to DiscoveryFixtures.rows().take(2)),
        )
        rule.composeRule.waitForIdle()
        rule.composeRule.onNodeWithTag(DISCOVERY_LOADING_TAG).assertDoesNotExist()
        rule.composeRule.onNodeWithText("Tes artistes").assertExists()

        tick++
        rule.composeRule.waitForIdle()
        rule.composeRule.onAllNodesWithTag(DISCOVERY_LOADING_TAG).assertCountEquals(0)

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }

    /** X3: the shelf page's loading placeholder gives way to its banner. */
    @Test
    fun theShelfPhaseReplacesInTheSameFrameAtEco() {
        var state by mutableStateOf(ReleaseShelfModel.State())
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ReleaseShelfScreen(state = state, actions = ShelfActions())
                }
            }
        }
        rule.composeRule.onNodeWithTag(DISCOVERY_SHELF_LOADING_TAG).assertExists()

        state = ReleaseShelfModel.State(screen = ScreenState.Ready(DiscoveryFixtures.shelfPage()))
        rule.composeRule.waitForIdle()
        rule.composeRule.onNodeWithTag(DISCOVERY_SHELF_LOADING_TAG).assertDoesNotExist()
        rule.composeRule.onNodeWithText("Tes artistes").assertExists()
    }

    /** X3: the story's loading skeleton gives way to the deck and its « Partager ». */
    @Test
    fun theRecapPhaseReplacesInTheSameFrameAtEco() {
        var state by mutableStateOf(RecapModel.State())
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    RecapScreen(state = state, actions = RecapActions())
                }
            }
        }
        rule.composeRule.onNodeWithText("Chargement de ton récap…").assertExists()

        state = RecapModel.State(
            years = DiscoveryFixtures.recapYears(),
            data = DiscoveryFixtures.recapDeck(),
            activeYear = 2026,
        )
        rule.composeRule.waitForIdle()
        rule.composeRule.onNodeWithText("Chargement de ton récap…").assertDoesNotExist()
        rule.composeRule.onNodeWithText("Partager").assertExists()
    }

    /** X3: the style tiles' skeleton gives way to the grid, in one swap. */
    @Test
    fun theExplorerPhaseReplacesInTheSameFrameAtEco() {
        var state by mutableStateOf(ExplorerModel.State())
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ExplorerPreview(
                        state = state,
                        onOpen = {},
                        onStation = {},
                        onAllStations = {},
                        onRetry = {},
                    )
                }
            }
        }
        rule.composeRule.onNodeWithTag(EXPLORER_LOADING_TAG).assertExists()

        state = ExplorerModel.State(screen = ScreenState.Ready(DiscoveryFixtures.explorerStyles()))
        rule.composeRule.waitForIdle()
        rule.composeRule.onNodeWithTag(EXPLORER_LOADING_TAG).assertDoesNotExist()
        rule.composeRule.onNodeWithText("Techno").assertExists()
    }

    /**
     * No help line (owner, 2026-11-27, C5): an empty shelf says its STATE and
     * nothing else. It said « Aucun style analysé pour l'instant : « Toutes les
     * stations » en propose un. » — a hint under a title, the line the juge
     * blocked #447 on (`02-rechercher`, run 46062575978) — while the
     * « Toutes les stations » action is right above it.
     */
    @Test
    fun theStylesEmptyShelfStatesItselfWithNoHelpLine() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ExplorerPreview(
                        state = ExplorerModel.State(
                            screen = ScreenState.Ready(
                                ExplorerStylesData(
                                    tiles = emptyList(),
                                    source = "unanalysed",
                                    tracks = 0u,
                                    empty = true,
                                ),
                            ),
                        ),
                        onOpen = {},
                        onStation = {},
                        onAllStations = {},
                        onRetry = {},
                    )
                }
            }
        }
        rule.composeRule.onNodeWithText("Aucun style analysé pour l'instant.").assertExists()
        rule.composeRule.onNodeWithText("en propose un", substring = true).assertDoesNotExist()
    }

    /** The same, when the person's own history is empty. */
    @Test
    fun theEmptyHistoryShelfStatesItselfWithNoHelpLine() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ExplorerPreview(
                        state = ExplorerModel.State(
                            screen = ScreenState.Ready(
                                ExplorerStylesData(
                                    tiles = emptyList(),
                                    source = "none",
                                    tracks = 0u,
                                    empty = true,
                                ),
                            ),
                        ),
                        onOpen = {},
                        onStation = {},
                        onAllStations = {},
                        onRetry = {},
                    )
                }
            }
        }
        rule.composeRule.onNodeWithText("Votre historique d'écoute est encore vide.").assertExists()
        rule.composeRule.onNodeWithText("apparaîtront ici", substring = true).assertDoesNotExist()
    }
}
