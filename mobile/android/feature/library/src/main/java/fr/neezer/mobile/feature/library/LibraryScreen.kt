package fr.neezer.mobile.feature.library

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.ListWindow
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzItemSpecs
import fr.neezer.mobile.designsystem.NzPaneEnter
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSkeletonKind
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.Skeleton
import fr.neezer.mobile.designsystem.nzPaneEnter
import fr.neezer.mobile.designsystem.rememberPaneEnter
import uniffi.nz_ffi.LibraryAlbum
import uniffi.nz_ffi.LibraryArtist
import uniffi.nz_ffi.TrackRow

/**
 * What the library screen may ask (lot MB8). One value per door, so the screen is
 * a pure function of [LibraryModel.State] and a render or a preview passes
 * no-ops.
 */
class LibraryActions(
    val selectLeaf: (LibraryLeaf) -> Unit = {},
    val retry: () -> Unit = {},
    val filterFavorites: (String) -> Unit = {},
    val sortFavorites: (FavoriteSort) -> Unit = {},
    val reverseFavorites: () -> Unit = {},
    val playLeaf: (LibraryLeaf) -> Unit = {},
    val shuffleFavorites: () -> Unit = {},
    val playFrom: (TrackRow) -> Unit = {},
    val queueNext: (TrackRow) -> Unit = {},
    val more: (TrackRow) -> Unit = {},
    val filterArtists: (String) -> Unit = {},
    val sortArtists: (ArtistSort) -> Unit = {},
    val openArtist: (String) -> Unit = {},
    val openAlbum: (String) -> Unit = {},
    val filterPlaylists: (String) -> Unit = {},
    val toggleSection: (String) -> Unit = {},
    val openFolderSheet: (String?, String) -> Unit = { _, _ -> },
    val editFolderTitle: (String) -> Unit = {},
    val closeFolderSheet: () -> Unit = {},
    val submitFolderSheet: () -> Unit = {},
    val deleteFolder: (String) -> Unit = {},
    val moveFolder: (String, Int) -> Unit = { _, _ -> },
    val organize: () -> Unit = {},
    val closeOrganizer: () -> Unit = {},
    val pickContainer: (String) -> Unit = {},
    val organizeMove: (Int, Int) -> Unit = { _, _ -> },
    val openMove: (String) -> Unit = {},
    val filterMoveTargets: (String) -> Unit = {},
    val movePlaylist: (String?) -> Unit = {},
    val closeMoveSheet: () -> Unit = {},
    val openPlaylist: (String) -> Unit = {},
    val openSearch: () -> Unit = {},
    /** L'icône de lien de l'en-tête (`routes/Library.svelte:611`). */
    val openImport: () -> Unit = {},
    /** « Nouvelle playlist » de la feuille d'ajout. */
    val newPlaylist: (String) -> Unit = {},
    val removeDislike: (TrackRow) -> Unit = {},
    /** The history leaf's body — `HistoryScreen`, owned by the same lot. */
    val historyBody: @Composable () -> Unit = {},
)

/**
 * Bibliothèque — lot MB8.
 *
 * The SPA's screen in the phone's shell: a head (title, counters, search, the
 * import link and the add action), a strip of leaves, the leaf's own tools, then
 * ONE leaf. The structures follow `routes/Library.svelte`:
 *
 * - the « Titres » table is windowed over the core's handle (`ListHandle` →
 *   `ListWindow`), its search and sort go through `appcore`'s own fold, and
 *   « Tout lire » / « Mélanger » hand the whole list to the player;
 * - the grids (artists, albums, playlists) are laid out in rows of cards and
 *   composed lazily, so a 2 000-playlist library costs nothing until it is read;
 * - the « Playlists » sections are the core's (`groupPlaylists` +
 *   `buildSections`), each foldable, and « Organiser » swaps them for the ordered
 *   rows the SPA's organiser has;
 * - every list has its empty state (`webapp.md` §6), and a failure is its cause
 *   with « Réessayer », never a spinner.
 *
 * `playingKey` is MB11's: the shell feeds it so the playing row is highlighted;
 * it is null today (the parameter exists, as on MB7's pages).
 */
