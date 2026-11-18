package fr.neezer.mobile.feature.search

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.NzToasts
import fr.neezer.mobile.feature.trackactions.TrackActions
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.EntityKind
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.SearchScope

/**
 * Rechercher — lot MB6 (docs/agents/mobile.md, feature inventory).
 *
 * The screen itself is [SearchScreen], a pure function of the model's state;
 * this entry only wires it to the app: the core's screens, the player behind
 * [SearchPlayer], and the shell's routes. Every door a person can take is here,
 * so nothing on the screen reaches for a `NavController` of its own:
 *
 * - an entity card opens its route through [NzNavigator]'s `AppLink`s (the one
 *   way a tap and a deep link land on the same screen);
 * - a row's ⋯ opens **MB10's** one context menu;
 * - a title taps into its own radio (`playThenRadio`: the row alone, then the
 *   player's refill), and « Lancer la radio » hands the plan to the same player;
 * - a pasted playlist link opens **MB9's** import route.
 */
@Composable
fun SearchEntry(q: String?, nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzSearchService) }
    val player: SearchPlayer = remember { SearchPlayerHolder.api()?.let(::NzSearchPlayer) ?: NoSearchPlayer }
    val scope = rememberCoroutineScope()
    val model = remember(service, player, scope) { service?.let { SearchModel(it, player, scope) } }

    if (model == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }

    LaunchedEffect(model, q) { model.start(q) }

    // Leaving remembers what was really searched (the SPA's `onDestroy`).
    DisposableEffect(model) { onDispose { model.leave() } }

    val state by model.state.collectAsState()
    SearchScreen(
        state = state,
        actions = SearchActions(
            query = model::onQuery,
            submit = model::submit,
            retry = model::retry,
            chooseScope = model::chooseScope,
            toggleYoutube = model::toggleYoutube,
            tab = model::chooseTab,
            useRecent = model::useRecent,
            clearRecent = model::clearRecent,
            launchRadio = model::launchRadio,
            retryFallback = model::retryFallback,
            // A title plays and then its radio: the queue is that title alone with
            // the `radio` context, and the player's own refill tops it up.
            playTrack = { row -> player.play(listOf(row), "radio", "track:${row.id}") },
            more = { row -> TrackActions.open(row, nav) },
            openEntity = { kind, id -> nav.go(linkOf(kind, id)) },
            playTop = { top ->
                if (top.kind == EntityKind.ARTIST) {
                    player.playEntity("artist", top.row.id)
                } else {
                    nav.go(AppLink.Podcast(top.row.deezerId))
                }
            },
            // MB13 owns the episode panel and the seek into a transcript's
            // moment; until it lands the tap says where the passage will open
            // (this lot's docs list it as not ported).
            openPassage = {
                NzToasts.show("La lecture d'un passage arrive avec le lot MB13.")
            },
            importLink = { offer -> nav.go(AppLink.Import(offer.url)) },
        ),
        modifier = modifier,
    )
}

/** The route an entity card opens: the tab the person tapped names the kind. */
private fun linkOf(kind: EntityKind, id: String): AppLink = when (kind) {
    EntityKind.ARTIST -> AppLink.Artist(id)
    EntityKind.ALBUM -> AppLink.Album(id)
    EntityKind.PLAYLIST -> AppLink.Playlist(id)
    EntityKind.SHOW -> AppLink.Podcast(id)
}

/** The scope a chip selects, in the strings the model and the screen share. */
internal fun scopeOf(podcasts: Boolean): SearchScope =
    if (podcasts) SearchScope.PODCASTS else SearchScope.LIBRARY
