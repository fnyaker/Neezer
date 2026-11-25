package fr.neezer.mobile.feature.nowplaying

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.advanceEventTime
import androidx.compose.ui.test.cancel
import androidx.compose.ui.test.down
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.moveBy
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.up
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.nz_ffi.PlayerCommand

/**
 * The full-screen player, closed by a finger (P2, `plan-mobile-ui-finish.md` §3
 * and §4).
 *
 * The sheet used to leave through `AnimatedVisibility(slideOutVertically)`: the
 * exit always started at 0 (a drag could not move it, and the chevron jumped the
 * player back to the top before leaving — `transitions.md` P1's own trap), and the
 * only decision was the clock.
 *
 * This suite drives the real [PlayerSheetFrame] with `performTouchInput` and holds
 * the plan's five cases, each on a NUMBER:
 *
 *  1. **40 % down at rest dismisses** — slowly (3000 ms, ≈107 dp/s, under the
 *     125 dp/s threshold), so only the distance can decide, and the frame after
 *     the release is still at the finger's offset (no jump to 0);
 *  2. **12 % down at rest springs back to 0** in ≤ 400 ms, the sheet still open;
 *  3. **10 % down at 1500 dp/s dismisses** — velocity alone;
 *  4. **a drag that starts on « Pause » does not toggle**, and a tap does, once;
 *  5. **a cancelled gesture is not a decision**: the sheet is back at 0, open.
 *
 * Plus the eco/reduced guard (`transitions.md` M14) and the 1:1 tracking, and the
 * clock is the Compose test clock — never a sleep: the settle is measured in the
 * animation's own time. The sheet's position is read from the node's bounds, so
 * this is the sheet that moved, not a state variable that says so.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class PlayerSheetGestureTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    private lateinit var port: FakePlayerPort
    private lateinit var model: NowPlayingModel

    @Before
    fun resetThePlayer() {
        NowPlayingState.hide()
        port = FakePlayerPort()
        model = NowPlayingModel(
            player = port,
            service = FakeService(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
    }

    // -- the five cases -------------------------------------------------------

    /**
     * **40 % at rest dismisses, and the release continues from the finger.**
     *
     * Slow enough (≈107 dp/s, under the 125 dp/s of the plan's table) that only
     * the 28 % positional threshold can decide. The frame after the release is
     * read with the clock in the test's hands: an exit that restarted at 0 — what
     * `slideOutVertically` did — would read 0 here.
     */
    @Test
    fun aFortyPercentDragAtRestDismissesAndLeavesFromTheFinger() {
        openSheet()
        rule.mainClock.autoAdvance = false
        drag(0.40f, durationMs = 3_000)
        rule.waitForIdle()
        val releasedAt = sheetTopDp()
        val height = sheetHeightDp()
        println(
            "MEASURE p2 dismissal: held at ${"%.1f".format(releasedAt)}dp of " +
                "${"%.1f".format(height)}dp (${"%.1f".format(releasedAt / height * 100)} %), " +
                "threshold 28 %",
        )
        assertTrue(
            "the drag moved the sheet past the 28 % threshold ($releasedAt / $height dp)",
            releasedAt > height * 0.28f,
        )
        rule.onNodeWithTag(TAG).performTouchInput { up() }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val firstFrame = sheetTopDp()
        assertTrue(
            "the sheet leaves from where the finger left it (0 would be the old jump), got $firstFrame",
            firstFrame > releasedAt - 2f,
        )

        val settle = stepUntil(millis = 2_000) { !NowPlayingState.open }
        rule.mainClock.autoAdvance = true
        println("MEASURE p2 dismissal settle after the release: $settle ms")
        assertFalse("a 40 % drag dismisses the player", NowPlayingState.open)
        assertTrue("the dismissal settles inside 2 s ($settle ms)", settle < 2_000)
        rule.onNodeWithTag(TAG).assertDoesNotExist()
    }

    /**
     * **12 % at rest does not dismiss: it springs back, from the finger, to 0.**
     *
     * The number the plan asks for is the settle time (≤ 400 ms), measured on the
     * animation clock.
     */
    @Test
    fun aTwelvePercentDragAtRestSpringsBackInUnder400Ms() {
        openSheet()
        rule.mainClock.autoAdvance = false
        drag(0.12f, durationMs = 2_000)
        rule.waitForIdle()
        val releasedAt = sheetTopDp()
        println("MEASURE p2 snap back: released at ${"%.1f".format(releasedAt)}dp")
        assertTrue("the drag really moved the sheet ($releasedAt dp)", releasedAt > 20f)
        rule.onNodeWithTag(TAG).performTouchInput { up() }

        val settle = stepUntil(millis = 2_000) { sheetTopDp() <= 1f }
        rule.mainClock.autoAdvance = true
        println("MEASURE p2 snap back settle: $settle ms (plan: ≤ 400 ms)")
        assertTrue("the sheet is back at 0 ($settle ms)", settle < 2_000)
        assertTrue("a 12 % drag at rest settles in ≤ 400 ms (got $settle ms)", settle <= 400)
        assertTrue("a 12 % drag at rest does not dismiss", NowPlayingState.open)
        assertEquals("and the sheet is back at the top", 0f, sheetTopDp(), 1f)
    }

    /**
     * **10 % at 1500 dp/s dismisses**: speed alone, the distance stays under 28 %.
     *
     * The plan says 1 500 dp/s; the gesture here is 10 % of the sheet in ~26 ms,
     * which is far faster — what matters is that the DISTANCE alone would not
     * decide (it is under 28 %) and the release still dismisses.
     */
    @Test
    fun aTenPercentFlickDismisses() {
        openSheet()
        rule.mainClock.autoAdvance = false
        // 3 × 60 px, 16 ms apart: ≈1 875 dp/s (the plan's 1 500 dp/s, said with the
        // same kind of flick), under three quarters of the sheet — the distance
        // alone would return it.
        // The release is in the SAME block: a separate `performTouchInput` restarts
        // its event clock, so the up lands milliseconds after the last move and
        // `VelocityTracker` reads the pause, not the flick (measured: 74 dp released,
        // velocity ≈ 0).
        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            repeat(3) {
                advanceEventTime(16)
                moveBy(Offset(0f, 60f))
            }
            up()
        }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val releasedAt = sheetTopDp()
        assertTrue(
            "the distance alone would not dismiss ($releasedAt dp of ${sheetHeightDp()} dp)",
            releasedAt < sheetHeightDp() * 0.28f,
        )
        val settle = stepUntil(millis = 2_000) { !NowPlayingState.open }
        rule.mainClock.autoAdvance = true
        println("MEASURE p2 flick: released at ${"%.1f".format(releasedAt)}dp, settled in $settle ms")
        assertFalse("a 10 % flick above the fling velocity dismisses", NowPlayingState.open)
        rule.onNodeWithTag(TAG).assertDoesNotExist()
    }

    /**
     * **The gesture layer does not eat a button.** A drag that STARTS on « Pause »
     * dismisses the sheet and sends no command (the click is cancelled by the
     * parent's consumption, which is what `waitForUpOrCancellation` is for); a tap
     * on « Pause » toggles exactly once.
     *
     * The plan's case 4 says a drag from « Pause » "released without moving" is "a
     * tap" and does not toggle, which contradicts its own Done-when ("a tap on
     * « Pause » still toggles"). The Done-when governs: a tap toggles, a drag
     * dismisses without toggling — both are asserted here.
     */
    @Test
    fun aDragFromPauseDismissesWithoutTogglingAndATapTogglesOnce() {
        openSheet()
        // 240 dp down the SCREEN (the node here is the 68 dp play button, so its
        // own height is not the sheet's): well past the 28 % threshold at rest.
        rule.onNodeWithContentDescription("Pause").performTouchInput {
            down(center)
            repeat(4) {
                advanceEventTime(500)
                moveBy(Offset(0f, 160f))
            }
            up()
        }
        rule.waitForIdle()
        assertEquals("the drag sent no transport command", 0, port.transportCommands())
        assertFalse("a drag from « Pause » still dismisses", NowPlayingState.open)

        // A fresh sheet for the tap, on the same player.
        NowPlayingState.show()
        rule.waitForIdle()
        rule.onNodeWithContentDescription("Pause").performClick()
        rule.waitForIdle()
        assertEquals("a tap on « Pause » toggles exactly once", 1, port.transportCommands())
    }

    /** **A cancel is never a decision** (`transitions.md` S8). */
    @Test
    fun aCancelledGestureLeavesTheSheetOpenAtZero() {
        openSheet()
        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            advanceEventTime(300)
            moveBy(Offset(0f, height * 0.5f))
            cancel()
        }
        rule.waitForIdle()
        assertTrue("a cancelled gesture does not dismiss", NowPlayingState.open)
        assertEquals("and the sheet is back at 0", 0f, sheetTopDp(), 1f)
    }

    /** **Eco / reduced motion lands at once** — no displacement, no duration. */
    @Test
    fun ecoLandsTheEntranceAndTheSettleAtOnce() {
        openSheet(NzMotionLevel.Off)
        // ONE frame, not an animation: the entrance is a snap.
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        assertEquals("the sheet is at 0 after one frame in eco", 0f, sheetTopDp(), 1f)

        rule.mainClock.autoAdvance = false
        drag(0.12f, durationMs = 2_000)
        rule.onNodeWithTag(TAG).performTouchInput { up() }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val after = sheetTopDp()
        rule.mainClock.autoAdvance = true
        println("MEASURE p2 eco: the 12 % drag is back at ${"%.1f".format(after)}dp after one frame")
        assertTrue("no spring under eco/reduced ($after dp)", after <= 1f)
        assertTrue("and the sheet stays open", NowPlayingState.open)
    }

    // -- the 1:1 property -----------------------------------------------------

    /**
     * **The sheet follows the finger.** A 100 dp drag moves the sheet 100 dp, less
     * the platform's touch slop paid once when the gesture is claimed — ±1 dp.
     *
     * The drag is read while the finger is still down, one move at a time: the
     * sheet's own placement, not a state variable.
     */
    @Test
    fun theSheetFollowsTheFingerOneToOne() {
        openSheet()
        rule.mainClock.autoAdvance = false
        var first = 0f
        var second = 0f
        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            advanceEventTime(400)
            moveBy(Offset(0f, 40.dp.toPx()))
        }
        rule.waitForIdle()
        first = sheetTopDp()
        rule.onNodeWithTag(TAG).performTouchInput {
            advanceEventTime(400)
            moveBy(Offset(0f, 60.dp.toPx()))
        }
        rule.waitForIdle()
        second = sheetTopDp()
        rule.onNodeWithTag(TAG).performTouchInput { cancel() }
        rule.waitForIdle()
        println(
            "MEASURE p2 tracking: finger moved 60dp, sheet moved " +
                "${"%.1f".format(second - first)}dp",
        )
        assertEquals("the sheet follows the finger 1:1 (±1 dp)", 60f, second - first, 1f)
    }

    // -- the harness ----------------------------------------------------------

    /** The sheet open and at rest (the entrance ran to completion). */
    private fun openSheet(motion: NzMotionLevel = NzMotionLevel.Full) {
        NowPlayingState.show()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = motion) {
                PlayerSheetFrame(
                    opened = NowPlayingState.open,
                    onDismiss = { NowPlayingState.hide() },
                ) {
                    // The tag sits INSIDE the frame's offset box: `Modifier.offset`
                    // moves the content without moving the node it is applied to, so
                    // a tag on the frame would read 0 at every position (measured:
                    // the first version of this test "proved" a 40 % drag moved the
                    // sheet 0 dp). Touch input on the content reaches the frame's
                    // gesture by hit-testing from the root.
                    val state by model.state.collectAsState()
                    Box(Modifier.fillMaxSize().testTag(TAG)) {
                        PlayerSurface(
                            state = state,
                            model = model,
                            nav = navigator,
                            lyrics = null,
                            about = null,
                            onClose = { NowPlayingState.hide() },
                            liveReads = false,
                        )
                    }
                }
            }
        }
        rule.waitForIdle()
        assertEquals("the sheet is at rest, at 0", 0f, sheetTopDp(), 1f)
    }

    /** Down, a drag of [fraction] of the sheet, and nothing else (no release). */
    private fun drag(fraction: Float, durationMs: Long) {
        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            val steps = 10
            repeat(steps) {
                advanceEventTime(durationMs / steps)
                moveBy(Offset(0f, height * fraction / steps))
            }
        }
    }

    private fun sheetTopDp(): Float =
        rule.onNodeWithTag(TAG).getUnclippedBoundsInRoot().top.value

    private fun sheetHeightDp(): Float =
        rule.onNodeWithTag(TAG).getUnclippedBoundsInRoot().let { it.bottom.value - it.top.value }

    /**
     * Steps the animation clock one frame at a time until [condition], and answers
     * the animation time it took — the settle number the plan asks for. A frame
     * step is the event; there is no sleep.
     */
    private fun stepUntil(millis: Int, condition: () -> Boolean): Int {
        var elapsed = 0
        while (!condition() && elapsed < millis) {
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            elapsed += 16
        }
        return elapsed
    }

    /** The transport commands the player was sent (never the queue's). */
    private fun FakePlayerPort.transportCommands(): Int = commands.count {
        it == PlayerCommand.Play || it == PlayerCommand.Pause
    }

    private companion object {
        const val TAG = "player-sheet"
    }
}
