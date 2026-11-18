package fr.neezer.mobile.feature.library

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import fr.neezer.mobile.designsystem.NzToasts
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
import uniffi.nz_ffi.FolderRow
import uniffi.nz_ffi.LibraryAlbum
import uniffi.nz_ffi.LibraryArtist
import uniffi.nz_ffi.LibraryTree
import uniffi.nz_ffi.PlaylistRow
import uniffi.nz_ffi.SortKey
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.UploadUsageData

/**
 * One leaf of the library, with the SPA's own URL id (`#/library/artists`) so a
 * deep link lands on the same leaf.
 */
enum class LibraryLeaf(val id: String, val label: String) {
    Favorites("favorites", "Titres"),
    History("history", "Historique"),
    Artists("artists", "Artistes"),
    Albums("albums", "Albums"),
    Playlists("playlists", "Playlists"),
    Local("local", "Mes fichiers"),
    Unavailable("gone", "Indisponibles"),

    /**
     * The titles the person said they do not like (`DislikedTracks.svelte`).
     *
     * The SPA draws this card in Réglages → Découverte; the phone keeps it beside
     * « Indisponibles », because both are the library's rare "manage these" lists
     * and a person manages their collection from the library. The same card is
     * `DislikedSection`, so MB16's Réglages can place it too.
     */
    Dislikes("dislikes", "Je n'aime pas"),
    ;

    companion object {
        /** The leaf a route asked for, or the SPA's default (`favorites`). */
        fun of(id: String?): LibraryLeaf = entries.firstOrNull { it.id == id } ?: Favorites
    }
}

/** The three orders of the « Artistes » grid (`Library.svelte#AR_SORTS`). */
enum class ArtistSort(val label: String) {
    Plays("Les plus écoutés"),
    Name("A-Z"),
    Recent("Ajoutés récemment"),
}

/** The favourites table's orders (`lib/tracklist.js#SORTS`). */
enum class FavoriteSort(val label: String, val key: SortKey) {
    Position("Ordre d'origine", SortKey.POSITION),
    Title("Titre", SortKey.TITLE),
    Artist("Artiste", SortKey.ARTIST),
    Album("Album", SortKey.ALBUM),
    Duration("Durée", SortKey.DURATION),
    Added("Date d'ajout", SortKey.ADDED),
}

/** Which folder sheet is open (`FolderSheet.svelte`). */
data class FolderSheet(
    /** The folder being renamed, `null` for a creation. */
    val renameOf: String? = null,
    val title: String = "",
    val error: String = "",
    val busy: Boolean = false,
)

/** « Déplacer vers » (`MoveToFolderSheet.svelte`). */
data class MoveTarget(
    val playlistId: String,
    val title: String,
    val currentFolder: String?,
    val query: String = "",
    val busy: Boolean = false,
)

/** One playlist card, as the section's grid draws it. */
data class PlaylistCardView(
    val id: String,
    val title: String,
    val cover: String?,
    val subtitle: String,
)

/** One section of the « Playlists » leaf. */
data class SectionView(
    val key: String,
    val title: String,
    val count: Int,
    val folderId: String?,
    val folded: Boolean,
    val rows: List<PlaylistCardView>,
)

/** One row of the organiser. */
data class OrganizeRowView(val id: String, val title: String, val detail: String)

/** One destination of « Déplacer vers ». */
data class FolderOptionView(val id: String, val title: String, val count: Int)

/**
 * The library's state machine (lot MB8).
 *
 * Every rule `routes/Library.svelte` and the components it draws taught is here,
 * in the order a person meets it, and the whole thing is plain Kotlin over
 * [LibraryService] and [LibraryPlayer] — `LibraryModelTest` proves each one on the
 * JVM with a fake: no JNA, no `.so`, no device.
 *
 * - **One leaf is ONE request.** The SPA loads the favourites, the playlists, the
 *   local files and the upload usage together on mount, whatever leaf is open;
 *   the phone opens one leaf and pays only for it (8 000 favourites: 1 request
 *   cold, 0 before a warm open paints, then one background refresh — the SPA's
 *   own `loadMyFavorites`).
 * - **The copy paints, the request replaces it** (`screenFlow`): a dead link keeps
 *   the copy and the screen says « Hors ligne · version mémorisée »; with nothing,
 *   the cause and « Réessayer ».
 * - **A folder write answers before its request and rolls back when it is
 *   refused** (`actions.js#createFolder`…): the tree moves first, the ONE request
 *   carries the complete order, and a failure keeps what was drawn and raises
 *   « Réessayer ». Two reorders in a row are coalesced into one request — the
 *   SPA's 400 ms debounce, restated as one write in flight.
 * - **Nothing is asked twice**: every leaf remembers its job, so the two `Ready`
 *   emissions of a warm open (`screenFlow`'s copy, then its answer) never ask the
 *   route again (MB5's and MB7's trap).
 */
