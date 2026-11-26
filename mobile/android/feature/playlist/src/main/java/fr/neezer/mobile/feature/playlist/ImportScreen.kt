package fr.neezer.mobile.feature.playlist

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.CoverImage
import fr.neezer.mobile.designsystem.EmptyState
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.LoadFailed
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzChip
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzSwap
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.nzDuration
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PlaylistImportCount
import uniffi.nz_ffi.PlaylistImportItem
import uniffi.nz_ffi.PlaylistImportStatus
import uniffi.nz_ffi.TrackRow

/**
 * Importer une playlist — lot MB9 (`routes/Import.svelte`,
 * `neezer/webui/playlist_import.py`).
 *
 * A shared link (Deezer, Spotify, YouTube, Apple Music) is read and matched BY THE
 * SERVER as a background job (rule 2): this screen starts it, polls its progress,
 * shows every line's outcome and its CAUSE when there is none, and keeps it as a
 * local playlist only when the person asks (`/save`). Nothing waits on a request
 * thread, and nothing is written until « Enregistrer ».
 */
@Composable
fun ImportEntry(url: String?, nav: NzNavigator, modifier: Modifier = Modifier) {
    val screens = remember(NzBridge.core) { NzBridge.core?.screens() }
    val service = remember(screens) { screens?.let(::NZPlaylistService) }
    val player: PlaylistPlayer = remember {
        PlaylistPlayerHolder.api()?.let(::NZPlaylistPlayer) ?: NoPlaylistPlayer
    }
    val scope = rememberCoroutineScope()
    val model = remember(service, player, scope) {
        service?.let { ImportModel(it, player, scope) }
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

    LaunchedEffect(model, url) { model.start(url) }
    val state by model.state.collectAsState()

    // The shell's own top bar names the route; a second one would double it.
    ImportPhases(
        state = state,
        modifier = modifier.fillMaxSize(),
        onLink = model::onLink,
        onSubmit = model::submit,
        onRetry = { model.start(state.link) },
        onClear = model::clear,
        onPlay = model::playAll,
        onShuffle = model::shufflePlay,
        onKeep = model::openKeep,
        onTitle = model::onTitle,
        onSave = model::save,
        onCancelKeep = model::cancelKeep,
        onAnother = model::clear,
        onSearch = { line -> nav.go(uniffi.nz_ffi.AppLink.Search(q = line)) },
        onOpen = { id -> nav.go(uniffi.nz_ffi.AppLink.Playlist(id)) },
    )
}

/** The import's three phases: a failure, the link box, the job (M7). */
internal const val IMPORT_FAILED = "echec"
internal const val IMPORT_INTRO = "intro"
internal const val IMPORT_BODY = "corps"

internal fun importPhase(state: ImportModel.State): String = when {
    state.failure != null && state.job == null -> IMPORT_FAILED
    state.job == null -> IMPORT_INTRO
    else -> IMPORT_BODY
}

/**
 * The screen's phase, replace-in-place (M7): the cause gives way to the link box,
 * which gives way to the job — 140 ms + 4 dp, and nothing at eco. Public to the
 * module's own test, which renders a [ImportModel.State] with no core behind it.
 */
@Composable
internal fun ImportPhases(
    state: ImportModel.State,
    modifier: Modifier = Modifier,
    onLink: (String) -> Unit = {},
    onSubmit: () -> Unit = {},
    onRetry: () -> Unit = {},
    onClear: () -> Unit = {},
    onPlay: () -> Unit = {},
    onShuffle: () -> Unit = {},
    onKeep: () -> Unit = {},
    onTitle: (String) -> Unit = {},
    onSave: () -> Unit = {},
    onCancelKeep: () -> Unit = {},
    onAnother: () -> Unit = {},
    onSearch: (String) -> Unit = {},
    onOpen: (String) -> Unit = {},
) {
    Column(modifier = modifier) {
        NzSwap(targetState = importPhase(state), label = "import") { phase ->
            when (phase) {
                IMPORT_FAILED -> LoadFailed(
                    cause = state.failure.orEmpty(),
                    onRetry = onRetry,
                    modifier = Modifier.padding(NzSpacing.l),
                )
                IMPORT_INTRO -> ImportIntro(state, onLink, onSubmit, onClear)
                else -> ImportBody(
                    state = state,
                    onPlay = onPlay,
                    onShuffle = onShuffle,
                    onKeep = onKeep,
                    onTitle = onTitle,
                    onSave = onSave,
                    onCancelKeep = onCancelKeep,
                    onAnother = onAnother,
                    onSearch = onSearch,
                    onOpen = onOpen,
                )
            }
        }
    }
}

@Composable
private fun ImportIntro(
    state: ImportModel.State,
    onLink: (String) -> Unit,
    onSubmit: () -> Unit,
    onClear: () -> Unit,
) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier.fillMaxWidth().padding(NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Text("Importer une playlist", style = Sillon.type.displaySmall, color = colors.tx)
        Text(
            "Colle le lien d'une playlist qu'on t'a partagée : Deezer, Spotify, YouTube ou Apple Music. " +
                "Chaque titre est retrouvé (ta bibliothèque d'abord), tu l'écoutes tout de suite, et tu la gardes seulement si tu veux.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        ImportField(
            value = state.link,
            onValueChange = onLink,
            placeholder = "https://open.spotify.com/playlist/…",
        )
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            NzButton(
                text = if (state.starting) "…" else "Importer",
                onClick = onSubmit,
                glyph = NzGlyph.Link,
                enabled = state.link.isNotBlank() && !state.starting,
            )
            if (state.link.isNotEmpty()) {
                NzButton(text = "Effacer", onClick = onClear, kind = NzButtonKind.Ghost)
            }
        }
        if (state.failure != null) {
            Text(state.failure, style = Sillon.type.bodySmall, color = colors.hrt)
        }
        state.detected?.let { label ->
            Text("Lien $label reconnu.", style = Sillon.type.labelSmall, color = colors.dim)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            listOf("Deezer", "Spotify", "YouTube", "Apple Music").forEach { source ->
                NzChip(text = source, selected = false, onClick = {})
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ImportBody(
    state: ImportModel.State,
    onPlay: () -> Unit,
    onShuffle: () -> Unit,
    onKeep: () -> Unit,
    onTitle: (String) -> Unit,
    onSave: () -> Unit,
    onCancelKeep: () -> Unit,
    onAnother: () -> Unit,
    onSearch: (String) -> Unit,
    onOpen: (String) -> Unit,
) {
    val colors = Sillon.colors
    val job = state.job ?: return
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(NzSpacing.l),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        item(key = "head") {
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                Text(
                    text = job.sourceLabel.ifEmpty { "Import" },
                    style = Sillon.type.mono,
                    color = colors.dim,
                )
                Text(
                    text = job.name.ifEmpty { "Lecture de la playlist…" },
                    style = Sillon.type.title,
                    color = colors.tx,
                )
                if (job.running) {
                    Text(
                        text = if (job.state == "reading") {
                            "Lecture de la playlist…"
                        } else {
                            "Recherche des titres · ${job.done} / ${job.total} · ${job.matched} trouvés"
                        },
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                } else {
                    Text(
                        text = "${job.percent} % · ${job.matched} titres retrouvés sur ${job.total}",
                        style = Sillon.type.bodySmall,
                        color = colors.dim,
                    )
                }
                // The row WRAPS: at 360 dp the three labelled buttons are wider
                // than the page, and a plain `Row` squeezed the third to 111.5 dp,
                // so « Enregistrer » was ellipsized to « Enregi… » (W-D2's native
                // half — measured: the button stays on screen and its LABEL is
                // cut, `NzButton` ellipsizes). `FlowRow` is the repo's own answer
                // to this class of overflow (`AdminParts.kt`'s actions), not a
                // second line of `Row`s.
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                    verticalArrangement = Arrangement.spacedBy(NzSpacing.xs),
                ) {
                    NzButton(text = "Lire", onClick = onPlay, glyph = NzGlyph.Play, enabled = job.percent > 0u)
                    NzButton(
                        text = "Mélanger",
                        onClick = onShuffle,
                        glyph = NzGlyph.Shuffle,
                        kind = NzButtonKind.Secondary,
                    )
                    if (job.state == "saved" && job.playlistId != null) {
                        NzButton(
                            text = "Ouvrir",
                            onClick = { job.playlistId?.let(onOpen) },
                            glyph = NzGlyph.Check,
                        )
                    } else if (job.state == "done" && !state.keeping) {
                        NzButton(text = "Enregistrer", onClick = onKeep, glyph = NzGlyph.Plus, kind = NzButtonKind.Secondary)
                    }
                }
                if (state.keeping) {
                    ImportField(value = state.title, onValueChange = onTitle, placeholder = "Nom de la playlist")
                    Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                        NzButton(text = "Enregistrer", onClick = onSave, enabled = state.title.isNotBlank())
                        NzButton(text = "Annuler", onClick = onCancelKeep, kind = NzButtonKind.Ghost)
                    }
                }
                if (job.state == "saving") {
                    Text("Enregistrement…", style = Sillon.type.bodySmall, color = colors.dim)
                }
                job.error?.let {
                    Text(it, style = Sillon.type.bodySmall, color = colors.hrt)
                }
                if (job.truncated) {
                    Text(
                        "Seuls les ${job.items.size} premiers titres de la playlist ont pu être lus.",
                        style = Sillon.type.labelSmall,
                        color = colors.dim,
                    )
                }
                CountsLine(job.counts)
                NzButton(
                    text = "Un autre lien",
                    onClick = onAnother,
                    kind = NzButtonKind.Ghost,
                    glyph = NzGlyph.Link,
                )
            }
        }
        val matched = job.items.filter { it.track != null }
        items(matched, key = { item -> "ok-${item.track?.key ?: item.line.title}" }) { item ->
            ImportRow(item, streamable = true, onSearch = onSearch)
        }
        val unsure = matched.filter { !it.sure }
        if (unsure.isNotEmpty()) {
            item(key = "unsure-title") {
                Text(
                    "À vérifier · ${unsure.size}",
                    style = Sillon.type.titleSmall,
                    color = Sillon.colors.tx,
                    modifier = Modifier.padding(top = NzSpacing.m),
                )
            }
            items(unsure, key = { "unsure-${it.track?.key}" }) { item ->
                ImportRow(item, streamable = true, onSearch = onSearch, unsure = true)
            }
        }
        val missing = job.items.filter { it.track == null }
        if (missing.isNotEmpty()) {
            item(key = "missing-title") {
                Text(
                    "Introuvables · ${missing.size}",
                    style = Sillon.type.titleSmall,
                    color = Sillon.colors.tx,
                    modifier = Modifier.padding(top = NzSpacing.m),
                )
                Text(
                    "Aucune version sûre de ces titres. Touchez-en un pour le chercher vous-même.",
                    style = Sillon.type.labelSmall,
                    color = Sillon.colors.dim,
                )
            }
            items(missing, key = { "missing-${it.line.title}-${it.line.artists.joinToString()}" }) { item ->
                ImportRow(item, streamable = false, onSearch = onSearch)
            }
        }
    }
}

