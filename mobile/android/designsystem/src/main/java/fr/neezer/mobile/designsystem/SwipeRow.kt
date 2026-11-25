package fr.neezer.mobile.designsystem

import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.changedToUpIgnoreConsumed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** Past this distance (dp) the release has asked for the row's action. */
private val CommitDistance = 60.dp

/**
 * Past this speed (dp/s) the release has asked for it too, whatever the distance.
 *
 * It is the SPA's **0.6 px/ms** (`webapp.md`, `TrackRow.svelte`) said in the phone's
 * unit: `LocalDensity` turns it into the px/s the `VelocityTracker` reports.
 */
private val CommitVelocity = 600.dp

/** How far the row may follow the finger before it stops (75 % of its width). */
private const val MaxFollowFraction = 0.75f

/** The settle of a release, from WWDC 2018 (S4): 85 % damping keeps a little momentum. */
private const val SettleDamping = 0.85f

/**
 * One row that can be dragged to the right to ask for its action — « Lire
 * ensuite » on every list of the app.
 *
 * It is the ONE implementation: the design system's `TrackRowView`, the entity
 * pages' row, the library's row and the playlist's row each carried a copy of
 * the same body, and every copy **teleported back to 0 in one frame** on the
 * release (`drag = 0f` under no animation) and **consumed a leftward drag**, so
 * a parent pager (`L1`) could never receive a horizontal swipe that started on a
 * row.
 *
 * The three numbers and where they come from (plan section 3, `SwipeRow`):
 *
 *  * **60 dp or 600 dp/s** commits — the SPA's 64 px / 0.6 px/ms, and WWDC
 *    2018's *projection* of a release (S4): a short, fast flick must act;
 *  * **`spring(dampingRatio = 0.85, stiffness = Spring.StiffnessMediumLow)`**
 *    returns the row, seeded with the release velocity (`initialVelocity`), so
 *    the motion continues from the finger instead of restarting from a value;
 *  * **`spring(dampingRatio = 1, …)`** for a cancelled gesture — a cancel is
 *    never a decision (MDN, `pointercancel`, S8).
 *
 * A **leftward** drag is never consumed, and neither is a mostly vertical one:
 * consumption is what decides who owns a gesture (S5). The row only claims the
 * pointer once the platform touch slop is crossed **to the right**, so the tab
 * pager above a list keeps its swipe and the list keeps its scroll.
 *
 * Under `NzMotionLevel.Reduced` or `Off` (`Sillon.motion.moves == false`) a
 * settle lands at once, exactly as `transitions.md` M14 asks: no displacement,
 * no duration.
 *
 * **Why a `Float` state and not an `Animatable` (measured on this lot):** the
 * pointer-input block is a *restricted* suspend coroutine — it may not call a
 * suspending member such as `Animatable.snapTo`, and wrapping every move event
 * in `scope.launch { snapTo(…) }` would allocate a coroutine per event (and lag
 * the finger by a frame under load). The live drag is therefore one state write
 * per event, and the settle is ONE `animate(initialVelocity = …)` coroutine that
 * the next `down` cancels. Same curve, no allocation in the drag path.
 *
 * @param onQueue the action a committed swipe asks for (the caller queues the
 *   track); called once per committed gesture.
 * @param hint the words painted behind the row while it moves.
 * @param content the row itself. It receives the `Modifier` that carries the
 *   translation and the background and must apply it **first**, so the whole row
 *   (its padding included) moves as one node.
 */
