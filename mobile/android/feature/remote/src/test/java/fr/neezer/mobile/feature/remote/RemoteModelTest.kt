package fr.neezer.mobile.feature.remote

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.RemoteCommand
import uniffi.nz_ffi.RemotePhase

/**
 * Télécommande (MB19), the rules of the SPA's sheet and claim screen, proven on
 * the JVM against a fake service and a VIRTUAL clock — never a sleep (worker
 * rule 6). Each test names the SPA behaviour it ports.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class RemoteModelTest {
    private val dispatcher = StandardTestDispatcher()

    private fun TestScope.model(
        service: FakeRemoteService,
        toasts: MutableList<Pair<String, Boolean>> = mutableListOf(),
    ): Pair<RemoteModel, MutableList<Pair<String, Boolean>>> {
        val model = RemoteModel(
            service = service,
            scope = backgroundScope,
            io = dispatcher,
            toast = { text, retry -> toasts += text to (retry != null) },
        )
        return model to toasts
    }

    /** `RemoteSheet.svelte#load`: the kept list paints first, the fresh one replaces it. */
    @Test
    fun a_kept_list_paints_before_the_read_and_the_fresh_one_replaces_it() = runTest(dispatcher) {
        val service = FakeRemoteService(links = mutableListOf(FakeRemoteService.link(id = "fresh")))
        service.cached += FakeRemoteService.link(id = "kept")
        val (model, _) = model(service)

        model.open()
        // The kept copy is on screen before any request answers.
        runCurrent()
        assertEquals(listOf("fresh"), model.state.value.links.map { it.id })
        assertFalse(model.state.value.linksStale)
        assertEquals(listOf("links"), service.calls.filter { it == "links" })
        model.close()
    }

    /** Rule 3: offline with a kept copy keeps it and says so, never a blank sheet. */
    @Test
    fun an_offline_read_keeps_the_remembered_links() = runTest(dispatcher) {
        val service = FakeRemoteService(links = mutableListOf(FakeRemoteService.link(id = "kept")))
        service.cached += FakeRemoteService.link(id = "kept")
        service.failLinks = "le serveur ne répond pas (réseau)"
        val (model, _) = model(service)
        model.open()
        runCurrent()
        assertEquals(listOf("kept"), model.state.value.links.map { it.id })
        assertTrue(model.state.value.linksStale)
        assertNull(model.state.value.failure)
        model.close()
    }

    /** Rule 3, the other half: nothing kept, the cause and « Réessayer ». */
    @Test
    fun a_read_with_nothing_kept_names_its_cause_and_offers_a_retry() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.failLinks = "le serveur ne répond pas (réseau)"
        val (model, _) = model(service)
        model.open()
        runCurrent()
        val failure = model.state.value.failure
        assertNotNull(failure)
        assertEquals("le serveur ne répond pas (réseau)", failure!!.cause)
        assertTrue(model.state.value.links.isEmpty())

        // « Réessayer » re-runs the read, and one click is one attempt.
        service.failLinks = null
        service.links.add(FakeRemoteService.link(id = "back"))
        failure.retry()
        runCurrent()
        assertNull(model.state.value.failure)
        assertEquals(listOf("back"), model.state.value.links.map { it.id })
        model.close()
    }

    /** `RemoteSheet.svelte#create` + `startHosting()`: a link, then the loops. */
    @Test
    fun creating_a_link_shows_it_invites_with_it_and_starts_answering() = runTest(dispatcher) {
        val service = FakeRemoteService()
        val (model, _) = model(service)
        model.open()
        runCurrent()
        model.setLevel("read")
        model.setDuration("none")
        model.setLabel("  Léa  ")
        model.create()
        runCurrent()

        assertEquals("createLink:read:none:Léa", service.calls.last { it.startsWith("createLink") })
        assertTrue(service.hosting)
        assertTrue(service.calls.contains("startHost"))
        val invite = model.state.value.invite
        assertNotNull(invite)
        assertEquals("la durée choisie, telle que le serveur l'a dite", "Sans limite", invite!!.expiry)
        assertEquals(1, model.state.value.links.size)
        assertEquals("le champ se vide après la création", "", model.state.value.label)
        model.close()
    }

    /** A failed create says its cause and offers « Réessayer ». */
    @Test
    fun a_refused_link_says_why_and_can_be_asked_again() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.failCreate = "Seul un administrateur peut prêter l'administration"
        val (model, toasts) = model(service)
        model.open()
        runCurrent()
        model.setLevel("admin")
        model.create()
        runCurrent()
        assertTrue(toasts.any { it.first.contains("administrateur") && it.second })
        assertFalse(model.state.value.creating)
        model.close()
    }

    /** `RemoteSheet.svelte#cut`: one link, one request, gone from the list. */
    @Test
    fun cutting_a_link_removes_it_and_says_so() = runTest(dispatcher) {
        val service = FakeRemoteService(links = mutableListOf(FakeRemoteService.link(id = "a")))
        val (model, toasts) = model(service)
        model.open()
        runCurrent()
        model.cut("a")
        runCurrent()
        assertTrue(model.state.value.links.isEmpty())
        assertTrue(service.calls.contains("revokeLink:a"))
        assertTrue(toasts.any { it.first == "Lien coupé" })
        model.close()
    }

    /** `RemoteSheet.svelte#cutEverything`: armed once, then it cuts. */
    @Test
    fun cutting_everything_asks_once_first() = runTest(dispatcher) {
        val service = FakeRemoteService(links = mutableListOf(FakeRemoteService.link(id = "a")))
        val (model, _) = model(service)
        model.open()
        runCurrent()
        model.cutAll()
        runCurrent()
        assertTrue(model.state.value.armedCut)
        assertFalse(service.calls.contains("revokeAll"))
        assertEquals(1, model.state.value.links.size)

        model.cutAll()
        runCurrent()
        assertTrue(service.calls.contains("revokeAll"))
        assertTrue(model.state.value.links.isEmpty())
        assertFalse(model.state.value.armedCut)
        model.close()
    }

    /** `routes/RemoteClaim.svelte#claim`: connecting, then the loan's screen. */
    @Test
    fun claiming_a_valid_link_starts_driving_it() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.grant = FakeRemoteService.grant("full")
        service.now = FakeRemoteService.nowOf()
        val (model, _) = model(service)
        model.claim("abcd1234efgh5678.abcdefghijklmnopqrstuvwx")
        runCurrent()
        assertEquals(ClaimPhase.Connected, model.state.value.claim)
        assertEquals("owner", model.state.value.grant?.owner)
        assertEquals(1, service.started)
        assertEquals("abcd1234efgh5678.abcdefghijklmnopqrstuvwx", service.claimedWith)
        // The control screen's first read landed.
        assertNotNull(model.state.value.control)
        assertEquals("Alpha", model.state.value.control?.track?.title)
    }

    /** An expired link is not a crash: the card says so, without a retry. */
    @Test
    fun an_invalid_link_says_it_is_no_longer_valid() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.failClaim = "Ce lien de télécommande n'est pas valide"
        val (model, _) = model(service)
        model.claim("mauvais")
        runCurrent()
        assertEquals(ClaimPhase.Invalid, model.state.value.claim)
        assertEquals(0, service.started)
    }

    /** Offline on a claim offers « Réessayer » (the SPA's own two causes). */
    @Test
    fun an_offline_claim_offers_a_retry() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.failClaim = "le serveur ne répond pas (réseau)"
        val (model, _) = model(service)
        model.claim("abcd1234efgh5678.abcdefghijklmnopqrstuvwx")
        runCurrent()
        assertEquals(ClaimPhase.Offline, model.state.value.claim)
    }

    /** An empty token means "this session IS the loan" (the button after a restart). */
    @Test
    fun an_empty_token_uses_the_grant_the_session_already_has() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.status = FakeRemoteService.statusOf(
            role = "controller",
            grant = FakeRemoteService.grant("queue"),
            phase = RemotePhase.LIVE,
            online = true,
        )
        val (model, _) = model(service)
        model.claim("")
        runCurrent()
        assertEquals(ClaimPhase.Connected, model.state.value.claim)
        assertFalse(service.calls.any { it.startsWith("claim") })
        assertEquals("File d'attente", model.state.value.grant?.levelLabel)
    }

    /** A refusal (level, offline) changes nothing and offers « Réessayer ». */
    @Test
    fun a_refused_command_says_why_and_offers_a_retry() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.grant = FakeRemoteService.grant("queue")
        service.failSend = "Ce lien ne permet que la lecture et la file d'attente"
        service.now = FakeRemoteService.nowOf(playing = false)
        val (model, toasts) = model(service)
        model.claim("abcd1234efgh5678.abcdefghijklmnopqrstuvwx")
        runCurrent()

        model.send(RemoteCommand.Shuffle)
        runCurrent()
        val toast = toasts.last()
        assertEquals("Ce lien ne permet que la lecture et la file d'attente", toast.first)
        assertTrue(toast.second)

        service.failSend = null
        model.send(RemoteCommand.Pause)
        runCurrent()
        assertEquals(listOf<RemoteCommand>(RemoteCommand.Pause), service.sent)
    }

    /** What Rust pushes: a controller arrived, then the loan ended. */
    @Test
    fun the_status_and_the_end_of_a_loan_are_pushed_not_polled() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.grant = FakeRemoteService.grant("read")
        val (model, _) = model(service)
        model.open()
        runCurrent()

        service.push(
            FakeRemoteService.statusOf(
                role = "controlled",
                who = "Contrôlé par Léa",
                controllers = listOf(RemoteControllerStub()),
            ),
        )
        assertEquals("Contrôlé par Léa", model.state.value.status?.who)
        assertTrue(model.state.value.status!!.driven)

        service.endLoan("friend")
        assertEquals("friend", model.state.value.ended)
        model.close()
    }

    /** The levels the sheet offers are the SERVICE's (no `admin` for a member). */
    @Test
    fun the_sheet_offers_the_levels_the_core_allows() = runTest(dispatcher) {
        val service = FakeRemoteService()
        val (model, _) = model(service)
        model.open()
        runCurrent()
        assertEquals(listOf("queue", "read", "full"), model.state.value.levels.map { it.id })
        assertEquals("le premier niveau est choisi", "queue", model.state.value.level)
        assertEquals(listOf("1h", "1d", "7d", "none"), model.state.value.durations.map { it.id })
        model.close()
    }

    /** Leaving hands the account back, and the banner says whose. */
    @Test
    fun leaving_says_which_account_comes_back() = runTest(dispatcher) {
        val service = FakeRemoteService()
        service.grant = FakeRemoteService.grant("queue")
        val (model, toasts) = model(service)
        model.claim("abcd1234efgh5678.abcdefghijklmnopqrstuvwx")
        runCurrent()
        model.leave()
        runCurrent()
        assertEquals(1, service.left)
        assertEquals(ClaimPhase.Idle, model.state.value.claim)
        assertNull(model.state.value.control)
        assertTrue(toasts.any { it.first.contains("friend") })
    }

    /** The invite text is what « Partager » hands out, URL included. */
    @Test
    fun the_invite_text_names_the_level_and_carries_the_link() = runTest(dispatcher) {
        val service = FakeRemoteService()
        val (model, _) = model(service)
        val row = FakeRemoteService.link(id = "abc", level = "full", label = "Léa").toRow()
        val text = model.inviteText(row)
        assertTrue(text, text.contains("complet"))
        assertTrue(text, text.contains("https://neezer.test/rc/abc"))
        model.close()
    }
}

private fun RemoteControllerStub() = uniffi.nz_ffi.RemoteController(
    link = "a",
    level = "read",
    levelLabel = "Lecture seule",
    label = "Léa",
)
