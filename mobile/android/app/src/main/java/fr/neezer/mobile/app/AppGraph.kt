package fr.neezer.mobile.app

import android.app.Application
import androidx.navigation.NavHostController
import fr.neezer.mobile.bridge.NzNavigator
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
        pendingLink?.let { url ->
            pendingLink = null
            openLink(url)
        }
    }

    /**
     * Keeps a link that arrived from an `Intent` (a share sheet's text, a
     * `neezer://` view). Opening it now would find no `NavHost` on a cold start,
     * so it waits for [attach].
     */
    fun openLinkLater(url: String) {
        if (controller == null) pendingLink = url else openLink(url)
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

    /** The full-screen player (MB11 shows the sheet). */
    override fun openPlayer() {
        // MB11: openPlayer() raises the sheet's state in the core.
    }

    /** The queue editor (MB11). */
    override fun openQueue() {
        // MB11: the queue route, scrolled to the current track.
    }

    /**
     * An incoming URL (`neezer://…`, a `/app/#/…` link, a shared Deezer or
     * Spotify URL) opened through [resolveLink] (MB2 owns the rule).
     */
    fun openLink(url: String) {
        resolveLink(url)?.let(::go)
    }
}
