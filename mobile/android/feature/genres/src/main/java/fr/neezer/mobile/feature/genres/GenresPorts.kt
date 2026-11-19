package fr.neezer.mobile.feature.genres

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.GenreBulkApplied
import uniffi.nz_ffi.GenreBulkPreview
import uniffi.nz_ffi.GenreCandidates
import uniffi.nz_ffi.GenreEmbedJob
import uniffi.nz_ffi.GenreExtractorTest
import uniffi.nz_ffi.GenreListener
import uniffi.nz_ffi.GenreMeasure
import uniffi.nz_ffi.GenreMeasureListener
import uniffi.nz_ffi.GenreModelTab
import uniffi.nz_ffi.GenreRankedTag
import uniffi.nz_ffi.GenreRelabelJob
import uniffi.nz_ffi.GenreSent
import uniffi.nz_ffi.GenreStudio
import uniffi.nz_ffi.GenreTrackSheet
import uniffi.nz_ffi.GenreTraining
import uniffi.nz_ffi.GenreTree
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.TrackRow

/**
 * Les lignes d'un arbre, telles que l'écran les fenêtre (règle 4).
 *
 * Le vrai chemin est un `ListHandle` : 1 000 nœuds restent dans Rust et l'écran
 * demande la page qu'il va peindre. Un faux n'a pas de handle (la JVM d'un test
 * n'a pas de `.so`) et fenêtre sa propre source : c'est ce qui rend la mesure de
 * l'acceptation (« 1 000 nœuds à p95 ≤ 16,7 ms ») exécutable sur la JVM.
 */
interface TreeRows {
    /** Le vrai handle, ou `null` pour un faux. */
    val handle: ListHandle?

    fun len(): Int

    fun window(start: Int, count: Int): List<TrackRow>

    fun version(): ULong
}

/** Le vrai handle de l'arbre, inchangé : les lignes ne traversent pas en entier. */
class HandleTreeRows(override val handle: ListHandle) : TreeRows {
    override fun len(): Int = handle.len().toInt()

    override fun window(start: Int, count: Int): List<TrackRow> =
        handle.window(start.coerceAtLeast(0).toUInt(), count.coerceAtLeast(0).toUInt())

    override fun version(): ULong = handle.version()
}

/**
 * Ce que le studio demande au noyau (lot MB18).
 *
 * Une interface, pas le `Screens` généré : la logique de l'écran est du Kotlin
 * ordinaire que `GenresModelTest` exécute sur la JVM avec un faux — ni JNA, ni
 * `.so`, ni appareil. [NzGenresService] est la seule implémentation réelle, une
 * délégation d'une ligne à `nz-ffi/src/genres.rs` (`impl Screens`).
 *
 * Chaque écran suit la même paire : `cachedX()` (le disque seul, pour la première
 * image) puis `x()` (le réseau). `screenFlow` les enchaîne. Le RANGEMENT de la
 * liste reste dans le noyau (`ranked`) : l'ordre décide quel genre `Entrée`
 * prend, donc il ne peut pas exister deux fois.
 */
interface GenresService {
    // -- le vocabulaire ------------------------------------------------------
    fun cachedStudio(): GenreStudio?
    suspend fun studio(): GenreStudio
    fun cachedCandidates(sort: String): GenreCandidates?
    suspend fun candidates(sort: String, last: String?): GenreCandidates
    fun ranked(reference: String, query: String): List<GenreRankedTag>
    suspend fun trackSheet(reference: String): GenreTrackSheet
    suspend fun label(reference: String, tag: Long?)

    /** `null` = le champ n'a pas changé (un `PATCH` ne porte que ce qui bouge). */
    suspend fun createTag(name: String, color: String?, archetype: String?): GenreStudio
    suspend fun editTag(
        id: Long,
        name: String? = null,
        color: String? = null,
        archetype: String? = null,
    ): GenreStudio
    suspend fun deleteTag(id: Long): GenreStudio
    suspend fun defaults(): GenreStudio

    // -- l'arbre ------------------------------------------------------------
    fun cachedTree(): GenreTree?
    suspend fun tree(): GenreTree

    /** Les lignes de l'arbre, déjà aplaties par le noyau (`expanded` = chemins ouverts). */
    fun treeList(expanded: List<String>, query: String): TreeRows
    suspend fun treeAdd(child: String, parent: String, weight: Double): GenreTree
    suspend fun treeCut(child: String, parent: String): GenreTree
    suspend fun treeMove(
        child: String,
        parent: String,
        oldParent: String?,
        weight: Double,
    ): GenreTree
    suspend fun treeUndo(): GenreTree
    suspend fun treeReset(): GenreTree

    // -- le lot -------------------------------------------------------------
    suspend fun bulkPreview(
        album: String? = null,
        playlist: String? = null,
        artist: String? = null,
        tag: String,
    ): GenreBulkPreview
    suspend fun bulkApply(tracks: List<String>, tag: String): GenreBulkApplied

    // -- le modèle ----------------------------------------------------------
    fun cachedModelTab(): GenreModelTab
    suspend fun modelTab(): GenreModelTab
    suspend fun relabel(): GenreRelabelJob
    suspend fun relabelStart(): GenreRelabelJob
    suspend fun embedStart(force: Boolean): GenreEmbedJob
    suspend fun embed(): GenreEmbedJob
    suspend fun extractorTest(): GenreExtractorTest
    suspend fun extractorTestStart(): GenreExtractorTest

