package fr.neezer.mobile.feature.genres

import uniffi.nz_ffi.GenreBulkApplied
import uniffi.nz_ffi.GenreBulkCounts
import uniffi.nz_ffi.GenreBulkPreview
import uniffi.nz_ffi.GenreBulkRow
import uniffi.nz_ffi.GenreCandidate
import uniffi.nz_ffi.GenreCandidates
import uniffi.nz_ffi.GenreCause
import uniffi.nz_ffi.GenreClass
import uniffi.nz_ffi.GenreConfusion
import uniffi.nz_ffi.GenreConstruction
import uniffi.nz_ffi.GenreEmbedJob
import uniffi.nz_ffi.GenreExtractor
import uniffi.nz_ffi.GenreExtractorTest
import uniffi.nz_ffi.GenreFailure
import uniffi.nz_ffi.GenreHead
import uniffi.nz_ffi.GenreListener
import uniffi.nz_ffi.GenreMeasure
import uniffi.nz_ffi.GenreMeasureListener
import uniffi.nz_ffi.GenreModelRow
import uniffi.nz_ffi.GenreModelTab
import uniffi.nz_ffi.GenreRankedTag
import uniffi.nz_ffi.GenreRelabelJob
import uniffi.nz_ffi.GenreSent
import uniffi.nz_ffi.GenreStudio
import uniffi.nz_ffi.GenreTagRow
import uniffi.nz_ffi.GenreTrackSheet
import uniffi.nz_ffi.GenreTraining
import uniffi.nz_ffi.GenreTree
import uniffi.nz_ffi.TrackRow

/**
 * Le studio sans noyau : un faux service pour `GenresModelTest` et pour les
 * captures. Il compte les requêtes comme le vrai écran les compte, refuse une
 * écriture sur commande, et ne touche ni JNA ni `.so`.
 */
