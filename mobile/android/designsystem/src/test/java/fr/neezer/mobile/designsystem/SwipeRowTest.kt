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
