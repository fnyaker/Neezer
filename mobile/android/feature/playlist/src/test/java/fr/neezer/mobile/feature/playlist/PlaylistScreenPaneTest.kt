package fr.neezer.mobile.feature.playlist

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
import uniffi.nz_ffi.TrackRow

/**
 * The playlist's PHASE is a pane, not an `AnimatedContent` (lot X5b): the body of
 * the phase is a `LazyColumn`, so wrapping it faded the BAND — the list's first
 * item, and this screen's header — with every row.
 *
 * What a Compose test can hold (X0's honest limit, repeated): `getUnclippedBoundsInRoot`
 * is a LAYOUT bound, so the pane's `graphicsLayer` alpha and rise are invisible to
 * it and the curve is `TransitionTest`'s (`rememberPaneEnter` at the three levels).
 * This file holds the WIRING, and it is a real guard: `NzSwap`'s
 * `slideInVertically` is a PLACEMENT offset, so a body wrapped in it starts 4 dp
 * below and the band's top moves in the change's first frame — the assertion below
 * fails on that tree (run once on it before this lot, see `docs/agents/mobile.md`,
 * X5b).
 *
 * No clock sleep: `mainClock` is advanced explicitly (worker rule 6).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class PlaylistScreenPaneTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** The screen's state source: a test changes the phase from OUTSIDE the screen. */
    private var state by mutableStateOf(PlaylistModel.State())

    private fun stateWith(rows: List<TrackRow>): PlaylistModel.State {
        val source = FakeRows(rows)
        return PlaylistModel.State(
            id = "pl-1",
            page = PlaylistFixtures.page(rows = source),
            rows = source,
        )
    }

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

    /** The band's title: the header's own node, the same one before and after. */
    private fun bandTitle() = rule.onNodeWithText("Playlist de 4 000 titres").getUnclippedBoundsInRoot()

    private fun host(level: NzMotionLevel) {
        state = stateWith(PlaylistFixtures.tracks(3))
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    PlaylistScreen(state = state, actions = PlaylistActions(), wide = false)
                }
            }
        }
        rule.waitForIdle()
    }

    /**
     * The band's first item is NOT part of the panel: at `Full`, the frame the
     * phase changes on already holds the band at the same top (the old tree's
     * `AnimatedContent` starts its content 4 dp lower), and the settled band is
     * the same box.
     */
    @Test
    fun aPhaseChangeKeepsTheBandInPlaceAtFull() {
        host(NzMotionLevel.Full)
        val before = bandTitle()
        rule.onNodeWithText("Titre 0").assertExists()

        rule.mainClock.autoAdvance = false
        state = stateWith(emptyList())
        twoFrames()
        val firstFrame = bandTitle()
        rule.mainClock.advanceTimeBy(300)
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Cette playlist est vide").assertExists()
        rule.onAllNodesWithText("Titre 0").assertCountEquals(0)
        assertEquals(
            "the band does not rise with the panel, in the change's own frame",
            before.top.value,
            firstFrame.top.value,
            0f,
        )
        val settled = bandTitle()
        assertEquals("the band's top is the same box", before.top.value, settled.top.value, 0f)
        assertEquals("the band's bottom is the same box", before.bottom.value, settled.bottom.value, 0f)
        println("MEASURE x5b playlist: band=${settled.top.value}..${settled.bottom.value}dp")
    }

    /**
     * At eco the panel is the new one in the FIRST frame that shows it: the empty
     * answer is there, the rows it replaced hold no node, and the band did not move
     * — no `AnimatedContent` to hold the old body for 140 ms.
     */
    @Test
    fun aPhaseChangeLandsInTheSameFrameAtEco() {
        host(NzMotionLevel.Off)
        val before = bandTitle()
        rule.onNodeWithText("Titre 0").assertExists()

        rule.mainClock.autoAdvance = false
        state = stateWith(emptyList())
        twoFrames()
        val frameZero = bandTitle()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Cette playlist est vide").assertExists()
        rule.onAllNodesWithText("Titre 0").assertCountEquals(0)
        assertEquals(
            "eco plays nothing: the band is settled at frame 0",
            before.top.value,
            frameZero.top.value,
            0f,
        )
    }

    /**
     * A refresh in flight (`loading = true` with the page on screen) re-emits the
     * SAME phase: the panel is not composed twice and nothing starts. The curve of a
     * real change is `TransitionTest`'s; what this holds is that the same key is
     * silent, which is what a naive `if (state != last)` wrapper would get wrong.
     */
    @Test
    fun aReEmitOfTheSamePhaseAddsNoNodeAndPlaysNothing() {
        host(NzMotionLevel.Full)
        val before = bandTitle()

        rule.mainClock.autoAdvance = false
        state = state.copy(loading = true)
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onAllNodesWithText("Titre 0").assertCountEquals(1)
        rule.onAllNodesWithText("Cette playlist est vide").assertCountEquals(0)
        assertEquals("the band did not move", before.top.value, bandTitle().top.value, 0f)
    }
}
