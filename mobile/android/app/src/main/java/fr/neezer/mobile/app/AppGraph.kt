package fr.neezer.mobile.app

import android.app.Application
import androidx.navigation.NavHostController
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.feature.nowplaying.NowPlayingState
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.resolveLink

/**
 * The manual dependency graph (docs/agents/mobile.md, "Repository layout":
 * `:app` holds `AppGraph`). One instance, created by the activity and passed to
 * the shell; no annotation processor, no service locator, no reflection — a
 * dependency is a constructor argument and a missing one is a compile error.
 *
 * It is also the ONE [NzNavigator]: a feature module asks for a route and never
 * sees a `NavController`, so a deep link and a tap land in the same place.
 */
class AppGraph(private val application: Application) : NzNavigator {

    /** Set by the shell as soon as the `NavHost` exists. */
    private var controller: NavHostController? = null

    /**
     * A link that arrived BEFORE the graph existed (a cold start from a shared
     * post): kept until [attach], then opened once.
     */
    private var pendingLink: String? = null

    fun attach(navController: NavHostController) {
        controller = navController
        // NOTHING is navigated here: `attach` runs at the top of the shell's
        // composition, BEFORE the `NavHost` composes — its graph does not exist
        // yet, and `NavController.getGraph()` throws
        // (`IllegalStateException: You must call setGraph() before calling
        // getGraph()`), killing the app on a cold start from a shared link.
        // [openPendingLink] runs once, from the shell, after the graph exists.
        // (MB14 measured the same crash from `neezer://releases` on
        // emulator-5640; the orchestrator's own fix landed on master first, so
        // this lot keeps master's shape and only its two `AppLink` branches.)
    }

    /**
     * Keeps a link that arrived from an `Intent` (a share sheet's text, a
     * `neezer://` view). It waits for a controller whose graph is really there,
     * which on a cold start is after the `NavHost`'s first composition — so the
     * intent that STARTED the app and one that arrives during the first frames
     * (`onNewIntent`) take the same door.
     */
    fun openLinkLater(url: String) {
        if (graphReady()) openLink(url) else pendingLink = url
    }

    /**
     * Opens the link a cold start kept, once the shell's `NavHost` has a graph.
     * Called from the shell's own effect (after `NavHost`), so the graph is
     * there; a composition that has not reached it yet simply leaves the link
     * pending for the next call.
     */
    fun openPendingLink() {
        val url = pendingLink ?: return
        if (!graphReady()) return
        pendingLink = null
        openLink(url)
    }

    /** Whether a controller exists AND has its graph (never throws on a cold start). */
    private fun graphReady(): Boolean = runCatching {
        controller?.currentBackStackEntry != null
    }.getOrDefault(false)

    /**
     * La route sur laquelle le shell s'ouvre, décidée UNE fois, avant que le
     * `NavHost` ne compose, et SANS réseau : la première image n'attend jamais
     * `GET /api/me` (rules 1 et 3).
     *
     * - pas de noyau : l'écran serveur (il dit pourquoi le noyau manque) ;
     * - pas de serveur enregistré : l'écran serveur ;
     * - un cookie de session sur l'appareil : l'accueil (la restauration tourne
     *   en fond depuis `NzApplication`) ;
     * - un serveur sans cookie : la connexion, serveur conservé.
     */
    fun startRoute(): Any {
        val core = NzBridge.core ?: return Routes.Setup
        if (core.server() == null) return Routes.Setup
        return if (core.hasSession()) Routes.Home else Routes.Login
    }

    override fun openHomeAfterSignIn() = replaceStackWith(Routes.Home)

    /** M2 : un jeton mort ou une déconnexion ramène à la connexion, pile vidée. */
    override fun openLogin() = replaceStackWith(Routes.Login)

    /** M2 : « Changer de serveur » ouvre l'écran serveur, rien n'est effacé. */
    override fun openSetup() = replaceStackWith(Routes.Setup)

    /** Une seule définition de « remplacer la pile » : chaque route laissée
     *  derrière est un piège à retour (M1 : la préparation ; M2 : la session). */
    private fun replaceStackWith(route: Any) {
        val nav = controller ?: return
        nav.navigate(route) {
            popUpTo(nav.graph.id) { inclusive = true }
            launchSingleTop = true
        }
    }

    /** The app's version, as the launcher shows it (Aide and the diagnostic log). */
    fun appVersion(): String = runCatching {
        application.packageManager.getPackageInfo(application.packageName, 0).versionName
    }.getOrNull() ?: "0"

    /**
     * Opens [link] — every `AppLink` variant has a route, so this `when` fails to
     * compile the day the Rust enum gains one.
     */
    override fun go(link: AppLink) {
        val nav = controller ?: return
        when (link) {
            is AppLink.Home -> nav.navigate(Routes.Home) { launchSingleTop = true }
            is AppLink.Search -> nav.navigate(Routes.Search(link.q)) { launchSingleTop = true }
            is AppLink.Artist -> nav.navigate(Routes.Artist(link.id))
            is AppLink.Album -> nav.navigate(Routes.Album(link.id))
            is AppLink.Playlist -> nav.navigate(Routes.Playlist(link.id))
            is AppLink.Track -> {
                // MB10 lands a track link on the track, inside the album the
                // server says it belongs to. There is no route of its own in the
                // inventory, and inventing one here would fork it.
            }
            is AppLink.Podcast -> nav.navigate(Routes.Show(link.id))
            is AppLink.Credits -> nav.navigate(Routes.Credits(link.name))
            is AppLink.SharedPlaylist -> nav.navigate(Routes.SharedPlaylist(link.id))
            is AppLink.Import -> nav.navigate(Routes.Import(link.url))
            is AppLink.Party -> nav.navigate(Routes.Party(link.id))
            is AppLink.RemoteClaim -> nav.navigate(Routes.RemoteClaim(link.token))
            is AppLink.Invite -> nav.navigate(Routes.Invite(link.code))
            is AppLink.Settings -> nav.navigate(Routes.Settings(link.section))
            is AppLink.Recap -> nav.navigate(Routes.Recap(link.year?.toInt()))
            // MB14's two routes: the releases feed, and one shelf behind it (the
            // door MB5 left out, and the page its cards open).
            is AppLink.Releases -> nav.navigate(Routes.Releases) { launchSingleTop = true }
            is AppLink.ReleaseShelf -> nav.navigate(Routes.ReleaseShelf(link.kind))
        }
    }

    /** Back goes to the SCREEN, not the route (webapp.md's rule). */
    override fun back() {
        val nav = controller ?: return
        if (!nav.popBackStack()) {
            // Nothing to pop: the person is on the first screen, and a back that
            // leaves the app is Android's own behaviour.
        }
    }

    /** The full-screen player: `:feature:nowplaying`'s own sheet (MB11). */
    override fun openPlayer() {
        NowPlayingState.show()
    }

    /** The queue editor, the player behind it, on the playing row (MB11). */
    override fun openQueue() {
        NowPlayingState.openQueue()
    }

    /**
     * The full-screen visualiser (MB21). One destination, reached from Réglages →
     * Animations (`VisualizerSettings.svelte#openScreen`'s door); `launchSingleTop`
     * so two taps do not stack two scenes, each with its own analysis view.
     */
    override fun openViz() {
        val nav = controller ?: return
        nav.navigate(Routes.Viz) { launchSingleTop = true }
    }

    /**
     * An incoming URL (`neezer://…`, a `/app/#/…` link, a shared Deezer or
     * Spotify URL) opened through [resolveLink] (MB2 owns the rule).
     */
    fun openLink(url: String) {
        resolveLink(url)?.let(::go)
    }
}
