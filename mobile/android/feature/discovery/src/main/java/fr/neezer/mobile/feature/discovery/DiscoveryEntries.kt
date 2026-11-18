package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.feature.trackactions.TrackActions
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.Screens

/**
 * Nouveautés — lot MB14 (docs/agents/mobile.md, feature inventory).
 *
 * `routes/Releases.svelte`: the two shelf cards, "Toutes les sorties" and the
 * unseen pill. Every door is here, so the screen never reaches for a
 * `NavController`: a shelf card is `AppLink.ReleaseShelf(kind)` (the variant this
 * lot added), a row's ⋯ is MB10's ONE menu.
 */
@Composable
fun ReleasesEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzDiscoveryService) }
    val player = remember { DiscoveryPlayer.port() }
    val scope = rememberCoroutineScope()
    val model = remember(service, player, scope) { service?.let { ReleasesModel(it, player, scope) } }

    if (model == null) {
        CoreMissing(modifier)
        return
    }

    LaunchedEffect(model) { model.load() }
    val state by model.state.collectAsState()

    ReleasesScreen(
        state = state,
        actions = ReleasesActions(
            playAll = model::playAll,
            playGroup = model::playGroup,
            toggleGroup = model::toggleGroup,
            toggleAll = model::toggleAll,
            openShelf = { kind ->
                model.seenOnShelf()
                nav.go(AppLink.ReleaseShelf(shelfKindOrFirst(kind)))
            },
            playGroupAt = model::playGroupAt,
            retry = model::retry,
        ),
        modifier = modifier,
    )
}

/**
 * Sorties — lot MB14. `routes/ReleaseShelf.svelte` behind `#/releases/artists`
 * and `#/releases/discovery`: a read-only page with a banner, a search and a
 * sort, whose data the feed already holds (0 requests after a card click).
 */
@Composable
fun ReleaseShelfEntry(kind: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzDiscoveryService) }
    val player = remember { DiscoveryPlayer.port() }
    val scope = rememberCoroutineScope()
    val shelf = shelfKindOrFirst(kind)
    val model = remember(service, player, scope, shelf) {
        service?.let { ReleaseShelfModel(shelf, it, player, scope) }
    }

    if (model == null) {
        CoreMissing(modifier)
        return
    }

    LaunchedEffect(model) { model.load() }
    val state by model.state.collectAsState()

    ReleaseShelfScreen(
        state = state,
        actions = ShelfActions(
            back = nav::back,
            playAll = model::playAll,
            shuffle = model::shuffle,
            search = model::search,
            sort = model::sort,
            reverse = model::reverse,
            playRow = model::playRow,
            queueNext = model::queueNext,
            more = { row -> TrackActions.open(row, nav) },
            retry = model::retry,
            openReleases = { nav.go(AppLink.Releases) },
        ),
        modifier = modifier,
    )
}

/**
 * Récap — lot MB14. `routes/Recap.svelte` behind `#/recap` and `#/recap/<year>`:
 * the year chips, the story, the glance panel and the poster share.
 */
@Composable
fun RecapEntry(year: Int?, nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzDiscoveryService) }
    val player = remember { DiscoveryPlayer.port() }
    val scope = rememberCoroutineScope()
    val model = remember(service, player, scope) { service?.let { RecapModel(it, player, scope) } }

    if (model == null) {
        CoreMissing(modifier)
        return
    }

    LaunchedEffect(model, year) { model.open(year) }
    val state by model.state.collectAsState()

    RecapScreen(
        state = state,
        actions = RecapActions(
            retry = model::retry,
            chooseYear = model::chooseYear,
            tap = model::tap,
            hold = model::hold,
            restart = model::restart,
            share = model::share,
            playTop = model::playTop,
            nearby = model::openNearby,
            openHistory = { nav.back() },
        ),
        modifier = modifier,
    )
}

/** The shell's one sentence when the native core could not be built. */
@Composable
private fun CoreMissing(modifier: Modifier) {
    Column(modifier.fillMaxSize().padding(NzSpacing.l)) {
        EmptyState(
            title = "Le noyau de l'application n'est pas disponible",
            body = NzBridge.failure ?: "Redémarre l'application.",
        )
    }
}
