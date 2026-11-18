package fr.neezer.mobile.feature.genres

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.GenreMeasure

/**
 * Les règles du studio, prouvées sur la JVM (lot MB18).
 *
 * Ce que ces tests comptent, ce sont des NOMBRES : les requêtes d'une ouverture
 * (les DEUX de la page, puis zéro à chaud), une écriture par clic, et le fait
 * qu'un arbre de 1 000 nœuds ne demande que les pages qu'il peint.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class GenresModelTest {
    private fun model(
        service: FakeGenresService,
        scope: TestScope,
        toasts: MutableList<String> = mutableListOf(),
    ) = GenresModel(
        service = service,
        scope = scope,
        io = UnconfinedTestDispatcher(scope.testScheduler),
        toast = { toasts += it },
    )

    /** Ouvrir le studio : les deux requêtes de la SPA, et rien de plus. */
    @Test
    fun openingTheStudioCostsThePagesTwoRequests() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        assertEquals(
            listOf("GET /genre/status", "GET /genre/candidates?sort=plays"),
            service.calls,
        )
        val state = model.state
        assertEquals(7, state.tags.size)
        assertEquals(74, state.labelled)
        assertEquals(2, state.candidates.data?.rows?.size)
        assertNotNull(state.current)
        assertEquals("1001", state.current?.reference)
    }

    /**
     * Une réouverture chaude PEINT sans attendre : la copie de l'appareil est là
     * avant que le réseau ait répondu (le rafraîchissement, lui, part derrière).
     */
    @Test
    fun aWarmOpenPaintsFromTheDeviceCopyBeforeTheNetwork() = runTest {
        val service = FakeGenresService()
        val first = model(service, this)
        first.open()
        advanceUntilIdle()
        service.calls.clear()
        // The network is held: whatever the screen shows now came from the copy.
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        service.gateStudio = gate
        val second = model(service, this)
        second.open()
        advanceUntilIdle()
        assertEquals(7, second.state.studio.data?.tags?.size)
        assertTrue("the copy is flagged as one: ${second.state.studio}", second.state.studio.stale)
        assertEquals(2, second.state.candidates.data?.rows?.size)
        assertTrue(service.calls.contains("GET /genre/status"))
        gate.complete(Unit)
        advanceUntilIdle()
        assertTrue("the fresh answer replaced the copy", !second.state.studio.stale)
    }

    /** Neuf gros boutons au plus, dans l'ordre du noyau. */
    @Test
    fun thePicksAreTheNucleusRanking() = runTest {
        val service = FakeGenresService()
        service.seedRanking()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        val picks = model.state.picks
        assertTrue(picks.size in 1..9)
        assertEquals("Hardtek", picks.first().name)
        assertEquals(0.52, picks.first().score!!, 1e-9)
    }

    /** Un clic : la ligne part, le compte bouge, une seule requête. */
    @Test
    fun aLabelIsOptimisticAndOneRequest() = runTest {
        val service = FakeGenresService()
        service.seedRanking()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        val tag = model.state.picks.first { it.name == "Hardtek" }
        model.label(tag)
        // AVANT la requête : la ligne a quitté la page.
        assertEquals(1, model.state.candidates.data?.rows?.size ?: -1)
        advanceUntilIdle()
        assertEquals(listOf<Pair<String, Long?>>("1001" to 4L), service.labels)
        assertEquals(1, service.calls.count { it.startsWith("POST /genre/label") })
        assertEquals("6 + 1 tags", 7, model.state.countOf("Hardtek"))
        assertEquals(75, model.state.labelled)
    }

    /** Un refus remet la ligne où elle était et dit la cause du serveur. */
    @Test
    fun aRefusalPutsTheRowBack() = runTest {
        val service = FakeGenresService()
        service.seedRanking()
        service.failures["label"] = "Le serveur a refusé cette étiquette."
        val toasts = mutableListOf<String>()
        val model = model(service, this, toasts)
        model.open()
        advanceUntilIdle()
        val before = model.state.candidates.data?.rows?.size ?: 0
        model.label(model.state.picks.first())
        advanceUntilIdle()
        assertEquals(before, model.state.candidates.data?.rows?.size)
        assertEquals(0, model.state.cursor)
        assertEquals("Le serveur a refusé cette étiquette.", model.state.actionError)
        assertTrue(toasts.any { it.contains("refusé") })
        assertNull(model.state.last)
    }

    /** Annuler : une requête, et le compte revient. */
    @Test
    fun undoIsOneWrite() = runTest {
        val service = FakeGenresService()
        service.seedRanking()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        model.label(model.state.picks.first())
        advanceUntilIdle()
        service.calls.clear()
        model.undo()
        // The row is back BEFORE the request — undo is as optimistic as tagging.
        assertEquals(2, model.state.candidates.data?.rows?.size ?: -1)
        advanceUntilIdle()
        assertEquals(listOf("POST /genre/label"), service.calls)
        assertEquals("1001" to null, service.labels.last() as Pair<String, Long?>)
        assertEquals(6, model.state.countOf("Hardtek"))
    }

    /** Changer d'ordre REINTERROGE : les titres incertains sont un autre ensemble. */
    @Test
    fun switchingTheOrderAsksAgain() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        service.calls.clear()
        model.setSort(GenresModel.SORT_ACTIVE)
        advanceUntilIdle()
        assertEquals(listOf("GET /genre/candidates?sort=active"), service.calls)
    }

    /** L'arbre : une lecture, puis un aplatissement local à chaque branche. */
    @Test
    fun theTreeIsFlattenedLocally() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        service.calls.clear()
        model.selectTab(GenresCopy.TAB_TREE)
        advanceUntilIdle()
        assertEquals(listOf("GET /genre/tree"), service.calls)
        assertEquals(5, model.state.treeRows?.len())
        service.calls.clear()
        model.toggleRow("Hardcore")
        advanceUntilIdle()
        assertTrue("toggling a branch must ask nothing: ${service.calls}", service.calls.isEmpty())
        // The fake's rows do not depend on `expanded`; the REAL flattening is the
        // nucleus's, measured in `nz-core`'s `the_tree_rows_are_local`.
        assertNotNull(model.state.treeRows)
    }

    /** Une écriture d'arbre peint la réponse du serveur, jamais une supposition. */
    @Test
    fun aTreeWritePaintsTheServersAnswer() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.open()
        service.calls.clear()
        model.selectTab(GenresCopy.TAB_TREE)
        advanceUntilIdle()
        service.calls.clear()
        model.treeAdd("Hardstyle", "Hardcore")
        advanceUntilIdle()
        assertEquals(listOf("POST /genre/tree/edge"), service.calls)
        assertEquals(6u, model.state.tree.data?.edges)
        assertTrue(model.state.tree.data?.canUndo == true)
    }

    /**
     * Le modèle : la copie peint d'abord, puis TROIS lectures (reclassement, job
     * d'empreintes, couverture) — les trois cartes de la page.
     */
    @Test
    fun theModelTabPaintsTheCopyThenReadsThree() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        service.calls.clear()
        model.selectTab(GenresCopy.TAB_MODEL)
        advanceUntilIdle()
        assertEquals(
            listOf("GET /genre/relabel", "GET /genre/embed", "GET /genre/construction"),
            service.calls,
        )
        assertEquals(1u, model.state.model.data?.embed?.failed)
        val again = model(service, this)
        again.open()
        advanceUntilIdle()
        service.calls.clear()
        // The network is held: whatever the tab shows came from the copy.
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        service.gateModelTab = gate
        again.selectTab(GenresCopy.TAB_MODEL)
        advanceUntilIdle()
        assertNotNull("the model tab paints from the copy", again.state.model.data)
        assertEquals(50u, again.state.model.data?.construction?.measured)
        assertTrue("the copy is flagged as one", again.state.model.stale)
        gate.complete(Unit)
        advanceUntilIdle()
        assertEquals(3, service.calls.size)
        assertTrue("the fresh answer replaced the copy", !again.state.model.stale)
    }

    /** L'entraînement rend la main, sa progression arrive par le fil, sa tête aussi. */
    @Test
    fun trainingReportsThroughItsListener() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.open()
        model.selectTab(GenresCopy.TAB_MODEL)
        advanceUntilIdle()
        model.setMode("linear")
        model.setSelfTrain(false)
        model.train()
        advanceUntilIdle()
        assertEquals(listOf("linear" to false), service.trainings)
        service.reportProgress("entraînement", 0.4)
        assertEquals("entraînement", model.state.training.stage)
        assertEquals(0.4, model.state.training.fraction, 1e-9)
        service.finishTraining(ok = true, head = FakeGenresService.head())
        assertEquals("terminé", model.state.training.stage)
        assertEquals(0.91, model.state.training.head!!.balanced, 1e-9)
        assertEquals("good", model.state.training.head!!.healthLevel)
    }

    /** Un entraînement qui échoue porte sa CAUSE, en français. */
    @Test
    fun aFailedTrainingKeepsItsCause() = runTest {
        val service = FakeGenresService()
        val toasts = mutableListOf<String>()
        val model = model(service, this, toasts)
        model.open()
        model.selectTab(GenresCopy.TAB_MODEL)
        advanceUntilIdle()
        model.train()
        advanceUntilIdle()
        service.finishTraining(ok = false, cause = "aucun titre étiqueté : étiquetez d'abord quelques titres")
        assertEquals(
            "aucun titre étiqueté : étiquetez d'abord quelques titres",
            model.state.training.cause,
        )
        assertTrue(toasts.any { it.contains("étiquetez") })
    }

    /** L'aperçu d'un lot : un désaccord arrive DÉCOCHÉ. */
    @Test
    fun theBulkPreviewArrivesPartiallyTicked() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.openBulk(tag = "Frenchcore", scopeLabel = "Album « Al »", album = "42")
        advanceUntilIdle()
        assertEquals(setOf("2001"), model.state.bulkTicked)
        assertEquals(3, model.state.bulk.data?.rows?.size)
        model.toggleBulk("2002")
        model.applyBulk()
        advanceUntilIdle()
        assertEquals(listOf(listOf("2001", "2002") to "Frenchcore"), service.applied)
        assertNull(model.state.sheet)
    }

    /** Règle 1 : l'écran transmet le drapeau au noyau, tel quel. */
    @Test
    fun theYieldingFlagReachesTheCore() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.setYielding(true)
        model.setYielding(false)
        assertEquals(listOf(true, false), service.yielding)
    }

    /** La mesure locale se conduit par le noyau : elle démarre, puis on lit son état. */
    @Test
    fun theLocalMeasureRunsFromTheCore() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.open()
        model.startMeasure()
        // Let the run start and its first poll happen, then let the CORE finish.
        advanceTimeBy(1)
        assertEquals(1, service.measures)
        service.finishMeasure(done = 3, failed = 1, total = 4, cause = "fichier tronqué")
        advanceUntilIdle()
        assertEquals(3u, model.state.measure?.done)
        assertEquals(1u, model.state.measure?.failed)
        assertEquals("fichier tronqué", model.state.measure?.causes?.first()?.reason)
        assertTrue("the poll stops when the run ends", model.state.measure?.running == false)
    }

    /** Le nom d'un genre vide est refusé sans requête. */
    @Test
    fun anEmptyGenreNameIsRefusedLocally() = runTest {
        val service = FakeGenresService()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        service.calls.clear()
        model.createTag("   ")
        advanceUntilIdle()
        assertTrue(service.calls.isEmpty())
        assertEquals(GenresCopy.NO_TAG_NAME, model.state.actionError)
    }

    /** Taper dans la recherche ne coûte aucune requête : le noyau re-range. */
    @Test
    fun searchingRanksLocally() = runTest {
        val service = FakeGenresService()
        service.seedRanking()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        service.calls.clear()
        model.setQuery("french")
        assertEquals(1, model.state.ranked.size)
        assertEquals("Frenchcore", model.state.ranked.first().name)
        assertTrue(service.calls.isEmpty())
    }

    /** La feuille d'étiquette d'un titre : une lecture, puis le rang local. */
    @Test
    fun theTrackSheetIsOneRead() = runTest {
        val service = FakeGenresService()
        service.seedRanking()
        val model = model(service, this)
        model.open()
        advanceUntilIdle()
        service.calls.clear()
        model.openSheet("1001", "Premier titre")
        advanceUntilIdle()
        assertEquals(listOf("GET /genre/track/1001"), service.calls)
        assertEquals("Hardtek", model.state.sheetTagId?.let { id ->
            model.state.sheetRanked.firstOrNull { it.id.toLong() == id }?.name
        })
    }
}
