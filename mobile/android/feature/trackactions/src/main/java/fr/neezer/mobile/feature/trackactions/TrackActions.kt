package fr.neezer.mobile.feature.trackactions

import android.content.Context
import android.content.Intent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzToasts
import uniffi.nz_ffi.TrackRow

/**
 * The ONE track-actions spec (lot MB10), shared by every list.
 *
 * Every surface that lists a title opens the SAME menu — a playlist row, an
 * album page, the queue, a search hit, a podcast's episode list, the history, the
 * "Je n'aime pas" list — so the actions cannot drift apart. What the menu
 * CONTAINS is decided in Rust (`nz-core::actions::menu`, the port of
 * `webapp/src/lib/trackmenu.js`); this object only says which row it is about and
 * where the sheet is drawn.
 *
 * A surface with facts the menu needs passes them in a [TrackActionsContext]: the
 * queue editor knows each row's position and the playhead, an episode list knows
 * it is showing episodes, a search result does not know the credits' artist ids.
 */
object TrackActions {
    /** One opening of the menu: the row, where a link goes, and the facts. */
    data class Request(
        val row: TrackRow,
        val nav: NzNavigator,
        val context: TrackActionsContext,
    )

    private val current = mutableStateOf<Request?>(null)

    /**
     * The credits sheet's door (MB7's `TrackCreditsHost`), installed by the shell.
     *
     * `:feature/entity` depends on THIS module (its rows open this menu), so this
     * module cannot depend on it: a Gradle project graph has no cycles. The shell
     * holds both, so it sets this once — `TrackActions.creditsSheet = { track,
     * dismiss -> TrackCreditsHost(track, dismiss, nav) }` — and until it does, the
     * menu simply has no "Voir les crédits" entry (a button that opens nothing is
     * a bug, rule 5).
     */
    var creditsSheet: (@Composable (TrackRow, () -> Unit) -> Unit)? = null

    /** What the host is drawing now, or `null`. Read by [TrackActionsHost]. */
    fun currentRequest(): Request? = current.value

    /**
     * Opens the context sheet for [row]. Called by a long press, a "..." button,
     * a right click on a Chromebook, or Shift+F10 from a hardware keyboard.
     */
    fun open(row: TrackRow?, nav: NzNavigator) {
        open(row, nav, TrackActionsContext())
    }

    /** The same, with the facts the surface knows. */
    fun open(row: TrackRow?, nav: NzNavigator, context: TrackActionsContext) {
        if (row == null) return
        current.value = Request(row, nav, context)
    }

    /** Closes the sheet (the veil, Escape, or an entry that did its job). */
    fun close() {
        current.value = null
    }

    /**
     * "Lire ensuite" — what a row's **swipe right** does (`webapp.md`: the row
     * follows the finger and a release past 72 dp queues it; `TrackRowView`'s
     * `onSwipeQueue` is the gesture, this is the verb every list passes it).
     */
    fun queueNext(row: TrackRow) {
        player()?.playNext(row)
        NzToasts.show("Lira ensuite « ${row.title} »")
    }

    /** The app's ONE player, or `null` before the core is built. */
    private fun player(): TrackActionPlayer? =
        TrackActionsPlayerHolder.api()?.let(::NzTrackActionPlayer)
}

/**
 * The facts a surface knows about its own rows — the menu's context.
 *
 * The defaults are the SPA's normal case: a locally-owned account looking at its
 * own list. A queue passes `surface = "queue"` with each row's index and the
 * playhead; an episode list passes `episode = true` and its show; a remote control
 * passes `canKeep = false`.
 */
data class TrackActionsContext(
    val surface: String = "list",
    val index: Int? = null,
    val currentIndex: Int? = null,
    val canKeep: Boolean = true,
    val canBrowse: Boolean = true,
    val remote: Boolean = false,
    val downloaded: Boolean = false,
    val episode: Boolean = false,
    val episodePlaying: Boolean = false,
    val channelId: String? = null,
    val offline: Boolean = false,
    val genreTag: Boolean = false,
    val markers: Boolean = false,
)

