package fr.neezer.mobile.feature.podcasts

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
import androidx.compose.ui.test.onNodeWithText
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
import uniffi.nz_ffi.EpisodeMarkerData

/**
 * A MARK added to an episode moves the episodes below it (lot X5).
 *
 * The marks live INSIDE an episode's row (the SPA's `.mark` chips), so adding one
 * grows that row and pushes every row under it down. That push is the list's
 * placement, not a height walk: `Modifier.animateItem(itemSpecs())` on the
 * episodes' `items` makes the rows below travel the new distance over the 200 ms
 * standard — and at eco they land in the same frame.
 *
 * **What is asserted, and the honest limit** (X0's, repeated): a LazyList placement
 * is a LAYOUT offset, so `getUnclippedBoundsInRoot` reads it; it cannot read the
 * row's fade or its curve, which stay `itemSpecs()`'s and `MotionTest`'s. No clock
 * sleep: `mainClock` is advanced explicitly (worker rule 6).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class ShowScreenMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun cards(count: Int = 6): Map<Int, uniffi.nz_ffi.EpisodeData> =
        (0 until count).associateWith { index -> episode("ep$index", title = "Épisode $index") }

    private fun initialState(
        cards: Map<Int, uniffi.nz_ffi.EpisodeData> = cards(),
    ): ShowModel.State = ShowModel.State(
        id = "show-1",
        show = show("show-1", "Les Histoires", episodes = cards.size),
        loading = false,
        loaded = cards.size,
        cards = cards,
    )

    /** Every gesture the screen can ask for, none of them used by this test. */
    private fun actions() = ShowActions(
        retry = {},
        onVisible = { _, _, _ -> },
        loadMore = {},
        play = {},
        playAll = {},
        playAt = { _, _ -> },
        toggleAuto = {},
        retryTranscribe = {},
        toggleTranscribe = {},
        setLanguage = {},
        setBacklog = {},
        setBatchCount = {},
        startBatch = {},
        startOne = {},
        stopOne = {},
        setSearchQuery = {},
        retrySearch = {},
        clearSearch = {},
        toggleChapters = {},
        openSheet = {},
        addMarker = { _, _ -> },
        removeMarker = {},
        openEpisode = {},
        consumeMessage = {},
    )

    private fun topOf(title: String): Float =
        rule.onNodeWithText(title).getUnclippedBoundsInRoot().top.value

    private fun marker(): EpisodeMarkerData = EpisodeMarkerData(
        id = "m1",
        episodeId = "ep1",
        positionMs = 725_000u,
        seconds = 725u,
        timeLabel = "12:05",
        label = "Reprendre ici",
    )

    /** The episode list, over the state the test moves (`state` is the source). */
    private var state by mutableStateOf(initialState())

    private fun host(level: NzMotionLevel) {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ShowScreen(
                        state = state,
                        actions = actions(),
                        isAdmin = false,
                        currentKey = null,
                        playing = false,
                        positionS = 0.0,
                    )
                }
            }
        }
    }

    @Test
    fun aMarkPushesTheRowBelowOverThePlacementAtFull() {
        host(NzMotionLevel.Full)
        rule.waitForIdle()

        rule.mainClock.autoAdvance = false
        val settledBefore = topOf("Épisode 2")
        // The person's mark lands on the episode ABOVE: that row grows, the one
        // below travels the same distance an insertion would.
        state = state.copy(cards = state.cards + (1 to state.cards.getValue(1).copy(markers = listOf(marker()))))
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(1)
        val atFrameZero = topOf("Épisode 2")

        rule.mainClock.advanceTimeBy(100)
        val midway = topOf("Épisode 2")
        rule.mainClock.advanceTimeBy(400)
        val settledAfter = topOf("Épisode 2")
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        assertTrue(
            "the row below moved DOWN: before $settledBefore, after $settledAfter",
            settledAfter > settledBefore,
        )
        assertEquals(
            "the first frame is still where the rows were (no teleport)",
            settledBefore,
            atFrameZero,
            1f,
        )
        assertTrue(
            "at 100 ms the row is strictly between $settledBefore and $settledAfter, was $midway",
            midway > settledBefore && midway < settledAfter,
        )
    }

    @Test
    fun aMarkLandsInTheSameFrameAtEco() {
        host(NzMotionLevel.Off)
        rule.waitForIdle()

        rule.mainClock.autoAdvance = false
        val settledBefore = topOf("Épisode 2")
        state = state.copy(cards = state.cards + (1 to state.cards.getValue(1).copy(markers = listOf(marker()))))
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(1)
        val atFrameZero = topOf("Épisode 2")
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        val settledAfter = topOf("Épisode 2")
        assertTrue("the row below did move down at eco too", settledAfter > settledBefore)
        assertEquals(
            "eco plays nothing: the row is settled at frame 0",
            settledAfter,
            atFrameZero,
            1f,
        )
    }
}