@Composable
private fun CountsLine(counts: List<PlaylistImportCount>) {
    if (counts.isEmpty()) return
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        counts.forEach { count ->
            NzChip(text = "${count.count} ${viaLabel(count.via)}", selected = false, onClick = {})
        }
    }
}

/** « dans ta bibliothèque » / « sur Deezer » (`Import.svelte#viaText`). */
internal fun viaLabel(via: String): String = when (via) {
    "" -> "introuvable"
    "local" -> "dans ta bibliothèque"
    "deezer" -> "sur Deezer"
    "youtube" -> "sur YouTube"
    else -> "sur $via"
}

@Composable
private fun ImportRow(
    item: PlaylistImportItem,
    streamable: Boolean,
    onSearch: (String) -> Unit,
    unsure: Boolean = false,
) {
    val colors = Sillon.colors
    val track = item.track
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .padding(vertical = NzSpacing.xxs),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        if (track != null) {
            CoverImage(url = track.cover, sizeDp = 40.dp, seed = track.id)
        } else {
            Icon(glyph = NzGlyph.Search, size = 20.dp, tint = colors.dim, contentDescription = null)
        }
        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs)) {
            Text(
                item.line.title,
                style = Sillon.type.body,
                color = colors.tx,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            val note = when {
                track == null -> item.line.artists.joinToString(", ")
                unsure -> "${track.title} · ${viaLabel(item.via)} · à vérifier"
                else -> track.artists
            }
            Text(
                note,
                style = Sillon.type.bodySmall,
                color = colors.dim,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (track != null && streamable) {
            Text(nzDuration(track.durationS.toInt()), style = Sillon.type.mono, color = colors.dim)
        } else {
            val query = listOfNotNull(item.line.artists.firstOrNull(), item.line.title)
                .filter { it.isNotBlank() }
                .joinToString(" ")
            NzButton(
                text = "Chercher",
                onClick = { onSearch(query) },
                kind = NzButtonKind.Ghost,
                glyph = NzGlyph.Search,
            )
        }
    }
}

@Composable
private fun ImportField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
) {
    androidx.compose.material3.OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        placeholder = { Text(placeholder, style = Sillon.type.bodySmall) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(
            keyboardType = KeyboardType.Uri,
            imeAction = ImeAction.Go,
        ),
        modifier = Modifier.fillMaxWidth(),
        shape = androidx.compose.foundation.shape.RoundedCornerShape(fr.neezer.mobile.designsystem.NzRadius.chip),
    )
}

