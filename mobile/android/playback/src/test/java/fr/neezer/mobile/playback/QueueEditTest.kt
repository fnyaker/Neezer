// :playback — the two translations that are not field-for-field
// (docs/agents/mobile.md, "MB3c"). JVM: a wrong pair is a listener whose skip
// lands on the wrong title.
package fr.neezer.mobile.playback

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.nz_ffi.Repeat

class QueueEditTest {

    /** `nz-player`'s cycle is off → all → one → off, NOT the enum's order. */
    @Test
    fun the_repeat_cycle_is_the_players_own() {
        assertEquals(0, QueueEdit.repeatCycles(Repeat.OFF, Repeat.OFF))
        assertEquals(1, QueueEdit.repeatCycles(Repeat.OFF, Repeat.ALL))
        assertEquals(1, QueueEdit.repeatCycles(Repeat.ALL, Repeat.ONE))
        assertEquals(1, QueueEdit.repeatCycles(Repeat.ONE, Repeat.OFF))
        assertEquals(2, QueueEdit.repeatCycles(Repeat.OFF, Repeat.ONE))
        assertEquals(2, QueueEdit.repeatCycles(Repeat.ALL, Repeat.OFF))
    }

    /** What Media3's range move means, computed independently of our pairs. */
    private fun rangeMove(items: List<String>, from: Int, to: Int, count: Int): List<String> {
        val result = items.toMutableList()
        val moved = result.subList(from, from + count).toList()
        repeat(count) { result.removeAt(from) }
        val insertAt = if (to < from) to else to - count
        result.addAll(insertAt, moved)
        return result
    }

    private fun singleMove(items: List<String>, from: Int, to: Int): List<String> {
        val result = items.toMutableList()
        val item = result.removeAt(from)
        result.add(to, item)
        return result
    }

    private fun assertRangeMove(items: List<String>, from: Int, to: Int, count: Int) {
        val expected = rangeMove(items, from, to, count)
        var actual = items
        for ((source, target) in QueueEdit.moveRange(from, to, count)) {
            actual = singleMove(actual, source, target)
        }
        assertEquals("move($from, $to, $count) from $items", expected, actual)
    }

    @Test
    fun a_backwards_range_move_is_the_range_in_ascending_order() {
        assertEquals(listOf(2 to 0, 3 to 1), QueueEdit.moveRange(2, 0, 2))
        assertRangeMove(listOf("A", "B", "C", "D"), 2, 0, 2)
        assertRangeMove(listOf("A", "B", "C", "D"), 3, 1, 1)
    }

    @Test
    fun a_forwards_range_move_is_the_range_in_descending_order() {
        assertEquals(listOf(1 to 2, 0 to 1), QueueEdit.moveRange(0, 3, 2))
        assertRangeMove(listOf("A", "B", "C", "D"), 0, 3, 2)
        assertRangeMove(listOf("A", "B", "C", "D"), 1, 3, 1)
    }

    @Test
    fun a_move_that_changes_nothing_is_no_command() {
        assertEquals(emptyList<Pair<Int, Int>>(), QueueEdit.moveRange(1, 1, 2))
        assertEquals(emptyList<Pair<Int, Int>>(), QueueEdit.moveRange(0, 2, 2))
        assertEquals(emptyList<Pair<Int, Int>>(), QueueEdit.moveRange(0, 3, 0))
    }

    /** A range removal runs from the END: every removal shifts the rest down. */
    @Test
    fun a_range_removal_runs_from_the_end() {
        assertEquals(listOf(5, 4, 3), QueueEdit.removeRange(3, 6))
        assertEquals(emptyList<Int>(), QueueEdit.removeRange(4, 4))
        assertEquals(emptyList<Int>(), QueueEdit.removeRange(6, 3))
    }
}