class LibraryModel(
    private val service: LibraryService,
    private val player: LibraryPlayer,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
) {
    data class State(
        val leaf: LibraryLeaf = LibraryLeaf.Favorites,
        /** The favourites: a windowed handle, and what the table's tools did. */
        val favorites: ScreenState<LibraryRows> = ScreenState.Loading,
        val favoriteFilter: String = "",
        val favoriteSort: FavoriteSort = FavoriteSort.Position,
        val favoriteDescending: Boolean = false,
        val artists: ScreenState<List<LibraryArtist>> = ScreenState.Loading,
        val artistQuery: String = "",
        val artistSort: ArtistSort = ArtistSort.Plays,
        val albums: ScreenState<List<LibraryAlbum>> = ScreenState.Loading,
        /** The playlist tree, with the sections the core grouped. */
        val tree: ScreenState<LibraryTree> = ScreenState.Loading,
        val playlistQuery: String = "",
        /** The sections the person folded shut, by key. */
        val closed: Set<String> = emptySet(),
        val organizing: Boolean = false,
        /** `folders`, `root`, or a folder's id (`Library.svelte#organizeContainer`). */
        val container: String = CONTAINER_FOLDERS,
        val organizeStatus: String = "",
        val local: ScreenState<LibraryRows> = ScreenState.Loading,
        /** The upload quota, loaded with « Mes fichiers » (its own request). */
        val usage: UploadUsageData? = null,
        val unavailable: ScreenState<LibraryRows> = ScreenState.Loading,
        val dislikes: ScreenState<LibraryRows> = ScreenState.Loading,
        /** `Some(false)` hides the folder affordances (an older server, rule 7). */
        val foldersSupported: Boolean? = null,
        val folderSheet: FolderSheet? = null,
        val moveTarget: MoveTarget? = null,
        /** The one mutation in flight, by key, so a second press is ignored. */
        val busy: String? = null,
    ) {
        /** The favourites' rows, when there is a list to draw. */
        val favoriteRows: LibraryRows? get() = (favorites as? ScreenState.Ready)?.data

        val artistRows: List<LibraryArtist>
            get() = (artists as? ScreenState.Ready)?.data.orEmpty()

        val albumRows: List<LibraryAlbum> get() = (albums as? ScreenState.Ready)?.data.orEmpty()

        val treeData: LibraryTree? get() = (tree as? ScreenState.Ready)?.data

        val localRows: LibraryRows? get() = (local as? ScreenState.Ready)?.data

        val unavailableRows: LibraryRows? get() = (unavailable as? ScreenState.Ready)?.data

        val dislikeRows: LibraryRows? get() = (dislikes as? ScreenState.Ready)?.data

        /** What is drawn came from the device and the refresh failed. */
        val stale: Boolean get() = (current() as? ScreenState.Ready<*>)?.stale == true

        private fun current(): ScreenState<*>? = when (leaf) {
            LibraryLeaf.Favorites -> favorites
            LibraryLeaf.Artists -> artists
            LibraryLeaf.Albums -> albums
            LibraryLeaf.Playlists -> tree
            LibraryLeaf.Local -> local
            LibraryLeaf.Unavailable -> unavailable
            LibraryLeaf.Dislikes -> dislikes
            // The history leaf owns its own flow (`HistoryModel`).
            LibraryLeaf.History -> null
        }

        /** The failure to print for the leaf on screen, when it has one. */
        val failure: ScreenState.Failed? get() = current() as? ScreenState.Failed

        /** The counters the header states — only what has been loaded. */
        val counts: String
            get() {
                val parts = mutableListOf<String>()
                favoriteRows?.let { parts += plural(it.len(), "titre", "titres") }
                (artists as? ScreenState.Ready)?.let {
                    parts += plural(it.data.size, "artiste", "artistes")
                }
                treeData?.let { parts += plural(it.playlists.size, "playlist", "playlists") }
                return parts.joinToString(" · ")
            }

        /** The artists the grid draws: the query and the sort applied. */
        val shownArtists: List<LibraryArtist>
            get() {
                val term = fold(artistQuery.trim())
                val rows = if (term.isEmpty()) {
                    artistRows
                } else {
                    artistRows.filter { fold(it.name).contains(term) }
                }
                return when (artistSort) {
                    ArtistSort.Plays -> rows
                    ArtistSort.Name -> rows.sortedBy { fold(it.name) }
                    ArtistSort.Recent -> rows.sortedByDescending { it.starred }
                }
            }

        /** The « Playlists » sections, filtered by the search box. */
        val sections: List<SectionView>
            get() {
                val tree = treeData ?: return emptyList()
                val term = playlistQuery.trim().lowercase()
                val raw = if (term.isEmpty()) {
                    tree.sections
                } else {
                    tree.sections.mapNotNull { section ->
                        when {
                            section.title.lowercase().contains(term) -> section
                            section.rows.any { it.title.lowercase().contains(term) } -> section.copy(
                                rows = section.rows.filter { it.title.lowercase().contains(term) },
                            )
                            else -> null
                        }
                    }
                }
                return raw.map { section ->
                    SectionView(
                        key = section.key,
                        title = section.title,
                        count = section.count.toInt(),
                        folderId = section.key.removePrefix("folder:").takeIf {
                            section.key.startsWith("folder:")
                        },
                        folded = closed.contains(section.key),
                        rows = section.rows.map { row ->
                            PlaylistCardView(
                                id = row.id,
                                title = row.title,
                                cover = row.cover,
                                subtitle = cardSubtitle(row),
                            )
                        },
                    )
                }
            }

        /** The rows of the container being organised (`organizeRows`). */
        val organizeRows: List<OrganizeRowView>
            get() {
                val tree = treeData ?: return emptyList()
                return when (container) {
                    CONTAINER_FOLDERS -> tree.folders.map { folder ->
                        OrganizeRowView(
                            id = folder.id,
                            title = folder.title,
                            detail = plural(folder.count.toInt(), "playlist", "playlists"),
                        )
                    }
                    else -> {
                        val id = if (container == CONTAINER_ROOT) null else container
                        containerPlaylists(tree, id).map { playlist ->
                            OrganizeRowView(
                                id = playlist.id,
                                title = playlist.title,
                                detail = plural(playlist.tracks.toInt(), "titre", "titres"),
                            )
                        }
                    }
                }
            }

        /** The folders « Déplacer vers » offers, filtered by its box. */
        val moveFolders: List<FolderOptionView>
            get() {
                val tree = treeData ?: return emptyList()
                val term = moveTarget?.query?.trim()?.lowercase().orEmpty()
                return tree.folders
                    .filter { term.isEmpty() || it.title.lowercase().contains(term) }
                    .map { folder ->
                        FolderOptionView(
                            id = folder.id,
                            title = folder.title,
                            count = folder.count.toInt(),
                        )
                    }
            }

        /** « Sans dossier »'s own count, for the move sheet (`rootCount`). */
        val rootCount: Int get() = treeData?.let { rootPlaylists(it).size } ?: 0

        /** The folder whose name the move sheet states as the current one. */
        val currentFolderName: String?
            get() {
                val target = moveTarget ?: return null
                val tree = treeData ?: return null
                val id = target.currentFolder ?: return null
                return tree.folders.firstOrNull { it.id == id }?.title
            }
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var favoritesJob: Job? = null
    private var artistsJob: Job? = null
    private var albumsJob: Job? = null
    private var treeJob: Job? = null
    private var localJob: Job? = null
    private var unavailableJob: Job? = null
    private var dislikesJob: Job? = null
    private var foldersProbe: Job? = null

    /** One order write at a time: the SPA's 400 ms debounce, as one in flight. */
    private var orderJob: Job? = null
    private var orderPending = false

    /**
     * Opens the library on [leaf] (a route's tab, or the SPA's default) and reads
     * that one leaf: one request, and nothing else.
     */
    fun load(leaf: LibraryLeaf = LibraryLeaf.Favorites) {
        if (_state.value.leaf == leaf && favoritesJob?.isActive == true) return
        _state.update { it.copy(leaf = leaf) }
        probeFolders()
        selectLeaf(leaf)
    }

    /** A leaf of the strip: its data is read the first time it opens. */
    fun selectLeaf(leaf: LibraryLeaf) {
        _state.update { it.copy(leaf = leaf) }
        when (leaf) {
            LibraryLeaf.Favorites -> loadFavorites()
            LibraryLeaf.History -> Unit // `HistoryModel` owns this leaf's flow.
            LibraryLeaf.Artists -> loadArtists()
            LibraryLeaf.Albums -> loadAlbums()
            LibraryLeaf.Playlists -> loadTree()
            LibraryLeaf.Local -> loadLocal()
            LibraryLeaf.Unavailable -> loadUnavailable()
            LibraryLeaf.Dislikes -> loadDislikes()
        }
    }

    /** « Réessayer », from the failure block of the leaf on screen. */
    fun retry() {
        val failed = _state.value.failure ?: return
        failed.retry()
    }

    // -- favourites ------------------------------------------------------------

    private fun loadFavorites() {
        if (favoritesJob?.isActive == true || _state.value.favoriteRows != null) return
        favoritesJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedFavorites() } },
                fresh = { withContext(io) { service.favorites() } },
            ).collect { screen ->
                _state.update { it.copy(favorites = screen) }
                if (screen is ScreenState.Ready) {
                    applyFavoriteTools()
                }
            }
        }
    }

    /** The table's search box (`fQuery`): `appcore`'s fold, one pass. */
    fun filterFavorites(query: String) {
        _state.update { it.copy(favoriteFilter = query) }
        applyFavoriteTools()
    }

    /** The table's sort (`fSort`). */
    fun sortFavorites(sort: FavoriteSort) {
        _state.update { it.copy(favoriteSort = sort) }
        applyFavoriteTools()
    }

    /** « Inverser l'ordre » (`fDir`). */
    fun reverseFavorites() {
        _state.update { it.copy(favoriteDescending = !it.favoriteDescending) }
        applyFavoriteTools()
    }

    /** Applies the query and the order to the handle the screen windows. */
    private fun applyFavoriteTools() {
        val rows = _state.value.favoriteRows ?: return
        val current = _state.value
        rows.filter(current.favoriteFilter)
        rows.sort(current.favoriteSort.key, current.favoriteDescending)
    }

    /** « Tout lire »: every favourite, from the first (`playFavorites`). */
    fun playFavorites() = playAll(LibraryLeaf.Favorites)

    /** « Mélanger »: every favourite, shuffled (`shuffleFavorites`). */
    fun shuffleFavorites() {
        val rows = _state.value.favoriteRows ?: return
        scope.launch {
            val all = withContext(io) { rows.allRows() }
            if (all.isNotEmpty()) player.playShuffled(all, CONTEXT_FAVORITES, CONTEXT_FAVORITES)
        }
    }

    /** Swipe right on a row: « Lire ensuite » (the queue is the player's). */
    fun queueNext(row: TrackRow) = player.playNext(row)

    /** A tap on a row: that leaf's list, from there (`TrackList`'s own rule). */
    fun playFrom(row: TrackRow, leaf: LibraryLeaf = _state.value.leaf) {
        val (rows, context) = rowsOf(leaf) ?: return
        scope.launch {
            val all = withContext(io) { rows.allRows() }
            if (all.isEmpty()) return@launch
            val start = all.indexOfFirst { it.key == row.key }.coerceAtLeast(0)
            player.play(all, start, context, context)
        }
    }

    /** « Tout lire » of a leaf: its whole list, from the first. */
    fun playAll(leaf: LibraryLeaf = _state.value.leaf) {
        val (rows, context) = rowsOf(leaf) ?: return
        scope.launch {
            val all = withContext(io) { rows.allRows() }
            if (all.isNotEmpty()) player.play(all, 0, context, context)
        }
    }

    private fun rowsOf(leaf: LibraryLeaf): Pair<LibraryRows, String>? = when (leaf) {
        LibraryLeaf.Favorites -> _state.value.favoriteRows?.let { it to CONTEXT_FAVORITES }
        LibraryLeaf.Local -> _state.value.localRows?.let { it to CONTEXT_LOCAL }
        LibraryLeaf.Unavailable -> _state.value.unavailableRows?.let { it to CONTEXT_UNAVAILABLE }
        LibraryLeaf.Dislikes -> _state.value.dislikeRows?.let { it to CONTEXT_DISLIKES }
        else -> null
    }

    // -- artists and albums ----------------------------------------------------

    private fun loadArtists() {
        if (artistsJob?.isActive == true || _state.value.artistRows.isNotEmpty()) return
        artistsJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedArtists() } },
                fresh = { withContext(io) { service.artists() } },
            ).collect { screen -> _state.update { it.copy(artists = screen) } }
        }
    }

    fun filterArtists(query: String) = _state.update { it.copy(artistQuery = query) }

    fun sortArtists(sort: ArtistSort) = _state.update { it.copy(artistSort = sort) }

    private fun loadAlbums() {
        if (albumsJob?.isActive == true || _state.value.albumRows.isNotEmpty()) return
        albumsJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedAlbums() } },
                fresh = { withContext(io) { service.albums() } },
            ).collect { screen -> _state.update { it.copy(albums = screen) } }
        }
    }

    // -- playlists and folders -------------------------------------------------

    private fun loadTree() {
        if (treeJob?.isActive == true || _state.value.treeData != null) return
        treeJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedTree() } },
                fresh = { withContext(io) { service.tree() } },
            ).collect { screen -> _state.update { it.copy(tree = screen) } }
        }
    }

    /**
     * The folder probe (`Feature::Folders`, one `OPTIONS`): it decides whether
     * « Nouveau dossier » and « Organiser » exist on this server at all (rule 7).
     * A doubt (`null`) hides nothing.
     */
    private fun probeFolders() {
        if (foldersProbe?.isActive == true || _state.value.foldersSupported != null) return
        foldersProbe = scope.launch {
            val supported = withContext(io) {
                runCatching { service.foldersSupported() }.getOrNull()
            }
            _state.update { it.copy(foldersSupported = supported) }
        }
    }

    fun filterPlaylists(query: String) = _state.update { it.copy(playlistQuery = query) }

    /** A section's heading: the chevron folds it (`toggleSection`). */
    fun toggleSection(key: String) {
        _state.update { state ->
            state.copy(
                closed = if (state.closed.contains(key)) state.closed - key else state.closed + key,
            )
        }
    }

    /** The create/rename sheet (`openFolderSheet`): `null` creates, an id renames. */
    fun openFolderSheet(renameId: String?, renameTitle: String = "") {
        _state.update {
            it.copy(folderSheet = FolderSheet(renameOf = renameId, title = renameTitle))
        }
    }

    fun editFolderTitle(title: String) {
        _state.update { state ->
            state.copy(folderSheet = state.folderSheet?.copy(title = title, error = ""))
        }
    }

    fun closeFolderSheet() {
        if (_state.value.folderSheet?.busy == true) return
        _state.update { it.copy(folderSheet = null) }
    }

    /** « Créer » / « Renommer »: the tree answers now, the request follows. */
    fun submitFolderSheet() {
        val sheet = _state.value.folderSheet ?: return
        val title = sheet.title.trim()
        if (title.isEmpty()) {
            _state.update {
                it.copy(folderSheet = sheet.copy(error = "Le nom du dossier ne peut pas être vide."))
            }
            return
        }
        if (sheet.busy) return
        _state.update { it.copy(folderSheet = sheet.copy(busy = true, error = "")) }
        scope.launch {
            try {
                val next = withContext(io) {
                    if (sheet.renameOf != null) {
                        service.renameFolder(sheet.renameOf, title)
                    } else {
                        service.createFolder(title)
                    }
                }
                _state.update { it.copy(tree = ScreenState.Ready(next), folderSheet = null) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(
                        folderSheet = sheet.copy(
                            busy = false,
                            error = "${NzErrors.text(e)}. Réessayer.",
                        ),
                    )
                }
            }
        }
    }

    /**
     * « Supprimer le dossier »: the tree loses it and its playlists fall back to
     * « Sans dossier » — then ONE `DELETE`. A refusal puts both back.
     *
     * The SPA's « Annuler » is NOT ported: the design system's toast carries no
     * action (`NzToasts.show(text)`) and adding one there is MB1's file. The
     * toast states how many playlists moved instead, so nothing is silent.
     */
    fun deleteFolder(folderId: String) {
        val before = _state.value.treeData ?: return
        if (_state.value.busy != null) return
        val released = containerPlaylists(before, folderId).size
        _state.update { it.copy(busy = "folder:$folderId") }
        scope.launch {
            try {
                val next = withContext(io) { service.deleteFolder(folderId) }
                _state.update { it.copy(tree = ScreenState.Ready(next), busy = null) }
                toast(
                    if (released > 0) {
                        "Dossier supprimé · ${plural(released, "playlist déplacée", "playlists déplacées")} dans « Sans dossier »"
                    } else {
                        "Dossier supprimé"
                    },
                )
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(tree = ScreenState.Ready(before), busy = null) }
                toast("Impossible de supprimer le dossier : ${NzErrors.text(e)}. Réessayer.")
            }
        }
    }

    /** « Déplacer le dossier vers le haut / le bas » (`sectionMenu`). */
    fun moveFolder(folderId: String, delta: Int) {
        val tree = _state.value.treeData ?: return
        val ids = tree.folders.map { it.id }.toMutableList()
        val from = ids.indexOf(folderId)
        val to = from + delta
        if (from < 0 || to < 0 || to >= ids.size) return
        ids.add(to, ids.removeAt(from))
        _state.update { it.copy(tree = ScreenState.Ready(orderFoldersLocally(tree, ids))) }
        writeFolderOrder()
    }

    /** « Organiser les playlists » (`openOrganizer`). */
    fun openOrganizer() {
        _state.update {
            it.copy(organizing = true, container = CONTAINER_FOLDERS, organizeStatus = "")
        }
    }

    /** « Terminer »: what the person left with is already saved. */
    fun closeOrganizer() = _state.update { it.copy(organizing = false) }

    /** A container chip of the organiser (`pickContainer`). */
    fun pickContainer(container: String) {
        _state.update { it.copy(container = container, organizeStatus = "") }
    }

    /** A row of the organiser moved one place up or down (`organizeMove`). */
    fun organizeMove(index: Int, delta: Int) {
        val tree = _state.value.treeData ?: return
        val rows = _state.value.organizeRows
        val to = index + delta
        if (to < 0 || to >= rows.size) return
        val ids = rows.map { it.id }.toMutableList()
        ids.add(to, ids.removeAt(index))
        val container = _state.value.container
        val next = if (container == CONTAINER_FOLDERS) {
            orderFoldersLocally(tree, ids)
        } else {
            orderContainerLocally(tree, if (container == CONTAINER_ROOT) null else container, ids)
        }
        _state.update {
            it.copy(tree = ScreenState.Ready(next), organizeStatus = "Déplacé en position ${to + 1} sur ${rows.size}")
        }
        if (container == CONTAINER_FOLDERS) writeFolderOrder() else writeContainerOrder(container)
    }

    /** The destination of « Déplacer… » (`openMoveToFolder`). */
    fun openMoveSheet(playlistId: String) {
        val tree = _state.value.treeData ?: return
        val playlist = tree.playlists.firstOrNull { it.id == playlistId } ?: return
        _state.update {
            it.copy(
                moveTarget = MoveTarget(
                    playlistId = playlistId,
                    title = playlist.title,
                    currentFolder = playlist.folderId,
                ),
            )
        }
    }

    fun filterMoveTargets(query: String) {
        _state.update { state -> state.copy(moveTarget = state.moveTarget?.copy(query = query)) }
    }

    fun closeMoveSheet() {
        if (_state.value.moveTarget?.busy == true) return
        _state.update { it.copy(moveTarget = null) }
    }

    /**
     * `actions.js#movePlaylistTo`: the playlist goes to the END of the folder,
     * locally first, and the ONE request is the destination's order. A refusal —
     * offline included — puts the row back and says « Réessayer ».
     */
    fun movePlaylist(folderId: String?) {
        val target = _state.value.moveTarget ?: return
        val before = _state.value.treeData ?: return
        if (target.busy) return
        if (target.currentFolder == folderId) {
            _state.update { it.copy(moveTarget = null) }
            return
        }
        val ids = containerPlaylists(before, folderId).map { it.id }.filter { it != target.playlistId } +
            target.playlistId
        _state.update {
            it.copy(
                tree = ScreenState.Ready(moveLocally(before, target.playlistId, folderId, ids)),
                moveTarget = target.copy(busy = true),
            )
        }
        scope.launch {
            try {
                val next = withContext(io) { service.movePlaylistTo(target.playlistId, folderId) }
                _state.update { it.copy(tree = ScreenState.Ready(next), moveTarget = null) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(tree = ScreenState.Ready(before), moveTarget = target.copy(busy = false))
                }
                toast("Impossible de déplacer la playlist : ${NzErrors.text(e)}. Réessayer.")
            }
        }
    }

    /** One write of the folders' order, the latest order winning. */
    private fun writeFolderOrder() {
        if (orderJob?.isActive == true) {
            orderPending = true
            return
        }
        val ids = _state.value.treeData?.folders?.map { it.id }.orEmpty()
        if (ids.isEmpty()) return
        orderJob = scope.launch {
            try {
                val next = withContext(io) { service.reorderFolders(ids) }
                _state.update { it.copy(tree = ScreenState.Ready(next)) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Ordre non enregistré : ${NzErrors.text(e)}. Réessayer.")
            } finally {
                // This job is over: the next write must see no write in flight
                // (checking `isActive` from inside the job's own `finally` would
                // always be true and the pending order would be lost).
                orderJob = null
                if (orderPending) {
                    orderPending = false
                    writeFolderOrder()
                }
            }
        }
    }

    /** The same, for one container's playlists. */
    private fun writeContainerOrder(container: String) {
        if (orderJob?.isActive == true) {
            orderPending = true
            return
        }
        val tree = _state.value.treeData ?: return
        val folder = if (container == CONTAINER_ROOT) null else container
        val ids = containerPlaylists(tree, folder).map { it.id }
        if (ids.isEmpty()) return
        orderJob = scope.launch {
            try {
                val next = withContext(io) { service.reorderContainer(folder, ids) }
                _state.update { it.copy(tree = ScreenState.Ready(next)) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Ordre non enregistré : ${NzErrors.text(e)}. Réessayer.")
            } finally {
                orderJob = null
                if (orderPending) {
                    orderPending = false
                    writeContainerOrder(container)
                }
            }
        }
    }

    // -- mes fichiers, indisponibles, je n'aime pas ----------------------------

    private fun loadLocal() {
        if (localJob?.isActive == true || _state.value.localRows != null) return
        localJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedLocal() } },
                fresh = { withContext(io) { service.local() } },
            ).collect { screen -> _state.update { it.copy(local = screen) } }
            // The quota bar is THIS leaf's own request, never the library's.
            runCatching { withContext(io) { service.cachedUsage() } }
                .getOrNull()
                ?.let { usage -> _state.update { it.copy(usage = usage) } }
            runCatching { withContext(io) { service.usage() } }
                .onSuccess { usage -> _state.update { it.copy(usage = usage) } }
        }
    }

    private fun loadUnavailable() {
        if (unavailableJob?.isActive == true || _state.value.unavailableRows != null) return
        unavailableJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedUnavailable() } },
                fresh = { withContext(io) { service.unavailable() } },
            ).collect { screen -> _state.update { it.copy(unavailable = screen) } }
        }
    }

    private fun loadDislikes() {
        if (dislikesJob?.isActive == true || _state.value.dislikeRows != null) return
        dislikesJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedDislikes() } },
                fresh = { withContext(io) { service.dislikes() } },
            ).collect { screen -> _state.update { it.copy(dislikes = screen) } }
        }
    }

    /**
     * « Retirer » on « Je n'aime pas » (`DislikedTracks.svelte#remove`): ONE
     * `POST /api/dislike {on:false}`, then the list is re-read.
     *
     * The re-read after a success is this port's one deliberate difference from
     * the SPA: a `ListHandle` is immutable by contract (MB2's list; MB10 owns the
     * mutation that patches every open list), so the list is read again instead
     * of being spliced locally — one `User`-lane request per removal.
     */
    fun removeDislike(row: TrackRow) {
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = "dislike:${row.key}") }
        scope.launch {
            try {
                withContext(io) { service.removeDislike(row.id) }
                _state.update { it.copy(busy = null) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(busy = null) }
                toast("Impossible de retirer ce titre : ${NzErrors.text(e)}. Réessayer.")
            }
            reloadDislikes()
        }
    }

    private fun reloadDislikes() {
        dislikesJob?.cancel()
        dislikesJob = scope.launch {
            runCatching { withContext(io) { service.dislikes() } }
                .onSuccess { rows -> _state.update { it.copy(dislikes = ScreenState.Ready(rows)) } }
                .onFailure { cause ->
                    if (cause is CancellationException) throw cause
                }
        }
    }

    companion object {
        /** `Library.svelte#organizeContainer`: the folders themselves. */
        const val CONTAINER_FOLDERS = "folders"

        /** `Library.svelte#organizeContainer`: « Sans dossier ». */
        const val CONTAINER_ROOT = "root"

        /** `Player.svelte`'s context kinds: what the queue's refill reads. */
        const val CONTEXT_FAVORITES = "favorites"
        const val CONTEXT_LOCAL = "local"
        const val CONTEXT_UNAVAILABLE = "unavailable"
        const val CONTEXT_DISLIKES = "dislikes"
    }
}

