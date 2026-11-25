package fr.neezer.mobile.app

import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.ScaffoldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.filter
import androidx.navigation.NavDestination
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzToastHost
import fr.neezer.mobile.designsystem.NzToasts
import fr.neezer.mobile.feature.discovery.RecapEntry
import fr.neezer.mobile.feature.discovery.ReleaseShelfEntry
import fr.neezer.mobile.feature.discovery.ReleasesEntry
import fr.neezer.mobile.feature.entity.AlbumEntry
import fr.neezer.mobile.feature.entity.ArtistEntry
import fr.neezer.mobile.feature.entity.CreditsEntry
import fr.neezer.mobile.feature.genres.GenresEntry
import fr.neezer.mobile.feature.lan.LanEntry
import fr.neezer.mobile.feature.lan.PeerSetsEntry
import fr.neezer.mobile.feature.library.HistoryEntry
import fr.neezer.mobile.feature.nowplaying.MiniPlayer
import fr.neezer.mobile.feature.nowplaying.NowPlayingHost
import fr.neezer.mobile.feature.onboarding.InviteEntry
import fr.neezer.mobile.feature.onboarding.OnboardingEntry
import fr.neezer.mobile.feature.party.PartyEntry
import fr.neezer.mobile.feature.playlist.ImportEntry
import fr.neezer.mobile.feature.playlist.PlaylistEntry
import fr.neezer.mobile.feature.playlist.SharedPlaylistEntry
import fr.neezer.mobile.feature.podcasts.ShowEntry
import fr.neezer.mobile.feature.remote.RemoteClaimEntry
import fr.neezer.mobile.feature.remote.RemoteHostNotice
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

    /** Appareils à proximité — the LAN exchange's own screen (LN14 fills it). */
    @Serializable
    data object Lan

    /** Sur vos appareils — the sets the connected peers expose (LN14 fills it). */
    @Serializable
    data object PeerSets

    @Serializable
    data class Party(val id: String)

    @Serializable
    data class RemoteClaim(val token: String)
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

    // La destination de départ est figée à la première composition : elle est
    // décidée avant, à partir de l'appareil seul (aucune requête).
    val startRoute = remember(graph) { graph.startRoute() }

    val backStackEntry by nav.currentBackStackEntryAsState()
    val destination = backStackEntry?.destination

    // L1 : les quatre racines d'onglet vivent dans UN pager, hissé ici pour que
    // la barre et le routeur lisent le même état — une image, deux lecteurs.
    // `TabPagerSync` est la seule couture entre « la page où le doigt s'est
    // arrêté » et « la route où la personne est ».
    val pager = rememberPagerState(initialPage = 0) { Tab.entries.size }
    val scope = rememberCoroutineScope()
    TabPagerSync(destination, pager) { tab -> nav.open(tab) }

    // A link that STARTED the app (a shared `neezer://` view, a post's link) is
    // kept by the graph until the `NavHost` has a graph — `attach` cannot open it,
    // the graph does not exist yet and `getGraph()` throws (the app died on a cold
    // start). This effect runs AFTER that composition and on every route change,
    // so the intent that opened the app and one that arrives during the first
    // frames (`onNewIntent`) take the same door.
    LaunchedEffect(nav) {
        snapshotFlow { nav.currentBackStackEntry != null }
            .filter { it }
            .collect { graph.openPendingLink() }
    }

    // La session, une fois pour tout le process : le noyau émet sur CHANGEMENT,
    // jamais sur un minuteur. Un 401 devient un toast et la connexion ; une
    // personne qui change (déconnexion) prend la même porte. Avant M2 ces deux
    // événements n'avaient aucun collecteur : un jeton mort laissait l'app
    // afficher des écrans qui échouaient en silence (audit, bloqueur 4).
    LaunchedEffect(graph) {
        NzBridge.sessions.collect { event ->
            when (event) {
                is NzBridge.Session.Ended -> {
                    NzToasts.show("Session expirée, reconnectez-vous")
                    graph.openLogin()
                }
                // M3 : une personne qui change (restauration au lancement, compte
                // invité, changement de personne) n'atterrit pas sur la coquille
                // vide quand le serveur dit que sa préparation reste à faire.
                // `val me =` : `event.me` est un `val` d'un autre module, un smart
                // cast y est refusé par Kotlin.
                is NzBridge.Session.MeChanged -> when (val me = event.me) {
                    null -> graph.openLogin()
                    else -> graph.openOnboardingIfNeeded(me)
                }
            }
        }
    }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val wide = maxWidth >= 640.dp
        if (wide) {
            Row(Modifier.fillMaxSize()) {
                WideShell(nav, destination)
                Column(Modifier.fillMaxSize()) {
                    // M6 : « quelqu'un pilote ce lecteur » se voit depuis N'IMPORTE
                    // quel écran, en haut de la colonne de contenu sur large.
                    RemoteHostNotice(Modifier.fillMaxWidth())
                    Box(Modifier.fillMaxSize()) {
                        NavGraph(graph, nav, pager, startRoute)
                        Hosts(graph)
                    }
                }
            }
        } else {
            Scaffold(
                // L1 : plus de bandeau. The default insets hand the status bar to
                // the content, so no screen needs `statusBarsPadding()` of its own
                // and nothing is drawn over the page — the bar the top used to hold
                // is gone, its two words live on: « Retour » is the detail row
                // below, « Réglages » is the bar's fifth item.
                contentWindowInsets = ScaffoldDefaults.contentWindowInsets,
                bottomBar = {
                    Column {
                        // M6 : la barre « Contrôlé par X · Couper » se glisse
                        // au-dessus du mini-lecteur, donc visible sur tous les
                        // écrans de la coquille téléphone (audit, bloqueur 6).
                        RemoteHostNotice()
                        // The bar IS the door to the full-screen player on a
                        // phone (the SPA's `PlayerBar#cardTap`): `MiniPlayer`'s
                        // default `onClick` is a no-op, so mounting it bare left
                        // the player unreachable from the bar (measured by MQ2,
                        // 2026-11-24). `AppGraph.openPlayer()` is the shell's
                        // one navigator, shared with a deep link.
                        MiniPlayer(onClick = { graph.openPlayer() })
                        TabBarHost(
                            destination = destination,
                            pager = pager,
                            scope = scope,
                            onOpenTab = { tab -> nav.open(tab) },
                            onOpenSettings = { nav.openSettings() },
                        )
                    }
                },
                containerColor = MaterialTheme.colorScheme.background,
            ) { padding ->
                Column(Modifier.fillMaxSize().padding(padding)) {
                    // La rangée du retour n'existe que si l'écran n'est pas une
                    // racine d'onglet (ni Réglages) ET qu'il y a quelque part où
                    // revenir : sur l'accueil, la flèche n'aurait rien à faire.
                    if (selectedIndexOf(destination) < 0 && nav.previousBackStackEntry != null) {
                        DetailBackRow { graph.back() }
                    }
                    Box(Modifier.weight(1f).fillMaxWidth()) {
                        NavGraph(graph, nav, pager, startRoute)
                        Hosts(graph)
                    }
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
    // M2 : la SEULE surface où un toast est dessiné. Il y avait quarante appels
    // à `NzToasts.show` sur master (réglages, file, album, party…) et aucune
    // coquille ne montait `NzToastHost` : chaque phrase partait dans une liste
    // que personne ne dessinait. Un toast est la moitié d'une règle 3 (un échec
    // dit pourquoi) : la session expirée est la première à en dépendre vraiment.
    NzToastHost(modifier = Modifier.fillMaxSize(), bottomPadding = NzSpacing.m)
}

/**
 * The wide shell's rail: the SAME five items and glyphs as the phone's bar (L1),
 * so the two shells no longer disagree about what the app has. It navigates
 * routes directly — on a wide window the reading column is not a pager.
 */
@Composable
private fun WideShell(nav: NavHostController, destination: NavDestination?) {
    val selected = selectedIndexOf(destination)
    NavigationRail {
        TAB_ITEMS.forEachIndexed { index, item ->
            NavigationRailItem(
                selected = selected == index,
                onClick = {
                    val tab = tabOfPage(index)
                    if (tab != null) nav.open(tab) else nav.openSettings()
                },
                icon = { Icon(item.glyph, size = 24.dp, contentDescription = null) },
                label = { Text(item.label) },
            )
        }
    }
}

/**
 * The graph itself: every route opens its entry composable, and the four tab
 * roots open the shell's pager (so a horizontal drag moves them).
 */
@Composable
private fun NavGraph(
    graph: AppGraph,
    nav: NavHostController,
    pager: PagerState,
    startDestination: Any,
) {
    val navigator: NzNavigator = graph
    NavHost(
        navController = nav,
        startDestination = startDestination,
        // Chaque écran possède son mouvement (`transitions.md`) : un onglet qui
        // se pose ne doit pas être ré-animé par le routeur — c'est exactement ce
        // double mouvement que la personne sent comme « ça se réinitialise après
        // le geste ».
        enterTransition = { EnterTransition.None },
        exitTransition = { ExitTransition.None },
        popEnterTransition = { EnterTransition.None },
        popExitTransition = { ExitTransition.None },
    ) {
        composable<Routes.Setup> { SetupEntry(nav = navigator) }
        composable<Routes.Login> { LoginEntry(nav = navigator) }
        composable<Routes.Invite> { entry -> InviteEntry(entry.toRoute<Routes.Invite>().code, navigator) }
        composable<Routes.Onboarding> { OnboardingEntry(nav = navigator) }
        composable<Routes.Home> { TabPager(navigator, pager) }
        composable<Routes.Search> { entry ->
            TabPager(navigator, pager, searchQuery = entry.toRoute<Routes.Search>().q)
        }
        composable<Routes.Artist> { entry -> ArtistEntry(entry.toRoute<Routes.Artist>().id, navigator) }
        composable<Routes.Album> { entry -> AlbumEntry(entry.toRoute<Routes.Album>().id, navigator) }
        composable<Routes.Credits> { entry -> CreditsEntry(entry.toRoute<Routes.Credits>().name, navigator) }
        composable<Routes.Playlist> { entry -> PlaylistEntry(entry.toRoute<Routes.Playlist>().id, navigator) }
        composable<Routes.SharedPlaylist> {
            entry -> SharedPlaylistEntry(entry.toRoute<Routes.SharedPlaylist>().id, navigator)
        }
        composable<Routes.Import> { entry -> ImportEntry(entry.toRoute<Routes.Import>().url, navigator) }
        composable<Routes.Library> { entry ->
            TabPager(navigator, pager, libraryTab = entry.toRoute<Routes.Library>().tab)
        }
        composable<Routes.History> { HistoryEntry(nav = navigator) }
        composable<Routes.Podcasts> { TabPager(navigator, pager) }
        composable<Routes.Show> { entry -> ShowEntry(entry.toRoute<Routes.Show>().id, navigator) }
        composable<Routes.Releases> { ReleasesEntry(nav = navigator) }
        composable<Routes.ReleaseShelf> {
            entry -> ReleaseShelfEntry(entry.toRoute<Routes.ReleaseShelf>().kind, navigator)
        }
        composable<Routes.Recap> { entry -> RecapEntry(entry.toRoute<Routes.Recap>().year, navigator) }
        composable<Routes.Genres> { GenresEntry(nav = navigator) }
        composable<Routes.Viz> { VizEntry(nav = navigator) }
        composable<Routes.Settings> { entry -> SettingsEntry(entry.toRoute<Routes.Settings>().section, navigator) }
        composable<Routes.Lan> { LanEntry(nav = navigator) }
        composable<Routes.PeerSets> { PeerSetsEntry(nav = navigator) }
        composable<Routes.Party> { entry -> PartyEntry(entry.toRoute<Routes.Party>().id, navigator) }
        composable<Routes.RemoteClaim> {
            entry -> RemoteClaimEntry(entry.toRoute<Routes.RemoteClaim>().token, navigator)
        }
    }
}

/**
 * A tab switch: one entry per tab on the back stack, its scroll position kept —
 * the SPA's rule, so tapping Bibliothèque twice does not stack two libraries.
 * The pager's own page follows, in `TabPagerSync`.
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

/** The bar's fifth item and the rail's: Réglages, one entry, never two. */
private fun NavHostController.openSettings() {
    navigate(Routes.Settings()) { launchSingleTop = true }
}