@Composable
fun LibraryScreen(
    state: LibraryModel.State,
    actions: LibraryActions,
    modifier: Modifier = Modifier,
    playingKey: String? = null,
) {
    val colors = Sillon.colors
    val listState = rememberLazyListState()
    var sectionMenu by remember { mutableStateOf<SectionView?>(null) }
    var addMenu by remember { mutableStateOf(false) }
    var newPlaylistSheet by remember { mutableStateOf(false) }
    var sortSheet by remember { mutableStateOf(false) }
    val openSectionMenu: (SectionView) -> Unit = { sectionMenu = it }

    // A leaf is a new pane: land at its top, as the SPA's panel does.
    LaunchedEffect(state.leaf) { listState.scrollToItem(0) }

    // Un nouvel ordre est un nouveau haut de liste : on ne laisse pas la personne
    // au milieu d'une liste qui vient de changer sous ses yeux (Apple/Spotify).
    LaunchedEffect(state.favoriteSort, state.favoriteDescending) {
        if (state.leaf == LibraryLeaf.Favorites) listState.scrollToItem(0)
    }

    val favoritesRows = state.favoriteRows
    val favoritesWindow = if (favoritesRows != null) windowFor(favoritesRows) else null
    val localRows = state.localRows
    val localWindow = if (localRows != null) windowFor(localRows) else null
    val unavailableRows = state.unavailableRows
    val unavailableWindow = if (unavailableRows != null) windowFor(unavailableRows) else null
    val listWindow: ListWindow? = when (state.leaf) {
        LibraryLeaf.Favorites -> favoritesWindow
        LibraryLeaf.Local -> localWindow
        LibraryLeaf.Unavailable -> unavailableWindow
        LibraryLeaf.Dislikes -> null // a short card, drawn whole (`DislikedSection`)
        else -> null
    }
    if (listWindow != null) {
        ReportWindow(listState, listWindow, WINDOW_OFFSET)
    }

    // The leaf's CONTENT is one pane: its key is the leaf AND its phase, so a leaf
    // change and a skeleton → content fade the content in, while the head, the
    // strip of leaves and the leaf's tools stay put (the decision note's D2 — an
    // `AnimatedContent` around the `LazyColumn` would fade the strip with it).
    // The History leaf is `HistoryScreen`'s own pane: it animates its phases itself,
    // and a second fade over it would multiply the two.
    val pane = rememberPaneEnter(state.leaf to libraryPhase(state))
    // Read here, in the composition: `leaf`/`trackRows` run in the LazyColumn's
    // scope, where a `CompositionLocal` cannot be read.
    val specs = Sillon.motion.itemSpecs()

    BoxWithConstraints(
        modifier
            .fillMaxSize()
            .semantics { contentDescription = LIBRARY_MARKER },
    ) {
        val columns = if (maxWidth >= 600.dp) 4 else 2
        Box(Modifier.fillMaxSize().background(colors.bg)) {
            LazyColumn(
                state = listState,
                modifier = Modifier.fillMaxSize().testTag(LIBRARY_LIST_TAG),
                contentPadding = PaddingValues(bottom = NzSpacing.pageBottom),
            ) {
                item(key = "head") {
                    LibraryHead(
                        state,
                        onAdd = { addMenu = true },
                        onImport = actions.openImport,
                        onSearch = actions.openSearch,
                    )
                }
                item(key = "leaves") { LeafBar(state, actions.selectLeaf) }
                item(key = "tools") {
                    LeafTools(state, actions, openSort = { sortSheet = true })
                }
                when (state.leaf) {
                    LibraryLeaf.History -> item(key = "history") { actions.historyBody() }
                    else -> leaf(state, actions, columns, playingKey, listWindow, openSectionMenu, pane, specs)
                }
            }

            // The overlays live above the page, as the SPA's sheets do.
            val sheet = state.folderSheet
            if (sheet != null) {
                FolderSheetView(sheet, actions)
            }
            val target = state.moveTarget
            if (target != null) {
                MoveSheetView(state, target, actions)
            }
            val menu = sectionMenu
            if (menu != null) {
                SectionSheet(
                    section = menu,
                    state = state,
                    onDismiss = { sectionMenu = null },
                    actions = actions,
                )
            }
            if (addMenu) {
                AddSheet(
                    onDismiss = { addMenu = false },
                    onNewFolder = { addMenu = false; actions.openFolderSheet(null, "") },
                    onOrganize = { addMenu = false; actions.organize() },
                    onNewPlaylist = { addMenu = false; newPlaylistSheet = true },
                    onImport = { addMenu = false; actions.openImport() },
                    canFolders = state.foldersSupported != false,
                )
            }
            if (newPlaylistSheet) {
                NewPlaylistSheet(
                    onDismiss = { newPlaylistSheet = false },
                    onCreate = { name -> newPlaylistSheet = false; actions.newPlaylist(name) },
                )
            }
            if (sortSheet) {
                SortSheet(
                    sort = state.favoriteSort,
                    descending = state.favoriteDescending,
                    onPick = { sort -> actions.sortFavorites(sort); sortSheet = false },
                    onReverse = actions.reverseFavorites,
                    onDismiss = { sortSheet = false },
                )
            }
        }
    }
}

/**
 * The page's own scrolling list, for the device test: the strip of leaves and the
 * sort chips scroll sideways, so `hasScrollAction()` alone names three nodes.
 * A test tag is invisible to TalkBack (unlike a content description).
 */
const val LIBRARY_LIST_TAG = "library-list"

/** The head's column, for S2's page-margin assertion (the title sits centred in
 * a 48 dp icon row, so its own top is not the page's margin). */
const val LIBRARY_HEAD_TAG = "library-head"

/** The strip of leaves: X4 asserts a leaf change never re-lays it out (± 0 dp). */
const val LIBRARY_LEAVES_TAG = "library-leaves"

/** The three lazily-composed items above a windowed list's first row. */
private const val WINDOW_OFFSET = 3

/**
 * What a leaf is drawing: its data arrived, its read failed, it has nothing to
 * show, or nothing yet. It is the phase of the leaf's PANE (the entrance its
 * content plays when the leaf or the phase changes), computed OUTSIDE the
 * `LazyColumn` from the same fields the branches below test — a second field
 * would be a second truth.
 */
private enum class LibraryPhase { Loading, Failed, Empty, Content }

