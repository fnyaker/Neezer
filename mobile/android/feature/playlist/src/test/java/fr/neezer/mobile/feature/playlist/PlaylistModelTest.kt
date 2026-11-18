package fr.neezer.mobile.feature.playlist

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.PlaylistConflict
import uniffi.nz_ffi.PlaylistRole

/**
 * The playlist's state machine, proven on the JVM (lot MB9).
 *
 * Every assertion is a COUNT or a piece of STATE the acceptance names — requests
 * per screen, the debounce's one `PUT`, the optimistic row that leaves before the
 * request, the row that comes back after a failure — and the clock is
 * `kotlinx-coroutines-test`'s virtual one: nothing sleeps (worker rule 6).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PlaylistModelTest {
    /**
     * One test's world: a fake service, player and sharer, a model over a scope
     * on the TEST scheduler (so `advanceTimeBy` drives the debounce and the polls
     * — never a sleep), and the toasts it raised.
     *
     * The scope is its own, not `backgroundScope`: in this `kotlinx-coroutines-test`
     * a `backgroundScope` task is NOT advanced by `advanceUntilIdle`, which made
     * every test read an empty model (measured here: `calls=[]` after idle). A
     * scope on `StandardTestDispatcher(testScheduler)` is the same scheduler the
     * test advances, and `close()` cancels what it started.
     */
    private class Harness(test: kotlinx.coroutines.test.TestScope) {
        val scope = kotlinx.coroutines.CoroutineScope(
            kotlinx.coroutines.test.StandardTestDispatcher(test.testScheduler),
        )
        val service = FakePlaylistService()
        val player = FakePlayer()
        val sharer = FakeSharer()
        val toasts = mutableListOf<String>()
        var retryValue: (() -> Unit)? = null

        val model = PlaylistModel(
            service = service,
            player = player,
            sharer = sharer,
            scope = scope,
            io = kotlinx.coroutines.Dispatchers.Unconfined,
            toast = { text, retry ->
                toasts += text
                retryValue = retry
            },
        )

        fun close() {
            scope.cancel()
        }
    }

    private var harness: Harness? = null

    private fun kotlinx.coroutines.test.TestScope.harness(): Harness =
        Harness(this).also { harness = it }

    @org.junit.After
    fun tearDown() {
        harness?.close()
        harness = null
    }

    /**
     * The acceptance's first number: a 4 000-row playlist opens with ONE page
     * request, and the window hands out pages of 64 — never the whole list.
     */
    @Test
    fun a_four_thousand_row_playlist_opens_with_one_request_and_windows_its_pages() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()

        val state = model.state.value
        assertEquals(4_000, state.trackCount)
        assertEquals("one page request", 1, harness.service.calls.count { it == "playlist" })
        // The disk read happens (a warm visit would paint it); here it answers
        // nothing, so what is on screen is the network's answer.
        assertFalse("the page on screen is the server's", state.stale)
        assertEquals(
            "the suggestions are their own beat",
            1,
            harness.service.calls.count { it == "recommendations" },
        )
        assertNotNull(state.page)
        assertTrue(state.editable)

        // The window: pages of 64, never the 4 000 rows at once.
        val rows = state.rows!!
        assertEquals(64, rows.source.window(0, 64).size)
        assertEquals(4_000, rows.source.len())
        assertEquals(10, rows.source.window(3_990, 64).size)
    }

    /** A warm visit paints the kept copy first, then the network replaces it. */
    @Test
    fun the_kept_copy_paints_before_the_network() = runTest {
        val harness = harness()
        harness.service.cachedPage = PlaylistFixtures.page()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        assertEquals(listOf("cached", "playlist", "recommendations"), harness.service.calls)
    }

    /** With nothing kept and a dead link: the cause, never a spinner. */
    @Test
    fun a_failed_read_names_its_cause_and_retries() = runTest {
        val harness = harness()
        harness.service.failure = RuntimeException("hors ligne")
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        val failed = model.state.value
        assertEquals("hors ligne", failed.failure)
        assertNull(failed.page)
        assertFalse(failed.loading)

        harness.service.failure = null
        model.retry()
        advanceUntilIdle()
        assertNull(model.state.value.failure)
        assertNotNull(model.state.value.page)
    }

    /**
     * The reorder: the move answers with no request, and a burst of moves becomes
     * ONE `PUT` after 400 ms.
     */
    @Test
    fun a_drag_answers_at_once_and_a_burst_is_one_put() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.toggleEdit()
        assertTrue(model.state.value.editing)

        model.move(0, 2)
        // `runCurrent`, not `advanceUntilIdle`: the coalescing window IS 400 ms of
        // virtual time, and advancing to idle would fire the `PUT` this asserts is
        // still pending.
        runCurrent()
        assertEquals("a move sends nothing yet", 0, harness.service.orderSaves)
        // Row 0 lands at index 2: the row THAT index now carries is the dragged one.
        assertEquals("Titre 0", model.state.value.rows!!.source.window(2, 1).first().title)
        assertEquals("Titre 1", model.state.value.rows!!.source.window(0, 1).first().title)

        // Three drags in less than the window → one PUT.
        model.move(2, 3)
        advanceTimeBy(100)
        model.move(3, 4)
        advanceTimeBy(100)
        model.move(4, 5)
        advanceTimeBy(PlaylistModel.ORDER_DEBOUNCE_MS + 1)
        advanceUntilIdle()
        assertEquals(1, harness.service.orderSaves)
    }

    /** A `409` on the order: the server's rows replace ours, and the person is told. */
    @Test
    fun a_conflict_on_the_order_adopts_the_servers_rows() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.toggleEdit()

        harness.service.writeConflict = PlaylistConflict.CONTENT
        harness.service.page = harness.service.page?.copy(
            meta = PlaylistFixtures.meta(version = 7u),
            rows = FakeRows(PlaylistFixtures.tracks(8)),
        )
        model.move(0, 1)
        advanceTimeBy(PlaylistModel.ORDER_DEBOUNCE_MS + 1)
        advanceUntilIdle()

        assertEquals("the server's order is what we show", 8, model.state.value.trackCount)
        assertTrue(
            "the conflict says so: ${harness.toasts}",
            harness.toasts.any { it.contains("changé entre-temps") },
        )
    }

    /** « Retirer »: the row leaves at once, and the write follows it in order. */
    @Test
    fun a_removal_leaves_at_once_and_the_write_follows() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.toggleEdit()

        model.removeAt(3)
        advanceUntilIdle()
        assertEquals(3_999, model.state.value.trackCount)
        assertEquals("Titre 4", model.state.value.rows!!.source.window(3, 1).first().title)
        assertTrue(harness.service.calls.indexOf("cut") < harness.service.calls.indexOf("saveCut"))
    }

    /** A failed removal puts the row back (the core's undo) and offers « Réessayer ». */
    @Test
    fun a_failed_removal_puts_the_row_back() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.toggleEdit()

        harness.service.writeFailure = RuntimeException("hors ligne")
        // The core puts the row back; the fake service answers the restored rows.
        model.removeAt(0)
        advanceUntilIdle()
        assertTrue("the list is re-read from the core", harness.service.calls.contains("rows"))
        assertTrue(
            "${harness.toasts}",
            harness.toasts.any { it.contains("n'a pas pu être retiré") },
        )
        assertNotNull("the toast offers Réessayer", harness.retryValue)
    }

    /** An append is one request, and the suggestions that were added leave. */
    @Test
    fun an_append_is_one_request_and_the_suggestions_leave() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        val recs = model.state.value.recs
        assertTrue(recs is RecsUi.Ready)
        assertEquals(2, (recs as RecsUi.Ready).tracks.size)

        model.addRecommendation(recs.tracks.first())
        advanceUntilIdle()
        assertEquals(1, harness.service.appends)
        assertEquals(1, harness.service.calls.count { it == "saveAppend" })
        assertEquals(4_001, model.state.value.rows?.len() ?: 0)
    }

    /** « Tout ajouter » appends the visible suggestions in ONE request. */
    @Test
    fun adding_all_the_suggestions_is_one_request() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.addAllRecommendations()
        advanceUntilIdle()
        assertEquals(1, harness.service.appends)
        assertEquals(4_002, model.state.value.rows?.len() ?: 0)
        val added = (model.state.value.recs as? RecsUi.Ready)?.tracks
        assertEquals(emptyList<Any>(), added)
    }

    /** A rename shows at once and comes back when the request fails. */
    @Test
    fun a_rename_is_optimistic_and_reverts() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()

        model.openMeta()
        assertTrue(model.state.value.editingMeta)
        model.onTitle("Nouveau nom")
        model.saveMeta()
        advanceUntilIdle()
        assertTrue(
            "the rename is the one request",
            harness.service.calls.contains("edit:Nouveau nom"),
        )
        assertFalse(model.state.value.editingMeta)

        harness.service.writeFailure = RuntimeException("refusé")
        model.openMeta()
        model.onTitle("Encore un autre")
        model.saveMeta()
        advanceUntilIdle()
        assertEquals(
            "the failed rename is put back",
            "Nouveau nom",
            model.state.value.displayTitle,
        )
    }

    /** The drag mode only exists in the natural order: a search or a sort leaves it. */
    @Test
    fun the_drag_mode_is_only_in_the_natural_order() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.toggleEdit()
        assertTrue(model.state.value.editing)

        model.onQuery("titre 1")
        advanceUntilIdle()
        assertFalse("a projection is not an order", model.state.value.editing)
        assertTrue(model.state.value.trackCount < 4_000)

        model.onQuery("")
        model.toggleEdit()
        model.chooseSort(PlaylistSort.Title)
        advanceUntilIdle()
        assertFalse(model.state.value.editing)

        model.chooseSort(PlaylistSort.Position)
        advanceUntilIdle()
        assertTrue(model.state.value.manualView)
    }

    /** The collaborators: an invite is one request; a refusal refreshes the rights. */
    @Test
    fun the_members_are_invited_and_a_refusal_is_named() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.openMembers()
        assertNotNull(model.state.value.members)
        assertEquals(1, model.state.value.members!!.members.size)

        model.onInviteName("camille")
        model.onInviteRole(PlaylistRole.EDITOR)
        harness.service.membersWrite = PlaylistFixtures.membersWrite(
            members = listOf(PlaylistFixtures.owner(), uniffi.nz_ffi.PlaylistMember("u2", "camille", PlaylistRole.EDITOR, null)),
        )
        model.invite()
        advanceUntilIdle()
        assertTrue(harness.service.calls.contains("invite:camille"))
        assertEquals(2, model.state.value.members!!.members.size)
        assertEquals("", model.state.value.members!!.username)

        harness.service.membersWrite = PlaylistFixtures.membersWrite(
            role = PlaylistRole.VIEWER,
            canEdit = false,
            canManage = false,
            denied = true,
        )
        model.onInviteName("sam")
        model.invite()
        advanceUntilIdle()
        val sheet = model.state.value.members!!
        assertTrue(sheet.denied)
        assertFalse(sheet.canManage)
        assertEquals(
            "Vous n'avez plus le droit de modifier cette playlist.",
            sheet.error,
        )
    }

    /** The export: the formats, the services, the job followed once per 800 ms. */
    @Test
    fun the_export_reads_its_formats_and_follows_the_job() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.openExport()
        advanceUntilIdle()
        val export = model.state.value.export!!
        assertEquals(listOf("opus320", "flac"), export.formats.map { it.id })
        assertEquals("opus320", export.fmt)
        assertEquals("the linked service is chosen", "spotify", export.service)

        // The POST, then ONE poll: the job runs, and the screen says so.
        model.sendExport()
        runCurrent()
        assertTrue(model.state.value.export!!.job!!.running)

        // One poll per 800 ms; the job ends when the server says so. A poll loop
        // is why this test steps the clock instead of `advanceUntilIdle` — an
        // always-running job would spin virtual time for ever (measured: the
        // first version of this test hung the suite for 4 minutes).
        harness.service.job = PlaylistFixtures.job(running = false, matched = 3u, missing = 1u)
        advanceTimeBy(PlaylistModel.POLL_MS + 1)
        runCurrent()
        val done = model.state.value.export!!.job!!
        assertFalse(done.running)
        assertEquals(3u, done.matched)
        assertEquals(1u, done.missingCount)
        assertTrue(done.percent > 0u)
    }

    /** The public link: read, created, and the URL shared with its origin. */
    @Test
    fun the_public_link_is_created_and_shared() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.openExport()
        advanceUntilIdle()
        assertNull(model.state.value.export!!.link)

        model.createLink()
        advanceUntilIdle()
        assertNotNull(model.state.value.export!!.link)
        assertTrue(harness.toasts.any { it == "Lien public créé" })

        model.revokeLink(confirmed = false)
        assertEquals(PlaylistModel.Confirm.RevokeLink, model.state.value.confirm)
        model.revokeLink(confirmed = true)
        advanceUntilIdle()
        assertNull(model.state.value.export!!.link)
        assertTrue(harness.service.calls.contains("revokeShare"))
    }

    /** Deleting asks first, and a deleted playlist takes the screen with it. */
    @Test
    fun deleting_asks_first_and_leaves() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()

        model.delete(confirmed = false)
        assertEquals(PlaylistModel.Confirm.Delete, model.state.value.confirm)
        assertFalse(harness.service.calls.contains("delete"))

        model.delete(confirmed = true)
        advanceUntilIdle()
        assertTrue(harness.service.calls.contains("delete"))
        assertTrue(model.state.value.gone)
    }

    /** Play and shuffle hand the playlist's context to the player. */
    @Test
    fun play_hands_the_whole_list_and_its_context_to_the_player() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()
        model.playAll()
        model.shufflePlay()
        assertEquals(listOf(4_000 to "pl-1", 4_000 to "shuffle:pl-1"), harness.player.played)
    }

    /** « Ajouter des titres » asks MB6's search once per settled term. */
    @Test
    fun the_add_sheet_searches_once_per_settled_term() = runTest {
        val harness = harness()
        val model = harness.model
        model.start("pl-1")
        advanceUntilIdle()

        model.openAddTracks()
        advanceUntilIdle()
        assertEquals(0, harness.service.calls.count { it.startsWith("search:") })

        // Nine keys in less than the box's beat: one request.
        for (term in listOf("d", "da", "daf", "daft", "daft p", "daft pu", "daft pun", "daft punk", "daft punk")) {
            model.onAddTerm(term)
        }
        advanceTimeBy(PlaylistModel.ADD_DEBOUNCE_MS + 1)
        advanceUntilIdle()
        assertEquals(1, harness.service.calls.count { it.startsWith("search:") })
        assertEquals(1, model.state.value.add!!.rows.size)

        model.addChosen(model.state.value.add!!.rows)
        advanceUntilIdle()
        assertNull("the sheet closes once it added", model.state.value.add)
        assertEquals(1, harness.service.appends)
    }
}
