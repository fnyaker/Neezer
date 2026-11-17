package fr.neezer.mobile.app

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigation.NavDestination
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.feature.discovery.RecapEntry
import fr.neezer.mobile.feature.discovery.ReleaseShelfEntry
import fr.neezer.mobile.feature.discovery.ReleasesEntry
import fr.neezer.mobile.feature.entity.AlbumEntry
import fr.neezer.mobile.feature.entity.ArtistEntry
import fr.neezer.mobile.feature.entity.CreditsEntry
import fr.neezer.mobile.feature.genres.GenresEntry
import fr.neezer.mobile.feature.home.HomeEntry
import fr.neezer.mobile.feature.library.HistoryEntry
import fr.neezer.mobile.feature.library.LibraryEntry
import fr.neezer.mobile.feature.nowplaying.MiniPlayer
import fr.neezer.mobile.feature.nowplaying.NowPlayingHost
import fr.neezer.mobile.feature.onboarding.InviteEntry
import fr.neezer.mobile.feature.onboarding.OnboardingEntry
import fr.neezer.mobile.feature.party.PartyEntry
import fr.neezer.mobile.feature.playlist.ImportEntry
import fr.neezer.mobile.feature.playlist.PlaylistEntry
import fr.neezer.mobile.feature.playlist.SharedPlaylistEntry
import fr.neezer.mobile.feature.podcasts.PodcastsEntry
import fr.neezer.mobile.feature.podcasts.ShowEntry
import fr.neezer.mobile.feature.remote.RemoteClaimEntry
import fr.neezer.mobile.feature.search.SearchEntry
import fr.neezer.mobile.feature.settings.SettingsEntry
import fr.neezer.mobile.feature.setup.LoginEntry
import fr.neezer.mobile.feature.setup.SetupEntry
import fr.neezer.mobile.feature.trackactions.TrackActionsHost
import fr.neezer.mobile.feature.viz.VizEntry
import kotlinx.serialization.Serializable

/**
 * Every route of the inventory (docs/agents/mobile.md, MB0 "Contract"), written
 * ONCE here: a feature lot fills its entry composable and never edits this file.
 *
 * Type-safe routes (Navigation Compose): the arguments are `@Serializable`
 * properties, so a missing one is a compile error instead of a blank screen.
 */
object Routes {
    @Serializable
    data object Setup

    @Serializable
    data object Login

    @Serializable
    data class Invite(val code: String)

    @Serializable
    data object Onboarding

    @Serializable
    data object Home

    @Serializable
    data class Search(val q: String? = null)

    @Serializable
    data class Artist(val id: String)

    @Serializable
    data class Album(val id: String)

    @Serializable
    data class Credits(val name: String)

    @Serializable
    data class Playlist(val id: String)

    @Serializable
    data class SharedPlaylist(val id: String)

    @Serializable
    data class Import(val url: String? = null)

    @Serializable
    data class Library(val tab: String? = null)

    @Serializable
    data object History

    @Serializable
    data object Podcasts

    @Serializable
    data class Show(val id: String)

    @Serializable
    data object Releases

    @Serializable
    data class ReleaseShelf(val kind: String)

    @Serializable
    data class Recap(val year: Int? = null)

    @Serializable
    data object Genres

    @Serializable
    data object Viz

    @Serializable
    data class Settings(val section: String? = null)

    @Serializable
    data class Party(val id: String)

    @Serializable
    data class RemoteClaim(val token: String)
}

/** The four destinations of the phone's bottom bar and the tablet's rail. */
private enum class Tab(val label: String) {
    Accueil("Accueil"),
    Rechercher("Rechercher"),
    Bibliotheque("Bibliothèque"),
    Podcasts("Podcasts"),
}

/**
 * The app's shell: one `NavHost`, a phone shell (bottom bar, mini player above
 * it, the full-screen player as a sheet) and a wide shell (rail + reading
 * columns) — the breakpoint is the SPA's own 640 dp.
 */