private fun libraryPhase(state: LibraryModel.State): LibraryPhase = when (state.leaf) {
    LibraryLeaf.Favorites -> libraryPhase(state.favoriteRows, state.failure, state.favoriteRows?.len() == 0)
    LibraryLeaf.Artists -> libraryPhase(
        (state.artists as? ScreenState.Ready)?.data,
        state.failure,
        state.shownArtists.isEmpty(),
    )
    LibraryLeaf.Albums -> libraryPhase(
        (state.albums as? ScreenState.Ready)?.data,
        state.failure,
        state.albumRows.isEmpty(),
    )
    LibraryLeaf.Playlists -> {
        val tree = state.treeData
        libraryPhase(tree, state.failure, tree != null && tree.playlists.isEmpty() && tree.folders.isEmpty())
    }
    LibraryLeaf.Local -> libraryPhase(state.localRows, state.failure, state.localRows?.len() == 0)
    LibraryLeaf.Unavailable -> libraryPhase(state.unavailableRows, state.failure, state.unavailableRows?.len() == 0)
    LibraryLeaf.Dislikes -> libraryPhase(state.dislikeRows, state.failure, state.dislikeRows?.len() == 0)
    // `HistoryScreen` owns this leaf's phases and its own pane; the library's is
    // settled here so a leaf change into or out of it plays exactly one fade.
    LibraryLeaf.History -> LibraryPhase.Content
}

private fun libraryPhase(data: Any?, failure: ScreenState.Failed?, empty: Boolean): LibraryPhase = when {
    failure != null && data == null -> LibraryPhase.Failed
    data == null -> LibraryPhase.Loading
    empty -> LibraryPhase.Empty
    else -> LibraryPhase.Content
}

