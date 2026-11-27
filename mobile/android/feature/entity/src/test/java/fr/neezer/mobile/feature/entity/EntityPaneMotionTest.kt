package fr.neezer.mobile.feature.entity

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The entity pages' pane (lot X4): a tab, a phase and a sheet's answer are
 * replaced IN PLACE, and the band does not move with them.
 *
 * What a Compose test can hold, as X0 wrote it down: `getUnclippedBoundsInRoot` is
 * a layout bound, so a `graphicsLayer` translation and an alpha are invisible to it.
 * The pane's CURVE is `TransitionTest`'s (`rememberPaneEnter` at the three levels);
 * this file holds the WIRING — at eco a change is one frame with no leftover, and
 * the band's bounds are identical on both sides of it, which is what fails if the
 * whole `LazyColumn` is wrapped in an `AnimatedContent`.
 *
 * No clock sleep: `mainClock` is advanced explicitly (worker rule 6).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class EntityPaneMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** The state setter, so a test changes a screen's state from OUTSIDE it. */
    private var updateArtist: ((ArtistModel.State) -> Unit)? = null
    private var updateAlbum: ((AlbumModel.State) -> Unit)? = null

    /**
     * One FRAME, composed and applied: `waitForIdle` returns before the
     * recomposition that sees the new state, and the write composes on the second
     * frame on this tree (measured in `TransitionTest`).
     */
    private fun twoFrames() {
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
    }

    private fun artistState(tab: ArtistTab): ArtistModel.State = ArtistModel.State(
        id = EntityFixtures.ARTIST_DEEZER,
        screen = ScreenState.Ready(EntityFixtures.artist()),
        tab = tab,
        discography = EntityFixtures.discography(),
        tracks = ScreenState.Ready(FakeRows((0 until 40).map { EntityFixtures.row("Titre $it", it) })),
        credits = ScreenState.Ready(EntityFixtures.person()),
    )

    /** The artist's two leaves swap, and the band is where it was (0 dp). */
    @Test
    fun anArtistTabChangeSwapsTheBodyAndNotTheBand() {
        var current by mutableStateOf(artistState(ArtistTab.Tracks))
        updateArtist = { next -> current = next }
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ArtistScreen(state = current, actions = ArtistActions())
                }
            }
        }
        rule.waitForIdle()
        val bandBefore = rule.onNodeWithTag(ARTIST_BAND_TAG).getUnclippedBoundsInRoot()
        rule.onNodeWithText("Titre 0").assertExists()

        rule.mainClock.autoAdvance = false
        updateArtist!!(artistState(ArtistTab.Credits))
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        // The credits tab's own bar: the person's role chips (`PersonCreditsBar`).
        rule.onNodeWithText("Compositeur").assertExists()
        rule.onAllNodesWithText("Titre 0").assertCountEquals(0)
        val bandAfter = rule.onNodeWithTag(ARTIST_BAND_TAG).getUnclippedBoundsInRoot()
        assertEquals("la bande ne bouge pas d'un onglet à l'autre", bandBefore.top.value, bandAfter.top.value, 0f)
        assertEquals(bandBefore.bottom.value, bandAfter.bottom.value, 0f)
        println("MEASURE x4 artiste: band=${bandBefore.top.value}..${bandAfter.bottom.value}dp")
    }

    /** The album's skeleton becomes the page in one frame, band included. */
    @Test
    fun anAlbumPhaseChangeLandsInOneFrameAtEco() {
        var current by mutableStateOf(AlbumModel.State(id = EntityFixtures.ALBUM_ID))
        updateAlbum = { next -> current = next }
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    AlbumScreen(state = current, actions = AlbumActions())
                }
            }
        }
        rule.waitForIdle()
        val skeletonBand = rule.onNodeWithTag(ALBUM_LIST_TAG).getUnclippedBoundsInRoot()
        rule.onAllNodesWithText("Ouverture").assertCountEquals(0)

        rule.mainClock.autoAdvance = false
        updateAlbum!!(
            AlbumModel.State(
                id = EntityFixtures.ALBUM_ID,
                screen = ScreenState.Ready(EntityFixtures.albumView()),
                onDeezerFetched = true,
            ),
        )
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Ouverture").assertExists()
        rule.onNodeWithText("Le matin").assertExists()
        val pageBand = rule.onNodeWithTag(ALBUM_LIST_TAG).getUnclippedBoundsInRoot()
        assertEquals("le haut de la page ne bouge pas", skeletonBand.top.value, pageBand.top.value, 0f)
        println("MEASURE x4 album: list.top=${pageBand.top.value}dp")
    }

    /**
     * The credits SHEET is a Column inside `NzSheet`, not a `LazyColumn`: its four
     * answers are an `NzSwap` (the one site of this lot where a layout composable
     * can wrap the phase) — at eco the new answer is there in the same frame, and
     * the skeleton it replaced holds no node.
     */
    @Test
    fun theCreditsSheetSwapsItsAnswerInTheSameFrameAtEco() {
        var current by mutableStateOf(
            TrackCreditsModel.State(track = EntityFixtures.row("Sinnerman", 0)),
        )
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Off) {
                TrackCreditsSheet(
                    state = current,
                    track = EntityFixtures.row("Sinnerman", 0),
                    visible = true,
                    onDismiss = {},
                    onRetry = {},
                    onArtist = {},
                    onPerson = {},
                )
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("Compositeur").assertDoesNotExist()

        rule.mainClock.autoAdvance = false
        current = TrackCreditsModel.State(
            track = EntityFixtures.row("Sinnerman", 0),
            screen = ScreenState.Ready(TrackCreditsView.of(EntityFixtures.trackCredits())),
        )
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Compositeur").assertExists()
        // One panel, not two: the swapped-away answer left no node behind.
        assertTrue(
            "the sheet draws its answer once",
            rule.onAllNodesWithText("Compositeur").fetchSemanticsNodes().size == 1,
        )
    }
}
