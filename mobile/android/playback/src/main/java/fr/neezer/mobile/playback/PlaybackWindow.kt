// :playback — the playlist Media3 is allowed to see (docs/agents/mobile.md,
// "MB3c": "the playlist exposed as a WINDOW — previous 1 + current + next 100 —
// Media3 rebuilds the list on every state, 4 000 items would cost a frame each
// time").
package fr.neezer.mobile.playback

/**
 * The arithmetic of that window, and the only place an index crosses between
 * Media3's world and the queue's.
 *
 * Media3 calls `getState()` on every interaction (a seek, a volume change, the
 * notification updating) and `SimpleBasePlayer` re-derives a `Timeline` from
 * the playlist it is handed each time. Handing it the real 4 000-row queue
 * would allocate 4 000 `MediaItemData` per call — measured as a frame's worth of
 * work on this machine's emulator — for a list nobody can scroll in a
 * notification. 102 rows is what the car's list, the notification and the lock
 * screen can actually show around the current track.
 *
 * Everything here is pure so the mapping is tested on the JVM, not guessed on a
 * device: a wrong `+ start` is a skip that plays the wrong track.
 */
object PlaybackWindow {
    /** How many titles after the current one Media3 may see (MB3c's "next 100"). */
    const val AHEAD = 100

    /** How many titles before the current one it may see. */
    const val BEFORE = 1

    /** The largest playlist `getState()` ever builds. */
    const val SIZE = BEFORE + 1 + AHEAD

    /**
     * A contiguous slice of the queue: `count` rows from `start` in the queue's
     * own indices.
     */
    data class Slice(val start: Int, val count: Int) {
        val end: Int get() = start + count

        /** The queue index a Media3 playlist index points at. */
        fun realOf(windowIndex: Int): Int = start + windowIndex

        /** The Media3 playlist index of a queue index, or -1 when outside. */
        fun windowOf(realIndex: Int): Int =
            if (realIndex in start until end) realIndex - start else -1

        fun contains(realIndex: Int): Boolean = realIndex in start until end

        companion object {
            val EMPTY = Slice(0, 0)
        }
    }

    /**
     * The window that shows queue index [index] of a [len]-row queue.
     *
     * The current title sits at `min(BEFORE, index)` so the FIRST title of a
     * queue has no hole before it (Media3 requires the current index to be
     * inside the playlist it was handed), and the window never runs past the
     * queue's end.
     */
    fun slice(len: Int, index: Int?): Slice {
        if (len <= 0 || index == null) return Slice.EMPTY
        val at = index.coerceIn(0, len - 1)
        val start = (at - BEFORE).coerceAtLeast(0)
        val count = minOf(SIZE, len - start)
        return Slice(start, count)
    }

    /**
     * Whether a built window still shows [index]: the state is rebuilt only when
     * it does not, so a position update does not re-window 102 rows.
     */
    fun stillShows(slice: Slice, len: Int, index: Int?): Boolean =
        index != null && len > 0 && slice.contains(index) && slice.end <= len
}
