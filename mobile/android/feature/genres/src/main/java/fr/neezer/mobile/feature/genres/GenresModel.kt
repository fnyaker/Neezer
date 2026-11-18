package fr.neezer.mobile.feature.genres

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.GenreBulkPreview
import uniffi.nz_ffi.GenreCandidates
import uniffi.nz_ffi.GenreEmbedJob
import uniffi.nz_ffi.GenreExtractorTest
import uniffi.nz_ffi.GenreMeasure
import uniffi.nz_ffi.GenreMeasureListener
import uniffi.nz_ffi.GenreModelTab
import uniffi.nz_ffi.GenreRankedTag
import uniffi.nz_ffi.GenreRelabelJob
import uniffi.nz_ffi.GenreStudio
import uniffi.nz_ffi.GenreTagRow
import uniffi.nz_ffi.GenreTraining
import uniffi.nz_ffi.GenreTree

/**
 * L'état du studio des genres (lot MB18).
 *
 * Les règles de la SPA, dans l'ordre où la personne les rencontre, en Kotlin
 * ordinaire au-dessus de [GenresService] — `GenresModelTest` les prouve sur la
 * JVM avec un faux, sans JNA, sans `.so`, sans appareil.
 *
 * - **Ouvrir le studio coûte les DEUX requêtes de la page** (`/genre/status` puis
 *   `/genre/candidates`) et une réouverture chaude n'en coûte AUCUNE : la copie de
 *   l'appareil peint d'abord (`screenFlow`), le réseau la remplace.
 * - **Étiqueter est un rythme, pas un formulaire.** Un clic fait disparaître la
 *   ligne et bouge le compte AVANT la requête ; un refus la remet où elle était et
 *   dit la phrase du serveur. Annuler coûte une clé, jamais une chasse.
 * - **Le classement vit dans le noyau** (`ranked`) : l'ordre décide quel genre
 *   `Entrée` prend, et deux appareils ne peuvent pas en inventer deux.
 * - **Chaque écriture d'arbre renvoie l'arbre ENTIER**, que l'écran peint — jamais
 *   sa supposition. Les lignes viennent d'un `ListHandle` (règle 4) : un arbre de
 *   1 000 nœuds ne traverse pas la frontière en entier.
 * - **Le travail long dit où il en est** (règle 2) : l'entraînement rend la main,
 *   sa progression arrive par le fil qui l'exécute, et son échec porte sa CAUSE.
 * - **Règle 1** : l'écran lève [setYielding] pendant qu'un titre démarre ; les
 *   deux travaux se garent alors.
 */
