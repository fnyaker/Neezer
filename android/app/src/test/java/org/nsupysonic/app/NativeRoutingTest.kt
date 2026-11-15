package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The app's own decisions while its activity is stopped (NativeRouting.kt): who
 * answers a media button, what a title that cannot be played costs, whether the
 * position tick is worth a frozen renderer's queue, and when the downloads yield
 * the link. Each of these was a phone bug that no foreground test could see.
 */
class NativeRoutingTest {

    // --- play / pause / seek from the notification, headset, lock screen, car ---

    @Test
    fun whileTheActivityIsShownEveryCommandStaysThePages() {
        for (cmd in listOf("play", "pause", "seek")) {
            assertEquals(cmd, TransportTarget.PAGE, NativeRouting.transport(true, true, cmd))
        }
    }

    @Test
    fun whileItIsStoppedTheAppAnswersPlayPauseAndSeek() {
        for (cmd in listOf("play", "pause", "seek")) {
            assertEquals(cmd, TransportTarget.APP, NativeRouting.transport(false, true, cmd))
        }
    }

    @Test
    fun noAppPlayerMeansThePageStillGetsTheCommand() {
        // The setting is off (or an older SPA plays): no slot exists, and the
        // page is then the only one that can apply anything.
        for (cmd in listOf("play", "pause", "seek")) {
            assertEquals(cmd, TransportTarget.PAGE, NativeRouting.transport(false, false, cmd))
        }
    }

    @Test
    fun nextAndPreviousKeepTheirOwnRoute() {
        // They carry a list and a trail (NativeAudio.skip), not a player state.
        for (cmd in listOf("next", "prev", "toggle", "stop")) {
            assertEquals(cmd, TransportTarget.PAGE, NativeRouting.transport(false, true, cmd))
        }
    }

    // --- a title that cannot be played ---

    @Test
    fun aVerdictOnOneTitleCostsThatTitleNotTheQueue() {
        // The reported failure: 404 on track 5 of 20, screen locked. The app must
        // consume it and start track 6, not stall the whole named list.
        assertEquals(
            FaultAction.ADVANCE,
            NativeRouting.fault(
                foreground = false, retryable = false, retriesLeft = true,
                hasNext = true, repeatOne = false, skipsLeft = true,
            ),
        )
    }

    @Test
    fun anEmptyListIsStillTheEndOfTheList() {
        assertEquals(
            FaultAction.STALL,
            NativeRouting.fault(false, retryable = false, retriesLeft = true, hasNext = false, repeatOne = false, skipsLeft = true),
        )
    }

    @Test
    fun inFrontOfThePageThePageDecides() {
        assertEquals(
            FaultAction.PAGE,
            NativeRouting.fault(true, retryable = false, retriesLeft = true, hasNext = true, repeatOne = false, skipsLeft = true),
        )
        // ...a retryable one included: the page's own ladder is the one that knows.
        assertEquals(
            FaultAction.PAGE,
            NativeRouting.fault(true, retryable = true, retriesLeft = true, hasNext = true, repeatOne = false, skipsLeft = true),
        )
    }

    @Test
    fun repeatOneDoesNotLoopAFailedTitleForever() {
        assertEquals(
            FaultAction.STALL,
            NativeRouting.fault(false, retryable = false, retriesLeft = true, hasNext = true, repeatOne = true, skipsLeft = true),
        )
    }

    @Test
    fun aBadMomentIsRetriedBeforeAnythingIsSkipped() {
        // A 5xx, a dropped connection: reopening the same source is right, and it
        // must never be read as a verdict about the title.
        assertEquals(
            FaultAction.RETRY,
            NativeRouting.fault(false, retryable = true, retriesLeft = true, hasNext = true, repeatOne = false, skipsLeft = true),
        )
        // Its budget spent, the session says "paused" rather than burning the list.
        assertEquals(
            FaultAction.STALL,
            NativeRouting.fault(false, retryable = true, retriesLeft = false, hasNext = true, repeatOne = false, skipsLeft = true),
        )
    }

    @Test
    fun aRunOfFailuresStopsInsteadOfBurningTheList() {
        // A server answering 404 to everything must not walk 200 titles in a
        // second: the skips are bounded, then the session pauses.
        assertEquals(
            FaultAction.STALL,
            NativeRouting.fault(false, retryable = false, retriesLeft = true, hasNext = true, repeatOne = false, skipsLeft = false),
        )
    }

    // --- the 4 Hz tick ---

    @Test
    fun theTickOnlyGoesToAPageThatCanRunIt() {
        assertTrue(NativeRouting.tickToPage(true))
        // Behind a locked screen it would queue up in the renderer, one JSON
        // object per 250 ms, until the phone is unlocked.
        assertFalse(NativeRouting.tickToPage(false))
    }

    // --- the link, for the downloads' gate ---

    @Test
    fun aStreamWithLittleInHandNeedsTheLink() {
        assertTrue(LinkNeed.wanted(active = true, network = true, aheadS = 0.0, held = false))
        assertTrue(LinkNeed.wanted(active = true, network = true, aheadS = 14.9, held = false))
        assertFalse(LinkNeed.wanted(active = true, network = true, aheadS = 15.0, held = false))
    }

    @Test
    fun theHysteresisKeepsItFromFlapping() {
        // Holding, it only lets go at 30 s; 20 s ahead stays held.
        assertTrue(LinkNeed.wanted(active = true, network = true, aheadS = 20.0, held = true))
        assertFalse(LinkNeed.wanted(active = true, network = true, aheadS = 30.0, held = true))
    }

    @Test
    fun nothingToShareWhenPausedOrLocal() {
        assertFalse(LinkNeed.wanted(active = false, network = true, aheadS = 0.0, held = true))
        assertFalse(LinkNeed.wanted(active = true, network = false, aheadS = 0.0, held = true))
        // No buffered range reported yet: the player is about to read, so it holds.
        assertTrue(LinkNeed.wanted(active = true, network = true, aheadS = Double.NaN, held = false))
    }
}
