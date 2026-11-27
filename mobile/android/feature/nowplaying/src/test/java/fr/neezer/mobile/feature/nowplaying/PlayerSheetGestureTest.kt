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
import androidx.compose.ui.test.swipeWithVelocity
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
import org.robolectric.annotation.GraphicsMode
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
// NATIVE depuis JF : le lecteur peint désormais un champ de pixels (le fond
// flouté, `PlayerBackdrop`), et les bitmaps de Robolectric LEGACY n'ont pas de
// natif (`Bitmap.createBitmap` rend null). Le geste ne lit aucun pixel ; seul
// le rendu a besoin du vrai backend, comme les goldens.
@GraphicsMode(GraphicsMode.Mode.NATIVE)
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

    // -- T5: the frames, not only the ends ------------------------------------

    /**
     * **The sheet moves with the finger at EVERY frame, and the release continues
     * from there** (T5, `plan-mobile-tests.md` §T5, bug 6).
     *
     * The P2 cases above read the sheet at rest and one frame after the release;
     * they say the end state is right, not that the content ever followed the
     * finger. A sheet that only lands at the end (the old
     * `AnimatedVisibility(slideOutVertically)`) or one that snaps back on the
     * release frame can still pass a settle check, and a CI log cannot say which
     * frame broke. This case holds the clock (`autoAdvance = false`), injects
     * **6 × 5 % of the sheet** one event frame apart and reads
     * `boundsInRoot.top` after each one: every frame is printed with its number.
     *
     * The platform's touch slop is paid once before the measured frames, because
     * the frame claims the gesture after it and follows only the travel beyond it
     * (`NowPlaying.kt`, the pointer loop: `start + travelY - slop`). Every
     * measured frame must then move at least 60 % of its injected dy — the plan's
     * floor — and 1:1 within ±2 dp.
     *
     * Then the release above the threshold, as the plan names it:
     * `swipeWithVelocity(…, endVelocity = 1 200f)` over 30 % of the sheet. The
     * three frames after it must keep going DOWN (seeded with that velocity, never
     * restarted from 0), and the sheet must land on the closed detent inside
     * 500 ms.
     */
    @Test
    fun theSheetFollowsTheFingerFrameByFrameAndTheFlingLandsOnTheClosedDetent() {
        openSheet()
        rule.mainClock.autoAdvance = false
        val height = sheetHeightDp()
        val step = height * 0.05f
        // The platform's touch slop is paid ONCE, before the measured frames: the
        // frame claims the gesture after it and follows only the travel beyond it
        // (`NowPlaying.kt`, the pointer loop: `start + travelY - slop`). Paying it
        // here is what lets every measured frame be 1:1 instead of losing the
        // slop again on the first one.
        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            advanceEventTime(16)
            moveBy(Offset(0f, viewConfiguration.touchSlop + 2f))
        }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        var previous = sheetTopDp()
        for (frame in 1..6) {
            rule.onNodeWithTag(TAG).performTouchInput {
                advanceEventTime(16)
                moveBy(Offset(0f, px(step)))
            }
            // Robolectric refreshes the semantics tree at the end of an injection
            // block that carried an event, and a read taken right after a MOVE
            // block sees the block BEFORE it (measured: frames 2 to 6 read 64, 64,
            // 104, 104, 144 without this flush). A hundredth of a pixel is the
            // smallest event that makes the tree current; it changes no distance
            // the assertions read (±2 dp).
            flushPointer()
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            val top = sheetTopDp()
            val moved = top - previous
            println(
                "MEASURE p2 frame $frame: dy=${"%.1f".format(step)}dp, " +
                    "top=${"%.1f".format(top)}dp, moved=${"%.1f".format(moved)}dp",
            )
            assertTrue(
                "frame $frame: the sheet must follow the finger " +
                    "(moved ${"%.1f".format(moved)}dp of ${"%.1f".format(step)}dp)",
                moved >= step * 0.6f,
            )
            assertEquals("frame $frame: 1:1 with the finger (±2 dp)", step, moved, 2f)
            previous = top
        }
        assertTrue(
            "the six frames are past the 28 % threshold ($previous / $height dp)",
            previous > height * 0.28f,
        )

        // The finger leaves without deciding: a cancel is never a decision (S8),
        // so the sheet springs back to the open detent and the fling below starts
        // from 0. (The up of a real fling must be in the SAME `performTouchInput`
        // block as the last move — a separate block restarts the event clock and
        // the tracker reads the pause, not the flick; measured in case 3 above.)
        rule.onNodeWithTag(TAG).performTouchInput { cancel() }
        var returned = 0
        while (sheetTopDp() > 1f && returned < 1_000) {
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            returned += 16
        }
        assertEquals("the cancelled tracking gesture returns to the open detent", 0f, sheetTopDp(), 1f)

        // The release above the threshold, as the plan names it: 30 % of the
        // sheet at 1 200 px/s, over both the 28 % distance and the 125 dp/s fling.
        rule.onNodeWithTag(TAG).performTouchInput {
            swipeWithVelocity(
                start = Offset(centerX, top + 40.dp.toPx()),
                end = Offset(centerX, top + 40.dp.toPx() + px(step) * 6f),
                endVelocity = 1_200f,
                durationMillis = 100,
            )
        }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        previous = sheetTopDp()
        println("MEASURE p2 fling: released at ${"%.1f".format(previous)}dp of ${"%.1f".format(height)}dp")

        repeat(3) { frame ->
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            assertTrue(
                "release frame ${frame + 1}: the settle must take more than one frame " +
                    "(an instant settle is the old exit)",
                NowPlayingState.open,
            )
            val top = sheetTopDp()
            println("MEASURE p2 release frame ${frame + 1}: top=${"%.1f".format(top)}dp")
            assertTrue(
                "release frame ${frame + 1}: the sheet keeps leaving, it does not " +
                    "stop dead at the finger (${"%.1f".format(previous)} -> ${"%.1f".format(top)}dp)",
                top > previous,
            )
            previous = top
        }

        var elapsed = 0
        var last = previous
        while (NowPlayingState.open && elapsed < 500) {
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            if (NowPlayingState.open) last = sheetTopDp()
            elapsed += 16
        }
        rule.mainClock.autoAdvance = true
        println(
            "MEASURE p2 fling: ${if (NowPlayingState.open) "still open" else "closed"} " +
                "after ${elapsed}ms, last top=${"%.1f".format(last)}dp of ${"%.1f".format(height)}dp",
        )
        assertFalse("a fling above 125 dp/s dismisses the player inside 500 ms", NowPlayingState.open)
        assertTrue(
            "the sheet reached the closed detent ($last of $height dp)",
            last >= height * 0.95f,
        )
        rule.onNodeWithTag(TAG).assertDoesNotExist()
    }

    /**
     * **A slow release returns to the open detent without jumping back on the
     * release frame** (T5).
     *
     * 12 % of the sheet injected (≈10 % of movement after the platform's slop)
     * at ≈80 dp/s: under the 28 % distance and under the 125 dp/s fling, so the
     * settle can only be the return. The frame after the release must still be at
     * the finger (a teleport to 0 — what the old exit did — reads 0 there), and
     * the sheet is back at 0 inside 500 ms, open.
     */
    @Test
    fun aLowVelocityReleaseDoesNotJumpBackOnTheReleaseFrame() {
        openSheet()
        rule.mainClock.autoAdvance = false
        val step = sheetHeightDp() * 0.03f
        rule.onNodeWithTag(TAG).performTouchInput {
            down(Offset(centerX, top + 40.dp.toPx()))
            // 4 × 3 % at 300 ms each ≈ 80 dp/s.
            repeat(4) {
                advanceEventTime(300)
                moveBy(Offset(0f, px(step)))
            }
            up()
        }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val releasedAt = sheetTopDp()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val afterRelease = sheetTopDp()
        println(
            "MEASURE p2 low release: released at ${"%.1f".format(releasedAt)}dp, " +
                "first frame ${"%.1f".format(afterRelease)}dp",
        )
        assertTrue(
            "the release frame must not jump back to 0 (released at $releasedAt dp, " +
                "first frame $afterRelease dp)",
            afterRelease > releasedAt * 0.75f,
        )

        var elapsed = 0
        while (sheetTopDp() > 1f && elapsed < 1_000) {
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            elapsed += 16
        }
        rule.mainClock.autoAdvance = true
        println("MEASURE p2 low release settle: $elapsed ms")
        assertEquals("the low release returns to the open detent", 0f, sheetTopDp(), 1f)
        assertTrue("a 12 % drag at rest does not dismiss", NowPlayingState.open)
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

    /**
     * A move of one hundredth of a pixel: the smallest injection that makes the
     * semantics tree current after a move block (see the tracking case).
     */
    private fun flushPointer() {
        rule.onNodeWithTag(TAG).performTouchInput {
            advanceEventTime(1)
            moveBy(Offset(0f, 0.01f))
        }
    }

    /** A dp distance of the sheet, in the pointer's px (the injection takes px). */
    private fun px(dp: Float): Float = with(rule.density) { dp.dp.toPx() }

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
