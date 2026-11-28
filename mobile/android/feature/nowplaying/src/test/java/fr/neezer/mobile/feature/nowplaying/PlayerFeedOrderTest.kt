package fr.neezer.mobile.feature.nowplaying

import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test

/**
 * Two Rust threads push snapshots; the one the screen keeps is the NEWEST by
 * `version`, whatever the order they arrive in (a late delivery used to put the
 * previous track back on screen until the next change).
 */
class PlayerFeedOrderTest {
    private val old = FakePlayerPort.snapshot(index = 1u).copy(version = 10uL)
    private val fresh = FakePlayerPort.snapshot(index = 2u).copy(version = 11uL)

    @Test
    fun a_late_older_snapshot_does_not_replace_the_newer_one() {
        assertSame(fresh, PlayerFeed.newer(fresh, old))
    }

    @Test
    fun a_newer_or_equal_snapshot_replaces_the_shown_one() {
        assertSame(fresh, PlayerFeed.newer(old, fresh))
        assertEquals(fresh.index, PlayerFeed.newer(fresh, fresh.copy()).index)
        assertSame(old, PlayerFeed.newer(null, old))
    }
}
