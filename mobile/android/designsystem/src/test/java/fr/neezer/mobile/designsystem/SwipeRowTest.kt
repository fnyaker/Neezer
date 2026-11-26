package fr.neezer.mobile.designsystem

import androidx.activity.ComponentActivity
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The five behaviours of [`SwipeRow`] (lot G1), on a real Compose measure, a real
 * gesture and a real spring — Robolectric on the host, no emulator (worker rule
 * 7). Every distance is converted to px with the rule's own density, because
 * `performTouchInput` takes px while the component's thresholds are dp.
 *
 * The five cases are the contract the plan publishes for `L1` and `P1`:
 *
 *  1. a slow 40 dp drag does not queue, and the row is back at rest;
 *  2. a 70 dp drag queues exactly once;
 *  3. a 20 dp flick at ~1 200 dp/s queues once (velocity, not distance);
 *  4. a leftward drag queues nothing **and the enclosing horizontal parent
 *     receives it** (the tab pager's passthrough);
 *  5. a cancelled gesture queues nothing and comes back to rest.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class SwipeRowTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private var queued = 0

    /** The row alone, as a list would mount it. */
    private fun row() {
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                SwipeRow(onQueue = { queued++ }) { rowModifier ->
                    Row(rowModifier.height(56.dp)) { Text(TITLE) }
                }
            }
        }
    }

    private fun restLeft() = rule.onNodeWithText(TITLE).getUnclippedBoundsInRoot().left.value

    /**
     * A move of one hundredth of a pixel: the smallest injection that makes the
     * semantics tree current after a move block (Robolectric refreshes it at the
     * end of a block that carried an event, so a read right after a move block
     * sees the block before it — measured: the second 35 dp read 1 dp).
     */
    private fun flushPointer() {
        rule.onNodeWithText(TITLE).performTouchInput {
            advanceEventTime(1)
            moveBy(Offset(0.01f, 0f))
        }
    }

    private fun settle() {
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(600)
        rule.waitForIdle()
    }

    @Test
    fun aSlowShortDragDoesNotQueueAndComesBack() {
        row()
        val before = restLeft()
        rule.onNodeWithText(TITLE).performTouchInput {
            down(center)
            // 8 × 5 dp every 50 ms: 40 dp of travel at ~100 dp/s, well under both
            // thresholds (60 dp, 600 dp/s).
            repeat(8) {
                advanceEventTime(50)
                moveBy(Offset(5.dp.toPx(), 0f))
            }
            up()
        }
        settle()
        assertEquals("a slow 40 dp drag must not queue", 0, queued)
        assertEquals("the row must spring back to rest", before, restLeft(), 0.5f)
    }

    @Test
    fun aDragPastTheCommitDistanceQueuesOnce() {
        row()
        rule.onNodeWithText(TITLE).performTouchInput {
            down(center)
            advanceEventTime(400)
            // Every drag eats the platform's own touch slop before it moves the row
            // (Compose hands it the over-slop only), so "past 60 dp" means 60 dp of
            // MOVEMENT: the injected finger travel is 60 dp + the device's slop.
            moveBy(Offset(60.dp.toPx() + viewConfiguration.touchSlop + 4f, 0f))
            up()
        }
        settle()
        assertEquals("a 60 dp movement is the commit distance", 1, queued)
    }

    /**
     * **A 70 dp drag follows the finger, and the release completes the action**
     * (T5, `plan-mobile-tests.md` §T5, bug 6).
     *
     * The case above proves the release queues; this one reads the row's position
     * WHILE the finger is down, one frame at a time, so a row that only moves at
     * the end — or teleports back on the release frame — fails at the frame it
     * breaks. The platform's slop is paid once, then two 35 dp steps must each
     * move the row by their own 35 dp (±2 dp); the release queues exactly once
     * and returns the row to rest.
     */
    @Test
    fun aSeventyDpDragFollowsTheFingerThenQueuesOnRelease() {
        row()
        val before = restLeft()
        rule.mainClock.autoAdvance = false
        rule.onNodeWithText(TITLE).performTouchInput {
            down(center)
            advanceEventTime(16)
            moveBy(Offset(viewConfiguration.touchSlop + 2f, 0f))
        }
        flushPointer()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val afterSlop = restLeft()
        rule.onNodeWithText(TITLE).performTouchInput {
            advanceEventTime(16)
            moveBy(Offset(35.dp.toPx(), 0f))
        }
        flushPointer()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val half = restLeft()
        rule.onNodeWithText(TITLE).performTouchInput {
            advanceEventTime(16)
            moveBy(Offset(35.dp.toPx(), 0f))
        }
        flushPointer()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val released = restLeft()
        println(
            "MEASURE g1 tracking: first 35 dp -> ${"%.1f".format(half - afterSlop)}dp, " +
                "70 dp -> ${"%.1f".format(released - before)}dp",
        )
        assertEquals("the row follows the finger on the first 35 dp", 35f, half - afterSlop, 2f)
        assertEquals("the row follows the finger on the 70 dp drag", 70f, released - before, 3f)

        // The release, in its own block: it is the distance (70 dp > 60 dp) that
        // decides, and the read above must happen while the finger is still down
        // (an `up` in that block would start the spring, and the first frame would
        // already be on its way back — measured: 36 dp instead of 70).
        rule.onNodeWithText(TITLE).performTouchInput { up() }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val afterRelease = restLeft()
        println(
            "MEASURE g1 release frame: ${"%.1f".format(afterRelease - before)}dp " +
                "after the up (released at ${"%.1f".format(released - before)}dp)",
        )
        assertEquals(
            "the release of a 70 dp drag queues exactly once",
            1,
            queued,
        )
        assertTrue(
            "the release frame must not teleport the row back to rest " +
                "(${"%.1f".format(afterRelease - before)}dp of " +
                "${"%.1f".format(released - before)}dp)",
            afterRelease - before > (released - before) * 0.75f,
        )
        rule.mainClock.autoAdvance = true
        settle()
        assertEquals("and the row springs back to rest", before, restLeft(), 0.5f)
    }

    @Test
    fun aShortFlickQueuesOnItsVelocity() {
        row()
        rule.onNodeWithText(TITLE).performTouchInput {
            down(center)
            // 30 dp in two moves ~16 ms apart ≈ 1 800 dp/s: under the 60 dp commit
            // distance (the slop is taken out) but over 600 dp/s. Two moves, not one:
            // Compose's VelocityTracker returns 0 below three samples, and a real
            // flick delivers dozens.
            moveBy(Offset(15.dp.toPx(), 0f))
            moveBy(Offset(15.dp.toPx(), 0f))
            up()
        }
        settle()
        assertEquals("a flick under the commit distance still queues", 1, queued)
    }

    @Test
    fun aLeftwardDragReachesTheEnclosingParent() {
        val scroll = ScrollState(0)
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                Box(Modifier.width(200.dp).horizontalScroll(scroll)) {
                    SwipeRow(onQueue = { queued++ }) { rowModifier ->
                        Row(rowModifier.height(56.dp).width(600.dp)) { Text(TITLE) }
                    }
                }
            }
        }
        val before = restLeft()
        rule.onNodeWithText(TITLE).performTouchInput {
            down(center)
            advanceEventTime(16)
            moveBy(Offset(-80.dp.toPx(), 0f))
            up()
        }
        settle()
        val after = restLeft()
        assertEquals("a leftward drag must never queue", 0, queued)
        assertTrue(
            "the enclosing horizontal parent must receive the drag (scroll=${scroll.value}px)",
            scroll.value > 0,
        )
        // The text moved only by the scroll, never by a translation of its own:
        // `before - after` equals the scroll offset. A translation would add to it.
        val movedBy = with(rule.density) { (before - after).dp.toPx() }
        assertEquals(
            "the row's own translation must stay 0",
            scroll.value.toFloat(),
            movedBy,
            2f,
        )
    }

    @Test
    fun aCancelledGestureDoesNotQueueAndComesBack() {
        row()
        val before = restLeft()
        rule.onNodeWithText(TITLE).performTouchInput {
            down(center)
            advanceEventTime(16)
            moveBy(Offset(40.dp.toPx(), 0f))
            cancel()
        }
        settle()
        assertEquals("a cancel is never a decision", 0, queued)
        assertEquals("a cancel springs back to rest", before, restLeft(), 0.5f)
    }

    private companion object {
        const val TITLE = "Titre de la piste"
    }
}
