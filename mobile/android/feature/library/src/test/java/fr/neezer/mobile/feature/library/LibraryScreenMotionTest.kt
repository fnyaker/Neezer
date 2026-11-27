package fr.neezer.mobile.feature.library

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
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.SortKey
import uniffi.nz_ffi.TrackRow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The library's pane and its moving rows (lot X4).
 *
 * Two facts this file holds, and why each one is honest:
 *
 *  * **a leaf change replaces the CONTENT only, in one frame** — at eco the pane
 *    plays nothing, so what the tree contains right after the change is what a
 *    person sees: the new leaf's marker, no leftover of the old one, and the head
 *    and the strip of leaves where they were (an `AnimatedContent` around the
 *    `LazyColumn` would fade them too, which is the wall X4 hit);
 *  * **an un-heart moves the rows below it** — a `LazyList` placement is a layout
 *    offset, so `getUnclippedBoundsInRoot` reads it: the neighbour's top has to
 *    pass strictly between its two settled positions at 100 ms, and to land in
 *    one frame at eco.
 *
 * No clock sleep: `mainClock` is advanced explicitly (worker rule 6). The curve
 * itself is `itemSpecs()`'s and `TransitionTest` holds it; this file holds the
 * wiring.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class LibraryScreenMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** A row's height in the favourites table (`heightIn(min = 56.dp)`). */
    private val rowDp = 56f

    /** The state setter, so a test changes the screen's state from OUTSIDE it. */
    private var update: ((LibraryModel.State) -> Unit)? = null

    private fun favoritesState(rows: LibraryRows): LibraryModel.State = LibraryModel.State(
        leaf = LibraryLeaf.Favorites,
        favorites = ScreenState.Ready(rows),
        artists = ScreenState.Ready(LibraryFixtures.artists()),
        albums = ScreenState.Ready(LibraryFixtures.albums()),
        tree = ScreenState.Ready(LibraryFixtures.tree()),
        local = ScreenState.Ready(FakeRows(LibraryFixtures.rows(3))),
        unavailable = ScreenState.Ready(FakeRows(emptyList())),
        dislikes = ScreenState.Ready(FakeRows(emptyList())),
        usage = LibraryFixtures.usage(),
        foldersSupported = true,
    )

    private fun host(level: NzMotionLevel, rows: LibraryRows) {
        var current by mutableStateOf(favoritesState(rows))
        update = { next -> current = next }
        rule.setContent {
            NzTheme(motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    LibraryScreen(
                        state = current,
                        // The leaf's door is the strip's own callback: the test drives
                        // the screen's STATE, and the chip's scroll is the strip's
                        // existing `revealSelected`, untouched by this lot.
                        actions = LibraryActions(
                            selectLeaf = { leaf -> current = current.copy(leaf = leaf) },
                        ),
                    )
                }
            }
        }
    }

    /**
     * One FRAME, composed and applied. `waitForIdle` returns before the
     * recomposition that sees the new state, and the state write composes on the
     * second frame on this tree (measured in `TransitionTest`).
     */
    private fun frame() {
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
    }

    private fun twoFrames() {
        frame()
        frame()
    }

    @Test
    fun aLeafChangeReplacesTheContentInTheSameFrameAtEco() {
        host(NzMotionLevel.Off, FakeRows(LibraryFixtures.rows(12)))
        rule.waitForIdle()
        val headBefore = rule.onNodeWithTag(LIBRARY_HEAD_TAG).getUnclippedBoundsInRoot()
        val stripBefore = rule.onNodeWithTag(LIBRARY_LEAVES_TAG).getUnclippedBoundsInRoot()

        rule.mainClock.autoAdvance = false
        update!!(favoritesState(FakeRows(LibraryFixtures.rows(12))).copy(leaf = LibraryLeaf.Artists))
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        // The new leaf's grid, and no leftover of the favourites' table.
        rule.onNodeWithText("Zazie").assertExists()
        rule.onAllNodesWithText("Ouverture 0").assertCountEquals(0)
        rule.onAllNodesWithText("Ouverture 0", useUnmergedTree = true).assertCountEquals(0)

        val headAfter = rule.onNodeWithTag(LIBRARY_HEAD_TAG).getUnclippedBoundsInRoot()
        assertEquals("le titre et ses compteurs ne bougent pas", headBefore.top.value, headAfter.top.value, 0f)
        assertEquals(headBefore.bottom.value, headAfter.bottom.value, 0f)
        // The strip itself does not move: same place, same height. (A CHIP inside
        // it slides, because `LeafBar` scrolls the chosen leaf into view — the SPA's
        // `revealSelected`, existing behaviour this lot does not touch.)
        val stripAfter = rule.onNodeWithTag(LIBRARY_LEAVES_TAG).getUnclippedBoundsInRoot()
        assertEquals("la bande ne bouge pas", stripBefore.top.value, stripAfter.top.value, 0f)
        assertEquals(stripBefore.bottom.value, stripAfter.bottom.value, 0f)
        assertEquals(stripBefore.left.value, stripAfter.left.value, 0f)
        assertEquals(stripBefore.right.value, stripAfter.right.value, 0f)
        println(
            "MEASURE x4 feuille: head=${headBefore.top.value}..${headAfter.bottom.value}dp " +
                "strip=${stripBefore.top.value}..${stripAfter.bottom.value}dp",
        )
    }

    @Test
    fun aReEmitOfTheSameLeafAddsNoNode() {
        host(NzMotionLevel.Full, FakeRows(LibraryFixtures.rows(12)))
        rule.waitForIdle()
        rule.onAllNodesWithText("Ouverture 0").assertCountEquals(1)

        rule.mainClock.autoAdvance = false
        update!!(favoritesState(FakeRows(LibraryFixtures.rows(12))))
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        // M6: the same pane re-emitted plays nothing and leaves ONE row behind.
        rule.onAllNodesWithText("Ouverture 0").assertCountEquals(1)
    }

    @Test
    fun anUnheartMovesTheRowsBelowOverThePlacementAtFull() {
        val rows = MutableRows(LibraryFixtures.rows(60))
        host(NzMotionLevel.Full, rows)
        rule.waitForIdle()

        rule.mainClock.autoAdvance = false
        val settledBefore = topOf("Sinnerman 3")
        update!!(favoritesState(rows.without(2)))
        twoFrames()
        rule.mainClock.advanceTimeBy(100)
        val midway = topOf("Sinnerman 3")
        rule.mainClock.advanceTimeBy(400)
        val settledAfter = topOf("Sinnerman 3")
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onAllNodesWithText("Ouverture 2").assertCountEquals(0)
        assertEquals(
            "the row below a removal rises by exactly one row",
            settledBefore - rowDp,
            settledAfter,
            1f,
        )
        assertTrue(
            "at 100 ms the row is strictly between $settledAfter and $settledBefore, was $midway",
            midway > settledAfter && midway < settledBefore,
        )
        println("MEASURE x4 defavori: before=$settledBefore midway=$midway after=$settledAfter dp")
    }

    @Test
    fun anUnheartLandsInOneFrameAtEco() {
        val rows = MutableRows(LibraryFixtures.rows(60))
        host(NzMotionLevel.Off, rows)
        rule.waitForIdle()

        rule.mainClock.autoAdvance = false
        val settledBefore = topOf("Sinnerman 3")
        update!!(favoritesState(rows.without(2)))
        twoFrames()
        val atFrameZero = topOf("Sinnerman 3")
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        assertEquals(
            "eco plays nothing: the row is settled at frame 0",
            settledBefore - rowDp,
            atFrameZero,
            1f,
        )
    }

    private fun topOf(title: String): Float =
        rule.onNode(hasText(title) and hasClickAction()).getUnclippedBoundsInRoot().top.value

    /**
     * The favourites' rows. A removal returns a NEW instance because that is what a
     * re-read does: a `ListHandle` is immutable by contract, so the screen gets a
     * new `LibraryRows`, the window is rebuilt, and only the KEYS keep the rows'
     * identity — which is exactly what `animateItem` needs to move them.
     */
    private class MutableRows(private val items: List<TrackRow>) : LibraryRows {

        override val handle: ListHandle? = null

        override fun len(): Int = items.size

        override fun window(start: Int, count: Int): List<TrackRow> {
            if (start >= items.size || count <= 0) return emptyList()
            return items.subList(start, minOf(start + count, items.size))
        }

        override fun filter(query: String): Int = items.size

        override fun sort(by: SortKey, descending: Boolean): Int = items.size

        override fun version(): ULong = 1uL

        fun without(index: Int): MutableRows =
            MutableRows(items.toMutableList().also { it.removeAt(index) })
    }
}