class GenresModel(
    private val service: GenresService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
    private val causeText: (Throwable) -> String = { NzErrors.text(it) },
    /** Un travail du serveur se sonde toutes les 400 ms, et seulement tant qu'il travaille. */
    private val pollMs: Long = 400,
) {
    /** Un écran, ses quatre états (règle 3). */
    data class Section<T>(
        val data: T? = null,
        val loading: Boolean = false,
        val stale: Boolean = false,
        val failure: String? = null,
    ) {
        val ready: Boolean get() = data != null && failure == null
    }

    /**
     * La dernière étiquette posée, pour l'annulation à une clé : la LIGNE entière,
     * pour qu'annuler la remette à sa place sans une requête (`Genres.svelte#undo`).
     */
    data class LastLabel(
        val candidate: uniffi.nz_ffi.GenreCandidate,
        val tagId: Long,
        val tagName: String,
        val at: Int,
        val rewritten: Boolean,
    )

    /** La feuille ouverte par-dessus l'écran. */
    sealed interface Sheet {
        /** L'étiquette d'UN titre (le menu d'un titre, MB10). */
        data class Tag(val reference: String, val title: String) : Sheet

        /** L'aperçu d'un lot (album, playlist, artiste). */
        data class Bulk(val tag: String, val scopeLabel: String) : Sheet
    }

    data class State(
        val tab: String = GenresCopy.TAB_TAG,
        val studio: Section<GenreStudio> = Section(),
        val candidates: Section<GenreCandidates> = Section(),
        /** L'ordre demandé au serveur : `plays` ou `active`. */
        val sort: String = SORT_PLAYS,
        val cursor: Int = 0,
        /** Le vocabulaire rangé pour le titre courant (le noyau le range). */
        val ranked: List<GenreRankedTag> = emptyList(),
        val query: String = "",
        val last: LastLabel? = null,
        val busy: Boolean = false,
        val actionError: String? = null,
        // -- l'arbre ---------------------------------------------------------
        val tree: Section<GenreTree> = Section(),
        val treeExpanded: Set<String> = emptySet(),
        val treeQuery: String = "",
        val treeRows: TreeRows? = null,
        // -- le modèle -------------------------------------------------------
        val model: Section<GenreModelTab> = Section(),
        val training: GenreTraining = GenreTraining(
            running = false,
            stage = "",
            fraction = 0.0,
            cause = null,
            mode = "auto",
            selfTrain = true,
            examples = 0u,
            pool = 0u,
            head = null,
        ),
        val mode: String = "auto",
        val selfTrain: Boolean = true,
        val extractorTest: GenreExtractorTest? = null,
        val measure: GenreMeasure? = null,
        val relabelRunning: Boolean = false,
        // -- les feuilles ----------------------------------------------------
        val sheet: Sheet? = null,
        val sheetRanked: List<GenreRankedTag> = emptyList(),
        val sheetQuery: String = "",
        val sheetReference: String? = null,
        val sheetTitle: String = "",
        val sheetTagId: Long? = null,
        val bulk: Section<GenreBulkPreview> = Section(),
        val bulkTicked: Set<String> = emptySet(),
        val bulkApplying: Boolean = false,
    ) {
        val tags: List<GenreTagRow> get() = studio.data?.tags ?: emptyList()

        val current: uniffi.nz_ffi.GenreCandidate?
            get() = candidates.data?.rows?.getOrNull(cursor)

        /** Les neuf gros boutons : le rang du titre, pas l'alphabet. */
        val picks: List<GenreRankedTag> get() = ranked.take(9)

        val labelled: Int get() = studio.data?.labelled?.toInt() ?: 0

        /** Le genre que le modèle propose pour le titre courant, s'il en a un. */
        val suggestion: GenreRankedTag?
            get() = ranked.firstOrNull()?.takeIf { it.score != null }

        /** Un genre n'est apprenable qu'à partir de cinq titres (`MIN_EXAMPLES`). */
        val eligible: List<GenreTagRow>
            get() = tags.filter { it.count.toInt() >= MIN_EXAMPLES }

        val trainable: Boolean get() = eligible.size >= 2

        /** La courbe d'avant-entraînement : les genres qui ont des exemples, du plus plein. */
        val readiness: List<Pair<String, Int>>
            get() = tags
                .map { it.name to it.count.toInt() }
                .filter { it.second > 0 }
                .sortedWith(compareByDescending<Pair<String, Int>> { it.second }.thenBy { it.first })

        val sheetTags: List<GenreRankedTag> get() = sheetRanked

        fun countOf(name: String): Int =
            tags.firstOrNull { it.name == name }?.count?.toInt() ?: 0
    }

    var state by mutableStateOf(State())
        private set

    private var opened = false
    private var candidatesJob: Job? = null
    private var pollJob: Job? = null
    private var measureJob: Job? = null

    // --- opening ------------------------------------------------------------

    /** Ouvre le studio : la copie de l'appareil d'abord, le réseau ensuite. */
    fun open() {
        if (opened) return
        opened = true
        subscribe()
        loadStudio()
        loadCandidates()
    }

    private fun loadStudio() {
        scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedStudio() } },
                fresh = { withContext(io) { service.studio() } },
            ).collect { applyStudio(it) }
        }
    }

    private fun applyStudio(incoming: ScreenState<GenreStudio>) {
        state = when (incoming) {
            is ScreenState.Loading -> state.copy(studio = Section(loading = true))
            is ScreenState.Ready -> state.copy(
                studio = Section(data = incoming.data, stale = incoming.stale),
                ranked = rankCurrent(incoming.data),
            )
            is ScreenState.Failed -> state.copy(
                studio = Section(failure = incoming.cause),
            )
        }
    }

    /** `listOf(...)` — l'ordre du noyau pour le titre courant, à la frappe près. */
    fun loadCandidates() {
        candidatesJob?.cancel()
        candidatesJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedCandidates(state.sort) } },
                fresh = { withContext(io) { service.candidates(state.sort, null) } },
            ).collect { incoming ->
                state = when (incoming) {
                    is ScreenState.Loading -> state.copy(candidates = Section(loading = true))
                    is ScreenState.Ready -> state.copy(
                        candidates = Section(data = incoming.data, stale = incoming.stale),
                        cursor = state.cursor.coerceIn(0, (incoming.data.rows.size - 1).coerceAtLeast(0)),
                        ranked = rankCurrent(state.studio.data),
                    )
                    is ScreenState.Failed -> state.copy(
                        candidates = Section(failure = incoming.cause),
                    )
                }
            }
        }
    }

    /** `ranked(reference, query)` : une seule implémentation, dans le noyau. */
    private fun rankCurrent(studio: GenreStudio?, query: String = state.query): List<GenreRankedTag> {
        val reference = state.candidates.data?.rows?.getOrNull(state.cursor)?.reference ?: return emptyList()
        return runCatching { service.ranked(reference, query) }.getOrDefault(emptyList())
    }

    // --- the tagging flow ---------------------------------------------------

    /** L'ordre des candidats : `plays` reste le défaut, `active` trie par incertitude. */
    fun setSort(sort: String) {
        if (sort == state.sort) return
        state = state.copy(sort = sort, cursor = 0, last = null)
        loadCandidates()
    }

    /** La recherche du genre à appliquer : le noyau re-range, le champ ne filtre pas. */
    fun setQuery(query: String) {
        state = state.copy(query = query, ranked = rankCurrent(state.studio.data, query))
    }

    /** Un clic, une requête : la ligne part AVANT la requête (optimiste). */
    fun label(tag: GenreRankedTag) {
        val candidate = state.current ?: return
        val at = state.cursor
        // Ré-étiqueter le MÊME titre ne compte pas un titre de plus.
        val rewritten = state.last?.candidate?.reference == candidate.reference
        val page = state.candidates.data
        val rows = page?.rows?.filter { it.reference != candidate.reference }.orEmpty()
        state = state.copy(
            candidates = if (page != null) {
                state.candidates.copy(data = page.copy(rows = rows))
            } else {
                state.candidates
            },
            cursor = at.coerceIn(0, (rows.size - 1).coerceAtLeast(0)),
            query = "",
            busy = true,
            last = LastLabel(
                candidate = candidate,
                tagId = tag.id,
                tagName = tag.name,
                at = at,
                rewritten = rewritten,
            ),
        )
        state = state.copy(ranked = rankCurrent(state.studio.data, ""))
        scope.launch {
            try {
                withContext(io) { service.label(candidate.reference, tag.id) }
                bump(tag.name, +1, !rewritten)
                state = state.copy(busy = false, actionError = null)
                if (state.candidates.data?.rows.isNullOrEmpty()) loadCandidates()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Le refus remet la ligne où elle était, et dit pourquoi.
                restore(candidate, at)
                state = state.copy(busy = false, actionError = causeText(e), last = null)
                toast(causeText(e))
            }
        }
    }

    /**
     * Annuler la dernière étiquette : une clé, UNE requête, et la ligne revient à
     * sa place — la page ne redemande pas la liste (`Genres.svelte#undo`).
     */
    fun undo() {
        val last = state.last ?: return
        state = state.copy(last = null, busy = true)
        bump(last.tagName, -1, !last.rewritten)
        restore(last.candidate, last.at)
        scope.launch {
            try {
                withContext(io) { service.label(last.candidate.reference, null) }
                state = state.copy(busy = false, actionError = null)
                toast(GenresCopy.undone(last.candidate.title))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Le refus remet la ligne DEHORS, et le compte avec.
                bump(last.tagName, +1, !last.rewritten)
                state = state.copy(busy = false, actionError = causeText(e), last = last)
                toast(causeText(e))
            }
        }
    }

    fun skip() {
        val size = state.candidates.data?.rows?.size ?: 0
        if (size == 0) return
        state = state.copy(
            cursor = (state.cursor + 1).coerceAtMost(size - 1),
            query = "",
            ranked = rankCurrent(state.studio.data, ""),
        )
    }

    fun back() {
        if (state.cursor == 0) return
        state = state.copy(
            cursor = state.cursor - 1,
            query = "",
            ranked = rankCurrent(state.studio.data, ""),
        )
    }

    private fun restore(candidate: uniffi.nz_ffi.GenreCandidate, at: Int) {
        val page = state.candidates.data ?: return
        val rows = page.rows.toMutableList()
        val where = at.coerceIn(0, rows.size)
        rows.add(where, candidate)
        state = state.copy(
            candidates = state.candidates.copy(data = page.copy(rows = rows)),
            cursor = where,
        )
    }

    private fun bump(name: String, by: Int, labelledToo: Boolean) {
        val studio = state.studio.data ?: return
        val tags = studio.tags.map { tag ->
            if (tag.name == name) tag.copy(count = (tag.count.toInt() + by).coerceAtLeast(0).toUInt()) else tag
        }
        state = state.copy(
            studio = state.studio.copy(
                data = studio.copy(
                    tags = tags,
                    labelled = (studio.labelled.toInt() + if (labelledToo) by else 0)
                        .coerceAtLeast(0).toUInt(),
                ),
            ),
        )
    }

    // --- the vocabulary editor ----------------------------------------------

    /** `POST /genre/tags` puis le vocabulaire du serveur (jamais une supposition). */
    fun createTag(name: String, color: String? = null, archetype: String? = null) {
        if (name.isBlank()) {
            state = state.copy(actionError = GenresCopy.NO_TAG_NAME)
            return
        }
        write { service.createTag(name.trim(), color, archetype) }
    }

    fun editTag(id: Long, name: String? = null, color: String? = null, archetype: String? = null) {
        write { service.editTag(id, name, color, archetype) }
    }

    fun deleteTag(id: Long) = write { service.deleteTag(id) }

    fun restoreDefaults() = write { service.defaults() }

    private fun write(action: suspend () -> GenreStudio) {
        if (state.busy) return
        state = state.copy(busy = true)
        scope.launch {
            try {
                val studio = withContext(io) { action() }
                state = state.copy(
                    studio = Section(data = studio),
                    busy = false,
                    actionError = null,
                )
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(busy = false, actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    // --- the tree ------------------------------------------------------------

    fun loadTree() {
        if (state.tree.loading) return
        state = state.copy(tree = state.tree.copy(loading = state.tree.data == null))
        scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedTree() } },
                fresh = { withContext(io) { service.tree() } },
            ).collect { incoming ->
                state = when (incoming) {
                    is ScreenState.Loading -> state.copy(tree = Section(loading = true))
                    is ScreenState.Ready -> {
                        val roots = if (state.treeExpanded.isEmpty()) incoming.data.roots.toSet() else state.treeExpanded
                        state.copy(
                            tree = Section(data = incoming.data, stale = incoming.stale),
                            treeExpanded = roots,
                        )
                    }
                    is ScreenState.Failed -> state.copy(tree = Section(failure = incoming.cause))
                }
                rebuildTree()
            }
        }
    }

    /** Ouvre ou ferme une branche : ZÉRO requête, le noyau re-aplatit. */
    fun toggleRow(key: String) {
        val expanded = state.treeExpanded.toMutableSet()
        if (!expanded.add(key)) expanded.remove(key)
        state = state.copy(treeExpanded = expanded)
        rebuildTree()
    }

    fun setTreeQuery(query: String) {
        state = state.copy(treeQuery = query)
        rebuildTree()
    }

    private fun rebuildTree() {
        val rows = runCatching {
            service.treeList(state.treeExpanded.toList(), state.treeQuery)
        }.getOrNull()
        state = state.copy(treeRows = rows)
    }

    fun treeAdd(child: String, parent: String, weight: Double = 1.0) =
        treeWrite { service.treeAdd(child, parent, weight) }

    fun treeCut(child: String, parent: String) = treeWrite { service.treeCut(child, parent) }

    fun treeMove(child: String, parent: String, oldParent: String?, weight: Double = 1.0) =
        treeWrite { service.treeMove(child, parent, oldParent, weight) }

    fun treeUndo() = treeWrite { service.treeUndo() }

    fun treeReset() = treeWrite { service.treeReset() }

    private fun treeWrite(action: suspend () -> GenreTree) {
        if (state.busy) return
        state = state.copy(busy = true)
        scope.launch {
            try {
                val tree = withContext(io) { action() }
                state = state.copy(
                    tree = Section(data = tree),
                    busy = false,
                    actionError = null,
                )
                rebuildTree()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(busy = false, actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    // --- the model tab -------------------------------------------------------

    fun loadModel() {
        if (state.model.loading) return
        state = state.copy(
            model = state.model.copy(loading = state.model.data == null),
            training = runCatching { service.training() }.getOrDefault(state.training),
        )
        scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedModelTab() } },
                fresh = { withContext(io) { service.modelTab() } },
            ).collect { incoming ->
                state = when (incoming) {
                    is ScreenState.Loading -> state.copy(model = Section(loading = true))
                    is ScreenState.Ready -> state.copy(
                        model = Section(data = incoming.data, stale = incoming.stale),
                        relabelRunning = incoming.data.relabel?.running == true,
                    )
                    is ScreenState.Failed -> state.copy(model = Section(failure = incoming.cause))
                }
            }
        }
    }

    fun setMode(mode: String) {
        state = state.copy(mode = mode)
    }

    fun setSelfTrain(on: Boolean) {
        state = state.copy(selfTrain = on)
    }

    /** Le lancement rend la main : la progression arrive par le fil d'entraînement. */
    fun train() {
        if (state.training.running) return
        scope.launch {
            try {
                withContext(io) { service.trainStart(state.mode, state.selfTrain) }
                state = state.copy(training = runCatching { service.training() }.getOrDefault(state.training))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    fun stopTraining() {
        runCatching { service.trainStop() }
    }

    /** Les deux abonnements du noyau : la progression, et la mesure locale. */
    private fun subscribe() {
        runCatching {
            service.subscribeTraining(object : uniffi.nz_ffi.GenreListener {
                override fun progress(stage: String, fraction: Double) {
                    state = state.copy(
                        training = state.training.copy(
                            running = true,
                            stage = stage,
                            fraction = fraction.coerceIn(0.0, 1.0),
                        ),
                    )
                }

                override fun finished(ok: Boolean, cause: String?) {
                    val head = runCatching { service.training() }.getOrNull()
                    state = state.copy(
                        training = head ?: state.training.copy(running = false, cause = cause),
                        actionError = cause,
                    )
                    if (!ok && cause != null) toast(cause)
                }
            })
            service.subscribeMeasure(object : GenreMeasureListener {
                override fun changed(state: GenreMeasure) {
                    this@GenresModel.state = this@GenresModel.state.copy(measure = state)
                }
            })
        }
        state = state.copy(training = runCatching { service.training() }.getOrDefault(state.training))
    }

    fun sendHead() {
        scope.launch {
            try {
                val sent = withContext(io) { service.sendHead() }
                state = state.copy(
                    actionError = null,
                    relabelRunning = sent.relabel?.running == true,
                )
                toast(
                    GenresCopy.countLabel(sent.labels.toInt(), "genre envoyé", "genres envoyés") +
                        " — il s'applique à toute la bibliothèque",
                )
                watchRelabel()
                loadStudio()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    fun disableHead() {
        scope.launch {
            try {
                withContext(io) { service.disableHead() }
                loadStudio()
                loadModel()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    fun relabelNow() {
        pollJob?.cancel()
        pollJob = scope.launch {
            try {
                var job = withContext(io) { service.relabelStart() }
                state = state.copy(relabelRunning = job.running)
                while (job.running) {
                    delay(pollMs)
                    job = withContext(io) { service.relabel() }
                    state = state.copy(relabelRunning = job.running)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(actionError = causeText(e))
            }
        }
    }

    /** Le reclassement se sonde SEULEMENT tant qu'il dit `running`. */
    private fun watchRelabel() {
        pollJob?.cancel()
        pollJob = scope.launch {
            try {
                var job = withContext(io) { service.relabel() }
                state = state.copy(relabelRunning = job.running)
                while (job.running) {
                    delay(pollMs)
                    job = withContext(io) { service.relabel() }
                    state = state.copy(relabelRunning = job.running)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // Un reclassement qu'on ne peut pas lire n'est pas une erreur d'écran.
            }
        }
    }

    fun startEmbed(force: Boolean = false) {
        pollJob?.cancel()
        pollJob = scope.launch {
            try {
                var job = withContext(io) { service.embedStart(force) }
                state = state.copy(model = state.model.copy(data = state.model.data?.copy(embed = job)))
                while (job.running) {
                    delay(pollMs)
                    job = withContext(io) { service.embed() }
                    state = state.copy(model = state.model.copy(data = state.model.data?.copy(embed = job)))
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    fun testExtractor() {
        scope.launch {
            try {
                val test = withContext(io) { service.extractorTestStart() }
                state = state.copy(extractorTest = test)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    /** La mesure locale : le noyau la conduit, l'écran affiche son état. */
    fun startMeasure() {
        measureJob?.cancel()
        measureJob = scope.launch {
            try {
                withContext(io) { service.measureStart() }
                var current = withContext(io) { service.measure() }
                state = state.copy(measure = current)
                while (current.running) {
                    delay(pollMs)
                    current = withContext(io) { service.measure() }
                    state = state.copy(measure = current)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    fun stopMeasure() {
        measureJob?.cancel()
        runCatching { service.measureStop() }
    }

    // --- the tag sheet of one track -----------------------------------------

    fun openSheet(reference: String, title: String) {
        state = state.copy(sheet = Sheet.Tag(reference, title))
        scope.launch {
            try {
                val sheet = withContext(io) { service.trackSheet(reference) }
                val ranked = runCatching {
                    service.ranked(reference, state.sheetQuery)
                }.getOrDefault(emptyList())
                state = state.copy(
                    sheetReference = reference,
                    sheetTitle = title,
                    sheetTagId = sheet.tag?.id,
                    sheetRanked = ranked,
                )
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(actionError = causeText(e))
                closeSheet()
                toast(causeText(e))
            }
        }
    }

    fun setSheetQuery(query: String) {
        val reference = state.sheetReference ?: return
        state = state.copy(
            sheetQuery = query,
            sheetRanked = runCatching { service.ranked(reference, query) }.getOrDefault(emptyList()),
        )
    }

    /** La feuille d'étiquette d'un titre : `null` retire le genre. */
    fun sheetLabel(tagId: Long?) {
        val reference = state.sheetReference ?: return
        val title = state.sheetTitle
        val previous = state.sheetTagId
        scope.launch {
            try {
                withContext(io) { service.label(reference, tagId) }
                state = state.copy(
                    sheetTagId = tagId,
                    sheetRanked = runCatching { service.ranked(reference, state.sheetQuery) }
                        .getOrDefault(state.sheetRanked),
                    actionError = null,
                )
                val name = tagId?.let { id -> state.sheetRanked.firstOrNull { it.id == id }?.name }
                toast(
                    if (name != null) GenresCopy.labelled(title, name) else GenresCopy.SHEET_REMOVE,
                )
                loadStudio()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(sheetTagId = previous, actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    fun closeSheet() {
        state = state.copy(
            sheet = null,
            sheetRanked = emptyList(),
            sheetQuery = "",
            sheetReference = null,
            sheetTitle = "",
            sheetTagId = null,
        )
    }

    // --- the bulk sheet ------------------------------------------------------

    /** L'aperçu d'un lot : un désaccord arrive DÉCOCHÉ (la règle de la page). */
    fun openBulk(tag: String, scopeLabel: String, album: String? = null, playlist: String? = null, artist: String? = null) {
        state = state.copy(sheet = Sheet.Bulk(tag, scopeLabel), bulk = Section(loading = true))
        scope.launch {
            try {
                val preview = withContext(io) {
                    service.bulkPreview(album = album, playlist = playlist, artist = artist, tag = tag)
                }
                state = state.copy(
                    bulk = Section(data = preview),
                    bulkTicked = preview.rows.filter { it.disagree.isEmpty() }.map { it.reference }.toSet(),
                )
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(bulk = Section(failure = causeText(e)))
            }
        }
    }

    fun toggleBulk(reference: String) {
        val ticked = state.bulkTicked.toMutableSet()
        if (!ticked.add(reference)) ticked.remove(reference)
        state = state.copy(bulkTicked = ticked)
    }

    /** Un clic, une écriture : les titres cochés, et rien d'autre. */
    fun applyBulk() {
        val sheet = state.sheet as? Sheet.Bulk ?: return
        if (state.bulkApplying || state.bulkTicked.isEmpty()) return
        val tracks = state.bulkTicked.toList()
        state = state.copy(bulkApplying = true)
        scope.launch {
            try {
                val applied = withContext(io) { service.bulkApply(tracks, sheet.tag) }
                state = state.copy(
                    bulkApplying = false,
                    sheet = null,
                    actionError = null,
                )
                toast(
                    GenresCopy.bulkApplied(applied.applied.toInt()) +
                        if (applied.missing > 0u) " · " + GenresCopy.bulkMissing(applied.missing.toInt()) else "",
                )
                loadStudio()
                loadCandidates()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                state = state.copy(bulkApplying = false, actionError = causeText(e))
                toast(causeText(e))
            }
        }
    }

    fun closeBulk() {
        state = state.copy(sheet = null, bulk = Section(), bulkTicked = emptySet())
    }

    // --- the tabs and the shell ----------------------------------------------

    fun selectTab(tab: String) {
        if (state.tab == tab) return
        state = state.copy(tab = tab)
        when (tab) {
            GenresCopy.TAB_TREE -> if (state.tree.data == null) loadTree()
            GenresCopy.TAB_MODEL -> loadModel()
            GenresCopy.TAB_LIBRARY -> Unit
        }
    }

    /** Règle 1 : dit au noyau de garer ses deux travaux. */
    fun setYielding(yielding: Boolean) {
        runCatching { service.setYielding(yielding) }
    }

    fun dismissError() {
        state = state.copy(actionError = null)
    }

    companion object {
        const val SORT_PLAYS = "plays"
        const val SORT_ACTIVE = "active"

        /** `health.js#MIN_EXAMPLES` : sous cinq titres un genre n'est pas apprenable. */
        const val MIN_EXAMPLES = 5
    }
}