@Composable
fun NzApp(graph: AppGraph) {
    val nav = rememberNavController()
    graph.attach(nav)

    val backStackEntry by nav.currentBackStackEntryAsState()
    val destination = backStackEntry?.destination

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val wide = maxWidth >= 640.dp
        if (wide) {
            Row(Modifier.fillMaxSize()) {
                WideShell(nav, destination)
                Box(Modifier.fillMaxSize()) {
                    NavGraph(graph, nav)
                    Hosts(graph)
                }
            }
        } else {
            Scaffold(
                // The shell's one permanent door to Réglages: on a phone the
                // SPA reaches it from the library's header, and the skeleton has
                // no header yet. It is what makes Aide (and its `nz_version()`)
                // reachable from the first screen.
                topBar = {
                    NzTopBar(destination = destination, onSettings = { nav.navigate(Routes.Settings()) })
                },
                bottomBar = {
                    Column {
                        MiniPlayer()
                        NavigationBar {
                            for (tab in Tab.entries) {
                                NavigationBarItem(
                                    selected = destination.isOn(tab),
                                    onClick = { nav.open(tab) },
                                    icon = {},
                                    label = { Text(tab.label) },
                                )
                            }
                        }
                    }
                },
                containerColor = MaterialTheme.colorScheme.background,
            ) { padding ->
                Box(Modifier.fillMaxSize().padding(padding)) {
                    NavGraph(graph, nav)
                    Hosts(graph)
                }
            }
        }
    }
}

/** The sheet hosts, placed once for the whole app. */
@Composable
private fun Hosts(graph: AppGraph) {
    NowPlayingHost(nav = graph)
    TrackActionsHost()
}

@Composable
private fun WideShell(nav: NavHostController, destination: NavDestination?) {
    NavigationRail {
        for (tab in Tab.entries) {
            NavigationRailItem(
                selected = destination.isOn(tab),
                onClick = { nav.open(tab) },
                icon = {},
                label = { Text(tab.label) },
            )
        }
        NavigationRailItem(
            selected = false,
            onClick = { nav.navigate(Routes.Settings()) },
            icon = {},
            label = { Text("Réglages") },
        )
    }
}

/**
 * The shell's title bar: the screen the person is on, and the Réglages action.
 *
 * The title comes from the destination — one `when` on the route type, so a new
 * route without a title is a compile error rather than an empty bar.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NzTopBar(destination: NavDestination?, onSettings: () -> Unit) {
    TopAppBar(
        title = { Text(destination.title()) },
        actions = { TextButton(onClick = onSettings) { Text("Réglages") } },
    )
}

/** The French title of the shown destination. */
private fun NavDestination?.title(): String {
    val route = this?.route ?: return "Neezer"
    val name = route.substringBefore('?').substringBefore('/')
    return when {
        name == Routes.Home::class.qualifiedName -> "Accueil"
        name == Routes.Search::class.qualifiedName -> "Rechercher"
        name == Routes.Library::class.qualifiedName -> "Bibliothèque"
        name == Routes.Podcasts::class.qualifiedName -> "Podcasts"
        name == Routes.Settings::class.qualifiedName -> "Réglages"
        name == Routes.Artist::class.qualifiedName -> "Artiste"
        name == Routes.Album::class.qualifiedName -> "Album"
        name == Routes.Playlist::class.qualifiedName -> "Playlist"
        name == Routes.Show::class.qualifiedName -> "Podcast"
        name == Routes.Releases::class.qualifiedName -> "Nouveautés"
        name == Routes.Recap::class.qualifiedName -> "Récap"
        name == Routes.Genres::class.qualifiedName -> "Genres"
        name == Routes.Viz::class.qualifiedName -> "Visualiseur"
        name == Routes.History::class.qualifiedName -> "Historique"
        name == Routes.Setup::class.qualifiedName -> "Serveur"
        name == Routes.Login::class.qualifiedName -> "Connexion"
        else -> "Neezer"
    }
}

/**
 * The graph itself: every route opens its entry composable, and MB0's entry
 * draws the screen's name (so "toutes les routes ouvrent leur stub" is
 * something a person can see).
 */