@Composable
fun SwipeRow(
    onQueue: () -> Unit,
    modifier: Modifier = Modifier,
    hint: String = "Lire ensuite",
    enabled: Boolean = true,
    content: @Composable (Modifier) -> Unit,
) {
    val colors = Sillon.colors
    var offset by remember { mutableFloatStateOf(0f) }
    val settleJob = remember { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    val tracker = remember { VelocityTracker() }
    val haptics = LocalHapticFeedback.current
    val density = LocalDensity.current
    val moves = Sillon.motion.moves
    val commitPx = with(density) { CommitDistance.toPx() }
    val commitVelocityPx = with(density) { CommitVelocity.toPx() }

    // One coroutine per gesture, cancelled by the next `down`: the return
    // continues from the finger's offset with the finger's velocity.
    fun settleToRest(velocity: Float, damping: Float) {
        settleJob.value?.cancel()
        if (!moves) {
            offset = 0f
            return
        }
        settleJob.value = scope.launch {
            animate(
                initialValue = offset,
                targetValue = 0f,
                initialVelocity = velocity,
                animationSpec = spring(
                    dampingRatio = damping,
                    stiffness = Spring.StiffnessMediumLow,
                    visibilityThreshold = 0.5f,
                ),
            ) { value, _ -> offset = value }
            offset = 0f
        }
    }

    Box(modifier = modifier.fillMaxWidth()) {
        Text(
            text = hint,
            style = Sillon.type.label,
            color = colors.acText,
            modifier = Modifier
                .align(Alignment.CenterStart)
                .padding(start = NzSpacing.l)
                .graphicsLayer { alpha = (offset / commitPx).coerceIn(0f, 1f) },
        )
        content(
            Modifier
                .fillMaxWidth()
                .graphicsLayer { translationX = offset }
                .background(colors.bg)
                .pointerInput(enabled) {
                    if (!enabled) return@pointerInput
                    awaitEachGesture {
                        // Read at GESTURE time: the pointer-input block starts before
                        // layout, and `size` then is 0 (measured: the follow was clamped
                        // to 0 px and no drag could ever commit).
                        val widthPx = size.width.toFloat()
                        val slop = viewConfiguration.touchSlop
                        val down = awaitFirstDown(requireUnconsumed = false)
                        tracker.resetTracking()
                        tracker.addPosition(down.uptimeMillis, down.position)
                        var claimed = false
                        var cancelled = false
                        var travelX = 0f
                        var travelY = 0f
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (change.changedToUpIgnoreConsumed()) {
                                tracker.addPosition(change.uptimeMillis, change.position)
                                break
                            }
                            if (!change.pressed) {
                                cancelled = true
                                break
                            }
                            val delta = change.positionChange()
                            // Fed from the first move, before the axis is even decided:
                            // a trailing-window tracker needs the travel that happened
                            // before the touch slop was crossed, or a flick under the
                            // commit distance reports no velocity at all (measured: two
                            // samples return 0 in `VelocityTracker1D`).
                            tracker.addPosition(change.uptimeMillis, change.position)
                            if (!claimed) {
                                travelX += delta.x
                                travelY += delta.y
                                // The platform's slop, never one of our own (S5).
                                if (abs(travelX) <= slop && abs(travelY) <= slop) continue
                                // A vertical drag is the list's; a leftward one is the
                                // pager's. Neither is ours: leave every change unconsumed.
                                if (abs(travelX) <= abs(travelY) || travelX <= 0f) break
                                claimed = true
                                settleJob.value?.cancel()
                                change.consume()
                                offset = (offset + travelX - slop)
                                    .coerceIn(0f, widthPx * MaxFollowFraction)
                                continue
                            }
                            change.consume()
                            offset = (offset + delta.x)
                                .coerceIn(0f, widthPx * MaxFollowFraction)
                        }
                        if (!claimed) return@awaitEachGesture
                        if (cancelled) {
                            settleToRest(velocity = 0f, damping = 1f)
                            return@awaitEachGesture
                        }
                        val vx = tracker.calculateVelocity().x.let { if (it.isNaN()) 0f else it }
                        if (offset > commitPx || vx > commitVelocityPx) {
                            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                            onQueue()
                        }
                        settleToRest(velocity = vx, damping = SettleDamping)
                    }
                },
        )
    }
}
