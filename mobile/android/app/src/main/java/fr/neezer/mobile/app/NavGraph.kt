package fr.neezer.mobile.app

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.filter
import androidx.navigation.NavDestination
import androidx.navigation.NavDestination.Companion.hasRoute
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
import fr.neezer.mobile.designsystem.Sillon
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
import fr.neezer.mobile.feature.nowplaying.NowPlayingState
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

    // SB2 : une route d'onglet ouverte par la barre arrive SANS argument (la
    // porte ne connaît que la route) ; les arguments déjà lus sont donc retenus,
    // sinon le terme cherché et le sous-onglet de la bibliothèque seraient
    // effacés, `LoadWhenShown` verrait une nouvelle clé et rechargerait au
    // retour. Seule une entrée qui PORTE l'argument l'écrase (un lien profond).
    var searchQuery by remember { mutableStateOf<String?>(null) }
    var libraryTab by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(backStackEntry) {
        val entry = backStackEntry ?: return@LaunchedEffect
        when {
            entry.destination.hasRoute<Routes.Search>() ->
                entry.toRoute<Routes.Search>().q?.let { searchQuery = it }
            entry.destination.hasRoute<Routes.Library>() ->
                entry.toRoute<Routes.Library>().tab?.let { libraryTab = it }
        }
    }

    // L1 : les quatre racines d'onglet vivent dans UN pager, hissé ici pour que
    // la barre et le routeur lisent le même état — une image, deux lecteurs.
    // `TabPagerSync` est la seule couture entre « la page où le doigt s'est
    // arrêté » et « la route où la personne est ».
    val pager = rememberPagerState(initialPage = 0) { Tab.entries.size }
    val scope = rememberCoroutineScope()
    TabPagerSync(destination, pager) { tab -> nav.openTab(tab) }

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
        ShellFrame(
            wide = wide,
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
                    //
                    // SB3 : plein écran, la barre est couverte ET le mini-lecteur
                    // cesse de lire la position quatre fois par seconde pour
                    // personne.
                    if (!NowPlayingState.open) MiniPlayer(onClick = { graph.openPlayer() })
                    TabBarHost(
                        destination = destination,
                        pager = pager,
                        scope = scope,
                        onOpenTab = { tab -> nav.openTab(tab) },
                        onOpenSettings = { nav.openSettings() },
                    )
                }
            },
            rail = { WideShell(nav, destination) },
            backRow = {
                // La rangée du retour n'existe que si l'écran n'est pas une
                // racine d'onglet (ni Réglages) ET qu'il y a quelque part où
                // revenir : sur l'accueil, la flèche n'aurait rien à faire.
                if (selectedIndexOf(destination) < 0 && nav.previousBackStackEntry != null) {
                    DetailBackRow { graph.back() }
                }
            },
            content = {
                TabLayer(
                    shown = selectedIndexOf(destination) in Tab.entries.indices,
                    nav = graph,
                    pager = pager,
                    searchQuery = searchQuery,
                    libraryTab = libraryTab,
                )
                NavGraph(graph, nav, startRoute)
                TrackActionsHost()
                // M2 : la SEULE surface où un toast est dessiné. Il y avait
                // quarante appels à `NzToasts.show` sur master (réglages, file,
                // album, party…) et aucune coquille ne montait `NzToastHost` :
                // chaque phrase partait dans une liste que personne ne
                // dessinait. Un toast est la moitié d'une règle 3 (un échec dit
                // pourquoi) : la session expirée est la première à en dépendre
                // vraiment.
                NzToastHost(modifier = Modifier.fillMaxSize(), bottomPadding = NzSpacing.m)
            },
            overlay = { NowPlayingHost(nav = graph) },
        )
    }
}

/**
 * Le cadre des deux coquilles, avec les surfaces plein écran AU-DESSUS des barres.
 *
 * [overlay] est composé en DERNIER, hors de la `Scaffold` : le lecteur plein
 * écran (et ses panneaux) couvre donc la barre d'onglets et le mini-lecteur.
 * Monté dans le contenu — ce qu'il faisait — il remplissait la zone entre l'inset
 * haut et la `bottomBar` et laissait celle-ci visible en dessous (bug du
 * propriétaire, 2026-11-26).
 */
@Composable
internal fun ShellFrame(
    wide: Boolean,
    bottomBar: @Composable () -> Unit,
    rail: @Composable () -> Unit,
    backRow: @Composable () -> Unit,
    content: @Composable () -> Unit,
    overlay: @Composable () -> Unit,
) {
    Box(Modifier.fillMaxSize()) {
        if (wide) {
            Row(Modifier.fillMaxSize()) {
                rail()
                Column(Modifier.fillMaxSize()) {
                    // M6 : « quelqu'un pilote ce lecteur » se voit depuis N'IMPORTE
                    // quel écran, en haut de la colonne de contenu sur large.
                    RemoteHostNotice(Modifier.fillMaxWidth())
                    Box(Modifier.fillMaxSize()) { content() }
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
                bottomBar = bottomBar,
                containerColor = MaterialTheme.colorScheme.background,
            ) { padding ->
                // `padding` ET `consumeWindowInsets` : la KDoc de Scaffold demande
                // les deux, et sans la seconde un écran qui appelle
                // `statusBarsPadding()` (le lecteur) compte l'inset deux fois.
                Column(Modifier.fillMaxSize().padding(padding).consumeWindowInsets(padding)) {
                    backRow()
                    Box(Modifier.weight(1f).fillMaxWidth()) { content() }
                }
            }
        }
        overlay()
    }
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
                    if (tab != null) nav.openTab(tab) else nav.openSettings()
                },
                icon = { Icon(item.glyph, size = 24.dp, contentDescription = null) },
                label = { Text(item.label) },
            )
        }
    }
}

