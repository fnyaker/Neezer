package fr.neezer.mobile.feature.nowplaying

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerCommand

/**
 * The queue's rows MOVE (lot X5): a removal is a 200 ms displacement of every row
 * below it, at Full, and nothing at all at eco.
 *
 * **How a Compose test sees a move, and the honest limit.** A LazyList placement
 * is a LAYOUT offset, so `getUnclippedBoundsInRoot` reads it (unlike a
 * `graphicsLayer` translation, which it cannot see — X0's note). What is asserted
 * is therefore the GEOMETRY the person sees: the neighbour's top has to pass
 * strictly between the two settled positions. The curve itself is `itemSpecs()`'s
 * and `MotionTest` holds it; this file holds the wiring — that the spec is on the
 * row, that the key survives the removal, and that eco really plays nothing.
 *
 * No clock sleep: `mainClock` is advanced explicitly (worker rule 6).
 *
 * Measured on this class (emulator-5660 numbers are X5's PR body): 4 000-row
 * queue, one remove → 1–3 layouts over the animation, p95 ≤ 16.7 ms, 0 requests.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class QueueSheetMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    /** The acceptance's worst realistic case, shortened to what one screen shows. */
    private fun port(rows: Int = 200): FakePlayerPort = FakePlayerPort(
        snapshot = FakePlayerPort.snapshot(
            current = FakePlayerPort.track(key = "9001", title = "Titre 0"),
            index = 0u,
            len = rows.toUInt(),
        ),
        queueRows = FakePlayerPort.queueOf(rows),
    )

    private fun model(port: FakePlayerPort): NowPlayingModel {
        val model = NowPlayingModel(
            player = port,
            service = FakeService(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        return model
    }

    private fun host(model: NowPlayingModel, level: NzMotionLevel) {
        rule.setContent {
            val state by model.state.collectAsState()
            NzTheme(kind = NzThemeKind.Encre, motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    QueueEditor(
                        state = state,
                        model = model,
                        nav = navigator,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
    }

    private fun topOf(title: String): Float =
        rule.onNodeWithText(title).getUnclippedBoundsInRoot().top.value

    /**
     * The person's remove: the ✕ on a row asks the engine, then the engine's
     * answer lands (Rust removed the entry and pushed a new queue version — the
     * real path the shell takes on `on_queue_changed`).
     */
    private fun remove(port: FakePlayerPort, model: NowPlayingModel, index: Int) {
        val before = port.commands.size
        rule.onNodeWithContentDescription("Retirer « Titre $index » de la file").performClick()
        assertEquals("the tap asks the engine to remove, once", before + 1, port.commands.size)
        assertTrue(
            "the command is a Remove of that entry",
            port.commands.last() is PlayerCommand.Remove,
        )
        port.queueRows = port.queueRows.filterIndexed { at, _ -> at != index }
        port.snapshot = port.snapshot.copy(len = port.queueRows.size.toUInt())
        model.onSnapshot(port.snapshot)
        model.onQueueChanged(1L)
    }

    @Test
    fun aRemovalMovesTheRowBelowOverThePlacementAtFull() {
        val port = port()
        val model = model(port)
        host(model, NzMotionLevel.Full)
        rule.waitForIdle()

        // The removal is the person's next act: from here the clock is ours.
        rule.mainClock.autoAdvance = false
        val settledBefore = topOf("Titre 4")
        remove(port, model, index = 3)
        rule.waitForIdle()
        // One frame of the NEW list: a LazyList's first placement is where the
        // displacement starts (or, at eco, where it already ends).
        rule.mainClock.advanceTimeBy(1)
        val atFrameZero = topOf("Titre 4")

        rule.mainClock.advanceTimeBy(100)
        val midway = topOf("Titre 4")
        rule.mainClock.advanceTimeBy(400)
        val settledAfter = topOf("Titre 4")
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Titre 3").assertDoesNotExist()
        assertTrue(
            "the row below moved UP: before $settledBefore, after $settledAfter",
            settledAfter < settledBefore,
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
        val port = port()
        val model = model(port)
        host(model, NzMotionLevel.Off)
        rule.waitForIdle()

        rule.mainClock.autoAdvance = false
        val settledBefore = topOf("Titre 4")
        remove(port, model, index = 3)
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(1)
        val atFrameZero = topOf("Titre 4")
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithText("Titre 3").assertDoesNotExist()
        assertEquals(
            "eco plays nothing: the row is settled at frame 0",
            settledBefore - ROW_HEIGHT_DP,
            atFrameZero,
            1f,
        )
    }

    @Test
    fun aMountMovesNoRow() {
        // The owner's own case: 4 000 rows, first composition.
        val port = port(rows = 4_000)
        val model = model(port)
        host(model, NzMotionLevel.Full)
        rule.mainClock.autoAdvance = false
        rule.waitForIdle()

        // Frame 0: every row is on its settled line — one row height apart.
        val tops = (0 until 6).map { topOf("Titre $it") }
        tops.zipWithNext().forEach { (above, below) ->
            assertEquals("a row is one height below the previous one", ROW_HEIGHT_DP, below - above, 1f)
        }

        // Nothing walks in: the same bounds a whole animation later.
        rule.mainClock.advanceTimeBy(300)
        val later = (0 until 6).map { topOf("Titre $it") }
        tops.zip(later).forEach { (frameZero, settled) ->
            assertEquals("a mounted row does not move (M4)", frameZero, settled, 0.5f)
        }
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()
    }

    private companion object {
        /** `--q-row-h`: the queue row's own height, 56 dp. */
        const val ROW_HEIGHT_DP = 56f
    }
}
