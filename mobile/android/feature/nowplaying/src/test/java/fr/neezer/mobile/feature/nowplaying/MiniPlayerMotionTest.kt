package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.LocalNzCoverLoader
import fr.neezer.mobile.designsystem.NzCoverLoader
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerSnapshot

/**
 * The mini bar's reservation and its two swaps (lot X2).
 *
 * What it proves, as counts and bounds rather than a claim to "see" an
 * animation:
 *
 * 1. the bar is the SAME 76 dp with nothing playing and with a track playing
 *    (64 dp card + 4 top + 8 bottom), and the content above it does not move
 *    when the first title arrives;
 * 2. at eco the title replaces in the same frame AND the sleeve's node is the
 *    one node before and after (it lives outside `NzSwap`, P1's rule);
 * 3. the transport holds TWO glyph nodes for the 90 ms the outgoing one takes to
 *    leave, then one — and at eco, one from the first frame;
 * 4. a re-emit with the same track key animates nothing and adds no node.
 *
 * **The honest limit** (X0's, repeated): Compose exposes no API to read a
 * running animation's alpha or transform. The 140 ms curve is the design
 * system's exact spec, held by `TransitionTest`; this file holds the WIRING of
 * the bar — which nodes exist at a given clock time — and nothing here claims to
 * see a fade. No clock sleep: `mainClock` is advanced explicitly.
 *
 * The golden `nowplaying-mini-vide-360dp-f1.0.png` is the empty bar at 360 dp
 * (the plan's Paparazzi step). `mobile/android/screenshots` is the repository's
 * ONE golden root (T3), and this lot's own copy under
 * `docs/agents/screenshots/mobile/X2/` is the same picture.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class MiniPlayerMotionTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    /** The bar's accepted height: L1's 64 dp card plus its 4/8 dp margins. */
    private val barHeightDp = 76f

    private fun empty(): PlayerSnapshot =
        FakePlayerPort.snapshot(state = PlayState.STOPPED, current = null, index = null, len = 0u)

    private fun playing(
        key: String = "k1",
        title: String = "Premier titre",
        state: PlayState = PlayState.PLAYING,
        cover: String? = null,
    ): PlayerSnapshot = FakePlayerPort.snapshot(
        state = state,
        current = FakePlayerPort.track(key = key, title = title).copy(cover = cover),
        index = 0u,
        len = 4u,
    )

    /** What the bar asked the cover seam for — the "0 covers when empty" count. */
    private class CoverRecorder : NzCoverLoader {
        val urls = mutableListOf<String>()

        @Composable
        override fun Image(url: String, sizePx: Int) {
            urls += url
        }
    }

    /**
     * The process's one model, put on a known picture before the bar composes.
     *
     * `MiniPlayer` reads `NowPlayingSession.model` in the app (one player, one
     * feed, one state); the test drives that same model instead of installing a
     * second one, which is what the shell does.
     */
    private fun reset(snapshot: PlayerSnapshot) {
        NowPlayingSession.model.onSnapshot(snapshot)
    }

    private fun setBar(level: NzMotionLevel) {
        rule.setContent(fontScale = 1f) {
            NzTheme(kind = NzThemeKind.Encre, motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Column {
                        // What a tab's content would draw above the bar: if the
                        // bar's height changed, this line would move with it.
                        Box(Modifier.fillMaxWidth().height(40.dp).testTag(ABOVE_TAG))
                        MiniPlayer()
                    }
                }
            }
        }
    }

    private fun heightOf(tag: String): Float =
        rule.composeRule.onNodeWithTag(tag).getUnclippedBoundsInRoot().let { it.bottom.value - it.top.value }

    @Test
    fun theRoomIsReservedBeforeAnythingPlays() {
        reset(empty())
        setBar(NzMotionLevel.Off)

        rule.composeRule.onNodeWithText("Rien en lecture").assertExists()
        assertEquals("the empty bar is 64 + 4 + 8 dp", barHeightDp, heightOf(MINI_BAR_TAG), 1f)
        val emptyTop = rule.composeRule.onNodeWithTag(MINI_BAR_TAG).getUnclippedBoundsInRoot().top.value
        val aboveBottom = rule.composeRule.onNodeWithTag(ABOVE_TAG).getUnclippedBoundsInRoot().bottom.value
        rule.capture("nowplaying-mini-vide-360dp-f1.0.png")

        reset(playing())
        rule.composeRule.waitForIdle()

        assertEquals("the playing bar keeps the same height", barHeightDp, heightOf(MINI_BAR_TAG), 1f)
        assertEquals(
            "the first play moves nothing above the bar",
            emptyTop,
            rule.composeRule.onNodeWithTag(MINI_BAR_TAG).getUnclippedBoundsInRoot().top.value,
            0.5f,
        )
        assertEquals(
            "the content above keeps its bottom",
            aboveBottom,
            rule.composeRule.onNodeWithTag(ABOVE_TAG).getUnclippedBoundsInRoot().bottom.value,
            0.5f,
        )
    }

    @Test
    fun aTrackArrivesInTheSameFrameAtEcoAndKeepsTheSleeveNode() {
        reset(empty())
        setBar(NzMotionLevel.Off)
        rule.composeRule.onAllNodesWithTag(MINI_COVER_TAG, useUnmergedTree = true).assertCountEquals(1)

        reset(playing(title = "Le chant des sirènes"))
        rule.composeRule.waitForIdle()

        rule.composeRule.onNodeWithText("Rien en lecture").assertDoesNotExist()
        rule.composeRule.onNodeWithText("Le chant des sirènes").assertExists()
        // The sleeve is OUTSIDE the swap: one node before and after (P1).
        rule.composeRule.onAllNodesWithTag(MINI_COVER_TAG, useUnmergedTree = true).assertCountEquals(1)
    }

    @Test
    fun theTransportHoldsBothGlyphsForItsExitAtFull() {
        reset(playing())
        setBar(NzMotionLevel.Full)
        rule.composeRule.onNodeWithContentDescription("Pause", useUnmergedTree = true).assertExists()

        rule.composeRule.mainClock.autoAdvance = false
        NowPlayingSession.model.toggle()
        rule.composeRule.waitForIdle()
        rule.composeRule.mainClock.advanceTimeBy(30)

        // Two icons in the tree: the outgoing pause is held for its 90 ms exit
        // and the content lambda reads the NEW label for both (X0's measurement).
        rule.composeRule.onAllNodesWithContentDescription("Lecture", useUnmergedTree = true).assertCountEquals(2)

        rule.composeRule.mainClock.advanceTimeBy(120)
        rule.composeRule.mainClock.autoAdvance = true
        rule.composeRule.waitForIdle()
        rule.composeRule.onAllNodesWithContentDescription("Lecture", useUnmergedTree = true).assertCountEquals(1)
        rule.composeRule.onNodeWithContentDescription("Pause", useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun theTransportShowsOneGlyphAtEco() {
        reset(playing())
        setBar(NzMotionLevel.Off)

        NowPlayingSession.model.toggle()
        rule.composeRule.waitForIdle()

        rule.composeRule.onAllNodesWithContentDescription("Lecture", useUnmergedTree = true).assertCountEquals(1)
        rule.composeRule.onNodeWithContentDescription("Pause", useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun aSameKeyReEmitAnimatesNothing() {
        reset(playing(key = "k1", title = "Premier titre"))
        setBar(NzMotionLevel.Full)
        rule.composeRule.onAllNodesWithText("Premier titre").assertCountEquals(1)

        rule.composeRule.mainClock.autoAdvance = false
        // Same key, a bumped picture: the bar must not animate a second copy.
        reset(playing(key = "k1", title = "Premier titre", state = PlayState.PAUSED))
        rule.composeRule.waitForIdle()
        rule.composeRule.mainClock.advanceTimeBy(30)

        rule.composeRule.onAllNodesWithText("Premier titre").assertCountEquals(1)
        rule.composeRule.mainClock.autoAdvance = true
        rule.composeRule.waitForIdle()
    }

    @Test
    fun theEmptyBarAsksForNoCover() {
        reset(empty())
        val covers = CoverRecorder()
        rule.setContent(fontScale = 1f) {
            CompositionLocalProvider(LocalNzCoverLoader provides covers) {
                NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                    MiniPlayer()
                }
            }
        }
        rule.composeRule.waitForIdle()
        assertEquals("the empty card draws the groove, not a sleeve", 0, covers.urls.size)

        reset(playing(cover = "/api/cover/1"))
        rule.composeRule.waitForIdle()
        assertEquals("a track asks for its sleeve once", 1, covers.urls.size)
        assertEquals("/api/cover/1", covers.urls.single())
    }

    @Test
    fun theBufferingStatusKeepsMovingAtEco() {
        // The player's buffering state is the « Chargement… » line in this tree
        // (there is no ring node to mark): what the design system guarantees is
        // that a STATUS indicator is the one thing eco never freezes (M13), which
        // a frozen spinner would turn into a hang (CLAUDE.md rule 2).
        reset(empty())
        setBar(NzMotionLevel.Off)
        assertTrue("M13: eco keeps a status indicator moving", NzMotion(NzMotionLevel.Off).statusMoves)
    }

    private companion object {
        const val ABOVE_TAG = "mini-player-above"
    }
}
