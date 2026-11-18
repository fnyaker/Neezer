package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The card's position, pure (see [CarPlacement] and [CarPositions]).
 *
 * The prototype's numbers on the two reference screens, the clamp at every
 * edge, rotation, a slot saved on a wider phone, and the
 * (navigation app, size) storage key. No device was available: these are the
 * dp the WindowManager params are computed from.
 */
class CarPlacementTest {

    private val insets = CarInsets(leftDp = 0f, topDp = 24f, rightDp = 0f, bottomDp = 48f)

    // The FULL/MINI card height from CarPlayerView's own paddings and rows:
    // 14 top + 72 header + 14 + 92 controls + 14 + 64 screens + 16 bottom.
    private val fullHeightDp = 286f

    @Test
    fun fullAndMiniKeepThePrototypePlaceOnA411WideScreen() {
        val slot = CarPlacement.defaults(CarSize.FULL, 411f)
        assertEquals(8f, slot.xDp, 0f)
        assertEquals(92f, slot.bottomDp, 0f)
        assertEquals(slot, CarPlacement.defaults(CarSize.MINI, 411f))
        // The window is screen − 16 dp wide: 395 dp at 411, 344 at 360.
        assertEquals(395f, CarPlacement.widthDp(CarSize.FULL, 411f), 0f)
        assertEquals(344f, CarPlacement.widthDp(CarSize.MINI, 360f), 0f)
    }

    @Test
    fun ultraIsFourteenFromTheEndAnd104FromTheBottom() {
        val at411 = CarPlacement.defaults(CarSize.ULTRA, 411f)
        assertEquals(321f, at411.xDp, 0f)   // 411 − 14 − 76
        assertEquals(104f, at411.bottomDp, 0f)
        val at360 = CarPlacement.defaults(CarSize.ULTRA, 360f)
        assertEquals(270f, at360.xDp, 0f)   // 360 − 14 − 76
        assertEquals(76f, CarPlacement.widthDp(CarSize.ULTRA, 360f), 0f)
    }

    @Test
    fun theTwoReferenceScreensFitTheirDefaults() {
        for (width in listOf(411f, 360f)) {
            val height = if (width == 411f) 891f else 640f
            val full = CarPlacement.clamp(
                CarPlacement.defaults(CarSize.FULL, width),
                CarPlacement.widthDp(CarSize.FULL, width), fullHeightDp,
                width, height, insets,
            )
            assertEquals(8f, full.xDp, 0f)
            assertEquals(92f, full.bottomDp, 0f)
            val ultra = CarPlacement.clamp(
                CarPlacement.defaults(CarSize.ULTRA, width),
                CarPlacement.widthDp(CarSize.ULTRA, width), 76f,
                width, height, insets,
            )
            assertEquals(width - 90f, ultra.xDp, 0f)
            assertEquals(104f, ultra.bottomDp, 0f)
        }
    }

    @Test
    fun aDragIsClampedToTheStatusBarNavigationBarAndEdges() {
        // 411 × 891: FULL is 395 wide and 286 tall.
        val clamped = CarPlacement.clamp(
            CarSlot(-50f, 5_000f), 395f, fullHeightDp, 411f, 891f, insets
        )
        assertEquals(0f, clamped.xDp, 0f)        // never off the left edge
        assertEquals(581f, clamped.bottomDp, 0f) // 891 − 286 − 24 (status bar)
        val other = CarPlacement.clamp(
            CarSlot(9_999f, -10f), 395f, fullHeightDp, 411f, 891f, insets
        )
        assertEquals(16f, other.xDp, 0f)         // 411 − 395, the right edge
        assertEquals(48f, other.bottomDp, 0f)    // above the navigation bar
    }