/** The head: the page's title, the counters, search and the one add action. */
@Composable
private fun LibraryHead(
    state: LibraryModel.State,
    onAdd: () -> Unit,
    onImport: () -> Unit,
    onSearch: () -> Unit,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = NzSpacing.pageX, end = NzSpacing.pageX, top = NzSpacing.pageY)
            .testTag(LIBRARY_HEAD_TAG),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = "Bibliothèque",
                style = Sillon.type.displaySmall,
                color = colors.tx,
                modifier = Modifier.weight(1f),
            )
            NzIconButton(
                glyph = NzGlyph.Search,
                contentDescription = "Explorer",
                onClick = onSearch,
            )
            NzIconButton(
                glyph = NzGlyph.Link,
                contentDescription = "Importer une playlist partagée",
                onClick = onImport,
            )
            NzIconButton(
                glyph = NzGlyph.Plus,
                contentDescription = "Ajouter à la bibliothèque",
                onClick = onAdd,
            )
        }
        if (state.counts.isNotEmpty()) {
            Text(
                text = state.counts,
                style = Sillon.type.mono,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/**
 * The strip of leaves, on one line (`LibraryNav.svelte`'s leaf row).
 *
 * A leaf deeper in the strip is scrolled back into view when it is chosen — the
 * SPA's `revealSelected` — so the ACTIVE leaf is never the one cut off at the
 * edge (measured at 360 dp: « Playlists » sat entirely off-screen otherwise).
 */
@Composable
private fun LeafBar(state: LibraryModel.State, onPick: (LibraryLeaf) -> Unit) {
    val leaves = LibraryLeaf.entries
    val strip = rememberLazyListState()
    LaunchedEffect(state.leaf) {
        val index = leaves.indexOf(state.leaf)
        if (index >= 0) strip.animateScrollToItem(index)
    }
    androidx.compose.foundation.lazy.LazyRow(
        state = strip,
        modifier = Modifier.fillMaxWidth().testTag(LIBRARY_LEAVES_TAG),
        contentPadding = PaddingValues(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        items(leaves.size) { index ->
            val leaf = leaves[index]
            NzChip(
                text = leaf.label,
                selected = state.leaf == leaf,
                onClick = { onPick(leaf) },
            )
        }
    }
}

/** The toolbar of the leaf on screen: its tools, and its stale note. */
@Composable
private fun LeafTools(
    state: LibraryModel.State,
    actions: LibraryActions,
    openSort: () -> Unit,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.pageX),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        if (state.stale) LibraryStaleNote()
        when (state.leaf) {
            LibraryLeaf.Favorites -> if (state.favoriteRows != null) {
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                    NzButton(
                        text = "Tout lire",
                        onClick = { actions.playLeaf(LibraryLeaf.Favorites) },
                        glyph = NzGlyph.Play,
                    )
                    NzButton(
                        text = "Mélanger",
                        onClick = actions.shuffleFavorites,
                        glyph = NzGlyph.Shuffle,
                        kind = NzButtonKind.Ghost,
                    )
                }
                NzSearchField(
                    value = state.favoriteFilter,
                    onValueChange = actions.filterFavorites,
                    placeholder = "Rechercher dans cette liste…",
                    label = "Filtrer dans cette liste",
                )
                SortDoor(
                    sort = state.favoriteSort,
                    descending = state.favoriteDescending,
                    onClick = openSort,
                )
            }
            LibraryLeaf.Artists -> {
                NzSearchField(
                    value = state.artistQuery,
                    onValueChange = actions.filterArtists,
                    placeholder = "Rechercher un artiste…",
                    label = "Rechercher un artiste",
                )
                Row(
                    modifier = Modifier.horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                ) {
                    ArtistSort.entries.forEach { sort ->
                        NzChip(
                            text = sort.label,
                            selected = state.artistSort == sort,
                            onClick = { actions.sortArtists(sort) },
                        )
                    }
                }
            }
            LibraryLeaf.Albums -> Unit
            LibraryLeaf.Playlists -> {
                if (!state.organizing) {
                    NzSearchField(
                        value = state.playlistQuery,
                        onValueChange = actions.filterPlaylists,
                        placeholder = "Rechercher dans mes playlists…",
                        label = "Rechercher une playlist",
                    )
                }
            }
            LibraryLeaf.Local -> if (state.localRows != null) {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    NzButton(
                        text = "Tout lire",
                        onClick = { actions.playLeaf(LibraryLeaf.Local) },
                        glyph = NzGlyph.Play,
                    )
                    MonoLine(plural(state.localRows?.len() ?: 0, "fichier", "fichiers"))
                }
                state.usage?.let { usage -> QuotaBar(usage) }
            }
            LibraryLeaf.Unavailable -> {
                Text(
                    text = "Ces titres ne peuvent plus être lus. Un remplaçant prend leur place dans toutes les playlists et les favoris.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
            LibraryLeaf.Dislikes -> {
                Text(
                    text = "Ils ne reviennent plus dans le Flow, les radios et les mixes. « Retirer » les réhabilite.",
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                )
            }
            LibraryLeaf.History -> Unit
        }
        Spacer(Modifier.height(NzSpacing.xxs))
    }
}

/** One leaf's content, as the `LazyColumn`'s items. */
private fun LazyListScope.leaf(
    state: LibraryModel.State,
    actions: LibraryActions,
    columns: Int,
    playingKey: String?,
    window: ListWindow?,
    onSectionMenu: (SectionView) -> Unit,
    pane: NzPaneEnter,
    specs: NzItemSpecs,
) {
    val side = Modifier.nzPaneEnter(pane).padding(horizontal = NzSpacing.pageX)
    when (state.leaf) {
        LibraryLeaf.Favorites -> {
            val failure = state.failure
            val rows = state.favoriteRows
            when {
                failure != null && rows == null -> item(key = "fail") {
                    LoadFailed(
                        cause = failure.cause,
                        onRetry = actions.retry,
                        title = "Tes favoris n'ont pas pu se charger",
                        modifier = side,
                    )
                }
                rows == null -> item(key = "skeleton") {
                    Skeleton(kind = NzSkeletonKind.List, count = 8, modifier = side)
                }
                rows.len() == 0 && state.favoriteFilter.isNotBlank() -> item(key = "none") {
                    Text(
                        text = "Aucun titre ne correspond à « ${state.favoriteFilter} ».",
                        style = Sillon.type.bodySmall,
                        color = Sillon.colors.dim,
                        modifier = side,
                    )
                }
                rows.len() == 0 -> item(key = "empty") {
                    EmptyState(
                        title = "Rien ici pour l'instant",
                        body = "Touche le cœur d'un titre pour le garder ici.",
                        glyph = NzGlyph.Heart,
                        actionLabel = "Explorer",
                        onAction = actions.openSearch,
                        modifier = Modifier.nzPaneEnter(pane),
                    )
                }
                window != null -> trackRows(
                    window = window,
                    playingKey = playingKey,
                    onPlay = actions.playFrom,
                    onMore = actions.more,
                    onQueue = actions.queueNext,
                    pane = pane,
                    specs = specs,
                )
            }
        }
        LibraryLeaf.Artists -> {
            val failure = state.failure
            val loaded = state.artists is ScreenState.Ready
            val artists = state.shownArtists
            when {
                failure != null && !loaded -> item(key = "fail") {
                    LoadFailed(
                        cause = failure.cause,
                        onRetry = actions.retry,
                        title = "Tes artistes n'ont pas pu se charger",
                        modifier = side,
                    )
                }
                !loaded -> item(key = "skeleton") {
                    Skeleton(kind = NzSkeletonKind.Shelf, modifier = side)
                }
                artists.isEmpty() -> item(key = "empty") {
                    EmptyState(
                        title = if (state.artistQuery.isBlank()) {
                            "Aucun artiste suivi"
                        } else {
                            "Aucun artiste ne correspond"
                        },
                        body = if (state.artistQuery.isBlank()) {
                            "Ouvre la page d'un artiste et touche le cœur pour le suivre : ses nouveautés arriveront sur l'accueil."
                        } else {
                            "Aucun artiste ne correspond à « ${state.artistQuery} »."
                        },
                        glyph = NzGlyph.User,
                        actionLabel = if (state.artistQuery.isBlank()) "Explorer" else null,
                        onAction = if (state.artistQuery.isBlank()) actions.openSearch else null,
                        modifier = Modifier.nzPaneEnter(pane),
                    )
                }
                else -> gridItems(key = "artists", rows = artists, columns = columns, pane = pane) { artist ->
                    ArtistTile(
                        artist = artist,
                        onClick = { actions.openArtist(artist.id) },
                    )
                }
            }
        }
        LibraryLeaf.Albums -> {
            val failure = state.failure
            val loaded = state.albums is ScreenState.Ready
            val albums = state.albumRows
            when {
                failure != null && !loaded -> item(key = "fail") {
                    LoadFailed(
                        cause = failure.cause,
                        onRetry = actions.retry,
                        title = "Tes albums n'ont pas pu se charger",
                        modifier = side,
                    )
                }
                !loaded -> item(key = "skeleton") {
                    Skeleton(kind = NzSkeletonKind.Shelf, modifier = side)
                }
                albums.isEmpty() -> item(key = "empty") {
                    EmptyState(
                        title = "Aucun album gardé",
                        body = "Touche le cœur sur la page d'un album pour le garder ici.",
                        glyph = NzGlyph.Disc,
                        actionLabel = "Parcourir les albums",
                        onAction = actions.openSearch,
                        modifier = Modifier.nzPaneEnter(pane),
                    )
                }
                else -> gridItems(key = "albums", rows = albums, columns = columns, pane = pane) { album ->
                    AlbumTile(card = album, onClick = { actions.openAlbum(album.id) })
                }
            }
        }
        LibraryLeaf.Playlists -> playlists(state, actions, columns, onSectionMenu, pane)
        LibraryLeaf.Local -> {
            val failure = state.failure
            val rows = state.localRows
            when {
                failure != null && rows == null -> item(key = "fail") {
                    LoadFailed(
                        cause = failure.cause,
                        onRetry = actions.retry,
                        title = "Tes fichiers n'ont pas pu se charger",
                        modifier = side,
                    )
                }
                rows == null -> item(key = "skeleton") {
                    Skeleton(kind = NzSkeletonKind.List, count = 6, modifier = side)
                }
                rows.len() == 0 -> item(key = "empty") {
                    EmptyState(
                        title = "Aucun fichier à toi",
                        body = "Tes propres fichiers apparaissent ici dès que le serveur les a importés.",
                        glyph = NzGlyph.Upload,
                        actionLabel = "Explorer",
                        onAction = actions.openSearch,
                        modifier = Modifier.nzPaneEnter(pane),
                    )
                }
                window != null -> trackRows(
                    window = window,
                    playingKey = playingKey,
                    onPlay = actions.playFrom,
                    onMore = actions.more,
                    onQueue = actions.queueNext,
                    pane = pane,
                    specs = specs,
                )
            }
        }
        LibraryLeaf.Unavailable -> {
            val failure = state.failure
            val rows = state.unavailableRows
            when {
                failure != null && rows == null -> item(key = "fail") {
                    LoadFailed(
                        cause = failure.cause,
                        onRetry = actions.retry,
                        title = "Les titres indisponibles n'ont pas pu se charger",
                        modifier = side,
                    )
                }
                rows == null -> item(key = "skeleton") {
                    Skeleton(kind = NzSkeletonKind.List, count = 6, modifier = side)
                }
                rows.len() == 0 -> item(key = "empty") {
                    EmptyState(
                        title = "Aucun titre indisponible",
                        body = "Quand un fournisseur retire un titre, il apparaît ici pour qu'un remplaçant prenne sa place.",
                        glyph = NzGlyph.Check,
                        actionLabel = "Écouter mes favoris",
                        onAction = { actions.selectLeaf(LibraryLeaf.Favorites) },
                        modifier = Modifier.nzPaneEnter(pane),
                    )
                }
                window != null -> trackRows(
                    window = window,
                    playingKey = playingKey,
                    onPlay = actions.playFrom,
                    onMore = actions.more,
                    onQueue = actions.queueNext,
                    pane = pane,
                    specs = specs,
                )
            }
        }
        LibraryLeaf.Dislikes -> {
            val failure = state.failure
            val rows = state.dislikeRows
            when {
                failure != null && rows == null -> item(key = "fail") {
                    LoadFailed(
                        cause = failure.cause,
                        onRetry = actions.retry,
                        title = "La liste n'a pas pu se charger",
                        modifier = side,
                    )
                }
                rows == null -> item(key = "skeleton") {
                    Skeleton(kind = NzSkeletonKind.List, count = 4, modifier = side)
                }
                rows.len() == 0 -> item(key = "empty") {
                    EmptyState(
                        title = "Aucun pour l'instant",
                        body = "Touche « Je n'aime pas » dans le menu d'un titre pour le retirer du Flow et des radios.",
                        glyph = NzGlyph.ThumbsDown,
                    )
                }
                else -> item(key = "dislikes") {
                    DislikedSection(
                        rows = rows,
                        busy = state.busy != null,
                        onRemove = actions.removeDislike,
                        onMore = actions.more,
                        modifier = Modifier.nzPaneEnter(pane),
                    )
                }
            }
        }
        LibraryLeaf.History -> Unit
    }
}

/** The « Playlists » leaf: the sections, or the organiser. */
private fun LazyListScope.playlists(
    state: LibraryModel.State,
    actions: LibraryActions,
    columns: Int,
    onSectionMenu: (SectionView) -> Unit,
    pane: NzPaneEnter,
) {
    val failure = state.failure
    val tree = state.treeData
    val side = Modifier.nzPaneEnter(pane).padding(horizontal = NzSpacing.pageX)
    when {
        failure != null && tree == null -> item(key = "fail") {
            LoadFailed(
                cause = failure.cause,
                onRetry = actions.retry,
                title = "Tes playlists n'ont pas pu se charger",
                modifier = side,
            )
        }
        tree == null -> item(key = "skeleton") {
            Skeleton(kind = NzSkeletonKind.Shelf, modifier = side)
        }
        state.organizing -> organizer(state, actions, pane)
        tree.playlists.isEmpty() && tree.folders.isEmpty() -> item(key = "empty") {
            EmptyState(
                title = "Aucune playlist",
                body = "Une playlist pour un moment, une humeur, une route : elle se crée depuis la recherche.",
                glyph = NzGlyph.Library,
                modifier = Modifier.nzPaneEnter(pane),
            )
        }
        else -> {
            if (state.playlistQuery.isNotBlank() && state.sections.isEmpty()) {
                item(key = "none") {
                    Text(
                        text = "Aucune playlist ne correspond à « ${state.playlistQuery} ».",
                        style = Sillon.type.bodySmall,
                        color = Sillon.colors.dim,
                        modifier = side,
                    )
                }
            }
            state.sections.forEach { section ->
                item(key = "h:${section.key}") {
                    SectionHeading(
                        section = section,
                        onToggle = { actions.toggleSection(section.key) },
                        onMore = if (section.folderId != null && state.foldersSupported != false) {
                            { onSectionMenu(section) }
                        } else {
                            null
                        },
                        modifier = side,
                    )
                }
                if (section.folded) return@forEach
                if (section.rows.isEmpty()) {
                    item(key = "e:${section.key}") {
                        Text(
                            text = "Ce dossier est vide. Déplacez-y une playlist depuis son menu.",
                            style = Sillon.type.bodySmall,
                            color = Sillon.colors.dim,
                            modifier = Modifier
                                .nzPaneEnter(pane)
                                .fillMaxWidth()
                                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
                        )
                    }
                    return@forEach
                }
                val chunks = section.rows.chunked(columns)
                items(chunks.size, key = { index -> "r:${section.key}:$index" }) { index ->
                    Row(
                        modifier = Modifier
                            .nzPaneEnter(pane)
                            .fillMaxWidth()
                            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xxs),
                        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    ) {
                        chunks[index].forEach { card ->
                            PlaylistCardTile(
                                card = card,
                                onClick = { actions.openPlaylist(card.id) },
                                onMore = { actions.openMove(card.id) },
                                modifier = Modifier.weight(1f),
                            )
                        }
                        repeat(columns - chunks[index].size) { Spacer(Modifier.weight(1f)) }
                    }
                }
            }
        }
    }
}

