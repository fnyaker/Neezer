package fr.neezer.mobile.feature.discovery

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.ReleaseGroupData
import uniffi.nz_ffi.ReleaseRowData
import uniffi.nz_ffi.ReleaseShelfData
import uniffi.nz_ffi.ReleasesData
import uniffi.nz_ffi.TrackRow

/**
 * Nouveautés' state machine (lot MB14), ported from `routes/Releases.svelte`.
 *
 * Every rule the page taught is here, and the whole thing is plain Kotlin over
 * [DiscoveryService] and [PlayPort] — `ReleasesModelTest` proves each one on the
 * JVM with a fake, no JNA, no `.so`, no device:
 *
 * - **The device's feed paints first, the network replaces it** (`screenFlow`):
 *   a warm open shows what the last visit kept and refreshes behind it; a dead
 *   link keeps it with « Hors ligne · version mémorisée »; with nothing, the
 *   cause and « Réessayer » (rule 3).
 * - **A card click costs nothing**: a shelf page and a group's titles are read
 *   from the feed already in memory (`ReleaseShelf.svelte`'s module scope), so
 *   the tap is answered in the frame.
 * - **"Toutes les sorties" is a view state**, remembered across the traversal
 *   the way the SPA's `lib/nav.js` remembers it.
 * - **One request for the whole screen**: `GET /releases` (and the badge's
 *   `POST /releases/seen` only when something is unseen).
 */
class ReleasesModel(
    private val service: DiscoveryService,
    private val player: PlayPort,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = ::discToast,
) {
    data class State(
        /** What to draw: loading, the feed (with the cache flag), or the cause. */
        val screen: ScreenState<ReleasesData> = ScreenState.Loading,
        /** "Toutes les sorties" is open. */
        val showAll: Boolean = false,
        /** The groups whose titles are shown, by release id. */
        val opened: Set<String> = emptySet(),
        /** The group whose titles are being read from the feed. */
        val opening: String? = null,
        /** A group's rows, once asked for (the handle's first page is drawn). */
        val groupRows: Map<String, List<TrackRow>> = emptyMap(),
    ) {
        val data: ReleasesData? get() = (screen as? ScreenState.Ready)?.data

        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true

        val shelves: List<ReleaseShelfData> get() = data?.shelves.orEmpty()

        val groups: List<ReleaseGroupData> get() = data?.groups.orEmpty()

        val others: List<ReleaseRowData> get() = data?.others.orEmpty()

    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null
    private var seenSent = false

    /** Reads the feed: the device's copy, then the server's own answer. */
    fun load() {
        if (loading?.isActive == true) return
        loading = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedReleases() } },
                fresh = { withContext(io) { service.releases() } },
            ).collect { screen ->
                _state.update { state ->
                    val all = (screen as? ScreenState.Ready)?.data
                    state.copy(
                        screen = screen,
                        showAll = state.showAll ||
                            (all != null && all.shelves.isEmpty() && all.releaseTotal > 0u),
                    )
                }
            }
        }
    }

    /** « Réessayer », from the failure block. */
    fun retry() {
        val failed = _state.value.screen as? ScreenState.Failed ?: return
        failed.retry()
    }

    /** "Toutes les sorties": the SPA's `toggleAll`, remembered across the trip. */
    fun toggleAll() = _state.update { it.copy(showAll = !it.showAll) }

    /**
     * A per-release disclosure: the group's titles are read from the kept feed
     * when first opened — no request, and the window is only the first page.
     */
    fun toggleGroup(id: String) {
        val opened = _state.value.opened
        if (opened.contains(id)) {
            _state.update { it.copy(opened = opened - id) }
            return
        }
        if (_state.value.groupRows.containsKey(id)) {
            _state.update { it.copy(opened = opened + id) }
            return
        }
        _state.update { it.copy(opened = opened + id, opening = id) }
        scope.launch {
            val rows = withContext(io) { runCatching { service.releaseGroupTracks(id).window(0, 64) }.getOrDefault(emptyList()) }
            _state.update {
                it.copy(opening = null, groupRows = it.groupRows + (id to rows))
            }
        }
    }

    /** « Tout lire · N »: every new title, from the first. */
    fun playAll() {
        val count = _state.value.data?.trackCount?.toInt() ?: 0
        if (count == 0) return
        scope.launch {
            val rows = withContext(io) { service.releasesTracks() }
            player.play(rows, 0, CONTEXT_RELEASES, "")
        }
    }

    /** One release's « Lire »: its own titles, from its first (`context:{album,id}`). */
    fun playGroup(group: ReleaseGroupData) = playGroupAt(group, 0)

    /** A row of an open group: the same queue, started at that title. */
    fun playGroupAt(group: ReleaseGroupData, start: Int) {
        scope.launch {
            val rows = withContext(io) { service.releaseGroupTracks(group.release.key) }
            if (rows.len() == 0) {
                toast("Cette sortie n'a pas encore ses titres.")
                return@launch
            }
            player.play(rows, start, CONTEXT_ALBUM, group.release.key)
        }
    }

    /**
     * Opening a shelf marks the feed seen — the SPA's own rule: the DISPLAYED
     * count stays this visit's, the kept feed is cleared so a later back
     * navigation does not announce the same releases twice.
     */
    fun seenOnShelf() {
        if (seenSent || _state.value.data?.unseen == 0u) return
        seenSent = true
        scope.launch {
            runCatching { withContext(io) { service.releasesSeen() } }
                .onFailure { toast("Impossible de marquer les sorties comme vues : ${NzErrors.text(it)}") }
        }
    }

    companion object {
        const val CONTEXT_RELEASES = "releases"
        const val CONTEXT_ALBUM = "album"
    }
}

