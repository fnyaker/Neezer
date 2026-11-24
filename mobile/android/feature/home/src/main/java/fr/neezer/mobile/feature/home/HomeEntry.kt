package fr.neezer.mobile.feature.home

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.feature.trackactions.TrackActions
import kotlinx.coroutines.CoroutineScope
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.Screens

/**
 * Accueil — lot MB5 (docs/agents/mobile.md, feature inventory).
 *
 * The screen itself is [HomeScreen], a pure function of the model's state; this
 * entry only wires it to the app: the core's screens, the player behind
 * [PlayerPort], and the shell's routes. Every door a person can take is here, so
 * nothing on the screen reaches for a `NavController` of its own:
 *
 * - the Récap card and a new release open their routes through [NzNavigator]'s
 *   `AppLink`s (the one way a tap and a deep link land on the same screen);
 * - a row's ⋯ opens **MB10's** one context menu;
 * - « Reprendre » opens the player, « Explorer » opens the search.
 */
@Composable
fun HomeEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzHomeService) }
    val port: PlayerPort = remember { AppPlayer.api()?.let(::NzPlayerPort) ?: NoPlayer }
    val scope = rememberCoroutineScope()
    val model = remember(service, port, scope) { service?.let { HomeModel(it, port, scope) } }

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

    // TTFD (docs/agents/mobile.md, *Measures*): `reportFullyDrawn()` marks the
    // launch only once the FIRST screen has what it draws — the home's data, not
    // a skeleton. It is the Home's fact, not the Activity's: the shell composes
    // in `onCreate`, and reporting there would time the skeleton's first frame
    // (the defect MQ2 measured as "TTFD: not measured"). One report per Activity:
    // a retry that re-enters Ready must not move the mark.
    val activity = LocalContext.current.findActivity()
    val reported = remember(activity) { java.util.concurrent.atomic.AtomicBoolean(false) }
    val painted = state.data != null
    LaunchedEffect(activity, painted) {
        if (activity == null || !painted) return@LaunchedEffect
        if (!reported.compareAndSet(false, true)) return@LaunchedEffect
        // The next frame is the one that DRAWS the data's composition.
        withFrameNanos { }
        activity.reportFullyDrawn()
    }

    Column(modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        HomeScreen(
            state = state,
            actions = HomeActions(
                launchFlow = model::launchFlow,
                tune = model::tune,
                notch = model::chooseNotch,
                discovery = model::chooseDiscovery,
                pickStyle = model::pickStyle,
                clearStyles = model::clearStyles,
                playRelease = model::playRelease,
                playAllNew = model::playAllNew,
                openReleases = { nav.go(AppLink.Releases) },
                openArtist = { id -> nav.go(AppLink.Artist(id)) },
                playReco = model::playReco,
                queueReco = model::queueNext,
                more = { row -> TrackActions.open(row, nav) },
                moreSuggestions = model::refreshReco,
                playMood = model::playMood,
                openRecap = { year -> nav.go(AppLink.Recap(year.toUInt())) },
                openPlayer = { nav.openPlayer() },
                openSearch = { nav.go(AppLink.Search(q = null)) },
                retry = model::retry,
                flowPanel = model::flowPanel,
                moreLikeThis = { model.pressFlow(uniffi.nz_ffi.FlowPress.MORE) },
                lessLikeThis = { model.pressFlow(uniffi.nz_ffi.FlowPress.LESS) },
                nudge = model::nudge,
                resetFlow = model::resetFlow,
            ),
        )
    }
}

/**
 * The hosting Activity, through any `ContextWrapper` a preview or a test host
 * puts in front of it (`LocalContext` is not guaranteed to be one in a Compose
 * host). `null` in a host with no Activity: reporting is a diagnostic, never a
 * condition of drawing.
 */
private fun Context.findActivity(): Activity? {
    var context: Context? = this
    while (context is ContextWrapper) {
        if (context is Activity) return context
        context = context.baseContext
    }
    return context as? Activity
}