    // -- l'entraînement (thread bulk, progression + causes) ------------------
    fun training(): GenreTraining
    suspend fun trainStart(mode: String, selfTrain: Boolean)
    fun trainStop()
    fun subscribeTraining(listener: GenreListener)
    suspend fun sendHead(): GenreSent
    suspend fun disableHead()

    // -- la mesure locale ---------------------------------------------------
    fun measure(): GenreMeasure
    suspend fun measureStart()
    fun measureStop()
    fun subscribeMeasure(listener: GenreMeasureListener)

    /** Règle 1 : les deux travaux se garent pendant qu'un titre démarre. */
    fun setYielding(yielding: Boolean)
}

/** La seule implémentation réelle : les méthodes de ce lot dans `nz-ffi`. */
class NzGenresService(private val screens: uniffi.nz_ffi.Screens) : GenresService {
    override fun cachedStudio(): GenreStudio? = screens.genreCachedStudio()

    override suspend fun studio(): GenreStudio = screens.genreStudio()

    override fun cachedCandidates(sort: String): GenreCandidates? =
        screens.genreCachedCandidates(sort)

    override suspend fun candidates(sort: String, last: String?): GenreCandidates =
        screens.genreCandidates(sort, last)

    override fun ranked(reference: String, query: String): List<GenreRankedTag> =
        screens.genreRanked(reference, query)

    override suspend fun trackSheet(reference: String): GenreTrackSheet =
        screens.genreTrackSheet(reference)

    override suspend fun label(reference: String, tag: Long?) = screens.genreLabel(reference, tag)

    override suspend fun createTag(
        name: String,
        color: String?,
        archetype: String?,
    ): GenreStudio = screens.genreTagCreate(name, color, archetype)

    override suspend fun editTag(
        id: Long,
        name: String?,
        color: String?,
        archetype: String?,
    ): GenreStudio = screens.genreTagEdit(id, name, color, archetype)

    override suspend fun deleteTag(id: Long): GenreStudio = screens.genreTagDelete(id)

    override suspend fun defaults(): GenreStudio = screens.genreTagsDefaults()

    override fun cachedTree(): GenreTree? = screens.genreCachedTree()

    override suspend fun tree(): GenreTree = screens.genreTree()

    override fun treeList(expanded: List<String>, query: String): TreeRows =
        HandleTreeRows(screens.genreTreeList(expanded, query))

    override suspend fun treeAdd(child: String, parent: String, weight: Double): GenreTree =
        screens.genreTreeAdd(child, parent, weight)

    override suspend fun treeCut(child: String, parent: String): GenreTree =
        screens.genreTreeCut(child, parent)

    override suspend fun treeMove(
        child: String,
        parent: String,
        oldParent: String?,
        weight: Double,
    ): GenreTree = screens.genreTreeMove(child, parent, oldParent, weight)

    override suspend fun treeUndo(): GenreTree = screens.genreTreeUndo()

    override suspend fun treeReset(): GenreTree = screens.genreTreeReset()

    override suspend fun bulkPreview(
        album: String?,
        playlist: String?,
        artist: String?,
        tag: String,
    ): GenreBulkPreview = screens.genreBulkPreview(album, playlist, artist, tag)

    override suspend fun bulkApply(tracks: List<String>, tag: String): GenreBulkApplied =
        screens.genreBulkApply(tracks, tag)

    override fun cachedModelTab(): GenreModelTab = screens.genreCachedModelTab()

    override suspend fun modelTab(): GenreModelTab = screens.genreModelTab()

    override suspend fun relabel(): GenreRelabelJob = screens.genreRelabel()

    override suspend fun relabelStart(): GenreRelabelJob = screens.genreRelabelStart()

    override suspend fun embedStart(force: Boolean): GenreEmbedJob = screens.genreEmbedStart(force)

    override suspend fun embed(): GenreEmbedJob = screens.genreEmbed()

    override suspend fun extractorTest(): GenreExtractorTest = screens.genreExtractorTest()

    override suspend fun extractorTestStart(): GenreExtractorTest =
        screens.genreExtractorTestStart()

    override fun training(): GenreTraining = screens.genreTraining()

    override suspend fun trainStart(mode: String, selfTrain: Boolean) =
        screens.genreTrainStart(mode, selfTrain)

    override fun trainStop() = screens.genreTrainStop()

    override fun subscribeTraining(listener: GenreListener) =
        screens.genreSubscribeTraining(listener)

    override suspend fun sendHead(): GenreSent = screens.genreSendHead()

    override suspend fun disableHead() = screens.genreDisableHead()

    override fun measure(): GenreMeasure = screens.genreMeasure()

    override suspend fun measureStart() = screens.genreMeasureStart()

    override fun measureStop() = screens.genreMeasureStop()

    override fun subscribeMeasure(listener: GenreMeasureListener) =
        screens.genreSubscribeMeasure(listener)

    override fun setYielding(yielding: Boolean) = screens.genreSetYielding(yielding)
}

/**
 * Le service du studio, ou `null` quand le noyau natif n'a pas pu être chargé
 * (l'écran le dit au lieu de planter).
 */
object AppGenres {
    fun screens(): uniffi.nz_ffi.Screens? = runCatching { NzBridge.core?.screens() }.getOrNull()

    fun service(): GenresService? = screens()?.let { NzGenresService(it) }
}
