package fr.neezer.mobile.feature.playlist

import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzToasts
import fr.neezer.mobile.feature.trackactions.TrackActions
import fr.neezer.mobile.feature.trackactions.TrackActionsContext
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.Screens

/**
 * Une playlist — lot MB9 (docs/agents/mobile.md, feature inventory).
 *
 * The screen is [PlaylistScreen] and its sheets, pure functions of the model's
 * state; this entry only wires them to the app: the core's screens, the player,
 * MB10's context menu, MB15's download button (inside the screen), the shell's
 * routes, and the platform's share sheet.
 *
 * Every door a person can take is here, so nothing on the screen reaches for a
 * `NavController` of its own:
 *
 * - a row taps into the queue (`context = playlist:<id>`, so the player's own
 *   refill and the history name it);
 * - a row's ⋯ opens **MB10's** one menu, with the playlist as its surface;
 * - the suggestions' `+` and « Tout ajouter » append through the model (one
 *   request);
 * - « Voir les collaborateurs », « Exporter » and « Renommer » open the sheets;
 * - deleting, leaving or revoking asks first (a sheet, never a silent tap).
 */
@Composable
fun PlaylistEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val screens: Screens? = remember(NzBridge.core) { NzBridge.core?.screens() }
    val service = remember(screens) { screens?.let(::NZPlaylistService) }
    val player: PlaylistPlayer = remember {
        PlaylistPlayerHolder.api()?.let(::NZPlaylistPlayer) ?: NoPlaylistPlayer
    }
    val sharer: PlaylistSharer = remember(context) { AndroidSharer(context) }
    val scope = rememberCoroutineScope()
    val model = remember(service, player, sharer, scope) {
        service?.let { PlaylistModel(it, player, sharer, scope) }
    }

    if (model == null) {
        Column(modifier.fillMaxSize().padding(NzSpacing.l)) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }

    LaunchedEffect(model, id) { model.start(id) }
    // Leaving sends the order still in the drag, and stops the export's polls.
    DisposableEffect(model) { onDispose { model.leaveScreen() } }

    val state by model.state.collectAsState()

    // A playlist that is gone (deleted, access revoked) takes the person back to
    // the library, exactly as the SPA pushes `/library` (`#markGone`).
    LaunchedEffect(state.gone) {
        if (state.gone) {
            NzToasts.show("Cette playlist ne vous est plus accessible.")
            nav.back()
        }
    }

    val sheetActions = PlaylistSheetActions(
        close = {
            if (state.members != null) model.closeMembers() else model.closeExport()
        },
        openMembers = model::openMembers,
        openExport = model::openExport,
        openRename = model::openMeta,
        delete = { model.delete(false) },
        leave = { model.leave(false) },
        confirm = {
            when (state.confirm) {
                PlaylistModel.Confirm.Delete -> model.delete(true)
                PlaylistModel.Confirm.Leave -> model.leave(true)
                PlaylistModel.Confirm.RevokeLink -> model.revokeLink(true)
                null -> Unit
            }
        },
        cancelConfirm = model::cancelConfirm,
        inviteName = model::onInviteName,
        inviteRole = model::onInviteRole,
        invite = model::invite,
        setRole = model::setMemberRole,
        removeMember = model::removeMember,
        chooseTab = model::chooseTab,
        chooseFormat = model::chooseFormat,
        chooseListFormat = model::chooseListFormat,
        chooseService = model::chooseService,
        exportName = model::onExportName,
        togglePublic = model::toggleExportPublic,
        sendExport = model::sendExport,
        resetJob = model::resetExportJob,
        toggleMissing = model::toggleMissing,
        downloadList = model::downloadList,
        copyMissing = model::copyMissing,
        createLink = model::createLink,
        shareLink = model::shareLink,
        revokeLink = { model.revokeLink(false) },
        renameTitle = model::onTitle,
        renameDescription = model::onDescription,
        saveRename = model::saveMeta,
    )

    PlaylistScreen(
        state = state,
        actions = PlaylistActions(
            back = nav::back,
            retry = model::retry,
            refresh = model::refresh,
            play = model::playAll,
            shuffle = model::shufflePlay,
            toggleEdit = model::toggleEdit,
            move = model::move,
            remove = model::removeAt,
            openRow = { _, index -> model.playFrom(index) },
            more = { row, index ->
                TrackActions.open(
                    row,
                    nav,
                    TrackActionsContext(surface = "playlist", index = index),
                )
            },
            queueNext = TrackActions::queueNext,
            query = model::onQuery,
            sort = model::chooseSort,
            direction = model::toggleDirection,
            openMore = model::openMore,
            openMeta = model::openMeta,
            addTracks = model::openAddTracks,
            addRecommendation = model::addRecommendation,
            addAllRecommendations = model::addAllRecommendations,
            retryRecommendations = model::retryRecommendations,
            staleRefresh = model::refresh,
            leavePlaylist = { model.leave(false) },
        ),
        modifier = modifier,
    )

    if (state.more) {
        PlaylistMoreSheet(state, sheetActions)
    }
    if (state.editingMeta) {
        PlaylistRenameSheet(state, sheetActions)
    }
    if (state.members != null) {
        PlaylistMembersSheet(state, sheetActions)
    }
    if (state.export != null) {
        PlaylistExportSheet(state, sheetActions)
    }
    PlaylistConfirmSheet(state, sheetActions)

    AddTracksSheet(
        state = state,
        onTerm = model::onAddTerm,
        onAdd = model::addChosen,
        onDismiss = model::closeAddTracks,
    )
}

/**
 * The platform's share sheet and clipboard: a public link or a list file is an
 * INTENT, never a copy in a log (and `FLAG_ACTIVITY_NEW_TASK` because the context
 * a Compose host sees is not always an Activity).
 */
class AndroidSharer(private val context: Context) : PlaylistSharer {
    override fun share(text: String, title: String) {
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
        val chooser = Intent.createChooser(send, title).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(chooser) }
    }

    override fun copy(text: String, label: String) {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? android.content.ClipboardManager
            ?: return
        clipboard.setPrimaryClip(android.content.ClipData.newPlainText(label, text))
        NzToasts.show(label)
    }
}

/** The route a playlist opens: the shell's `NavGraph` owns the screen itself. */
fun playlistLink(id: String): AppLink = AppLink.Playlist(id)