class FakeGenresService(
    admin: Boolean = true,
    private val tagCount: Int = 7,
) : GenresService {
    /** Le vocabulaire, tel que `/genre/status` le rend. */
    var tags: List<GenreTagRow> = listOf(
        tag(1, "Hardcore", 30),
        tag(2, "Frenchcore", 12),
        tag(3, "Gabber", 8),
        tag(4, "Hardtek", 6),
        tag(5, "Hardstyle", 7),
        tag(6, "Hardhouse", 9),
        tag(7, "Zebda", 2),
    )

    var candidates: List<GenreCandidate> = listOf(
        candidate("1001", "Premier titre", "Hardtek", "Frenchcore"),
        candidate("1002", "Second titre", "Frenchcore", "Hardtek"),
    )

    /** Le vocabulaire rangé du titre courant, comme le noyau le calcule. */
    var ranking: MutableMap<String, List<GenreRankedTag>> = mutableMapOf()

    /** Ce que le faux a reçu, dans l'ordre — la mesure des requêtes de l'écran. */
    val calls = mutableListOf<String>()

    var failures = mutableMapOf<String, String>()
    var caches = false
    var labels = mutableListOf<Pair<String, Long?>>()
    var applied = mutableListOf<Pair<List<String>, String>>()
    var trainings = mutableListOf<Pair<String, Boolean>>()
    var yielding = mutableListOf<Boolean>()
    var sentHead = false
    var disabledHead = false
    var measures = 0

    var trainingState: GenreTraining = idleTraining()
    var measureState: GenreMeasure = GenreMeasure(
        running = false,
        done = 0u,
        failed = 0u,
        total = 0u,
        current = "",
        causes = emptyList(),
        stopped = null,
    )

    var tree: GenreTree = GenreTree(
        version = "v1",
        defaultTree = true,
        canUndo = false,
        nodes = 6u,
        edges = 5u,
        roots = listOf("Hardcore", "Hardhouse"),
    )
    var treeRows: List<TrackRow> = listOf(
        treeRow("Hardcore", null, 0u, 3u, open = true, branch = true),
        treeRow("Frenchcore", "Hardcore", 1u, 0u, open = false, branch = false),
        treeRow("Gabber", "Hardcore", 1u, 1u, open = false, branch = true),
        treeRow("Hardtek", "Hardcore", 1u, 0u, open = false, branch = false),
        treeRow("Hardhouse", null, 0u, 1u, open = false, branch = true),
    )

    var relabelJob = GenreRelabelJob(
        running = false,
        scanned = 900u,
        total = 900u,
        changed = 31u,
        byTag = 12u,
        byModel = 9u,
        byHeuristic = 8u,
        byFamily = 2u,
        error = null,
    )
    var embedJob = GenreEmbedJob(
        running = false,
        scanned = 120u,
        done = 119u,
        failed = 1u,
        total = 120u,
        error = null,
        causes = listOf(GenreCause(reason = "ffmpeg a disparu", count = 1u)),
        failures = listOf(GenreFailure(title = "Casse", reason = "ffmpeg a disparu")),
        force = false,
    )
    var modelTab: GenreModelTab = GenreModelTab(
        model = null,
        relabel = relabelJob,
        embed = embedJob,
        construction = GenreConstruction(labelled = 74u, measured = 50u, missing = 2u),
    )

    private val trainingListeners = mutableListOf<GenreListener>()
    private val measureListeners = mutableListOf<GenreMeasureListener>()

    // --- reads --------------------------------------------------------------

    override fun cachedStudio(): GenreStudio? =
        if (caches) studioNow() else null

    /** Quand il est posé, la lecture fraîche ATTEND : la copie doit peindre avant. */
    var gateStudio: kotlinx.coroutines.CompletableDeferred<Unit>? = null

    override suspend fun studio(): GenreStudio {
        calls += "GET /genre/status"
        gateStudio?.await()
        fail("studio")
        caches = true
        return studioNow()
    }

    override fun cachedCandidates(sort: String): GenreCandidates? =
        if (caches) candidatesNow(sort) else null

    override suspend fun candidates(sort: String, last: String?): GenreCandidates {
        calls += "GET /genre/candidates?sort=$sort"
        fail("candidates")
        return candidatesNow(sort)
    }

    override fun ranked(reference: String, query: String): List<GenreRankedTag> {
        val rows = ranking[reference] ?: defaultRanking()
        val needle = query.trim().lowercase()
        if (needle.isEmpty()) return rows
        return rows.filter { it.name.lowercase().contains(needle) }
    }

    override suspend fun trackSheet(reference: String): GenreTrackSheet {
        calls += "GET /genre/track/$reference"
        fail("sheet")
        val tag = tags.firstOrNull { it.name == "Hardtek" }
        return GenreTrackSheet(reference = reference, tag = tag, ranked = emptyList())
    }

    override suspend fun label(reference: String, tag: Long?) {
        calls += "POST /genre/label"
        fail("label")
        labels += reference to tag
    }

    override suspend fun createTag(name: String, color: String?, archetype: String?): GenreStudio {
        calls += "POST /genre/tags"
        fail("tag")
        tags = tags + tag((tags.size + 1).toLong(), name, 0)
        return studioNow()
    }

    override suspend fun editTag(
        id: Long,
        name: String?,
        color: String?,
        archetype: String?,
    ): GenreStudio {
        calls += "PATCH /genre/tags/$id"
        fail("tag")
        tags = tags.map { if (it.id == id) it.copy(name = name ?: it.name) else it }
        return studioNow()
    }

    override suspend fun deleteTag(id: Long): GenreStudio {
        calls += "DELETE /genre/tags/$id"
        fail("tag")
        tags = tags.filterNot { it.id == id }
        return studioNow()
    }

    override suspend fun defaults(): GenreStudio {
        calls += "POST /genre/tags/defaults"
        fail("tag")
        return studioNow()
    }

    override fun cachedTree(): GenreTree? = if (caches) tree else null

    override suspend fun tree(): GenreTree {
        calls += "GET /genre/tree"
        fail("tree")
        caches = true
        return tree
    }

    override fun treeList(expanded: List<String>, query: String): TreeRows =
        FakeTreeRows(treeRows)

    override suspend fun treeAdd(child: String, parent: String, weight: Double): GenreTree {
        calls += "POST /genre/tree/edge"
        fail("tree")
        tree = tree.copy(edges = tree.edges + 1u, canUndo = true, defaultTree = false)
        return tree
    }

    override suspend fun treeCut(child: String, parent: String): GenreTree {
        calls += "DELETE /genre/tree/edge"
        fail("tree")
        tree = tree.copy(edges = (tree.edges - 1u).coerceAtLeast(0u), canUndo = true)
        return tree
    }

    override suspend fun treeMove(
        child: String,
        parent: String,
        oldParent: String?,
        weight: Double,
    ): GenreTree {
        calls += "POST /genre/tree/move"
        fail("tree")
        tree = tree.copy(canUndo = true, defaultTree = false)
        return tree
    }

    override suspend fun treeUndo(): GenreTree {
        calls += "POST /genre/tree/undo"
        fail("tree")
        tree = tree.copy(canUndo = false, defaultTree = true)
        return tree
    }

    override suspend fun treeReset(): GenreTree {
        calls += "POST /genre/tree/reset"
        fail("tree")
        tree = tree.copy(canUndo = false, defaultTree = true)
        return tree
    }

    override suspend fun bulkPreview(
        album: String?,
        playlist: String?,
        artist: String?,
        tag: String,
    ): GenreBulkPreview {
        calls += "POST /genre/bulk/preview"
        fail("bulk")
        return GenreBulkPreview(
            tag = tags.firstOrNull { it.name == tag },
            rows = listOf(
                GenreBulkRow(
                    reference = "2001",
                    title = "Déjà bon",
                    artist = "A",
                    album = "Al",
                    durationS = 180u,
                    current = tag,
                    same = true,
                    disagree = "",
                    opinion = null,
                    opinionConfidence = null,
                    opinionLevel = "",
                ),
                GenreBulkRow(
                    reference = "2002",
                    title = "Autre étiquette",
                    artist = "B",
                    album = "Al",
                    durationS = 181u,
                    current = "Gabber",
                    same = false,
                    disagree = "tag",
                    opinion = null,
                    opinionConfidence = null,
                    opinionLevel = "",
                ),
                GenreBulkRow(
                    reference = "2003",
                    title = "Le modèle n'est pas d'accord",
                    artist = "C",
                    album = "Al",
                    durationS = 182u,
                    current = null,
                    same = false,
                    disagree = "model",
                    opinion = "Hardtek",
                    opinionConfidence = 0.91,
                    opinionLevel = "label",
                ),
            ),
            counts = GenreBulkCounts(
                tracks = 3u,
                missing = 0u,
                same = 1u,
                otherTag = 1u,
                model = 1u,
                noVector = 0u,
                unjudged = 0u,
            ),
            headActive = true,
            headKnows = true,
        )
    }

    override suspend fun bulkApply(tracks: List<String>, tag: String): GenreBulkApplied {
        calls += "POST /genre/bulk"
        fail("bulk")
        applied += tracks to tag
        return GenreBulkApplied(applied = tracks.size.toUInt(), missing = 0u)
    }

    /** Quand il est posé, les trois lectures ATTENDENT : la copie peint avant. */
    var gateModelTab: kotlinx.coroutines.CompletableDeferred<Unit>? = null

    override fun cachedModelTab(): GenreModelTab = modelTab

    override suspend fun modelTab(): GenreModelTab {
        gateModelTab?.await()
        calls += "GET /genre/relabel"
        calls += "GET /genre/embed"
        calls += "GET /genre/construction"
        fail("model")
        return modelTab
    }

    override suspend fun relabel(): GenreRelabelJob {
        calls += "GET /genre/relabel"
        fail("model")
        return relabelJob
    }

    override suspend fun relabelStart(): GenreRelabelJob {
        calls += "POST /genre/relabel"
        fail("model")
        relabelJob = relabelJob.copy(running = true)
        return relabelJob
    }

    override suspend fun embedStart(force: Boolean): GenreEmbedJob {
        calls += "POST /genre/embed"
        fail("model")
        embedJob = embedJob.copy(running = true)
        return embedJob
    }

    override suspend fun embed(): GenreEmbedJob {
        calls += "GET /genre/embed"
        fail("model")
        embedJob = embedJob.copy(running = false)
        return embedJob
    }

    override suspend fun extractorTest(): GenreExtractorTest {
        calls += "GET /genre/extractor/test"
        return GenreExtractorTest(running = false, ok = true, error = null, progress = emptyList())
    }

    override suspend fun extractorTestStart(): GenreExtractorTest {
        calls += "POST /genre/extractor/test"
        fail("extractor")
        return GenreExtractorTest(running = false, ok = true, error = null, progress = emptyList())
    }

    override fun training(): GenreTraining = trainingState

    override suspend fun trainStart(mode: String, selfTrain: Boolean) {
        calls += "train"
        fail("train")
        trainings += mode to selfTrain
        trainingState = trainingState.copy(running = true, stage = "entraînement", mode = mode)
    }

    override fun trainStop() {
        calls += "train/stop"
        trainingState = trainingState.copy(running = false, cause = "entraînement interrompu")
    }

    override fun subscribeTraining(listener: GenreListener) {
        trainingListeners += listener
    }

    override suspend fun sendHead(): GenreSent {
        calls += "PUT /genre/model"
        fail("send")
        sentHead = true
        return GenreSent(labels = 2u, relabel = relabelJob)
    }

    override suspend fun disableHead() {
        calls += "DELETE /genre/model"
        fail("send")
        disabledHead = true
    }

    override fun measure(): GenreMeasure = measureState

    override suspend fun measureStart() {
        calls += "measure"
        fail("measure")
        measures += 1
        measureState = measureState.copy(running = true, total = 3u)
    }

    override fun measureStop() {
        calls += "measure/stop"
        measureState = measureState.copy(running = false, stopped = "arrêt demandé")
    }

    override fun subscribeMeasure(listener: GenreMeasureListener) {
        measureListeners += listener
    }

    override fun setYielding(yielding: Boolean) {
        this.yielding += yielding
    }

    // --- what a listener would be told ---------------------------------------

    fun finishTraining(ok: Boolean, cause: String? = null, head: GenreHead? = null) {
        trainingState = trainingState.copy(running = false, stage = if (ok) "terminé" else "échec", head = head, cause = cause)
        trainingListeners.forEach { it.finished(ok, cause) }
    }

    fun reportProgress(stage: String, fraction: Double) {
        trainingState = trainingState.copy(running = true, stage = stage, fraction = fraction)
        trainingListeners.forEach { it.progress(stage, fraction) }
    }

    fun reportMeasure(state: GenreMeasure) {
        measureState = state
        measureListeners.forEach { it.changed(state) }
    }

    /** Le noyau a fini sa passe : le prochain sondage de l'écran lit `running = false`. */
    fun finishMeasure(done: Int, failed: Int, total: Int, cause: String? = null) {
        measureState = GenreMeasure(
            running = false,
            done = done.toUInt(),
            failed = failed.toUInt(),
            total = total.toUInt(),
            current = "",
            causes = if (cause == null) emptyList() else {
                listOf(GenreCause(reason = cause, count = failed.toUInt()))
            },
            stopped = null,
        )
        measureListeners.forEach { it.changed(measureState) }
    }

    // --- fixtures ------------------------------------------------------------

    private fun fail(key: String) {
        failures[key]?.let { throw IllegalStateException(it) }
    }

    private fun studioNow() = GenreStudio(
        archetypes = listOf("groove", "voice"),
        tags = tags,
        labelled = 74u,
        model = if (sentHead) {
            GenreModelRow(
                usable = true,
                version = 3,
                created = "2026-11-18T10:00:00",
                labels = listOf("Hardtek", "Frenchcore"),
                kind = "linear",
                dim = 2560u,
                health = "watch",
                balanced = 0.82,
            )
        } else {
            null
        },
        extractor = GenreExtractor(
            available = true,
            dim = 1280u,
            model = "discogs-effnet.onnx",
            runtime = true,
            reason = null,
            sessionState = "ok",
            sessionError = null,
        ),
    )

    private fun candidatesNow(sort: String) = GenreCandidates(
        rows = candidates,
        labelled = 74u,
        sort = sort,
        truncated = false,
    )

    /** Le rang par défaut du titre courant : le score décide, puis l'usage. */
    private fun defaultRanking(): List<GenreRankedTag> = listOf(
        GenreRankedTag(4, "Hardtek", null, 6u, 0.52, 0.44, 0.51),
        GenreRankedTag(2, "Frenchcore", null, 12u, 0.31, 0.22, 0.38),
        GenreRankedTag(1, "Hardcore", null, 30u, null, null, null),
        GenreRankedTag(5, "Hardstyle", null, 7u, null, null, null),
        GenreRankedTag(6, "Hardhouse", null, 9u, null, null, null),
        GenreRankedTag(3, "Gabber", null, 8u, null, null, null),
        GenreRankedTag(7, "Zebda", null, 2u, null, null, null),
    )

    fun seedRanking() {
        ranking["1001"] = defaultRanking()
        ranking["1002"] = defaultRanking()
    }

    companion object {
        fun tag(id: Long, name: String, count: Int) = GenreTagRow(
            id = id,
            name = name,
            color = null,
            archetype = null,
            family = if (name == "Hardcore" || name == "Frenchcore") "Hardcore" else null,
            count = count.toUInt(),
        )

        fun candidate(reference: String, title: String, first: String, second: String) =
            GenreCandidate(
                reference = reference,
                id = reference,
                title = title,
                artist = "Artiste",
                album = "Album",
                durationS = 212u,
                playCount = 7u,
                bpm = 174.0,
                predicted = first,
                confidence = 0.42,
                uncertainty = 0.58,
            )

        fun treeRow(
            name: String,
            parent: String?,
            depth: UInt,
            children: UInt,
            open: Boolean,
            branch: Boolean,
        ): TrackRow = TrackRow(
            key = if (parent == null) name else "$parent/$name",
            id = name,
            title = name,
            artists = parent ?: "",
            album = "",
            albumId = null,
            durationS = children,
            cover = null,
            explicit = false,
            liked = false,
            disliked = false,
            unavailable = false,
            downloaded = open,
            source = if (depth == 0u) "root" else if (branch) "branch" else "leaf",
            position = depth,
        )

        fun idleTraining() = GenreTraining(
            running = false,
            stage = "",
            fraction = 0.0,
            cause = null,
            mode = "auto",
            selfTrain = true,
            examples = 0u,
            pool = 0u,
            head = null,
        )

        fun head() = GenreHead(
            kind = "linear",
            labels = listOf("Hardtek", "Frenchcore"),
            dim = 2560u,
            hidden = 0u,
            hidden2 = 0u,
            examples = 12u,
            classes = 2u,
            folds = 5u,
            accuracy = 0.92,
            balanced = 0.91,
            fitBalanced = 0.93,
            temperature = 1.1,
            grouped = true,
            projected = true,
            bagged = 3u,
            seed = 1u,
            skipped = 0u,
            cleanup = listOf("2 doublons écartés"),
            perClass = listOf(
                GenreClass(label = "Hardtek", examples = 6u, recall = 0.9, precision = 0.86),
                GenreClass(label = "Frenchcore", examples = 6u, recall = 0.92, precision = 0.95),
            ),
            confusions = listOf(
                GenreConfusion(from = "Hardtek", to = "Frenchcore", count = 1u, share = 0.16),
            ),
            healthLevel = "good",
            healthRemarks = listOf("Fiable : 91 % de bonnes réponses"),
        )
    }
}

/** Le faux handle : sa propre source, puisqu'un test n'a pas de `.so`. */
class FakeTreeRows(private val rows: List<TrackRow>) : TreeRows {
    /** Le nombre d'appels à `window` — la mesure d'un défilement. */
    var windows: Int = 0
        private set

    override val handle = null

    override fun len(): Int = rows.size

    override fun window(start: Int, count: Int): List<TrackRow> {
        windows += 1
        if (start >= rows.size) return emptyList()
        return rows.subList(start, (start + count).coerceAtMost(rows.size))
    }

    override fun version(): ULong = 1uL
}