/**
 * The graph itself: every route opens its entry composable. The four TAB routes
 * draw nothing — the pages live once in [TabLayer], under this `NavHost` (SB2):
 * the routes only say where the person is, for the back stack, the bar's
 * selection and the deep links.
 */
@Composable
private fun NavGraph(
    graph: AppGraph,
    nav: NavHostController,
    startDestination: Any,
) {
    val navigator: NzNavigator = graph
    val motion = Sillon.motion
    // A transition lambda is not composable: read the density before it. `moveM`
    // is already 0 dp when the level has no displacement (reduced/eco).
    val risePx = with(LocalDensity.current) { motion.moveM.roundToPx() }
    NavHost(
        navController = nav,
        startDestination = startDestination,
        // L1's rule stays a GUARD, not a blanket: a sweep between the four pager
        // pages draws nothing (`isTabSwap`), every other route change is a push —
        // a fade plus the SPA's 8 dp rise, the old page gone in the same frame
        // (M3). A back fades both pages 140 ms, the one case Compose cannot cut
        // without a jump.
        enterTransition = { routeEnter(motion, risePx, tabSwap = isTabSwap()) },
        exitTransition = { routeExit() },
        popEnterTransition = { routePopEnter(motion) },
        popExitTransition = { routePopExit(motion) },
        // Never a size walk: one layout per frame, and it clips (M10).
        sizeTransform = { null },
    ) {
        composable<Routes.Setup> { SetupEntry(nav = navigator) }
        composable<Routes.Login> { LoginEntry(nav = navigator) }
        composable<Routes.Invite> { entry -> InviteEntry(entry.toRoute<Routes.Invite>().code, navigator) }
        composable<Routes.Onboarding> { OnboardingEntry(nav = navigator) }
        composable<Routes.Home> { TabRouteSlot() }
        composable<Routes.Search> { TabRouteSlot() }
        composable<Routes.Artist> { entry -> ArtistEntry(entry.toRoute<Routes.Artist>().id, navigator) }
        composable<Routes.Album> { entry -> AlbumEntry(entry.toRoute<Routes.Album>().id, navigator) }
        composable<Routes.Credits> { entry -> CreditsEntry(entry.toRoute<Routes.Credits>().name, navigator) }
        composable<Routes.Playlist> { entry -> PlaylistEntry(entry.toRoute<Routes.Playlist>().id, navigator) }
        composable<Routes.SharedPlaylist> {
            entry -> SharedPlaylistEntry(entry.toRoute<Routes.SharedPlaylist>().id, navigator)
        }
        composable<Routes.Import> { entry -> ImportEntry(entry.toRoute<Routes.Import>().url, navigator) }
        composable<Routes.Library> { TabRouteSlot() }
        composable<Routes.History> { HistoryEntry(nav = navigator) }
        composable<Routes.Podcasts> { TabRouteSlot() }
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
 * Une route d'onglet ne dessine rien : la page est dans [TabLayer], sous le graphe.
 */
@Composable
private fun TabRouteSlot() = Unit

/**
 * Un changement d'onglet : la porte de la barre et celle du rail.
 *
 * **Aucun état de pile n'est sauvegardé ni restauré.** La version d'avant passait
 * `popUpTo(startDestination) { saveState = true }` + `restoreState = true` : le
 * KDoc de `NavController.popBackStack` dit que `saveState` sauvegarde « the back
 * stack and the state of all destinations » entre la destination courante et la
 * route — donc Réglages, poussé au-dessus de l'onglet, était sauvegardé AVEC lui
 * et `restoreState` le restaurait au sommet : l'écran des paramètres revenait et
 * le tap semblait sans effet (mesuré par `ShellTabsTest.aTabFromSettingsLeavesSettings`).
 *
 * Ce que la pile perd (la position de défilement d'un onglet) est tenu par la
 * composition depuis SB2 : les quatre pages du pager ne sont jamais disposées.
 * `launchSingleTop` reste : taper Bibliothèque deux fois n'empile pas deux
 * bibliothèques.
 */
internal fun NavHostController.openTab(tab: Tab) {
    val route = when (tab) {
        Tab.Accueil -> Routes.Home
        Tab.Rechercher -> Routes.Search()
        Tab.Bibliotheque -> Routes.Library()
        Tab.Podcasts -> Routes.Podcasts
    }
    navigate(route) {
        popUpTo(graph.findStartDestination().id) { saveState = false }
        launchSingleTop = true
        restoreState = false
    }
}

/** The bar's fifth item and the rail's: Réglages, one entry, never two. */
private fun NavHostController.openSettings() {
    navigate(Routes.Settings()) { launchSingleTop = true }
}
