package fr.neezer.mobile.feature.podcasts

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.nz_ffi.EpisodeData

/**
 * The episode panel's two segments (lot X5b): the SAME rule as the two screens
 * whose body is a `LazyColumn`, applied to the one panel that is not one — and the
 * reason it keeps `NzSwap`.
 *
 * `EpisodePanelBody` is a `Column` inside `NzSheet`: its tab bodies (Chapitres ·
 * Transcription) are ordinary composables, not `LazyListScope` items, so the
 * decision note's D2 gives this panel the layout swap it already had — "les écrans
 * en `Column` ou `Box` gardent `NzSwap`", and X4's credits sheet is the same site.
 * What the pane adds nowhere here is a MODIFIER on each item: there are no items.
 *
 * The assertions are the seam's own, then: the segmented control and the episode's
 * title are the header and must not move, and at eco the new segment is there in
 * the first frame that shows it. A `sizeTransform` on the swap (which X0 forbids,
 * M10) or a swap wrapped around the whole panel would move the header, and these
 * two bounds are what say so.
 *
 * No clock sleep: `mainClock` is advanced explicitly (worker rule 6).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class EpisodePanelTabTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val episode: EpisodeData = episode(
        "ep1",
        title = "Le grand récit",
        chapters = 3,
        transcript = transcriptInfo("ready"),
    )

    /** The service the panel reads: every call it makes is counted here. */
    private val service = FakePodcastService()

    /**
     * One FRAME, composed and applied: `waitForIdle` returns before the
     * recomposition that sees the new state, and the write composes on the second
     * frame on this tree (measured in `TransitionTest`).
     */
    private fun twoFrames() {
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
    }

    /** The panel's header: the episode's title, the same node before and after. */
    private fun header() = rule.onNodeWithText("Le grand récit").getUnclippedBoundsInRoot()

    private fun host(level: NzMotionLevel) {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    EpisodePanelSheet(
                        episode = episode,
                        service = service,
                        chapters = chaptersOf("ep1"),
                        chaptersBusy = false,
                        chaptersFailure = null,
                        markers = emptyList(),
                        isCurrent = false,
                        positionS = 0.0,
                        admin = true,
                        onLoadChapters = {},
                        onSeek = {},
                        onResume = {},
                        onAddMarker = {},
                        onRemoveMarker = {},
                        onStartTranscript = {},
                        onStopTranscript = {},
                        onRestartTranscript = {},
                        onDeleteTranscript = {},
                        onOpenShow = {},
                        onDismiss = {},
                    )
                }
            }
        }
        rule.waitForIdle()
    }

    /**
     * At eco, tapping « Transcription » shows the transcript in the first frame that
     * shows the new segment, the chapters it replaced hold no node, and the header
     * is exactly where it was.
     */
    @Test
    fun aTabChangeLandsInTheSameFrameAtEco() {
        host(NzMotionLevel.Off)
        // The chapters are the panel's first segment: they are on screen, the
        // transcript's own field is not.
        rule.onNodeWithText("Chapitre 2 · Le village").assertExists()
        rule.onNodeWithContentDescription("Rechercher dans la transcription").assertDoesNotExist()
        val before = header()
        // The tab is a state write, not a read: the panel already asked for the
        // transcript when it opened, and switching segment asks for nothing.
        val requestsBefore = service.calls.size

        rule.mainClock.autoAdvance = false
        rule.onNodeWithText("Transcription").performClick()
        twoFrames()
        val frameZero = header()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithContentDescription("Rechercher dans la transcription").assertExists()
        rule.onAllNodesWithText("Chapitre 2 · Le village").assertCountEquals(0)
        assertEquals(
            "eco plays nothing: the header is settled at frame 0",
            before.top.value,
            frameZero.top.value,
            0f,
        )
        assertEquals(
            "a tab change adds no request",
            requestsBefore,
            service.calls.size,
        )
        println("MEASURE x5b panneau: requetes=${service.calls.size - requestsBefore}")
    }

    /**
     * At `Full`, the header is at the same top in the swap's own frame and in the
     * same box once it has landed, and a second tap on the segment already selected
     * adds no node and starts nothing (`NzSwap`'s `contentKey` is the segment).
     */
    @Test
    fun theTabChangeLeavesTheHeaderWhereItWasAtFull() {
        host(NzMotionLevel.Full)
        val before = header()

        rule.mainClock.autoAdvance = false
        rule.onNodeWithText("Transcription").performClick()
        twoFrames()
        val firstFrame = header()
        rule.mainClock.advanceTimeBy(300)
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()

        rule.onNodeWithContentDescription("Rechercher dans la transcription").assertExists()
        assertEquals(
            "the header does not move with the segment that replaces the body",
            before.top.value,
            firstFrame.top.value,
            0f,
        )
        val settled = header()
        assertEquals("the header's top is the same box", before.top.value, settled.top.value, 0f)
        assertEquals("the header's bottom is the same box", before.bottom.value, settled.bottom.value, 0f)
        println("MEASURE x5b panneau: header=${settled.top.value}..${settled.bottom.value}dp")

        // The same segment again: no second body, no start.
        rule.mainClock.autoAdvance = false
        rule.onNodeWithText("Transcription").performClick()
        twoFrames()
        rule.mainClock.autoAdvance = true
        rule.waitForIdle()
        rule.onAllNodesWithText("Chapitre 2 · Le village").assertCountEquals(0)
        assertEquals(
            "the header did not move on a re-emit",
            before.top.value,
            header().top.value,
            0f,
        )
    }
}
