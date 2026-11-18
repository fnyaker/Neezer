// :playback — the two pure decisions of the Auto tree, on the JVM (docs/agents/
// mobile.md, worker rule 6: an event, not a clock; and "Kotlin, JVM:
// `gradle testDebugUnitTest` (pure policy classes)").
//
// What is asserted here is the ONLY thing Kotlin decides about the tree: which
// media ids name a node a tap plays, and how one page of children is sliced.
// Everything else — the five nodes, their order, their cost, what a tap queues —
// is Rust's and is measured in `nz-core/src/auto/tests.rs` against a real HTTP
// server.
package fr.neezer.mobile.playback.auto

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AutoNodeTest {
    /**
     * The ids are `nz-core/src/auto/mod.rs`'s constants. Kotlin cannot read
     * Rust's constants, so the two lists are written twice on purpose and this
     * test is the seam: if a lot renames a node, one test on each side fails
     * rather than the car showing an empty screen with no cause.
     */
    @Test
    fun the_ids_are_the_ones_the_rust_tree_builds() {
        assertEquals("neezer:root", AutoNode.ROOT)
        assertEquals("neezer:favorites", AutoNode.FAVORITES)
        assertEquals("neezer:playlists", AutoNode.PLAYLISTS)
        assertEquals("neezer:flow", AutoNode.FLOW)
        assertEquals("neezer:recent", AutoNode.RECENT)
        assertEquals("neezer:downloads", AutoNode.DOWNLOADS)
        assertEquals("neezer:playlist:", AutoNode.PLAYLIST_PREFIX)
        assertEquals("neezer:track:", AutoNode.TRACK_PREFIX)
    }

    @Test
    fun every_section_is_recognised() {
        assertEquals(AutoNodeKind.Root, AutoNode.kind(AutoNode.ROOT))
        assertEquals(AutoNodeKind.Favorites, AutoNode.kind(AutoNode.FAVORITES))
        assertEquals(AutoNodeKind.Playlists, AutoNode.kind(AutoNode.PLAYLISTS))
        assertEquals(AutoNodeKind.Flow, AutoNode.kind(AutoNode.FLOW))
        assertEquals(AutoNodeKind.Recent, AutoNode.kind(AutoNode.RECENT))
        assertEquals(AutoNodeKind.Downloads, AutoNode.kind(AutoNode.DOWNLOADS))
        val playlist = "neezer:playlist:aaaaaaaa-1111-4111-8111-000000000001"
        assertEquals(AutoNodeKind.Playlist, AutoNode.kind(playlist))
        assertEquals("aaaaaaaa-1111-4111-8111-000000000001", AutoNode.argument(playlist))
        assertEquals(AutoNodeKind.Track, AutoNode.kind("neezer:track:deezer:123"))
        assertEquals("deezer:123", AutoNode.argument("neezer:track:deezer:123"))
    }

    /**
     * A tap on one of these queues what the node showed; a tap on a title, the
     * root, the playlist folder or anything hostile keeps Media3's default path.
     */
    @Test
    fun only_the_playable_nodes_are_containers() {
        for (container in
            listOf(
                AutoNode.FAVORITES,
                "neezer:playlist:aaaaaaaa-1111-4111-8111-000000000001",
                AutoNode.FLOW,
                AutoNode.RECENT,
                AutoNode.DOWNLOADS,
            )) {
            assertTrue(container, AutoNode.isContainer(container))
        }
        for (leaf in
            listOf(
                AutoNode.ROOT,
                AutoNode.PLAYLISTS,
                "neezer:track:11111111-1111-4111-8111-000000000001",
                "",
                "neezer:",
                "playlist:1",
            )) {
            assertFalse(leaf, AutoNode.isContainer(leaf))
        }
    }

    /**
     * A media id the tree did not build is never a node: it can carry no route,
     * no path and no query, exactly as `nz_core::auto::parse` refuses it.
     */
    @Test
    fun a_hostile_id_is_never_a_node() {
        for (hostile in
            listOf(
                "neezer:playlist:../etc/passwd",
                "neezer:playlist:aaaaaaaa/bbbb",
                "neezer:playlist:aaaaaaaa?x=1",
                "neezer:playlist:aaaaaaaa#frag",
                "neezer:playlist:",
                "neezer:track:a/b",
                "neezer:track:a\u0000b",
            )) {
            val kind = AutoNode.kind(hostile)
            assertTrue(hostile, kind == AutoNodeKind.Unknown)
            assertFalse(hostile, AutoNode.isContainer(hostile))
        }
        // A padded id is trimmed, as Rust trims it: the two sides read the same
        // string the same way.
        assertEquals(AutoNodeKind.Favorites, AutoNode.kind(" neezer:favorites "))
    }

    /** One page is at most a hundred rows, whatever the controller asks for. */
    @Test
    fun a_page_is_at_most_a_hundred_rows() {
        val rows = (0 until 250).toList()
        assertEquals(100, AutoPage.slice(rows, 0, Int.MAX_VALUE).size)
        assertEquals(0, AutoPage.slice(rows, 0, Int.MAX_VALUE).first())
        assertEquals(100, AutoPage.slice(rows, 1, 100).first())
        assertEquals(50, AutoPage.slice(rows, 2, 100).size)
        assertEquals(0, AutoPage.slice(rows, 9, 100).size)
        assertEquals("une page nulle vaut une page pleine", 100, AutoPage.slice(rows, -3, 0).size)
        assertEquals(7, AutoPage.slice(rows, 0, 7).size)
        assertTrue(AutoPage.slice(emptyList<Int>(), 0, 100).isEmpty())
        assertEquals("250 demandés se ramènent à une page", 100, AutoPage.slice(rows, 0, 250).size)
    }
}