/** `Library.svelte`'s `plural`: « 1 titre », « 12 titres ». */
internal fun plural(count: Int, one: String, many: String): String =
    "$count ${if (count > 1) many else one}"

/** `Library.svelte#cardSubtitle`: an owner and a count for a shared list. */
internal fun cardSubtitle(row: PlaylistRow): String {
    val tracks = plural(row.tracks.toInt(), "titre", "titres")
    return if (row.role != "owner" && row.role.isNotEmpty() && row.owner.isNotEmpty()) {
        "${row.owner} · $tracks"
    } else {
        tracks
    }
}

/** `fold` in `Library.svelte`: accents and case, the way the SPA filters names. */
internal fun fold(text: String): String =
    java.text.Normalizer.normalize(text, java.text.Normalizer.Form.NFD)
        .replace(Regex("\\p{Mn}+"), "")
        .lowercase()

/** The playlists one container holds, in the tree's order. */
internal fun containerPlaylists(tree: LibraryTree, folder: String?): List<PlaylistRow> =
    tree.playlists.filter { it.folderId == folder }

internal fun rootPlaylists(tree: LibraryTree): List<PlaylistRow> = containerPlaylists(tree, null)

/** `orderFolders`, locally: the ids' order, then anything they did not name. */
internal fun orderFoldersLocally(tree: LibraryTree, ids: List<String>): LibraryTree {
    val byId = tree.folders.associateBy { it.id }
    val folders = ids.mapNotNull { byId[it] } + tree.folders.filter { it.id !in ids }
    return tree.copy(folders = folders)
}

