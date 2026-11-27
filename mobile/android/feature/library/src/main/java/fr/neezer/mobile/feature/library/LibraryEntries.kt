package fr.neezer.mobile.feature.library

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.LoadWhenShown
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.feature.trackactions.TrackActions
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.Screens

/**
 * Bibliothèque — lot MB8 (docs/agents/mobile.md, feature inventory).
 *
 * The screen itself is [LibraryScreen], a pure function of the model's state;
 * this entry only wires it to the app: the core's screens, the player behind
 * [LibraryPlayer], the shell's routes, and the history leaf's own model (the SPA
 * mounts `History` inside the library, and the same body is the `Historique`
 * route's).
 *
 * Every door a person can take is here, so nothing below reaches for a
 * `NavController` of its own:
 *
 * - an artist or an album card opens its route through [NzNavigator]'s
 *   `AppLink`s (the one way a tap and a deep link land on the same screen);
 * - a playlist card opens MB9's playlist route;
 * - a row's ⋯ opens **MB10's** one context menu (`TrackActions.open`);
 * - the head's magnifier opens the search;
 * - the head's link icon opens « Importer une playlist partagée »
 *   (`Library.svelte:611`) and the « + » menu's « Nouvelle playlist » creates one
 *   on the server, then opens it (`Library.svelte:531`);
 * - « Je n'aime pas », « Indisponibles » and « Mes fichiers » are leaves of this
 *   same screen, as in the SPA.
 */
@Composable
fun LibraryEntry(tab: String?, nav: NzNavigator, visible: Boolean = true, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzLibraryService) }
    val historyService = remember(screens) { screens?.let(::NzHistoryService) }
    val player: LibraryPlayer = remember { AppLibraryPlayer.api()?.let(::NzLibraryPlayer) ?: NoLibraryPlayer }
    val scope = rememberCoroutineScope()
    val model = remember(service, player, scope) { service?.let { LibraryModel(it, player, scope) } }
    val historyModel = remember(historyService, player, scope) {
        historyService?.let { HistoryModel(it, player, scope) }
    }

    if (model == null || historyModel == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }

    val leaf = remember(tab) { LibraryLeaf.of(tab) }
    LoadWhenShown(visible, leaf) { model.load(leaf) }

    val state by model.state.collectAsState()
    val historyState by historyModel.state.collectAsState()

    // The history leaf reads its own page the first time it opens (and only then).
    LaunchedEffect(state.leaf) {
        if (state.leaf == LibraryLeaf.History) historyModel.load()
    }

    // Une playlist créée s'ouvre (le SPA : `push("/playlist/" + r.id)`).
    LaunchedEffect(state.openPlaylist) {
        val id = state.openPlaylist ?: return@LaunchedEffect
        model.consumeOpenPlaylist()
        nav.go(AppLink.Playlist(id))
    }

    LibraryScreen(
        state = state,
        actions = LibraryActions(
            selectLeaf = { next ->
                model.selectLeaf(next)
                if (next == LibraryLeaf.History) historyModel.load()
            },
            retry = model::retry,
            filterFavorites = model::filterFavorites,
            sortFavorites = model::sortFavorites,
            reverseFavorites = model::reverseFavorites,
            playLeaf = { next -> model.playAll(next) },
            shuffleFavorites = model::shuffleFavorites,
            playFrom = { row -> model.playFrom(row) },
            queueNext = model::queueNext,
            more = { row -> TrackActions.open(row, nav) },
            filterArtists = model::filterArtists,
            sortArtists = model::sortArtists,
            openArtist = { id -> nav.go(AppLink.Artist(id)) },
            openAlbum = { id -> nav.go(AppLink.Album(id)) },
            filterPlaylists = model::filterPlaylists,
            toggleSection = model::toggleSection,
            openFolderSheet = { id, title -> model.openFolderSheet(id, title) },
            editFolderTitle = model::editFolderTitle,
            closeFolderSheet = model::closeFolderSheet,
            submitFolderSheet = model::submitFolderSheet,
            deleteFolder = model::deleteFolder,
            moveFolder = model::moveFolder,
            organize = model::openOrganizer,
            closeOrganizer = model::closeOrganizer,
            pickContainer = model::pickContainer,
            organizeMove = model::organizeMove,
            openMove = model::openMoveSheet,
            filterMoveTargets = model::filterMoveTargets,
            movePlaylist = model::movePlaylist,
            closeMoveSheet = model::closeMoveSheet,
            openPlaylist = { id -> nav.go(AppLink.Playlist(id)) },
            openSearch = { nav.go(AppLink.Search(q = null)) },
            openImport = { nav.go(AppLink.Import(url = "")) },
            newPlaylist = model::newPlaylist,
            removeDislike = model::removeDislike,
            historyBody = {
                HistoryScreen(
                    state = historyState,
                    actions = HistoryActions(
                        setFilter = historyModel::setFilter,
                        loadMore = historyModel::loadMore,
                        retry = historyModel::retry,
                        play = historyModel::play,
                        playTrack = historyModel::playTrack,
                        more = { row -> TrackActions.open(row, nav) },
                        openArtist = { id -> nav.go(AppLink.Artist(id)) },
                        openRecap = { nav.go(AppLink.Recap(year = null)) },
                        openSearch = { nav.go(AppLink.Search(q = null)) },
                    ),
                )
            },
        ),
        modifier = modifier,
    )
}

/**
 * Historique — lot MB8, as its own route (`Routes.History`).
 *
 * The same body the library's history leaf draws: one screen, one model, so a
 * play recorded in one place shows in the other without a second implementation.
 */
@Composable
fun HistoryEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzHistoryService) }
    val player: LibraryPlayer = remember { AppLibraryPlayer.api()?.let(::NzLibraryPlayer) ?: NoLibraryPlayer }
    val scope = rememberCoroutineScope()
    val model = remember(service, player, scope) { service?.let { HistoryModel(it, player, scope) } }

    if (model == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }

    LaunchedEffect(model) { model.load() }
    val state by model.state.collectAsState()
    HistoryScreen(
        state = state,
        actions = HistoryActions(
            setFilter = model::setFilter,
            loadMore = model::loadMore,
            retry = model::retry,
            play = model::play,
            playTrack = model::playTrack,
            more = { row -> TrackActions.open(row, nav) },
            openArtist = { id -> nav.go(AppLink.Artist(id)) },
            openRecap = { nav.go(AppLink.Recap(year = null)) },
            openSearch = { nav.go(AppLink.Search(q = null)) },
        ),
        modifier = modifier,
    )
}