/**
 * The import's state machine (lot MB9).
 *
 * **The job is the server's**: this starts it, polls it every 700 ms
 * (`Import.svelte#POLL_MS`), and stops as soon as its state is final — so a
 * 1 000-line import costs no more than its own progress. The item list is only
 * there once the read is complete (the server's own rule), and the polls during
 * a save carry none, so the list on screen is kept rather than re-rendered.
 */
class ImportModel(
    private val service: PlaylistService,
    private val player: PlaylistPlayer,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { fr.neezer.mobile.designsystem.NzToasts.show(it) },
) {
    data class State(
        val link: String = "",
        val job: PlaylistImportStatus? = null,
        val failure: String? = null,
        val starting: Boolean = false,
        val keeping: Boolean = false,
        val title: String = "",
    ) {
        /** The source a recognised link names, when it is one (`linkimport.js`). */
        val detected: String? get() = detectSource(link)
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var pollJob: Job? = null

    /** A link handed over by the shell (the search box, an app link): start at once. */
    fun start(url: String?) {
        val text = url?.trim().orEmpty()
        if (text.isEmpty()) return
        if (text == _state.value.link && (_state.value.job != null || _state.value.starting)) return
        _state.update { it.copy(link = text) }
        submit()
    }

    fun onLink(text: String) = _state.update { it.copy(link = text, failure = null) }

    fun clear() {
        pollJob?.cancel()
        _state.value = State()
    }

    /** « Importer »: the server reads and matches it as a job. */
    fun submit() {
        val state = _state.value
        val url = state.link.trim()
        if (url.isEmpty() || state.starting) return
        _state.update { it.copy(starting = true, failure = null, job = null, keeping = false) }
        scope.launch {
            try {
                val job = withContext(io) { service.importStart(url) }
                _state.update {
                    it.copy(
                        starting = false,
                        job = pending(job, url),
                    )
                }
                poll(job)
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(starting = false, failure = NzErrors.text(e)) }
            }
        }
    }

    private fun poll(job: String) {
        pollJob?.cancel()
        pollJob = scope.launch {
            while (true) {
                val answer = runCatching { withContext(io) { service.importStatus(job) } }
                val status = answer.getOrNull()
                if (status == null) {
                    val cause = answer.exceptionOrNull()
                    val expired = cause is NzException.Server && cause.status.toInt() == 404
                    if (expired) {
                        // The server restarted, or the import sat unsaved for
                        // hours: the SPA says so and asks for another link.
                        _state.update {
                            it.copy(job = null, failure = "Cet import a expiré : relancez-le.")
                        }
                        return@launch
                    }
                    delay(POLL_MS * 3)
                    continue
                }
                _state.update {
                    it.copy(
                        job = status,
                        failure = null,
                        // A save's polls carry no item list: keep the one on screen.
                        keeping = it.keeping || status.state == "saving",
                    )
                }
                if (!status.running) {
                    if (status.state == "saved") onSaved(status)
                    return@launch
                }
                delay(POLL_MS)
            }
        }
    }

    fun openKeep() {
        val job = _state.value.job ?: return
        _state.update { it.copy(keeping = true, title = job.name) }
    }

    fun cancelKeep() = _state.update { it.copy(keeping = false) }

    fun onTitle(text: String) = _state.update { it.copy(title = text) }

    /** « Enregistrer »: the matched lines become a local playlist, on the server. */
    fun save() {
        val state = _state.value
        val job = state.job ?: return
        val title = state.title.trim()
        if (title.isEmpty()) return
        scope.launch {
            try {
                val saved = withContext(io) { service.importSave(job.id, title, emptyList()) }
                _state.update { it.copy(job = saved, keeping = false) }
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Impossible d'enregistrer la playlist : ${NzErrors.text(e)}")
            }
        }
    }

    private fun onSaved(status: PlaylistImportStatus) {
        toast("Playlist enregistrée dans ta bibliothèque")
        _state.update { it.copy(job = status, keeping = false) }
    }

    fun playAll() {
        val job = _state.value.job ?: return
        val rows = job.items.mapNotNull { it.track }
        if (rows.isEmpty()) return
        player.play(rows, "import", job.id)
    }

    fun shufflePlay() {
        val job = _state.value.job ?: return
        val rows = job.items.mapNotNull { it.track }
        if (rows.isEmpty()) return
        player.shuffle(rows, "import", job.id)
    }

    /** The job the sheet draws before the first poll lands. */
    private fun pending(job: String, url: String): PlaylistImportStatus = PlaylistImportStatus(
        id = job,
        state = "reading",
        source = "",
        sourceLabel = "",
        kind = "",
        url = url,
        name = "",
        cover = null,
        total = 0u,
        truncated = false,
        done = 0u,
        matched = 0u,
        error = null,
        playlistId = null,
        counts = emptyList(),
        items = emptyList(),
        running = true,
        percent = 0u,
    )

    companion object {
        /** `Import.svelte#POLL_MS`. */
        const val POLL_MS = 700L
    }
}

/**
 * The service a pasted link names, when it names one (`lib/linkimport.js`): the
 * same four sources, recognised the same way, so the box can say « Lien Spotify
 * reconnu » before anything is sent.
 */
internal fun detectSource(text: String): String? {
    val url = text.trim().lowercase()
    return when {
        url.contains("open.spotify.com/playlist/") -> "Spotify"
        url.contains("deezer.com/") && url.contains("/playlist/") -> "Deezer"
        url.contains("music.youtube.com/playlist") || url.contains("youtube.com/playlist") -> "YouTube"
        url.contains("music.apple.com/") && url.contains("/playlist/") -> "Apple Music"
        else -> null
    }
}

/** Opens a capability URL in the browser (the public page's own player). */
fun openStream(context: Context, url: String) {
    val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    runCatching { context.startActivity(intent) }
}

/** The tracks of an import, for the queue: kept here so both paths agree. */
internal fun PlaylistImportStatus.tracks(): List<TrackRow> = items.mapNotNull { it.track }

/** A row of the import that was matched but not surely. */
internal fun PlaylistImportItem.isUnsure(): Boolean = track != null && !sure

/** LocalContext's own reader for the shared page (kept out of the composable). */
fun Context.playShared(url: String) = openStream(this, url)