/** `orderContainer`, locally: the named rows in that order, then the rest. */
internal fun orderContainerLocally(
    tree: LibraryTree,
    folder: String?,
    ids: List<String>,
): LibraryTree {
    val inside = containerPlaylists(tree, folder)
    val ordered = ids.mapNotNull { id -> inside.firstOrNull { it.id == id } } +
        inside.filter { it.id !in ids }
    val outside = tree.playlists.filter { it.folderId != folder }
    return tree.copy(playlists = outside + ordered)
}

/** `movePlaylistTo`, locally: the row joins the END of the destination. */
internal fun moveLocally(
    tree: LibraryTree,
    playlistId: String,
    folder: String?,
    ids: List<String>,
): LibraryTree {
    val moved = tree.playlists.firstOrNull { it.id == playlistId } ?: return tree
    val destination = ids.mapNotNull { id ->
        if (id == playlistId) {
            moved.copy(folderId = folder)
        } else {
            tree.playlists.firstOrNull { it.id == id }?.copy(folderId = folder)
        }
    }
    val outside = tree.playlists.filter { row -> destination.none { it.id == row.id } }
    return tree.copy(playlists = outside + destination)
}

/** The whole list, for a « Tout lire » that must not cost a request. */
internal fun LibraryRows.allRows(): List<TrackRow> {
    val total = len()
    if (total <= 0) return emptyList()
    val rows = ArrayList<TrackRow>(total)
    var start = 0
    while (start < total) {
        val page = window(start, minOf(128, total - start))
        if (page.isEmpty()) break
        rows += page
        start += page.size
    }
    return rows
}

/** « Sans dossier »'s destination row in « Déplacer vers » (`FolderOptionView`). */
internal fun rootFolderOption(state: LibraryModel.State): FolderOptionView =
    FolderOptionView(id = "", title = "Sans dossier", count = state.rootCount)