@Composable
private fun NavGraph(graph: AppGraph, nav: NavHostController) {
    val navigator: NzNavigator = graph
    NavHost(navController = nav, startDestination = Routes.Home) {
        composable<Routes.Setup> { SetupEntry(nav = navigator) }
        composable<Routes.Login> { LoginEntry(nav = navigator) }
        composable<Routes.Invite> { entry -> InviteEntry(entry.toRoute<Routes.Invite>().code, navigator) }
        composable<Routes.Onboarding> { OnboardingEntry(nav = navigator) }
        composable<Routes.Home> { HomeEntry(nav = navigator) }
        composable<Routes.Search> { entry -> SearchEntry(entry.toRoute<Routes.Search>().q, navigator) }
        composable<Routes.Artist> { entry -> ArtistEntry(entry.toRoute<Routes.Artist>().id, navigator) }
        composable<Routes.Album> { entry -> AlbumEntry(entry.toRoute<Routes.Album>().id, navigator) }
        composable<Routes.Credits> { entry -> CreditsEntry(entry.toRoute<Routes.Credits>().name, navigator) }
        composable<Routes.Playlist> { entry -> PlaylistEntry(entry.toRoute<Routes.Playlist>().id, navigator) }
        composable<Routes.SharedPlaylist> {
            entry -> SharedPlaylistEntry(entry.toRoute<Routes.SharedPlaylist>().id, navigator)
        }
        composable<Routes.Import> { entry -> ImportEntry(entry.toRoute<Routes.Import>().url, navigator) }
        composable<Routes.Library> { entry -> LibraryEntry(entry.toRoute<Routes.Library>().tab, navigator) }
        composable<Routes.History> { HistoryEntry(nav = navigator) }
        composable<Routes.Podcasts> { PodcastsEntry(nav = navigator) }
        composable<Routes.Show> { entry -> ShowEntry(entry.toRoute<Routes.Show>().id, navigator) }
        composable<Routes.Releases> { ReleasesEntry(nav = navigator) }
        composable<Routes.ReleaseShelf> {
            entry -> ReleaseShelfEntry(entry.toRoute<Routes.ReleaseShelf>().kind, navigator)
        }
        composable<Routes.Recap> { entry -> RecapEntry(entry.toRoute<Routes.Recap>().year, navigator) }
        composable<Routes.Genres> { GenresEntry(nav = navigator) }
        composable<Routes.Viz> { VizEntry(nav = navigator) }
        composable<Routes.Settings> { entry -> SettingsEntry(entry.toRoute<Routes.Settings>().section, navigator) }
        composable<Routes.Party> { entry -> PartyEntry(entry.toRoute<Routes.Party>().id, navigator) }
        composable<Routes.RemoteClaim> {
            entry -> RemoteClaimEntry(entry.toRoute<Routes.RemoteClaim>().token, navigator)
        }
    }
}

/** Whether the shown destination IS [tab] (so the bar highlights it). */
private fun NavDestination?.isOn(tab: Tab): Boolean {
    val name = this?.route ?: return false
    val route = when (tab) {
        Tab.Accueil -> Routes.Home::class.qualifiedName
        Tab.Rechercher -> Routes.Search::class.qualifiedName
        Tab.Bibliotheque -> Routes.Library::class.qualifiedName
        Tab.Podcasts -> Routes.Podcasts::class.qualifiedName
    }
    return name.startsWith(route ?: return false)
}

/**
 * A tab switch: one entry per tab on the back stack, its scroll position kept —
 * the SPA's rule, so tapping Bibliothèque twice does not stack two libraries.
 */
private fun NavHostController.open(tab: Tab) {
    val route = when (tab) {
        Tab.Accueil -> Routes.Home
        Tab.Rechercher -> Routes.Search()
        Tab.Bibliotheque -> Routes.Library()
        Tab.Podcasts -> Routes.Podcasts
    }
    navigate(route) {
        popUpTo(graph.findStartDestination().id) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }
}