/**
 * The sheet's host, placed once by the shell: one menu instance for the whole
 * app, so two lists can never show two menus.
 *
 * It builds the model over the REAL core (`Screens::track_menu` and this lot's
 * writes), the app's ONE player, the platform's share sheet, and the shell's
 * navigator — then draws whichever sheet the state says: the menu, the playlist
 * picker, or the replacement sheet.
 */
@Composable
fun TrackActionsHost(modifier: Modifier = Modifier) {
    val request = TrackActions.currentRequest() ?: return
    val core = NzBridge.core
    val screens = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzTrackActionsService) }
    val player = remember { TrackActionsPlayerHolder.api()?.let(::NzTrackActionPlayer) ?: NoTrackActionPlayer }
    val context = LocalContext.current
    val sharer = remember(context) { AndroidSharer(context) }
    val scope = rememberCoroutineScope()
    val creditsSheet = TrackActions.creditsSheet
    val credits = remember(creditsSheet) { creditsSheet != null }

    if (service == null) {
        // A process whose core could not be built: no sheet rather than one that
        // does nothing; Aide says why (`NzBridge.failure`).
        LaunchedEffect(Unit) { TrackActions.close() }
        return
    }

    val model = remember(request) {
        TrackActionsModel(
            service = service,
            player = player,
            nav = request.nav,
            sharer = sharer,
            context = menuContext(
                surface = request.context.surface,
                index = request.context.index,
                currentIndex = request.context.currentIndex,
                canKeep = request.context.canKeep,
                canBrowse = request.context.canBrowse,
                remote = request.context.remote,
                downloaded = request.context.downloaded,
                episode = request.context.episode,
                episodePlaying = request.context.episodePlaying,
                channelId = request.context.channelId,
                // The credits door is the shell's: hidden until it is installed.
                credits = credits,
                offline = request.context.offline,
                genreTag = request.context.genreTag,
                markers = request.context.markers,
            ),
            scope = scope,
            creditsDoor = credits,
            closeHost = TrackActions::close,
            initialRow = request.row,
        )
    }
    val state by model.state.collectAsState()

    val subject = state.credits
    if (creditsSheet != null && subject != null) {
        // MB7's own sheet, over the screen the menu came from: one component for
        // the full-screen player's « À propos » and for this menu.
        creditsSheet.invoke(subject) { model.dismissCredits() }
        return
    }

    when (state.sheet) {
        TrackActionsModel.Sheet.Menu -> TrackMenuSheet(
            state = state,
            onRun = model::run,
            onToggleSub = model::toggleSub,
            onDismiss = model::close,
            modifier = modifier,
        )
        TrackActionsModel.Sheet.Picker -> PlaylistPickerSheet(
            state = state,
            onFilter = model::filter,
            onPick = { choice -> model.addToPlaylist(choice.id, choice.title) },
            onCreate = model::createPlaylist,
            onStartCreating = model::startCreating,
            onCancelCreating = model::cancelCreating,
            onName = model::namePlaylist,
            onRetry = model::loadPicker,
            onDismiss = model::backToMenu,
            modifier = modifier,
        )
        TrackActionsModel.Sheet.Replace -> ReplaceSheet(
            state = state,
            onSearch = model::searchReplace,
            onFallbacks = model::searchFallbacks,
            onCommit = model::commitReplace,
            onDelete = model::deleteTarget,
            onRetry = model::retryCandidates,
            onDismiss = model::backToMenu,
            modifier = modifier,
        )
    }
}

/**
 * The platform's share sheet: what "Partager…" hands the address to.
 *
 * A share is an INTENT, never a copy to the log (the ARL rule's spirit: a link is
 * the person's to send). `FLAG_ACTIVITY_NEW_TASK` because the context a Compose
 * host sees is not always an Activity.
 */
class AndroidSharer(private val context: Context) : TrackActionSharer {
    override fun share(text: String, title: String) {
        val send = Intent(Intent.ACTION_SEND)
            .setType("text/plain")
            .putExtra(Intent.EXTRA_TEXT, text)
        val chooser = Intent.createChooser(send, title)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(chooser) }
    }
}
