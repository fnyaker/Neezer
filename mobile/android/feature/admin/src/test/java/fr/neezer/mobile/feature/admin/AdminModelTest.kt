package fr.neezer.mobile.feature.admin

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Les règles des pages d'administration, sur la JVM : les comptes demandés (le
 * nombre que l'acceptation tient), les quatre états d'une carte (règle 3), le
 * clic optimiste et son retour arrière, une décision = une requête, et un
 * travail qui n'est interrogé que tant qu'il tourne (règle 2).
 *
 * Aucun JNA, aucun `.so`, aucun appareil : le faux est [FakeAdminService].
 */
@OptIn(ExperimentalCoroutinesApi::class)
class AdminModelTest {
    private val scopes = mutableListOf<CoroutineScope>()

    @After
    fun closeScreens() {
        scopes.forEach { it.cancel() }
        scopes.clear()
    }

    private fun TestScope.model(
        service: FakeAdminService,
        toasts: MutableList<String> = mutableListOf(),
    ): AdminModel {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val scope = CoroutineScope(dispatcher)
        scopes += scope
        return AdminModel(
            service = service,
            scope = scope,
            io = dispatcher,
            toast = { toasts.add(it) },
            clock = { 0L },
        )
    }

    /**
     * Le nombre que l'acceptation tient : *Utilisateurs* froid coûte les DEUX
     * lectures de la SPA (`/admin/users`, `/admin/invites`), et une ouverture
     * chaude peint la copie de l'appareil AVANT que le réseau ait répondu.
     */
    @Test
    fun a_cold_users_page_asks_two_routes_and_a_warm_one_paints_the_copy() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_USERS)

        assertEquals(
            "one request per card, exactly the SPA's own two",
            listOf("users", "invites"),
            service.calls,
        )
        assertEquals(2, model.state.rows.users().size)
        assertEquals("the count line says how many exist", 3, model.state.total)
        assertEquals("the cursor continues from the last row", "u-2", model.state.next)
        assertEquals(1, model.state.invites.size)

        // A warm open: the copy paints in the first frame, while the link is
        // still out — held at a gate, an event, never a sleep.
        val warmService = FakeAdminService(
            copy = AdminFixtures.usersPage(),
            copyInvites = listOf(AdminFixtures.invite()),
        )
        val gate = CompletableDeferred<Unit>()
        warmService.usersGate = gate
        val warm = model(warmService)
        warm.open(AdminModel.SECTION_USERS)
        assertNotNull("the copy paints first", warm.state.rows)
        assertTrue("and it is flagged as the device's copy", warm.state.usersStale)
        assertEquals(1, warmService.calls.count { it == "users" })
        gate.complete(Unit)
    }

    /**
     * Un membre n'obtient rien ET ne demande rien : `/api/me`'s `user.admin` est
     * lu de la session (zéro requête), et l'écran dit pourquoi la page est vide.
     */
    @Test
    fun a_member_gets_no_page_and_opens_no_socket() = runTest {
        val service = FakeAdminService(admin = false)
        val model = model(service)
        model.open(AdminModel.SECTION_USERS)
        model.open(AdminModel.SECTION_SERVER)

        assertEquals("not one request: the identity is the session's, in memory", emptyList<String>(), service.calls)
        assertTrue("and it was read once, from memory", service.identityReads >= 1)
        assertEquals(false, model.state.admin)
        assertTrue(model.state.rows.isEmpty())
        assertNull(model.state.server.data)
    }

    /** La recherche attend deux caractères et UN délai : une frappe = une requête. */
    @Test
    fun the_search_waits_for_two_characters_and_one_debounce() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_USERS)
        service.calls.clear()

        model.setQuery("L")
        advanceTimeBy(600)
        assertEquals("one character filters nothing and asks nothing", 0, service.calls.size)

        model.setQuery("Lé")
        advanceTimeBy(310)
        assertEquals("two characters: one search", listOf("users"), service.calls)

        service.calls.clear()
        model.setQuery("Léa")
        advanceTimeBy(100)
        model.setQuery("Léa ")
        advanceTimeBy(310)
        assertEquals("a thumb typing is ONE search", 1, service.calls.size)

        service.calls.clear()
        model.clearQuery()
        assertEquals("clearing the field re-reads the first page", listOf("users"), service.calls)
    }

    /** « Charger plus » continue le curseur au lieu de redemander la première page. */
    @Test
    fun load_more_continues_the_cursor_and_appends() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_USERS)
        service.calls.clear()

        model.loadMore()
        assertEquals(listOf("users"), service.calls)
        assertEquals(3, model.state.rows.users().size)
        assertNull("the last page has no cursor", model.state.next)
        assertFalse(model.state.showMore)
    }

    /**
     * Un rôle change DANS L'IMAGE : la ligne bouge avant que le serveur ait
     * répondu, et un refus la remet avec la phrase du serveur.
     */
    @Test
    fun a_role_moves_before_the_answer_and_comes_back_if_refused() = runTest {
        val service = FakeAdminService()
        val toasts = mutableListOf<String>()
        val gate = CompletableDeferred<Unit>()
        service.writeGate = gate
        val model = model(service, toasts)
        model.open(AdminModel.SECTION_USERS)
        val leo = model.state.rows.users().first { it.id == "u-2" }

        model.setRole(leo, true)
        assertTrue(
            "the row is an administrator before the server answered",
            model.state.rows.users().first { it.id == "u-2" }.admin,
        )
        assertTrue("the write is still in flight", model.state.busy)
        gate.complete(Unit)
        assertTrue(toasts.contains("Droits d'administrateur accordés"))
    }

    /** Un refus remet la ligne et nomme la cause. */
    @Test
    fun a_refused_role_rolls_the_row_back_with_its_cause() = runTest {
        val service = FakeAdminService()
        service.failures["setRole"] = IllegalStateException("Impossible : c'est le dernier administrateur actif.")
        val model = model(service)
        model.open(AdminModel.SECTION_USERS)
        val leo = model.state.rows.users().first { it.id == "u-2" }

        model.setRole(leo, true)
        assertFalse("the tap was rolled back", model.state.rows.users().first { it.id == "u-2" }.admin)
        assertEquals(
            "Impossible : c'est le dernier administrateur actif.",
            model.state.actionError,
        )
        assertFalse(model.state.busy)
    }

    /**
     * La suppression exige le NOM tapé (le serveur refuse autre chose), retire la
     * ligne et son total, et dit ce qui a été réattribué.
     */
    @Test
    fun deleting_an_account_needs_its_typed_name() = runTest {
        val service = FakeAdminService()
        val toasts = mutableListOf<String>()
        val model = model(service, toasts)
        model.open(AdminModel.SECTION_USERS)
        val leo = model.state.rows.users().first { it.id == "u-2" }
        service.calls.clear()

        model.confirmDelete(leo, "Léo ")
        assertEquals("the name matches once trimmed", listOf("deleteUser"), service.calls)
        assertEquals(1, model.state.rows.users().size)
        assertEquals(2, model.state.total)
        assertTrue(
            toasts.toString(),
            toasts.any { it.contains("4 fichiers réattribués") && it.contains("1 émission réattribuée") },
        )
    }

    /** Un nom qui ne correspond pas ne supprime rien — pas même une requête. */
    @Test
    fun a_wrong_typed_name_asks_nothing() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_USERS)
        val leo = model.state.rows.users().first { it.id == "u-2" }
        service.calls.clear()

        model.confirmDelete(leo, "Leo")
        model.confirmDelete(leo, "Léa")
        assertEquals(emptyList<String>(), service.calls)
        assertEquals(2, model.state.rows.users().size)
    }

    /** Révoquer retire la ligne tout de suite et le dit. */
    @Test
    fun revoking_an_invitation_removes_the_row_at_once() = runTest {
        val service = FakeAdminService()
        val toasts = mutableListOf<String>()
        val model = model(service, toasts)
        model.open(AdminModel.SECTION_USERS)
        val invite = model.state.invites.first()

        model.confirmRevoke(invite)
        assertTrue("the invitation left the table", model.state.invites.isEmpty())
        assertEquals(listOf("revokeInvite"), service.calls.filter { it == "revokeInvite" })
        assertTrue(toasts.contains("Invitation révoquée"))
    }

    /** Sans copie, la cause est nommée et « Réessayer » refait UNE tentative. */
    @Test
    fun without_a_copy_the_cause_is_named_and_retry_asks_again() = runTest {
        val service = FakeAdminService()
        service.failures["users"] = IllegalStateException("réseau indisponible")
        val model = model(service)
        model.open(AdminModel.SECTION_USERS)
        assertEquals("réseau indisponible", model.state.usersFailure)
        assertTrue("no account to draw", model.state.rows.users().isEmpty())

        service.failures.remove("users")
        service.calls.clear()
        model.retryUsers()
        assertEquals(listOf("users"), service.calls)
        assertNull(model.state.usersFailure)
        assertEquals(2, model.state.rows.users().size)
    }

    /**
     * Une panne avec une copie GARDE la copie et la signale : c'est la version
     * mémorisée, pas une erreur à crier (règle 3).
     */
    @Test
    fun a_failure_keeps_the_copy_and_flags_it() = runTest {
        val service = FakeAdminService(
            copy = AdminFixtures.usersPage(),
            copyInvites = listOf(AdminFixtures.invite()),
        )
        service.failures["users"] = IllegalStateException("hors ligne")
        val model = model(service)
        model.open(AdminModel.SECTION_USERS)

        assertEquals(2, model.state.rows.users().size)
        assertTrue("the copy is flagged", model.state.usersStale)
        assertNull("and it is not a failure to shout about", model.state.usersFailure)
    }

    /**
     * *Serveur* froid coûte les lectures de la SPA : ses dix routes plus les deux
     * de la carte d'entretien (`/analysis/plan`, `/analysis/backfill`) — ici 13
     * appels de service, dont l'un (`serverCard`) fait les 2 requêtes `/settings`
     * et `/deezer/status` que le noyau envoie.
     */
    @Test
    fun a_cold_server_page_asks_the_spas_reads() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_SERVER)

        assertEquals(
            setOf(
                "serverCard", "storage", "rules", "cleanup", "archiveStatus", "migrate",
                "plan", "analysis", "enrich", "credits", "similar", "automatch", "transcribe",
            ),
            service.calls.toSet(),
        )
        assertEquals(13, service.calls.size)
        assertNotNull(model.state.server.data)
        assertNotNull(model.state.storage.data)
        assertNotNull(model.state.rules.data)
        assertNotNull(model.state.cleanup.data)
        assertNotNull(model.state.plan.data)
        assertNotNull(model.state.automatch.data)
        assertNotNull(model.state.transcribe.data)
        assertEquals("12.5", model.state.quotaValue)
    }

    /**
     * Un travail lancé est interrogé TANT QU'IL TOURNE, et pas une fois de plus :
     * le faux finit au deuxième sondage, et le troisième n'existe pas.
     */
    @Test
    fun a_job_is_polled_only_while_it_runs() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_SERVER)
        service.calls.clear()

        model.startAnalysis(force = false, workers = 0u)
        assertEquals(listOf("startAnalysis"), service.calls)
        assertTrue(model.state.analysis.data?.running == true)

        advanceTimeBy(1_600)
        assertEquals("one poll", 1, service.calls.count { it == "analysis" })
        assertTrue("still running, so the screen says so", model.state.analysis.data?.running == true)

        advanceTimeBy(1_600)
        assertEquals("the second poll saw it end", 2, service.calls.count { it == "analysis" })
        assertFalse(model.state.analysis.data?.running == true)

        advanceTimeBy(10_000)
        assertEquals("and nothing ticks for a finished job", 2, service.calls.count { it == "analysis" })
    }

    /** Une règle = UNE décision = une requête, et le serveur la relit. */
    @Test
    fun one_rule_is_one_decision() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_SERVER)
        service.calls.clear()

        model.saveRule("on_fav_album", "true")
        assertEquals(listOf("saveRule:on_fav_album"), service.calls)
        assertTrue(
            "the card draws the server's own rule set",
            model.state.rules.data?.events?.first { it.id == "on_fav_album" }?.on == true,
        )
    }

    /** Le nettoyage : l'aperçu, puis le seul bouton qui supprime — et ce qu'il libère. */
    @Test
    fun the_cleanup_previews_then_frees_and_says_how_much() = runTest {
        val service = FakeAdminService()
        val toasts = mutableListOf<String>()
        val model = model(service, toasts)
        model.open(AdminModel.SECTION_SERVER)
        assertEquals(2_147_483_648uL, model.state.cleanup.data?.neededBytes)
        service.calls.clear()

        model.runCleanup()
        // The run, then what the preview says now, then the disk — the SPA's own
        // two reads after `runCleanup` (`Promise.all([loadCleanup, loadStorage])`).
        assertEquals(listOf("runCleanup", "cleanup", "storage"), service.calls)
        assertEquals(0uL, model.state.cleanup.data?.neededBytes)
        assertTrue(toasts.toString(), toasts.any { it.contains("2 fichier(s) supprimé(s)") })
    }

    /** Le quota est un champ, la transcription un champ : une écriture chacun. */
    @Test
    fun the_quota_and_the_transcription_save_one_field_each() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_SERVER)
        service.calls.clear()

        model.setQuotaDraft("25")
        model.saveQuota()
        assertEquals(listOf("saveQuota"), service.calls)
        assertEquals("25", model.state.quotaValue)

        service.calls.clear()
        model.saveTranscribe("max_minutes", "30")
        assertEquals(listOf("saveTranscribe:max_minutes"), service.calls)
        assertEquals(30u.toUInt(), model.state.transcribe.data?.maxMinutes)

        service.calls.clear()
        model.probeTranscribe()
        assertEquals("Tester le moteur runs the engine once", listOf("transcribe:probe"), service.calls)
    }

    /** L'auto-match se règle ET se lance ; l'arrêt est un appel de plus. */
    @Test
    fun the_automatch_switches_runs_and_stops() = runTest {
        val service = FakeAdminService()
        val model = model(service)
        model.open(AdminModel.SECTION_SERVER)
        service.calls.clear()

        model.setAutomatch(false)
        assertEquals(listOf("setAutomatch"), service.calls)
        assertFalse(model.state.automatch.data?.enabled == true)

        model.runAutomatch()
        assertTrue(model.state.automatch.data?.running == true)
        model.cancelAutomatch()
        assertTrue(model.state.automatch.data?.cancel == true)
    }
}