/** The organiser: container chips and the ordered rows (`Library.svelte#org`). */
private fun LazyListScope.organizer(
    state: LibraryModel.State,
    actions: LibraryActions,
    pane: NzPaneEnter,
) {
    val tree = state.treeData ?: return
    item(key = "org:head") {
        Row(
            modifier = Modifier
                .nzPaneEnter(pane)
                .fillMaxWidth()
                .padding(horizontal = NzSpacing.pageX),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Organiser les playlists",
                style = Sillon.type.titleSmall,
                color = Sillon.colors.tx,
                modifier = Modifier.weight(1f),
            )
            NzButton(
                text = "Terminer",
                onClick = { actions.closeOrganizer() },
                glyph = NzGlyph.Check,
            )
        }
    }
    item(key = "org:chips") {
        Row(
            modifier = Modifier
                .nzPaneEnter(pane)
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs),
        ) {
            NzChip(
                text = "Dossiers",
                selected = state.container == LibraryModel.CONTAINER_FOLDERS,
                onClick = { actions.pickContainer(LibraryModel.CONTAINER_FOLDERS) },
            )
            NzChip(
                text = "Sans dossier",
                selected = state.container == LibraryModel.CONTAINER_ROOT,
                onClick = { actions.pickContainer(LibraryModel.CONTAINER_ROOT) },
            )
            tree.folders.forEach { folder ->
                NzChip(
                    text = folder.title,
                    selected = state.container == folder.id,
                    onClick = { actions.pickContainer(folder.id) },
                )
            }
        }
    }
    item(key = "org:status") {
        Text(
            text = state.organizeStatus,
            style = Sillon.type.bodySmall,
            color = Sillon.colors.dim,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = NzSpacing.pageX)
                .semantics { liveRegion = LiveRegionMode.Polite },
        )
    }
    val rows = state.organizeRows
    if (rows.isEmpty()) {
        item(key = "org:empty") {
            Text(
                text = if (state.container == LibraryModel.CONTAINER_FOLDERS) {
                    "Aucun dossier pour l'instant."
                } else {
                    "Aucune playlist dans ce conteneur."
                },
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
                modifier = Modifier
                    .nzPaneEnter(pane)
                    .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.s),
            )
        }
        return
    }
    items(rows.size, key = { index -> "org:${rows[index].id}" }) { index ->
        val row = rows[index]
        OrganizeRowView(
            row = row,
            first = index == 0,
            last = index == rows.lastIndex,
            busy = state.busy != null,
            onUp = { actions.organizeMove(index, -1) },
            onDown = { actions.organizeMove(index, 1) },
            onMove = if (state.container != LibraryModel.CONTAINER_FOLDERS) {
                { actions.openMove(row.id) }
            } else {
                null
            },
            modifier = Modifier.nzPaneEnter(pane).padding(horizontal = NzSpacing.pageX, vertical = 2.dp),
        )
    }
}

