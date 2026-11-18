package fr.neezer.mobile.feature.onboarding

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.AccountRun
import uniffi.nz_ffi.ImportState
import uniffi.nz_ffi.OnboardingStep
import uniffi.nz_ffi.RelatedState

/**
 * The onboarding's state machine, on the JVM over its two ports.
 *
 * Every wait is the VIRTUAL clock (the page's 350 ms debounce, its 900/1 500 ms
 * neighbour poll, its 1 500 ms import poll): a test advances time, it never
 * sleeps. The numbers the lot is held to are asserted here — the visit's two
 * reads, a like's one request, a search's one per settled term, an import's one
 * to start and its poll.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class OnboardingModelTest {
    private fun TestScope.model(
        service: FakeOnboardingService = FakeOnboardingService(),
        rules: FakeRules = FakeRules(),
        done: () -> Unit = {},
    ) = OnboardingModel(
        service = service,
        rules = rules,
        scope = backgroundScope,
        describe = { it.message ?: "erreur" },
        onDone = done,
    )

    @Test
    fun the_kept_copy_paints_then_the_visit_replaces_it() = runTest {
        val service = FakeOnboardingService()
        service.cachedValue = OnboardingFixtures.data(
            page = OnboardingFixtures.page(items = OnboardingFixtures.forest().take(2)),
        )
        service.loadValue = OnboardingFixtures.data(
            page = OnboardingFixtures.page(items = OnboardingFixtures.forest()),
        )
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals(listOf("cached", "load"), service.calls)
        assertEquals(6, model.state.value.tiles.size)
        assertFalse(model.state.value.stale)
        assertFalse(model.state.value.loading)
        assertNull(model.state.value.failure)
    }

    @Test
    fun a_failed_refresh_keeps_the_copy_and_says_so() = runTest {
        val service = FakeOnboardingService()
        service.loadError = uniffi.nz_ffi.NzException.Offline()
        val model = model(service)
        model.start()
        runCurrent()
        assertTrue(model.state.value.stale)
        assertNull(model.state.value.failure)
        assertFalse(model.state.value.online)
    }

    @Test
    fun a_failed_read_with_nothing_painted_names_its_cause() = runTest {
        val service = FakeOnboardingService()
        service.cachedValue = null
        service.loadError = IllegalStateException("Le serveur ne répond pas")
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals("Le serveur ne répond pas", model.state.value.failure)
    }

    @Test
    fun a_like_is_visible_before_the_request_and_settles_with_its_neighbours() = runTest {
        val service = FakeOnboardingService()
        service.likeValue = OnboardingFixtures.relatedReady(
            OnboardingFixtures.artist("900", "Zero 7", via = "related", parent = "100"),
            OnboardingFixtures.artist("901", "Télépopmusik", via = "related", parent = "100"),
        )
        val model = model(service)
        model.start()
        runCurrent()
        val tile = model.state.value.tiles.first()
        model.toggle(tile)
        // The same frame: the heart and the count, before any answer.
        assertTrue(model.state.value.likedIds.contains("100"))
        assertEquals(2, model.state.value.liked)
        assertTrue(model.state.value.pendingIds.contains("100"))
        runCurrent()
        assertEquals(listOf("cached", "load", "like:100"), service.calls)
        assertFalse(model.state.value.pendingIds.contains("100"))
        assertEquals(8, model.state.value.tiles.size)
        assertEquals("2 proches ajoutés", model.state.value.notice)
    }

    @Test
    fun a_refused_like_rolls_back_and_offers_retry() = runTest {
        val service = FakeOnboardingService()
        service.likeError = IllegalStateException("Hors ligne")
        val model = model(service)
        model.start()
        runCurrent()
        model.toggle(model.state.value.tiles.first())
        runCurrent()
        assertFalse(model.state.value.likedIds.contains("100"))
        assertEquals(1, model.state.value.liked)
        val error = model.state.value.actionError
        assertNotNull(error)
        assertEquals("Votre choix n'a pas été enregistré.", error!!.message)
        assertNotNull(error.retry)
    }

    @Test
    fun a_pending_neighbour_request_is_polled_on_the_pages_beats() = runTest {
        val service = FakeOnboardingService()
        service.likeValue = OnboardingFixtures.relatedPending()
        service.relatedValue = OnboardingFixtures.relatedReady(
            OnboardingFixtures.artist("900", "Zero 7", via = "related", parent = "100"),
        )
        val model = model(service)
        model.start()
        runCurrent()
        model.toggle(model.state.value.tiles.first())
        runCurrent()
        assertEquals(OnboardingModel.RelatedSlot.Pending, model.state.value.relatedSlots["100"])
        advanceTimeBy(901)
        runCurrent()
        assertFalse(service.calls.none { it == "related:100" })
        assertEquals(7, model.state.value.tiles.size)
        assertEquals(null, model.state.value.relatedSlots["100"])
    }

    @Test
    fun a_failed_neighbour_request_has_its_own_line_and_a_retry() = runTest {
        val service = FakeOnboardingService()
        service.likeValue = OnboardingFixtures.relatedError()
        val model = model(service)
        model.start()
        runCurrent()
        model.toggle(model.state.value.tiles.first())
        runCurrent()
        assertEquals(OnboardingModel.RelatedSlot.Failed, model.state.value.relatedSlots["100"])
        service.relatedValue = OnboardingFixtures.relatedReady(
            OnboardingFixtures.artist("900", "Zero 7", via = "related", parent = "100"),
        )
        model.retryRelated("100")
        runCurrent()
        assertEquals(7, model.state.value.tiles.size)
    }

    @Test
    fun an_unlike_takes_the_block_out_and_rolls_back_when_refused() = runTest {
        val service = FakeOnboardingService()
        service.likeValue = OnboardingFixtures.relatedReady(
            OnboardingFixtures.artist("900", "Zero 7", via = "related", parent = "100"),
        )
        val model = model(service)
        model.start()
        runCurrent()
        val tile = model.state.value.tiles.first()
        model.toggle(tile)
        runCurrent()
        assertEquals(7, model.state.value.tiles.size)

        service.unlikeError = IllegalStateException("Hors ligne")
        model.toggle(model.state.value.tiles.first())
        runCurrent()
        assertTrue(model.state.value.likedIds.contains("100"))
        assertEquals(2, model.state.value.liked)
        assertNotNull(model.state.value.actionError)
    }

    @Test
    fun typing_is_one_search_after_the_pages_debounce() = runTest {
        val service = FakeOnboardingService()
        val model = model(service)
        model.start()
        runCurrent()
        service.calls.clear()
        for (term in listOf("a", "ay", "aya", "aya ", "aya n")) {
            model.onQuery(term)
            advanceTimeBy(100)
        }
        assertTrue(service.calls.none { it.startsWith("search") })
        advanceTimeBy(300)
        runCurrent()
        assertEquals(listOf("search:aya n:36"), service.calls)
        assertEquals(listOf("Aya Nakamura"), model.state.value.items.map { it.name })
    }

    @Test
    fun a_term_under_two_characters_asks_nothing_and_clears() = runTest {
        val service = FakeOnboardingService()
        val model = model(service)
        model.start()
        runCurrent()
        service.calls.clear()
        model.onQuery("a")
        advanceTimeBy(400)
        runCurrent()
        assertTrue(service.calls.isEmpty())
        assertEquals("", model.state.value.query)
        assertNull(model.state.value.searchItems)
    }

    @Test
    fun a_failed_search_says_so_and_retries_the_same_term() = runTest {
        val service = FakeOnboardingService()
        service.searchError = IllegalStateException("Le serveur ne répond pas")
        val model = model(service)
        model.start()
        runCurrent()
        model.onQuery("aya")
        advanceTimeBy(351)
        runCurrent()
        assertEquals("Le serveur ne répond pas", model.state.value.searchFailure)
        service.searchError = null
        model.retrySearch()
        runCurrent()
        assertNull(model.state.value.searchFailure)
        assertEquals(1, model.state.value.items.size)
    }

    @Test
    fun clearing_the_search_puts_the_feed_back() = runTest {
        val service = FakeOnboardingService()
        val model = model(service)
        model.start()
        runCurrent()
        model.onQuery("aya")
        advanceTimeBy(351)
        runCurrent()
        assertEquals(1, model.state.value.items.size)
        model.clearSearch()
        assertEquals(6, model.state.value.items.size)
        assertEquals("", model.state.value.query)
    }

    @Test
    fun the_finish_gate_refuses_before_any_request() = runTest {
        val service = FakeOnboardingService()
        service.loadValue = OnboardingFixtures.data(
            state = OnboardingFixtures.state(liked = 0),
            page = OnboardingFixtures.page(),
        )
        val model = model(service)
        model.start()
        runCurrent()
        service.calls.clear()
        model.finish()
        runCurrent()
        assertTrue(service.calls.isEmpty())
        assertEquals(
            "Choisissez au moins un artiste pour terminer.",
            model.state.value.status,
        )
    }

    @Test
    fun finishing_asks_the_server_and_opens_the_account_step() = runTest {
        val service = FakeOnboardingService()
        service.artistsDoneValue = OnboardingFixtures.artistsDone(OnboardingStep.ACCOUNT)
        val model = model(service)
        model.start()
        runCurrent()
        service.calls.clear()
        model.finish()
        runCurrent()
        assertEquals(listOf("artistsDone:false", "options"), service.calls)
        assertEquals(OnboardingStep.ACCOUNT, model.state.value.step)
        assertNotNull(model.state.value.options)
    }

    @Test
    fun passing_goes_straight_home_when_the_server_says_done() = runTest {
        val service = FakeOnboardingService()
        service.artistsDoneValue = OnboardingFixtures.artistsDone(OnboardingStep.DONE, skipped = true)
        var done = 0
        val model = model(service, done = { done += 1 })
        model.start()
        runCurrent()
        model.pass()
        runCurrent()
        assertTrue(service.calls.contains("artistsDone:true"))
        assertEquals(1, done)
        assertEquals(OnboardingStep.DONE, model.state.value.step)
    }

    @Test
    fun the_account_step_paints_what_the_server_offers() = runTest {
        val service = FakeOnboardingService()
        service.optionsValue = OnboardingFixtures.options(
            deezer = OnboardingFixtures.provider(available = false, canLink = false, reason = "deezer_disabled"),
            youtube = OnboardingFixtures.provider(),
        )
        val model = model(service)
        model.start()
        runCurrent()
        model.gotoAccountForStart()
        model.loadOptions()
        runCurrent()
        assertEquals(1, service.calls.count { it == "options" })
        assertFalse(model.state.value.options!!.deezer.available)
        assertTrue(model.state.value.options!!.youtube.canLink)
    }

    @Test
    fun a_failed_options_read_is_a_cause_with_retry() = runTest {
        val service = FakeOnboardingService()
        service.optionsError = IllegalStateException("Le serveur ne répond pas")
        val model = model(service)
        model.start()
        runCurrent()
        model.gotoAccountForStart()
        model.loadOptions()
        runCurrent()
        assertEquals("Le serveur ne répond pas", model.state.value.optionsFailure)
    }

    @Test
    fun linking_an_account_says_the_import_started_on_the_server() = runTest {
        val service = FakeOnboardingService()
        service.linkValue = OnboardingFixtures.options(
            deezer = OnboardingFixtures.provider(
                available = true,
                canLink = false,
                linked = true,
                account = OnboardingFixtures.linkedAccount(
                    running = AccountRun(phase = "import", done = 0u, total = 42u),
                ),
            ),
        )
        val model = model(service)
        model.start()
        runCurrent()
        model.gotoAccountForStart()
        model.link("deezer", "un-arl")
        runCurrent()
        assertEquals(listOf("link:deezer"), service.calls.filter { it == "link:deezer" })
        assertTrue(model.state.value.accountNotice!!.contains("démarré sur le serveur"))
        assertTrue(model.state.value.options!!.deezer.linked)
    }

    @Test
    fun an_import_shows_its_progress_then_each_lines_cause() = runTest {
        val service = FakeOnboardingService()
        service.startedValue = uniffi.nz_ffi.StartedImport(
            job = OnboardingFixtures.job(state = ImportState.READING),
            options = OnboardingFixtures.options(),
        )
        service.jobStatus = OnboardingFixtures.finishedJob()
        val model = model(service)
        model.start()
        runCurrent()
        model.gotoAccountForStart()
        model.startImport("https://open.spotify.com/playlist/x")
        runCurrent()
        assertEquals(ImportState.READING, model.state.value.importJob!!.state)
        advanceTimeBy(1_501)
        runCurrent()
        val job = model.state.value.importJob!!
        assertEquals(ImportState.DONE, job.state)
        assertEquals(2, job.unmatched.toInt())
        assertEquals(
            listOf("Inconnu 1", "Inconnu 2"),
            job.items.filter { !it.matched }.map { it.title },
        )
        assertTrue(job.error!!.contains("Deezer"))
    }

    @Test
    fun a_finished_import_is_kept_as_a_local_playlist() = runTest {
        val service = FakeOnboardingService()
        service.startedValue = uniffi.nz_ffi.StartedImport(
            job = OnboardingFixtures.job(state = ImportState.READING),
            options = OnboardingFixtures.options(),
        )
        service.jobStatus = OnboardingFixtures.finishedJob()
        service.savedJob = OnboardingFixtures.job(
            state = ImportState.SAVED,
            total = 3,
            done = 3,
            matched = 1,
        )
        val model = model(service)
        model.start()
        runCurrent()
        model.gotoAccountForStart()
        model.startImport("https://open.spotify.com/playlist/x")
        runCurrent()
        advanceTimeBy(1_501)
        runCurrent()
        model.saveImport()
        runCurrent()
        assertTrue(service.calls.contains("importSave:job-1"))
        assertEquals(ImportState.SAVED, model.state.value.importJob!!.state)
    }

    @Test
    fun a_running_import_is_resumed_by_the_account_step() = runTest {
        val service = FakeOnboardingService()
        service.optionsValue = OnboardingFixtures.options(
            job = OnboardingFixtures.job(id = "job-running", state = ImportState.MATCHING, done = 42, total = 120),
        )
        service.jobStatus = OnboardingFixtures.job(id = "job-running", state = ImportState.DONE, matched = 1, total = 1)
        val model = model(service)
        model.start()
        runCurrent()
        model.gotoAccountForStart()
        model.loadOptions()
        runCurrent()
        assertEquals("job-running", model.state.value.importJob!!.id)
        assertTrue(model.state.value.importRunning)
        advanceTimeBy(1_501)
        runCurrent()
        assertEquals(ImportState.DONE, model.state.value.importJob!!.state)
    }

    @Test
    fun finishing_the_account_step_during_an_import_arms_a_second_press() = runTest {
        val service = FakeOnboardingService()
        service.optionsValue = OnboardingFixtures.options(
            job = OnboardingFixtures.job(state = ImportState.MATCHING),
        )
        val model = model(service)
        model.start()
        runCurrent()
        model.gotoAccountForStart()
        model.loadOptions()
        runCurrent()
        service.calls.clear()
        model.accountDone()
        runCurrent()
        assertTrue(service.calls.none { it == "accountDone" })
        assertTrue(model.state.value.finishArmed)
        assertTrue(model.state.value.accountNotice!!.contains("continuera sur le serveur"))
        model.accountDone()
        runCurrent()
        assertTrue(service.calls.contains("accountDone"))
    }

    @Test
    fun the_server_can_refuse_the_end_and_send_the_person_back_to_artists() = runTest {
        val service = FakeOnboardingService()
        service.accountDoneError = IllegalStateException("Termine d'abord l'étape des artistes.")
        val model = model(service)
        model.start()
        runCurrent()
        model.gotoAccountForStart()
        model.accountDone()
        runCurrent()
        assertNotNull(model.state.value.actionError)
        model.gotoArtists()
        assertEquals(OnboardingStep.ARTISTS, model.state.value.step)
        assertNull(model.state.value.actionError)
    }

    @Test
    fun skipping_a_card_hides_it_until_it_is_unskipped() = runTest {
        val model = model()
        model.start()
        runCurrent()
        model.skip("deezer")
        assertTrue(model.state.value.skipped.contains("deezer"))
        model.unskip("deezer")
        assertFalse(model.state.value.skipped.contains("deezer"))
    }

    @Test
    fun more_tiles_are_one_page_and_reconcile_in_place() = runTest {
        val service = FakeOnboardingService()
        service.loadValue = OnboardingFixtures.data(page = OnboardingFixtures.page(next = "c2"))
        service.pageValue = OnboardingFixtures.page(
            items = listOf(OnboardingFixtures.artist("107", "M83")),
            next = null,
        )
        val model = model(service)
        model.start()
        runCurrent()
        model.loadMore()
        runCurrent()
        assertEquals(7, model.state.value.tiles.size)
        assertNull(model.state.value.next)
        assertTrue(model.state.value.endOfList)
    }

    @Test
    fun a_refused_feed_rebuild_is_a_notice_not_a_blank_page() = runTest {
        val service = FakeOnboardingService()
        service.stateValue = OnboardingFixtures.state(
            feed = OnboardingFixtures.feed(state = "unavailable"),
        )
        service.feedRetryError = IllegalStateException("Une reconstruction vient d'être demandée.")
        service.pageValue = OnboardingFixtures.page(items = emptyList())
        val model = model(service)
        model.start()
        runCurrent()
        service.calls.clear()
        model.retryFeed()
        runCurrent()
        assertTrue(service.calls.contains("feedRetry"))
        assertTrue(model.state.value.notice!!.contains("Une reconstruction vient d'être demandée."))
        assertEquals(1, model.state.value.feedAttempts)
    }

    @Test
    fun leaving_cancels_the_poll_the_screen_started() = runTest {
        val service = FakeOnboardingService()
        service.likeValue = OnboardingFixtures.relatedPending()
        val model = model(service)
        model.start()
        runCurrent()
        model.toggle(model.state.value.tiles.first())
        runCurrent()
        model.leave()
        service.calls.clear()
        advanceTimeBy(5_000)
        runCurrent()
        assertTrue(service.calls.none { it.startsWith("related") })
    }
}
