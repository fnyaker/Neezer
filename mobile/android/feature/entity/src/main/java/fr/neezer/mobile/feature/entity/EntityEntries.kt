package fr.neezer.mobile.feature.entity

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.feature.trackactions.TrackActions
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.TrackRow

/**
 * Artiste — lot MB7 (docs/agents/mobile.md, feature inventory).
 *
 * The screen itself is [ArtistScreen], a pure function of the model's state; this
 * entry only wires it to the app: the core's screens, the player behind
 * [EntityPlayer], and the shell's routes. Every door a person can take is here,
 * so nothing on the screen reaches for a `NavController` of its own:
 *
 * - a release, a similar artist and a name in the credits open their routes
 *   through [NzNavigator]'s `AppLink`s (the one way a tap and a deep link land on
 *   the same screen);
 * - a row's ⋯ opens **MB10's** one context menu;
 * - the player is the app's ONE player (`NzCore::player()`), behind the port.
 */
@Composable
fun ArtistEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val artistService = remember(screens) { screens?.let(::NzArtistService) }
    val creditsService = remember(screens) { screens?.let(::NzCreditsService) }
    val player: EntityPlayer = remember { EntityPlayerHolder.api()?.let(::NzEntityPlayer) ?: NoEntityPlayer }
    val scope = rememberCoroutineScope()
    val model = remember(artistService, creditsService, scope) {
        if (artistService == null || creditsService == null) {
            null
        } else {
            ArtistModel(artistService, creditsService, player, scope)
        }
    }

    if (model == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }

    LaunchedEffect(model, id) { model.load(id) }
    val state by model.state.collectAsState()

    ArtistScreen(
        state = state,
        actions = ArtistActions(
            retry = model::retry,
            tab = model::selectTab,
            playTop = model::playTop,
            playTopAll = model::playTopAll,
            radio = model::startRadio,
            favorite = model::toggleFavorite,
            playAllTracks = model::playAllTracks,
            playTrack = model::playFromTracks,
            queueNext = model::queueNext,
            more = { row -> TrackActions.open(row, nav) },
            retryTracks = model::retryTracks,
            retryCredits = model::retryCredits,
            creditsRole = { role ->
                if (role.isEmpty()) model.clearCreditsRole() else model.loadCredits(role)
            },
            playCredits = model::playCredits,
            playFromCredits = model::playFromCredits,
            openAlbum = { albumId -> nav.go(AppLink.Album(albumId)) },
            openArtist = { artistId -> nav.go(AppLink.Artist(artistId)) },
            openPerson = { name -> nav.go(AppLink.Credits(name)) },
        ),
        modifier = modifier,
    )
}

/**
 * Album — lot MB7. The same wiring as [ArtistEntry]: the page, the player behind
 * its port, and the routes ([NzNavigator]'s `AppLink`s for the artist).
 */
@Composable
fun AlbumEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val albumService = remember(screens) { screens?.let(::NzAlbumService) }
    val player: EntityPlayer = remember { EntityPlayerHolder.api()?.let(::NzEntityPlayer) ?: NoEntityPlayer }
    val scope = rememberCoroutineScope()
    val model = remember(albumService, scope) {
        albumService?.let { AlbumModel(it, player, scope) }
    }

    if (model == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }

    LaunchedEffect(model, id) { model.load(id) }
    val state by model.state.collectAsState()

    AlbumScreen(
        state = state,
        actions = AlbumActions(
            retry = model::retry,
            playAll = model::playAll,
            shuffle = model::playShuffled,
            favorite = model::toggleFavorite,
            playFrom = model::playFrom,
            playMissing = model::playMissing,
            queueNext = model::queueNext,
            more = { row -> TrackActions.open(row, nav) },
            openArtist = { artistId -> nav.go(AppLink.Artist(artistId)) },
        ),
        modifier = modifier,
    )
}

/**
 * Crédits — lot MB7: one person's page. The route argument is the name the
 * credits spell (`/api/credits/person?name=`), decoded by the navigation graph.
 */
@Composable
fun CreditsEntry(name: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzCreditsService) }
    val player: EntityPlayer = remember { EntityPlayerHolder.api()?.let(::NzEntityPlayer) ?: NoEntityPlayer }
    val scope = rememberCoroutineScope()
    val model = remember(service, scope) { service?.let { PersonModel(it, player, scope) } }

    if (model == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }

    LaunchedEffect(model, name) { model.load(name) }
    val state by model.state.collectAsState()

    CreditsScreen(
        state = state,
        actions = CreditsActions(
            retry = model::retry,
            role = model::selectRole,
            playAll = model::playAll,
            playFrom = model::playFrom,
            queueNext = model::queueNext,
            more = { row -> TrackActions.open(row, nav) },
        ),
        modifier = modifier,
    )
}

/**
 * The track-credits sheet's host — **the door MB10's menu and MB11's player panel
 * open** ("Voir les crédits"), and the one place a sheet exists.
 *
 * The caller keeps the sheet's SUBJECT: `track` is the title to show, `null`
 * closes it. The last title is remembered so the exit has something to animate,
 * and the sheet's own model does the one request (kept on the device, painted
 * from the copy first). A name in it opens the artist's page or the person's
 * credits page, and following it closes the sheet (the SPA's `hashchange` rule).
 *
 * It is a composable of THIS module so a lot that shows the sheet only calls it:
 * `TrackCreditsHost(track = row, onDismiss = { open = null }, nav = nav)`.
 */
@Composable
fun TrackCreditsHost(
    track: TrackRow?,
    onDismiss: () -> Unit,
    nav: NzNavigator,
    modifier: Modifier = Modifier,
) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzCreditsService) }
    val scope = rememberCoroutineScope()
    val model = remember(service, scope) { service?.let { TrackCreditsModel(it, scope) } }
    val state = model?.state?.collectAsState()?.value ?: TrackCreditsModel.State()
    var shown by remember { mutableStateOf<TrackRow?>(null) }

    LaunchedEffect(track?.key) {
        if (track != null) {
            shown = track
            model?.load(track)
        } else {
            model?.clear()
        }
    }

    TrackCreditsSheet(
        state = state,
        track = shown,
        visible = track != null,
        onDismiss = onDismiss,
        onRetry = { model?.retry() },
        onArtist = { artistId ->
            onDismiss()
            nav.go(AppLink.Artist(artistId))
        },
        onPerson = { person ->
            onDismiss()
            nav.go(AppLink.Credits(person))
        },
        modifier = modifier,
    )
}