/** A grid leaf's cards, [columns] to a row — one lazily-composed item per row. */
private fun <T> LazyListScope.gridItems(
    key: String,
    rows: List<T>,
    columns: Int,
    pane: NzPaneEnter,
    tile: @Composable (T) -> Unit,
) {
    val chunks = rows.chunked(columns)
    items(chunks.size, key = { index -> "$key:$index" }) { index ->
        Row(
            modifier = Modifier
                .nzPaneEnter(pane)
                .fillMaxWidth()
                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.xxs),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
        ) {
            chunks[index].forEach { item ->
                Box(Modifier.weight(1f)) { tile(item) }
            }
            repeat(columns - chunks[index].size) { Spacer(Modifier.weight(1f)) }
        }
    }
}

/**
 * « Je n'aime pas » (`DislikedTracks.svelte`) as its own card.
 *
 * The SPA draws it in Réglages → Découverte; the phone's library keeps it as a
 * leaf, and MB16's Réglages can place this very composable without a second copy.
 * The list is short by nature, so it is drawn whole rather than windowed.
 */
@Composable
fun DislikedSection(
    rows: LibraryRows,
    busy: Boolean,
    onRemove: (TrackRow) -> Unit,
    onMore: (TrackRow) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    val window = windowFor(rows)
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = NzSpacing.pageX),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        for (index in 0 until window.len) {
            val row = window.rowAt(index) ?: continue
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 56.dp)
                    .padding(vertical = NzSpacing.xxs),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                fr.neezer.mobile.designsystem.CoverImage(
                    url = row.cover,
                    sizeDp = 44.dp,
                    seed = row.id,
                )
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        text = row.title,
                        style = Sillon.type.body,
                        color = colors.tx,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = row.artists,
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                NzButton(
                    text = "Retirer",
                    onClick = { onRemove(row) },
                    kind = NzButtonKind.Ghost,
                    enabled = !busy,
                )
                NzIconButton(
                    glyph = NzGlyph.More,
                    contentDescription = "Plus d'actions pour ${row.title}",
                    onClick = { onMore(row) },
                )
            }
        }
    }
}

