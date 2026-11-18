package fr.neezer.mobile.feature.playlist

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzDuration
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.PlaylistSharedData
import uniffi.nz_ffi.PlaylistSharedRow

/**
 * Playlist partagée — lot MB9 (`routes/SharedPlaylist.svelte`).
 *
 * The public, read-only view of a capability link: whoever holds the token — no
 * account, no session — sees the playlist, and the titles the owner has archived
 * play from `/api/shared/playlist/<token>/stream/<mid>`. Nothing is editable.
 *
 * The ONE gesture this page does not hand to the app's player, and why: the
 * player streams the LIBRARY of the signed-in person (`/api/stream/<id>`), while
 * this link is a capability for a server this device may not even be signed in
 * to. A playable row therefore opens its capability URL with the platform's own
 * viewer (`openStream`), which is what the SPA's `<audio>` element does with it.
 */
@Composable
fun SharedPlaylistEntry(id: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val screens = remember(NzBridge.core) { NzBridge.core?.screens() }
    val service = remember(screens) { screens?.let(::NZPlaylistService) }
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    val model = remember(service, scope) { service?.let { SharedModel(it, scope) } }

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
    val state by model.state.collectAsState()

    SharedPlaylistScreen(
        state = state,
        onBack = nav::back,
        onRetry = model::retry,
        onPlay = { row -> row.streamPath?.let { context.playShared(it) } },
        modifier = modifier,
    )
}

/** The public page as a pure function of its state. */
@Composable
fun SharedPlaylistScreen(
    state: SharedModel.State,
    onBack: () -> Unit,
    onRetry: () -> Unit,
    onPlay: (PlaylistSharedRow) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    // The shell's own top bar names the route; a second one would double it.
    Column(modifier = modifier.fillMaxSize()) {
        when {
            state.failure != null -> LoadFailed(
                cause = state.failure,
                onRetry = onRetry,
                modifier = Modifier.padding(NzSpacing.l),
            )

            state.missing -> EmptyState(
                title = "Ce lien n'existe plus",
                body = "Le propriétaire l'a révoqué, ou la playlist n'est plus partagée.",
                glyph = NzGlyph.Link,
                modifier = Modifier.padding(NzSpacing.l),
            )

            state.data == null -> Column(
                modifier = Modifier.fillMaxWidth().padding(NzSpacing.l),
                verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
            ) {
                Box(
                    Modifier
                        .height(96.dp)
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(NzRadius.cover))
                        .background(colors.s2),
                )
                repeat(4) {
                    Box(Modifier.fillMaxWidth().height(56.dp).background(colors.s1))
                }
            }

            else -> {
                val data = state.data
                val listState = rememberLazyListState()
                LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
                    item(key = "head") {
                        Column(
                            modifier = Modifier.padding(NzSpacing.l),
                            verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
                        ) {
                            Row(
                                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
                                verticalAlignment = Alignment.Bottom,
                            ) {
                                CoverImage(url = data.header.cover, sizeDp = 96.dp, seed = data.token)
                                Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
                                    Text(
                                        text = "${data.header.nbTracks} titres · ${data.header.owner}",
                                        style = Sillon.type.mono,
                                        color = colors.dim,
                                    )
                                    Text(
                                        text = data.header.title,
                                        style = Sillon.type.title,
                                        color = colors.tx,
                                        maxLines = 3,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                }
                            }
                            if (data.header.description.isNotEmpty()) {
                                Text(
                                    data.header.description,
                                    style = Sillon.type.bodySmall,
                                    color = colors.dim,
                                )
                            }
                            Text(
                                "Lien public en lecture seule · ${data.playable} titres écoutables sur ${data.rows.size}",
                                style = Sillon.type.labelSmall,
                                color = colors.dim,
                            )
                        }
                    }
                    items(data.rows, key = { row -> row.row.key }) { row ->
                        SharedRowView(row, onPlay)
                    }
                    item(key = "tail") {
                        Box(Modifier.height(NzSpacing.xl))
                    }
                }
            }
        }
    }
}

@Composable
private fun SharedRowView(row: PlaylistSharedRow, onPlay: (PlaylistSharedRow) -> Unit) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .clickable(enabled = row.streamable, role = Role.Button) { onPlay(row) }
            .padding(horizontal = NzSpacing.m, vertical = NzSpacing.xs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        CoverImage(url = row.row.cover, sizeDp = 44.dp, seed = row.row.id)
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
            Text(
                row.row.title,
                style = Sillon.type.body,
                color = if (row.streamable) colors.tx else colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                if (row.streamable) row.row.artists else "${row.row.artists} · pas encore archivé",
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Text(nzDuration(row.row.durationS.toInt()), style = Sillon.type.mono, color = colors.dim)
        Icon(
            glyph = if (row.streamable) NzGlyph.Play else NzGlyph.CloudOff,
            size = 18.dp,
            tint = if (row.streamable) colors.acText else colors.dim,
            contentDescription = if (row.streamable) "Écouter ${row.row.title}" else "Pas encore archivé",
        )
    }
}

/**
 * The public page's read (lot MB9): the copy this device kept paints first, then
 * the network. A `404` is not a failure but the answer: the link is dead.
 */
class SharedModel(
    private val service: PlaylistService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) {
    data class State(
        val data: PlaylistSharedData? = null,
        val failure: String? = null,
        /** The link is dead (revoked, or never existed): a page, not a retry loop. */
        val missing: Boolean = false,
        val token: String = "",
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    fun start(token: String) {
        if (token.isEmpty()) return
        _state.value = State(token = token)
        load()
    }

    fun retry() = load()

    private fun load() {
        val token = _state.value.token
        if (token.isEmpty()) return
        scope.launch {
            val kept = withContext(io) { service.cachedSharedPlaylist(token) }
            if (kept != null) _state.update { it.copy(data = kept, failure = null) }
            try {
                val fresh = withContext(io) { service.sharedPlaylist(token) }
                _state.update { it.copy(data = fresh, failure = null, missing = false) }
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                val status = (e as? uniffi.nz_ffi.NzException.Server)?.status?.toInt()
                _state.update {
                    when {
                        status == 404 -> it.copy(missing = true, failure = null)
                        it.data != null -> it.copy(failure = null)
                        else -> it.copy(failure = NzErrors.text(e))
                    }
                }
            }
        }
    }
}

/** A context that opens a capability URL (`SharedPlaylistEntry`'s one gesture). */
internal fun playSharedOn(context: Context, url: String) = openStream(context, url)