    @Test
    fun ultraStaysInsideItsOwnSquare() {
        // 76 dp disc on 360 × 640, same safe area.
        val clamped = CarPlacement.clamp(
            CarSlot(9_999f, 9_999f), 76f, 76f, 360f, 640f, insets
        )
        assertEquals(284f, clamped.xDp, 0f)      // 360 − 76
        assertEquals(540f, clamped.bottomDp, 0f) // 640 − 76 − 24
        val left = CarPlacement.clamp(CarSlot(-1f, 104f), 76f, 76f, 360f, 640f, insets)
        assertEquals(0f, left.xDp, 0f)
    }

    @Test
    fun rotatingReclampsTheStoredSlot() {
        // Saved on a 411 × 891 portrait (ULTRA 321/104), then landscape:
        // 891 × 411, height 76, 24 top / 48 bottom.
        val landscape = CarPlacement.clamp(
            CarSlot(321f, 104f), 76f, 76f, 891f, 411f, insets
        )
        assertEquals(321f, landscape.xDp, 0f)   // still in bounds
        assertEquals(104f, landscape.bottomDp, 0f)
        // A FULL card that was dragged low keeps its bottom distance; the
        // bottom bound now sits at 411 − 286 − 24 = 101.
        val full = CarPlacement.clamp(
            CarSlot(8f, 581f), 395f, fullHeightDp, 891f, 411f, insets
        )
        assertEquals(101f, full.bottomDp, 0f)
    }

    @Test
    fun aSlotSavedOnAWiderPhoneIsPulledBack() {
        val on360 = CarPlacement.clamp(
            CarSlot(400f, 104f), 76f, 76f, 360f, 640f, insets
        )
        assertEquals(284f, on360.xDp, 0f)
    }

    @Test
    fun beforeTheFirstLayoutOnlyTheBottomLowerBoundIsEnforced() {
        // windowHeightDp = 0: the card's measured height is unknown, so the
        // bottom cannot be capped yet, but it cannot go under the nav bar.
        val slot = CarPlacement.clamp(
            CarSlot(8f, 1_000f), 395f, 0f, 411f, 891f, insets
        )
        assertEquals(867f, slot.bottomDp, 0f)   // 891 − 0 − 24
    }

    @Test
    fun theSameAppKeepsOneSlotPerSize() {
        val pkg = "com.google.android.apps.maps"
        val full = CarPositions.key(pkg, CarSize.FULL)
        val mini = CarPositions.key(pkg, CarSize.MINI)
        assertEquals(pkg, CarPositions.packageOf(full))
        assertEquals(pkg, CarPositions.packageOf(mini))
        assertTrue(full != mini)
        assertEquals(pkg to CarSize.ULTRA, CarPositions.parse(CarPositions.key(pkg, CarSize.ULTRA)))
        assertEquals(null to CarSize.ULTRA, CarPositions.parse(CarPositions.key(null, CarSize.ULTRA)))
    }

    @Test
    fun foreignAndMalformedKeysAreNotPositions() {
        assertFalse(CarPositions.isPositionKey("overlayAlpha"))
        assertFalse(CarPositions.isPositionKey("pins"))
        assertNull(CarPositions.parse("overlayAlpha"))
        assertNull(CarPositions.parse("position:com.waze"))
        assertNull(CarPositions.parse("position:com.waze:BOGUS"))
        assertNull(CarPositions.parse("position::"))
        assertTrue(CarPositions.isPositionKey("position::FULL"))
    }

    @Test
    fun aSlotRoundTripsThroughItsStoredString() {
        val slot = CarSlot(321.5f, 104.25f)
        assertEquals(slot, CarPositions.decode(CarPositions.encode(slot)))
        assertNull(CarPositions.decode(null))
        assertNull(CarPositions.decode(""))
        assertNull(CarPositions.decode("8"))
        assertNull(CarPositions.decode("x,92"))
        assertNull(CarPositions.decode("8,y"))
        assertNull(CarPositions.decode("8,92,3"))
        assertNull(CarPositions.decode("-1,92"))
        assertNull(CarPositions.decode("8,-1"))
        assertNull(CarPositions.decode("NaN,92"))
        assertNull(CarPositions.decode("8,Infinity"))
    }
}