/** The create/rename sheet (`FolderSheet.svelte`). */
@Composable
private fun FolderSheetView(sheet: FolderSheet, actions: LibraryActions) {
    LibraryOverlay(
        title = if (sheet.renameOf != null) "Renommer le dossier" else "Nouveau dossier",
        onDismiss = actions.closeFolderSheet,
    ) {
        Text(text = "Nom du dossier", style = Sillon.type.mono, color = Sillon.colors.dim)
        BasicTextField(
            value = sheet.title,
            onValueChange = actions.editFolderTitle,
            singleLine = true,
            textStyle = Sillon.type.body.copy(color = Sillon.colors.tx),
            cursorBrush = SolidColor(Sillon.colors.acText),
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .clip(RoundedCornerShape(NzRadius.card))
                .background(Sillon.colors.bg)
                .padding(horizontal = NzSpacing.m, vertical = NzSpacing.s)
                .semantics { contentDescription = "Nom du dossier" },
        )
        if (sheet.error.isNotEmpty()) {
            Text(text = sheet.error, style = Sillon.type.bodySmall, color = Sillon.colors.acText)
        }
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.s, Alignment.End),
        ) {
            NzButton(
                text = "Annuler",
                onClick = actions.closeFolderSheet,
                kind = NzButtonKind.Ghost,
                enabled = !sheet.busy,
            )
            NzButton(
                text = if (sheet.renameOf != null) "Renommer" else "Créer",
                onClick = actions.submitFolderSheet,
                enabled = !sheet.busy,
            )
        }
    }
}

