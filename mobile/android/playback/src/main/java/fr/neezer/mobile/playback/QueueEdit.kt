// :playback — the queue edits a Media3 controller can ask for, translated into
// the commands `nz-core` actually has (docs/agents/mobile.md, "MB3c": "commands
// go to Rust", "the facade holds no logic").
package fr.neezer.mobile.playback

import uniffi.nz_ffi.Repeat

/**
 * The two translations that are NOT a field-for-field mapping, kept pure so they
 * are tested on the JVM instead of discovered by a listener whose skip landed on
 * the wrong title.
 */
object QueueEdit {
    /** `nz-player`'s cycle (`state.rs`: "off → all → one → off"). */
    private val REPEAT_ORDER = listOf(Repeat.OFF, Repeat.ALL, Repeat.ONE)

    /**
     * How many `CycleRepeat` commands take the player from [from] to [to]. The
     * facade cannot loop until it sees the snapshot change — the snapshot arrives
     * on another thread — so it counts the steps itself.
     */
    fun repeatCycles(from: Repeat, to: Repeat): Int {
        val a = REPEAT_ORDER.indexOf(from)
        val b = REPEAT_ORDER.indexOf(to)
        if (a < 0 || b < 0) return 0
        return (b - a + REPEAT_ORDER.size) % REPEAT_ORDER.size
    }

    /**
     * The single-title `Move`s that reproduce Media3's range move.
     *
     * `Player.moveMediaItems(from, to, count)` moves the range `[from, from +
     * count)` to `to`, with **pre-move** indices. `nz-player` moves ONE title at
     * a time, so the range becomes a sequence:
     *
     * - a move BACKWARDS is the same range in ascending order (`move(2, 0, 2)` on
     *   A B C D → `C D A B` is `(2→0)` then `(3→1)`);
     * - a move FORWARDS is the range in DESCENDING order, each title landing at
     *   `to - count + i` (`move(0, 3, 2)` on A B C D → `C A B D` is `(1→2)` then
     *   `(0→1)`).
     *
     * Both cases are asserted in `QueueEditTest`, including the identity move.
     */
    fun moveRange(from: Int, to: Int, count: Int): List<Pair<Int, Int>> {
        if (count <= 0 || from == to) return emptyList()
        val pairs =
            if (to < from) {
                (0 until count).map { from + it to to + it }
            } else {
                (count - 1 downTo 0).map { from + it to (to - count + it) }
            }
        return pairs.filter { (source, target) -> source != target && source >= 0 && target >= 0 }
    }

    /**
     * The order a range removal must be issued in: from the END, because every
     * removal shifts the indices after it.
     */
    fun removeRange(from: Int, to: Int): List<Int> {
        if (to <= from) return emptyList()
        return (to - 1 downTo from).toList()
    }
}
