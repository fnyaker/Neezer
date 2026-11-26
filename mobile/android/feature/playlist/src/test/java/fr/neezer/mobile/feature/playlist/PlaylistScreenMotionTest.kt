package fr.neezer.mobile.feature.playlist

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The playlist's rows MOVE (lot X5): a row removed from the 200-row list plays a
 * 200 ms placement of every row below it, at Full, and nothing at eco.
 *
 * Same measured facts as the queue's (`QueueSheetMotionTest`): a LazyList
 * placement is a layout offset, so `getUnclippedBoundsInRoot` reads it — the
 * neighbour's top has to pass strictly between its two settled positions. The
 * curve stays `itemSpecs()`'s and `MotionTest` holds it; this file holds the
 * wiring. The drag-reorder's gesture (G1's `SwipeRow`, the long-press handles) is
 * untouched and not asserted here, as the lot's scope says.
 *
 * No clock sleep: `mainClock` is advanced explicitly (worker rule 6).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class PlaylistScreenMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** Edit mode's row height: the 76 dp `EditRow` the ✕ lives in. */
    private val editRowDp = 76f

    private fun rows(count: Int = 200): FakeRows = FakeRows(PlaylistFixtures.tracks(count))

    private fun state(
        rows: FakeRows,
        wake: Long = 0,
        editing: Boolean = true,
    ): PlaylistModel.State {
        val page = PlaylistFixtures.page(rows = rows)
        return PlaylistModel.State(
            id = "pl-1",
            page = page,
            rows = page.rows,
            wake = wake,
            editing = editing,
        )
    }

    private fun topOf(title: String): Float =
        rule.onNodeWithText(title).getUnclippedBoundsInRoot().top.value

    /**
     * The screen with a person's remove wired the way the model does it: the row
     * leaves the source, the state re-emits with a fresh `wake` so the window
     * re-reads its pages, and the screen re-lays out.
     */
    private fun host(
        rows: FakeRows,
        level: NzMotionLevel,
    ) {
        var current by mutableStateOf(state(rows))
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    PlaylistScreen(
                        state = current,
                        actions = PlaylistActions(
                            remove = { index ->
                                val left = rows.all.toMutableList().also { it.removeAt(index) }
                                rows.replace(left)
                                current = current.copy(wake = current.wake + 1)
                            },
                        ),
                        wide = false,
                    )
                }
            }
        }
    }

    @Test
    fun aRemovalMovesTheRowBelowOverThePlacementAtFull() {
        val rows = rows()
        host(rows, NzMotionLevel.Full)
        rule.waitForIdle()

        rule.mainClock.autoAdvance = false
        val settledBefore = topOf("Titre 4")
        rule.onNodeWithContentDescription("Retirer Titre 3 de la playlist").performClick()
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(1)
        val atFrameZero = topOf("Titre 4")

        rule.mainClock.advanceTimeBy(100)
        val midway = topOf("Titre 4")
        rule.mainClock.advanceTimeBy(400)
        val settledAfter = topOf("Titre 4")
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Titre 3").assertDoesNotExist()
        assertEquals(
            "a row's removal lifts the rows below by one row",
            settledBefore - editRowDp,
            settledAfter,
            1f,
        )
        assertEquals(
            "the first frame is still where the rows were (no teleport)",
            settledBefore,
            atFrameZero,
            1f,
        )
        assertTrue(
            "at 100 ms the row is strictly between $settledAfter and $settledBefore, was $midway",
            midway > settledAfter && midway < settledBefore,
        )
    }

    @Test
    fun aRemovalLandsInTheSameFrameAtEco() {
        val rows = rows()
        host(rows, NzMotionLevel.Off)
        rule.waitForIdle()

        rule.mainClock.autoAdvance = false
        val settledBefore = topOf("Titre 4")
        rule.onNodeWithContentDescription("Retirer Titre 3 de la playlist").performClick()
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(1)
        val atFrameZero = topOf("Titre 4")
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Titre 3").assertDoesNotExist()
        assertEquals(
            "eco plays nothing: the row is settled at frame 0",
            settledBefore - editRowDp,
            atFrameZero,
            1f,
        )
    }
}