/**
 * One shelf's page (lot MB14), ported from `routes/ReleaseShelf.svelte`.
 *
 * It is a read-only sibling of a playlist: a banner, its facts, « Lire » and
 * « Mélanger », then the windowed titles with a search and a sort. A card click
 * reaches it with the feed already in memory, so its first paint costs no
 * request; only a deep link asks (`loadFeed`).
 */
class ReleaseShelfModel(
    private val kind: String,
    private val service: DiscoveryService,
    private val player: PlayPort,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = ::discToast,
) {
    data class State(
        val screen: ScreenState<ShelfPage> = ScreenState.Loading,
        val query: String = "",
        /** `SORTS`'s key: `default`, `title`, `artist`, `album`, `duration`, `added`. */
        val sort: String = SORT_DEFAULT,
        val descending: Boolean = false,
        /** The rows the list currently paints (one page, filtered and sorted). */
        val rows: List<TrackRow> = emptyList(),
        /** How many rows the filter left, which the handle owns. */
        val rowCount: Int = 0,
    ) {
        val page: ShelfPage? get() = (screen as? ScreenState.Ready)?.data

        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true

        val searching: Boolean get() = query.isNotBlank()
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null

    fun load() {
        if (loading?.isActive == true) return
        loading = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.releaseShelf(kind) } },
                fresh = { withContext(io) { freshPage() } },
            ).collect { screen ->
                _state.update { it.copy(screen = screen) }
                if (screen is ScreenState.Ready) refreshRows()
            }
        }
    }

    fun retry() {
        val failed = _state.value.screen as? ScreenState.Failed ?: return
        failed.retry()
    }

    /**
     * A deep link: the SPA asks the feed, then builds the shelf from it
     * (`ReleaseShelf.svelte#load`). One screen open, the SPA's own requests.
     */
    private suspend fun freshPage(): ShelfPage {
        service.releases()
        return service.releaseShelf(kind)
            ?: throw IllegalStateException("Cette sélection n'a pas pu être chargée.")
    }

    /** The search box: the handle folds (`appcore::fold`), accents included. */
    fun search(query: String) {
        _state.update { it.copy(query = query) }
        scope.launch { refreshRows() }
    }

    /** `SORTS`'s picker, and the arrow that reverses it. */
    fun sort(by: String) {
        _state.update { it.copy(sort = by) }
        scope.launch { refreshRows() }
    }

    fun reverse() {
        _state.update { it.copy(descending = !it.descending) }
        scope.launch { refreshRows() }
    }

    /** « Lire »: the shelf's titles, in the order they are shown. */
    fun playAll() {
        val rows = _state.value.rows
        if (rows.isEmpty()) return
        player.playList(rows, 0, CONTEXT_RELEASES, kind)
    }

    /** « Mélanger »: the same rows, shuffled by the player's own shuffle. */
    fun shuffle() {
        val rows = _state.value.rows
        if (rows.isEmpty()) return
        player.playList(rows.shuffled(), 0, CONTEXT_RELEASES, kind)
    }

    /** One row of the list: it plays, its ⋯ is MB10's menu (the entry's). */
    fun playRow(row: TrackRow) {
        val rows = _state.value.rows
        val start = rows.indexOfFirst { it.key == row.key }.coerceAtLeast(0)
        player.playList(rows, start, CONTEXT_RELEASES, kind)
    }

    fun queueNext(row: TrackRow) = player.playNext(row)

    /**
     * The list as the toolbar leaves it: the handle folds the search
     * (`appcore::fold`, accents included), sorts, then hands one PAGE back —
     * a shelf of 400 titles is 64 rows on screen, never 400 records.
     */
    private suspend fun refreshRows() {
        val page = _state.value.page ?: return
        val state = _state.value
        val count = withContext(io) {
            val filtered = page.rows.filter(state.query)
            page.rows.sort(sortKey(state.sort), state.descending)
            filtered
        }
        val rows = withContext(io) { page.rows.window(0, PAGE) }
        _state.update { it.copy(rows = rows, rowCount = count) }
    }

    companion object {
        const val CONTEXT_RELEASES = "releases"
        const val PAGE = 64
        const val SORT_DEFAULT = "default"

        /** `lib/tracklist.js#SORTS`, in its own order and with its own words. */
        val SORTS: List<Pair<String, String>> = listOf(
            SORT_DEFAULT to "Ordre par défaut",
            "title" to "Titre",
            "artist" to "Artiste",
            "album" to "Album",
            "duration" to "Durée",
            "added" to "Ajouté",
        )

        fun sortKey(key: String): uniffi.nz_ffi.SortKey = when (key) {
            "title" -> uniffi.nz_ffi.SortKey.TITLE
            "artist" -> uniffi.nz_ffi.SortKey.ARTIST
            "album" -> uniffi.nz_ffi.SortKey.ALBUM
            "duration" -> uniffi.nz_ffi.SortKey.DURATION
            "added" -> uniffi.nz_ffi.SortKey.ADDED
            else -> uniffi.nz_ffi.SortKey.POSITION
        }
    }
}

/** A shelf's title and copy, by kind (`Releases.svelte#SHELF_COPY`'s fallback). */
fun shelfKindOrFirst(kind: String): String = if (kind == "discovery") "discovery" else "artists"

/** `discToast`: one place for the failure sentences this lot raises. */
fun discToast(message: String) {
    fr.neezer.mobile.designsystem.NzToasts.show(message)
}
