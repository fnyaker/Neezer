package fr.neezer.mobile.feature.podcasts

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
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The show's PHASE is a pane, not an `AnimatedContent` (lot X5b): the episodes are
 * `LazyColumn` items, so a swap around the body would carry the show's header with
 * it — and the header's own item must not move.
 *
 * What a Compose test can hold (X0's honest limit): `getUnclippedBoundsInRoot` is a
 * layout bound, so the pane's alpha and 4 dp rise are invisible to it and the curve
 * is `TransitionTest`'s. This file holds the WIRING, and it is a real guard against
 * the mistake this lot forbids: `NzSwap` slides its content by a PLACEMENT offset,
 * so a body wrapped in it starts 4 dp lower and the header's top moves in the
 * change's first frame.
 *
 * No clock sleep: `mainClock` is advanced explicitly (worker rule 6).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class ShowScreenPaneTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** The screen's state source: a test changes the phase from OUTSIDE the screen. */
    private var state by mutableStateOf(showState(cards = 3))

    private fun episodes(count: Int): Map<Int, uniffi.nz_ffi.EpisodeData> =
        (0 until count).associateWith { index -> episode("ep$index", title = "Épisode $index") }

    private fun showState(cards: Int, loading: Boolean = false): ShowModel.State = ShowModel.State(
        id = "show-1",
        show = show("show-1", "Les Histoires", episodes = 2_000),
        loading = loading,
        loaded = cards,
        cards = episodes(cards),
    )

    /** Every gesture the screen can ask for, none of them used by this test. */
    private fun actions() = ShowActions(
        retry = {},
        onVisible = { _, _, _ -> },
        loadMore = {},
        play = {},
        playAll = {},
        playAt = { _, _ -> },
        toggleAuto = {},
        retryTranscribe = {},
        toggleTranscribe = {},
        setLanguage = {},
        setBacklog = {},
        setBatchCount = {},
        startBatch = {},
        startOne = {},
        stopOne = {},
        setSearchQuery = {},
        retrySearch = {},
        clearSearch = {},
        toggleChapters = {},
        openSheet = {},
        addMarker = { _, _ -> },
        removeMarker = {},
        openEpisode = {},
        consumeMessage = {},
    )

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

    /** The show's title: the header's own node, the same one before and after. */
    private fun header() = rule.onNodeWithText("Les Histoires").getUnclippedBoundsInRoot()

    private fun host(level: NzMotionLevel) {
        state = showState(cards = 3)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ShowScreen(
                        state = state,
                        actions = actions(),
                        isAdmin = false,
                        currentKey = null,
                        playing = false,
                        positionS = 0.0,
                    )
                }
            }
        }
        rule.waitForIdle()
    }

    /**
     * An episode list that empties: at `Full`, the header is at the same top in the
     * change's own frame (a body wrapped in a swap starts it 4 dp lower) and in the
     * same box once the panel has landed.
     */
    @Test
    fun aPhaseChangeKeepsTheHeaderInPlaceAtFull() {
        host(NzMotionLevel.Full)
        val before = header()
        rule.onNodeWithText("Épisode 0").assertExists()

        rule.mainClock.autoAdvance = false
        state = showState(cards = 0)
        twoFrames()
        val firstFrame = header()
        rule.mainClock.advanceTimeBy(300)
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Aucun épisode").assertExists()
        rule.onAllNodesWithText("Épisode 0").assertCountEquals(0)
        assertEquals(
            "the header does not rise with the panel, in the change's own frame",
            before.top.value,
            firstFrame.top.value,
            0f,
        )
        val settled = header()
        assertEquals("the header's top is the same box", before.top.value, settled.top.value, 0f)
        assertEquals("the header's bottom is the same box", before.bottom.value, settled.bottom.value, 0f)
        println("MEASURE x5b podcast: header=${settled.top.value}..${settled.bottom.value}dp")
    }

    /**
     * At eco the panel is the new one in the FIRST frame that shows it: the empty
     * answer is there, the episodes it replaced hold no node, the header is still.
     */
    @Test
    fun aPhaseChangeLandsInTheSameFrameAtEco() {
        host(NzMotionLevel.Off)
        val before = header()
        rule.onNodeWithText("Épisode 0").assertExists()

        rule.mainClock.autoAdvance = false
        state = showState(cards = 0)
        twoFrames()
        val frameZero = header()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Aucun épisode").assertExists()
        rule.onAllNodesWithText("Épisode 0").assertCountEquals(0)
        assertEquals(
            "eco plays nothing: the header is settled at frame 0",
            before.top.value,
            frameZero.top.value,
            0f,
        )
    }

    /**
     * A refresh in flight (`loading = true` with the show on screen) re-emits the
     * SAME phase: no second copy of the panel and nothing starts. The curve of a real
     * change is `TransitionTest`'s; this holds that the same key is silent.
     */
    @Test
    fun aReEmitOfTheSamePhaseAddsNoNodeAndPlaysNothing() {
        host(NzMotionLevel.Full)
        val before = header()

        rule.mainClock.autoAdvance = false
        state = state.copy(loading = true)
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onAllNodesWithText("Épisode 0").assertCountEquals(1)
        rule.onAllNodesWithText("Aucun épisode").assertCountEquals(0)
        assertEquals("the header did not move", before.top.value, header().top.value, 0f)
    }
}
