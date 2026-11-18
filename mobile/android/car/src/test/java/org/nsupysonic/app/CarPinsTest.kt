package org.nsupysonic.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The pin ceiling behind `CarPrefs.pin` and the SPA's `carPin`.
 *
 * The SPA's pin action is reachable from any page of the same origin and the
 * whole list is rewritten into SharedPreferences on every pin: without a cap a
 * page could grow it without limit. 24 pins; an existing id can always be
 * refreshed; a 25th new one is refused (carPin then answers false).
 */
class CarPinsTest {

    private fun pin(i: Int) = PinnedPlaylist("id-$i", "Playlist $i", i)

    private fun fullList(): List<PinnedPlaylist> = (1..24).map(::pin)

    @Test
    fun twentyFourPinsAreAccepted() {
        var list = emptyList<PinnedPlaylist>()
        for (i in 1..24) {
            val next = CarPins.add(list, pin(i))
            assertNotNull("pin $i must be accepted", next)
            list = next!!
        }
        assertEquals(24, list.size)
        assertEquals(24, CarPins.MAX_PINS)
        assertEquals(list.map { it.id }, (1..24).map { "id-$it" })
    }

    @Test
    fun theTwentyFifthPinIsRefused() {
        val full = fullList()
        assertNull(CarPins.add(full, pin(25)))
        assertEquals("a refused pin must not touch the list", 24, full.size)
        assertFalse(full.any { it.id == "id-25" })
    }

    @Test
    fun anExistingPinIsStillAcceptedAtTheCeiling() {
        val full = fullList()
        val updated = CarPins.add(full, PinnedPlaylist("id-3", "Nouveau titre", 42))
        assertNotNull("an existing id is always accepted", updated)
        assertEquals("the counter must stay at 24", 24, updated!!.size)
        assertEquals("the rank is kept", "id-3", updated[2].id)
        assertEquals("Nouveau titre", updated[2].title)
        assertEquals(42, updated[2].nbTracks)
        assertEquals(24, updated.map { it.id }.distinct().size)
    }

    @Test
    fun rePinningTheSameIdDoesNotGrowTheList() {
        var list = CarPins.add(emptyList(), pin(1))!!
        repeat(30) {
            list = CarPins.add(list, pin(1)) ?: list
        }
        assertEquals(1, list.size)
        assertEquals("id-1", list.single().id)
    }

    @Test
    fun theOrderIsKeptWhenAnExistingPinIsUpdated() {
        val full = fullList()
        val updated = CarPins.add(full, pin(7))!!
        assertEquals(full.map { it.id }, updated.map { it.id })
        assertEquals(24, updated.size)
    }

    @Test
    fun removingOneMakesRoomForExactlyOneMore() {
        val full = fullList()
        val room = full.filterNot { it.id == "id-24" }
        val accepted = CarPins.add(room, pin(25))
        assertNotNull(accepted)
        assertEquals(24, accepted!!.size)
        assertEquals("id-25", accepted.last().id)
        assertTrue(accepted.map { it.id }.containsAll(room.map { it.id }))
    }

    @Test
    fun aBlankIdIsNeverStored() {
        assertNull(CarPins.add(emptyList(), PinnedPlaylist("", "Sans id", 1)))
        assertNull(CarPins.add(emptyList(), PinnedPlaylist("   ", "Blanc", 1)))
    }
}