/** « Déplacer vers » (`MoveToFolderSheet.svelte`). */
@Composable
private fun MoveSheetView(
    state: LibraryModel.State,
    target: MoveTarget,
    actions: LibraryActions,
) {
    val colors = Sillon.colors
    LibraryOverlay(title = "Déplacer vers", onDismiss = actions.closeMoveSheet) {
        Text(
            text = target.title,
            style = Sillon.type.bodySmall,
            color = colors.dim,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (state.moveFolders.size > 6) {
            NzSearchField(
                value = target.query,
                onValueChange = actions.filterMoveTargets,
                placeholder = "Filtrer les dossiers…",
                label = "Filtrer les dossiers",
            )
        }
        MoveChoice(
            glyph = NzGlyph.FolderMinus,
            title = "Sans dossier",
            detail = plural(state.rootCount, "playlist", "playlists"),
            current = target.currentFolder == null,
            enabled = !target.busy,
            onChoose = { actions.movePlaylist(null) },
        )
        state.moveFolders.forEach { folder ->
            MoveChoice(
                glyph = NzGlyph.Folder,
                title = folder.title,
                detail = plural(folder.count, "playlist", "playlists"),
                current = target.currentFolder == folder.id,
                enabled = !target.busy,
                onChoose = { actions.movePlaylist(folder.id) },
            )
        }
        if (state.moveFolders.isEmpty() && target.query.isNotBlank()) {
            Text(
                text = "Aucun dossier ne correspond à « ${target.query.trim()} ».",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
        state.currentFolderName?.let { name -> MonoLine("Dossier actuel · $name") }
        NzButton(
            text = "Fermer",
            onClick = actions.closeMoveSheet,
            kind = NzButtonKind.Ghost,
            enabled = !target.busy,
        )
    }
}

/** One destination of « Déplacer vers ». */
@Composable
private fun MoveChoice(
    glyph: NzGlyph,
    title: String,
    detail: String,
    current: Boolean,
    enabled: Boolean,
    onChoose: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .padding(vertical = NzSpacing.xxs)
            .semantics { contentDescription = "Déplacer vers $title, $detail" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Icon(glyph = glyph, size = 20.dp, tint = colors.dim)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = title,
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            MonoLine(detail)
        }
        if (current) {
            Icon(glyph = NzGlyph.Check, size = 18.dp, tint = colors.acText)
        } else {
            NzButton(
                text = "Choisir",
                onClick = onChoose,
                kind = NzButtonKind.Ghost,
                enabled = enabled,
            )
        }
    }
}

/** The folder's own actions (`Library.svelte#sectionMenu`). */
@Composable
private fun SectionSheet(
    section: SectionView,
    state: LibraryModel.State,
    onDismiss: () -> Unit,
    actions: LibraryActions,
) {
    val folderId = section.folderId ?: return
    val folders = state.treeData?.folders.orEmpty()
    val index = folders.indexOfFirst { it.id == folderId }
    LibraryOverlay(title = section.title, onDismiss = onDismiss) {
        NzButton(
            text = "Renommer le dossier",
            onClick = { onDismiss(); actions.openFolderSheet(folderId, section.title) },
            glyph = NzGlyph.Edit,
            kind = NzButtonKind.Ghost,
        )
        if (index > 0) {
            NzButton(
                text = "Déplacer le dossier vers le haut",
                onClick = { onDismiss(); actions.moveFolder(folderId, -1) },
                glyph = NzGlyph.ChevronUp,
                kind = NzButtonKind.Ghost,
            )
        }
        if (index in 0 until folders.size - 1) {
            NzButton(
                text = "Déplacer le dossier vers le bas",
                onClick = { onDismiss(); actions.moveFolder(folderId, 1) },
                glyph = NzGlyph.ChevronDown,
                kind = NzButtonKind.Ghost,
            )
        }
        NzButton(
            text = "Supprimer le dossier",
            onClick = { onDismiss(); actions.deleteFolder(folderId) },
            glyph = NzGlyph.Trash,
            kind = NzButtonKind.Ghost,
        )
        NzButton(text = "Fermer", onClick = onDismiss, kind = NzButtonKind.Ghost)
    }
}

/**
 * The one add menu (`Library.svelte#addMenu`, without the lots that own its
 * other items: upload is MB15's, the Deezer sync is MB16's).
 */
@Composable
private fun AddSheet(
    onDismiss: () -> Unit,
    onNewFolder: () -> Unit,
    onOrganize: () -> Unit,
    onNewPlaylist: () -> Unit,
    onImport: () -> Unit,
    canFolders: Boolean,
) {
    LibraryOverlay(title = "Ajouter à la bibliothèque", onDismiss = onDismiss) {
        NzButton(
            text = "Nouvelle playlist",
            onClick = onNewPlaylist,
            glyph = NzGlyph.Plus,
            kind = NzButtonKind.Ghost,
        )
        if (canFolders) {
            NzButton(
                text = "Nouveau dossier",
                onClick = onNewFolder,
                glyph = NzGlyph.FolderPlus,
                kind = NzButtonKind.Ghost,
            )
            NzButton(
                text = "Organiser les playlists",
                onClick = onOrganize,
                glyph = NzGlyph.Sliders,
                kind = NzButtonKind.Ghost,
            )
        } else {
            Text(
                text = "Les dossiers de playlists demandent une version plus récente du serveur.",
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
            )
        }
        NzButton(
            text = "Importer une playlist partagée",
            onClick = onImport,
            glyph = NzGlyph.Link,
            kind = NzButtonKind.Ghost,
        )
        NzButton(text = "Fermer", onClick = onDismiss, kind = NzButtonKind.Ghost)
    }
}

/** « Nom de la playlist ? » — le `window.prompt` du SPA (`Library.svelte:531`), en feuille. */
@Composable
private fun NewPlaylistSheet(
    onDismiss: () -> Unit,
    onCreate: (String) -> Unit,
) {
    var name by remember { mutableStateOf("") }
    LibraryOverlay(title = "Nouvelle playlist", onDismiss = onDismiss) {
        NzSearchField(
            value = name,
            onValueChange = { name = it },
            placeholder = "Nom de la playlist",
            label = "Nom de la playlist",
        )
        NzButton(
            text = "Créer",
            onClick = { onCreate(name) },
            enabled = name.isNotBlank(),
            glyph = NzGlyph.Check,
        )
        NzButton(text = "Annuler", onClick = onDismiss, kind = NzButtonKind.Ghost)
    }
}

/** A sheet's shell: `NzSheet` over the page, with its title and content. */
@Composable
internal fun LibraryOverlay(
    title: String,
    onDismiss: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    NzSheet(visible = true, onDismiss = onDismiss) {
        Text(text = title, style = Sillon.type.titleSmall, color = Sillon.colors.tx)
        content()
    }
}
