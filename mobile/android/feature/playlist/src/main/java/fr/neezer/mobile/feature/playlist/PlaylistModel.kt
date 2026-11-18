package fr.neezer.mobile.feature.playlist

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PlaylistConflict
import uniffi.nz_ffi.PlaylistExportFormat
import uniffi.nz_ffi.PlaylistExportJob
import uniffi.nz_ffi.PlaylistMember
import uniffi.nz_ffi.PlaylistMembersWrite
import uniffi.nz_ffi.PlaylistRole
import uniffi.nz_ffi.PlaylistService as PlaylistServiceRow
import uniffi.nz_ffi.PlaylistShareLink
import uniffi.nz_ffi.TrackRow

/** The export sheet's four tabs (`ExportSheet.svelte`'s `mode`). */
enum class ExportTab { Service, Audio, List, Link }

/** Where the recommendations stand on screen. */
sealed interface RecsUi {
    data object Idle : RecsUi
    data object Loading : RecsUi
    data class Ready(val tracks: List<TrackRow>) : RecsUi
    data object Failed : RecsUi
}

/**
 * The playlist screen's state machine (lot MB9), `routes/Playlist.svelte` ported.
 *
 * Every rule the SPA taught is here, in the order the person meets it, and the
 * whole thing is plain Kotlin over [PlaylistService], [PlaylistPlayer] and
 * [PlaylistSharer] — `PlaylistModelTest` proves each one on the JVM with fakes, on
 * a VIRTUAL clock (no `sleep`, worker rule 6).
 *
 * - **The kept page paints first, the fresh one replaces it** (`screenFlow`): a
 *   4 000-row playlist opened before is on screen before a socket opens; a dead
 *   link keeps it and says « Hors ligne · version mémorisée »; with nothing kept,
 *   the cause and « Réessayer » (rule 3).
 * - **ONE request for the page**, then the recommendations as their own beat, so
 *   the list never waits for them.
 * - **Every write is optimistic and serialized**: a reorder moves in the frame
 *   (`move`, no request) and its `PUT` is coalesced into ONE after 400 ms; a
 *   removal and an append touch the list at once and send one request; a failure
 *   puts the list back (the core's own undo) and offers « Réessayer ».
 * - **A `409` is never replayed**: the answer carries the server's page and which
 *   half moved, the screen says so, and the person drags again from the truth.
 * - **The sort and the search project the list in Rust** (`appcore`'s fold), and
 *   the drag is only offered in the natural order — the SPA's rule.
 */
class PlaylistModel(
    private val service: PlaylistService,
    private val player: PlaylistPlayer,
    private val sharer: PlaylistSharer,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String, (() -> Unit)?) -> Unit = { text, retry ->
        NzToasts.show(text, if (retry != null) "Réessayer" else null, retry)
    },
) {
    data class State(
        val id: String = "",
        /** The page as the server (or the device's copy) last answered it. */
        val page: PlaylistPage? = null,
        /** The rows the list draws — replaced by every write, never mutated. */
        val rows: PlaylistRows? = null,
        /** Bumped whenever [rows] changed: the window re-reads its pages. */
        val wake: Long = 0,
        /** What is drawn is the copy this device kept (offline, or a refresh failed). */
        val stale: Boolean = false,
        val loading: Boolean = false,
        /** The cause of a read that failed with nothing to draw (« Réessayer »). */
        val failure: String? = null,
        /** The access went away under the page: the screen leaves the playlist. */
        val gone: Boolean = false,
        /** An optimistic rename, shown before the request lands. */
        val title: String? = null,
        val description: String? = null,
        /** Edit mode: drag handles, up/down, remove (natural order only). */
        val editing: Boolean = false,
        val editingMeta: Boolean = false,
        val draftTitle: String = "",
        val draftDescription: String = "",
        val query: String = "",
        val sort: PlaylistSort = PlaylistSort.Position,
        val descending: Boolean = false,
        val recs: RecsUi = RecsUi.Idle,
        /** Recommendation keys whose own add is in flight. */
        val pendingRecs: Set<String> = emptySet(),
        val recsBusy: Boolean = false,
        val members: MembersUi? = null,
        val export: ExportUi? = null,
        /** « Ajouter des titres »: the search box and its hits (`AddTracksSheet`). */
        val add: AddState? = null,
        /** The ⋯ sheet: exporter, collaborateurs, renommer, supprimer, quitter. */
        val more: Boolean = false,
        /** The one destructive act waiting for its confirmation. */
        val confirm: Confirm? = null,
    ) {
        val editable: Boolean get() = page?.editable == true
        val canManage: Boolean get() = page?.canManage == true
        val role: PlaylistRole get() = page?.role ?: PlaylistRole.VIEWER
        val collaborative: Boolean get() = page?.collaborative == true
        /** The sort/search that coexist with the drag-reorder edit mode. */
        val manualView: Boolean get() = sort == PlaylistSort.Position && query.isBlank()
        /** The number of rows the list draws NOW (after a filter). */
        val trackCount: Int get() = rows?.len() ?: 0
        val displayTitle: String get() = title ?: page?.meta?.title.orEmpty()
        val displayDescription: String get() = description ?: page?.meta?.description.orEmpty()
        val ownerName: String get() = page?.meta?.owner.orEmpty()
        /** « Modifié par … · aujourd'hui à 14:02 » — the screen formats the moment. */
        val lastEditedBy: String? get() = page?.meta?.lastEditedBy
        val lastEditedAt: String? get() = page?.meta?.lastEditedAt
        /** The public page's own fact: a viewer can read and play, nothing else. */
        val readOnlyNote: Boolean get() = page?.readOnly == true
    }

    /** One destructive act the screen confirms first. */
    enum class Confirm { Delete, Leave, RevokeLink }

    /** What the collaborators sheet holds (`PlaylistMembersSheet.svelte`). */
    data class MembersUi(
        val members: List<PlaylistMember>,
        val membersVersion: Int,
        val role: PlaylistRole,
        val canManage: Boolean,
        val admin: Boolean,
        val username: String = "",
        val newRole: PlaylistRole = PlaylistRole.EDITOR,
        /** `""`, `"invite"`, `"leave"`, or `"member:<id>"`. */
        val busy: String = "",
        val error: String? = null,
        /** The caller's right went away: the sheet says so and closes. */
        val denied: Boolean = false,
    )

    /**
     * « Ajouter des titres » (`AddTracksSheet.svelte`): the library searched through
     * MB6's own `search`, and the rows the person picked. A playlist's page does
     * not carry the library, so the sheet asks for it — once per settled term.
     */
    data class AddState(
        val term: String = "",
        val rows: List<TrackRow> = emptyList(),
        val loading: Boolean = false,
        val failure: String? = null,
        /** The keys already in the playlist: they are shown, not addable. */
        val existing: Set<String> = emptySet(),
    )

    /** What the export sheet holds (`ExportSheet.svelte`). */
    data class ExportUi(
        val tab: ExportTab = ExportTab.Audio,
        val formats: List<PlaylistExportFormat> = emptyList(),
        val defaultFormat: String? = null,
        val fmt: String? = null,
        val formatsLoading: Boolean = true,
        val formatsFailure: String? = null,
        val services: List<PlaylistServiceRow>? = null,
        val servicesFailure: String? = null,
        val listFormats: List<PlaylistExportFormat> = DEFAULT_LIST_FORMATS,
        val listFmt: String = "csv",
        val service: String? = null,
        val name: String = "",
        val isPublic: Boolean = false,
        val job: PlaylistExportJob? = null,
        val sending: Boolean = false,
        val missingShown: Boolean = false,
        val link: PlaylistShareLink? = null,
        val linkLoading: Boolean = false,
        val linkBusy: Boolean = false,
        val linkFailure: String? = null,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** One writer, in the person's own order: two index-based writes never race. */
    private val writes = Channel<suspend () -> Unit>(Channel.UNLIMITED)

    private var loadJob: Job? = null
    private var orderJob: Job? = null
    private var recsJob: Job? = null
    private var pollJob: Job? = null
    private var addJob: Job? = null

    init {
        scope.launch {
            for (write in writes) write()
        }
    }

    /** The screen opened on [id]. */
    fun start(id: String) {
        if (id.isEmpty()) return
        if (id == _state.value.id && _state.value.page != null) return
        _state.value = State(id = id)
        load()
    }

    /** « Réessayer » on a read that failed with nothing to draw. */
    fun retry() = load()

    /** A refresh (the stale pill, a pull): the kept copy stays until the answer. */
    fun refresh() = load()

    private fun load() {
        val id = _state.value.id
        if (id.isEmpty()) return
        loadJob?.cancel()
        loadJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedPlaylist(id) } },
                fresh = { withContext(io) { service.playlist(id) } },
            ).collect { answer ->
                when (answer) {
                    is ScreenState.Loading -> _state.update {
                        it.copy(loading = it.page == null, failure = null)
                    }

                    is ScreenState.Ready -> {
                        paint(answer.data, stale = answer.stale)
                        if (!answer.stale) loadRecommendations()
                    }

                    is ScreenState.Failed -> _state.update {
                        it.copy(loading = false, failure = answer.cause, stale = false)
                    }
                }
            }
        }
    }

    /** One page on screen; the rows the list draws come with it. */
    private fun paint(page: PlaylistPage, stale: Boolean) {
        _state.update {
            it.copy(
                page = page,
                rows = page.rows,
                wake = it.wake + 1,
                stale = stale,
                loading = false,
                failure = null,
                title = null,
                description = null,
                editing = it.editing && page.editable,
            )
        }
        applyProjection()
    }

    // -- the band ---------------------------------------------------------------

    fun openMeta() {
        val state = _state.value
        if (!state.canManage) return
        _state.update {
            it.copy(
                editingMeta = true,
                draftTitle = it.displayTitle,
                draftDescription = it.displayDescription,
            )
        }
    }

    fun cancelMeta() = _state.update { it.copy(editingMeta = false) }

    fun onTitle(text: String) = _state.update { it.copy(draftTitle = text) }

    fun onDescription(text: String) = _state.update { it.copy(draftDescription = text) }

    /** « Enregistrer »: the band changes at once, one `PATCH` follows. */
    fun saveMeta() {
        val state = _state.value
        if (!state.canManage) return
        val title = state.draftTitle.trim()
        if (title.isEmpty()) return
        val description = state.draftDescription.trim()
        val previousTitle = state.displayTitle
        val previousDescription = state.displayDescription
        _state.update {
            it.copy(editingMeta = false, title = title, description = description)
        }
        enqueue {
            try {
                val write = withContext(io) { service.edit(state.id, title, description) }
                adopt(write.page)
                if (write.conflict != null) conflicted(write.conflict)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(title = previousTitle, description = previousDescription)
                }
                toast("Le nom n'a pas été enregistré : ${NzErrors.text(e)}") { saveMeta() }
            }
        }
    }

    // -- the list ---------------------------------------------------------------

    fun onQuery(text: String) {
        _state.update {
            // A search leaves the drag mode: its rows are a projection, and an
            // index in a projection is not an index the server knows.
            it.copy(query = text, editing = it.editing && text.isBlank() && it.sort == PlaylistSort.Position)
        }
        applyProjection()
    }

    fun chooseSort(sort: PlaylistSort) {
        _state.update {
            it.copy(
                sort = sort,
                editing = it.editing && sort == PlaylistSort.Position && it.query.isBlank(),
            )
        }
        applyProjection()
    }

    fun toggleDirection() {
        _state.update { it.copy(descending = !it.descending) }
        applyProjection()
    }

    /** « Modifier » / « Terminé » — the drag mode, only in the natural order. */
    fun toggleEdit() {
        val state = _state.value
        if (!state.editable) return
        val next = !state.editing
        _state.update { it.copy(editing = next) }
        if (!next) flushOrder()
    }

    /** A drag (or an up/down tap): the rows move NOW, the `PUT` waits 400 ms. */
    fun move(from: Int, to: Int) {
        val state = _state.value
        if (!state.manualView || !state.editable || from == to) return
        scope.launch {
            val order = runCatching { withContext(io) { service.move(state.id, from, to) } }
                .getOrNull() ?: return@launch
            _state.update { it.copy(rows = order.rows, wake = it.wake + 1) }
            armOrderSave()
        }
    }

    /** Sends the pending order now (leaving edit mode, closing the screen). */
    fun flushOrder() {
        orderJob?.cancel()
        orderJob = null
        enqueue { saveOrder() }
    }

    private fun armOrderSave() {
        orderJob?.cancel()
        orderJob = scope.launch {
            delay(ORDER_DEBOUNCE_MS)
            enqueue { saveOrder() }
        }
    }

    private suspend fun saveOrder() {
        val state = _state.value
        if (state.id.isEmpty() || state.rows == null) return
        try {
            val answer = withContext(io) { service.saveOrder(state.id) }
            _state.update {
                it.copy(
                    rows = answer.rows,
                    wake = it.wake + 1,
                    page = it.page?.let { page ->
                        page.copy(meta = page.meta.copy(version = answer.version.toUInt()))
                    },
                )
            }
            if (answer.conflict != null) conflicted(answer.conflict)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            // The core never replays a reorder: it answers the order it holds, and
            // the person drags again from the truth.
            refreshRows()
            toast("L'ordre n'a pas été enregistré : ${NzErrors.text(e)}") { flushOrder() }
        }
    }

    /** « Retirer »: the row leaves at once, one `DELETE` follows. */
    fun removeAt(index: Int) {
        val state = _state.value
        if (!state.editable || index < 0) return
        scope.launch {
            val order = runCatching { withContext(io) { service.cut(state.id, listOf(index)) } }
                .getOrNull() ?: return@launch
            _state.update { it.copy(rows = order.rows, wake = it.wake + 1) }
            enqueue { saveCut(index) }
        }
    }

    private suspend fun saveCut(index: Int) {
        val id = _state.value.id
        try {
            val write = withContext(io) { service.saveCut(id, listOf(index)) }
            adopt(write.page)
            if (write.conflict != null) conflicted(write.conflict)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            refreshRows()
            toast("Le titre n'a pas pu être retiré : ${NzErrors.text(e)}") { removeAt(index) }
        }
    }

    /** Appends the rows the sheet (or a suggestion) handed over; one request. */
    fun addTracks(tracks: List<TrackRow>) {
        val state = _state.value
        if (!state.editable || tracks.isEmpty()) return
        scope.launch {
            val order = runCatching { withContext(io) { service.append(state.id, tracks) } }
                .getOrNull()
            if (order == null) {
                toast("L'ajout a échoué.") { addTracks(tracks) }
                return@launch
            }
            _state.update { it.copy(rows = order.rows, wake = it.wake + 1) }
            dropRecommendations()
            enqueue {
                try {
                    val write = withContext(io) { service.saveAppend(state.id) }
                    adopt(write.page)
                    if (write.conflict != null) {
                        conflicted(write.conflict)
                    } else {
                        toast(
                            if (tracks.size > 1) "${tracks.size} titres ajoutés à la playlist"
                            else "Titre ajouté à la playlist",
                            null,
                        )
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    refreshRows()
                    toast("L'ajout a échoué : ${NzErrors.text(e)}") { addTracks(tracks) }
                }
            }
        }
    }

    // -- the recommendations ----------------------------------------------------

    private fun loadRecommendations() {
        val state = _state.value
        if (!state.editable) return
        if (state.recs is RecsUi.Ready || state.recs is RecsUi.Loading) return
        recsJob?.cancel()
        _state.update { it.copy(recs = RecsUi.Loading) }
        recsJob = scope.launch {
            try {
                val answer = withContext(io) { service.recommendations(state.id, RECOMMENDATIONS) }
                // The suggestions must never repeat a row the playlist holds
                // (`Playlist.svelte` filters on `existingIds`).
                val rows = _state.value.rows
                val fresh = if (rows == null) {
                    answer.tracks
                } else {
                    withContext(io) { answer.tracks.filter { rows.indexOfKey(it.key) < 0 } }
                }
                _state.update { it.copy(recs = RecsUi.Ready(fresh)) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // Its own line with « Réessayer »: the list above stays (rule 3).
                _state.update { it.copy(recs = RecsUi.Failed) }
            }
        }
    }

    fun retryRecommendations() {
        _state.update { it.copy(recs = RecsUi.Idle) }
        loadRecommendations()
    }

    /** A suggestion's own `+`: the row leaves the section at once. */
    fun addRecommendation(track: TrackRow) {
        val state = _state.value
        if (state.recsBusy || track.key in state.pendingRecs) return
        val recs = state.recs
        _state.update {
            it.copy(
                pendingRecs = it.pendingRecs + track.key,
                recs = when (recs) {
                    is RecsUi.Ready -> RecsUi.Ready(recs.tracks.filterNot { row -> row.key == track.key })
                    else -> recs
                },
            )
        }
        scope.launch {
            val order = runCatching { withContext(io) { service.append(state.id, listOf(track)) } }
                .getOrNull()
            _state.update { it.copy(pendingRecs = it.pendingRecs - track.key) }
            if (order == null) {
                // The row comes back where it was, and the section says why.
                _state.update { it.copy(recs = recs) }
                toast("L'ajout a échoué.") { addRecommendation(track) }
                return@launch
            }
            _state.update { it.copy(rows = order.rows, wake = it.wake + 1) }
            enqueue {
                try {
                    val write = withContext(io) { service.saveAppend(state.id) }
                    adopt(write.page)
                    if (write.conflict != null) {
                        conflicted(write.conflict)
                    } else {
                        toast("« ${track.title} » ajouté à la playlist", null)
                    }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    refreshRows()
                    retryRecommendations()
                    toast("L'ajout a échoué : ${NzErrors.text(e)}") { addRecommendation(track) }
                }
            }
        }
    }

    /** « Tout ajouter »: the visible suggestions, in their order, ONE request. */
    fun addAllRecommendations() {
        val state = _state.value
        val recs = state.recs
        if (state.recsBusy || recs !is RecsUi.Ready || recs.tracks.isEmpty()) return
        val batch = recs.tracks
        _state.update { it.copy(recsBusy = true) }
        scope.launch {
            val order = runCatching { withContext(io) { service.append(state.id, batch) } }
                .getOrNull()
            _state.update { it.copy(recsBusy = false) }
            if (order == null) {
                toast("L'ajout des suggestions a échoué.") { addAllRecommendations() }
                return@launch
            }
            _state.update {
                it.copy(
                    rows = order.rows,
                    wake = it.wake + 1,
                    recs = RecsUi.Ready(emptyList()),
                )
            }
            enqueue {
                try {
                    val write = withContext(io) { service.saveAppend(state.id) }
                    adopt(write.page)
                    toast(
                        if (batch.size > 1) "${batch.size} titres ajoutés à la playlist"
                        else "Titre ajouté à la playlist",
                        null,
                    )
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    refreshRows()
                    retryRecommendations()
                    toast("L'ajout des suggestions a échoué : ${NzErrors.text(e)}") { addAllRecommendations() }
                }
            }
        }
    }

    private fun dropRecommendations() = _state.update { it.copy(recs = RecsUi.Idle) }

    // -- the collaborators ------------------------------------------------------

    fun openMore() = _state.update { it.copy(more = true) }

    fun closeMore() = _state.update { it.copy(more = false) }

    fun openMembers() {
        val state = _state.value
        val page = state.page ?: return
        _state.update {
            it.copy(
                members = MembersUi(
                    members = page.members,
                    membersVersion = page.meta.membersVersion.toInt(),
                    role = page.role,
                    canManage = page.canManage,
                    admin = page.admin,
                ),
            )
        }
    }

    fun closeMembers() = _state.update { it.copy(members = null) }

    fun onInviteName(text: String) = _state.update {
        it.copy(members = it.members?.copy(username = text, error = null))
    }

    fun onInviteRole(role: PlaylistRole) = _state.update {
        it.copy(members = it.members?.copy(newRole = role))
    }

    fun invite() {
        val state = _state.value
        val sheet = state.members ?: return
        val name = sheet.username.trim()
        if (name.isEmpty()) {
            _state.update { it.copy(members = sheet.copy(error = "Indiquez un nom d'utilisateur.")) }
            return
        }
        if (sheet.busy.isNotEmpty() || !sheet.canManage) return
        _state.update { it.copy(members = sheet.copy(busy = "invite", error = null)) }
        scope.launch {
            val answer = members { service.invite(state.id, name, sheet.newRole) }
            if (answer != null && !answer.denied) {
                _state.update {
                    it.copy(
                        members = it.members?.copy(
                            username = "",
                            newRole = PlaylistRole.EDITOR,
                        ),
                    )
                }
            }
        }
    }

    fun setMemberRole(member: PlaylistMember, role: PlaylistRole) {
        val state = _state.value
        val sheet = state.members ?: return
        if (sheet.busy.isNotEmpty() || !sheet.canManage) return
        _state.update { it.copy(members = sheet.copy(busy = "member:${member.id}", error = null)) }
        scope.launch { members { service.setRole(state.id, member.id, role) } }
    }

    fun removeMember(member: PlaylistMember) {
        val state = _state.value
        val sheet = state.members ?: return
        if (sheet.busy.isNotEmpty() || !sheet.canManage) return
        _state.update { it.copy(members = sheet.copy(busy = "member:${member.id}", error = null)) }
        scope.launch { members { service.removeMember(state.id, member.id) } }
    }

    /** « Quitter la playlist » — the owner cannot (the server answers `409`). */
    fun leave(confirmed: Boolean = false) {
        if (!confirmed) {
            _state.update { it.copy(confirm = Confirm.Leave) }
            return
        }
        val state = _state.value
        _state.update { it.copy(confirm = null) }
        scope.launch {
            try {
                withContext(io) { service.leave(state.id) }
                toast("Vous avez quitté la playlist", null)
                _state.update { it.copy(gone = true) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (e is NzException.Conflict) {
                    toast("Le propriétaire ne peut pas quitter sa playlist.", null)
                } else {
                    toast("Impossible de quitter la playlist : ${NzErrors.text(e)}") { leave(true) }
                }
            }
        }
    }

    /** « Supprimer la playlist » — confirmed first, then terminal. */
    fun delete(confirmed: Boolean = false) {
        if (!confirmed) {
            _state.update { it.copy(confirm = Confirm.Delete) }
            return
        }
        val state = _state.value
        _state.update { it.copy(confirm = null) }
        scope.launch {
            try {
                withContext(io) { service.delete(state.id) }
                _state.update { it.copy(gone = true) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (e is NzException.Server && e.status.toInt() == 404) {
                    // Already gone: the screen leaves as if it had worked.
                    _state.update { it.copy(gone = true) }
                } else {
                    toast("La playlist n'a pas été supprimée : ${NzErrors.text(e)}") { delete(true) }
                }
            }
        }
    }

    fun cancelConfirm() = _state.update { it.copy(confirm = null) }

    /** The confirmation's own word, for the dialog's title. */
    fun confirmTitle(): String = when (_state.value.confirm) {
        Confirm.Delete -> "Supprimer la playlist « ${_state.value.displayTitle} » ?"
        Confirm.Leave -> "Quitter la playlist « ${_state.value.displayTitle} » ?"
        Confirm.RevokeLink -> "Révoquer le lien public ?"
        null -> ""
    }

    // -- the export sheet -------------------------------------------------------

    fun openExport() {
        val state = _state.value
        val page = state.page ?: return
        val canLink = state.canManage
        _state.update {
            it.copy(
                export = ExportUi(
                    tab = ExportTab.Audio,
                    name = page.meta.title,
                    link = null,
                ),
            )
        }
        loadFormats()
        loadServices()
        if (canLink) loadLink()
    }

    fun closeExport() {
        pollJob?.cancel()
        pollJob = null
        _state.update { it.copy(export = null) }
    }

    fun chooseTab(tab: ExportTab) = _state.update { it.copy(export = it.export?.copy(tab = tab)) }

    fun chooseFormat(fmt: String) = _state.update { it.copy(export = it.export?.copy(fmt = fmt)) }

    fun chooseListFormat(fmt: String) = _state.update {
        it.copy(export = it.export?.copy(listFmt = fmt))
    }

    fun chooseService(id: String) = _state.update {
        it.copy(export = it.export?.copy(service = id))
    }

    fun onExportName(text: String) = _state.update {
        it.copy(export = it.export?.copy(name = text))
    }

    fun toggleExportPublic() = _state.update {
        val export = it.export ?: return@update it
        it.copy(export = export.copy(isPublic = !export.isPublic))
    }

    private fun loadFormats() {
        scope.launch {
            try {
                val answer = withContext(io) { service.exportFormats() }
                _state.update {
                    val export = it.export ?: return@update it
                    val chosen = export.fmt?.takeIf { fmt -> answer.formats.any { it.id == fmt } }
                        ?: answer.default.ifEmpty { answer.formats.firstOrNull()?.id }
                    it.copy(
                        export = export.copy(
                            formats = answer.formats,
                            defaultFormat = answer.default,
                            fmt = chosen,
                            formatsLoading = false,
                            formatsFailure = null,
                        ),
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(
                        export = it.export?.copy(
                            formatsLoading = false,
                            formatsFailure = NzErrors.text(e),
                        ),
                    )
                }
            }
        }
    }

    private fun loadServices() {
        scope.launch {
            try {
                val answer = withContext(io) { service.services() }
                _state.update {
                    val export = it.export ?: return@update it
                    val usable = answer.services.filter { row -> row.linked }
                    it.copy(
                        export = export.copy(
                            services = answer.services,
                            listFormats = if (answer.formats.isEmpty()) DEFAULT_LIST_FORMATS else answer.formats,
                            service = export.service?.takeIf { id -> usable.any { it.id == id } }
                                ?: usable.firstOrNull()?.id,
                            servicesFailure = null,
                        ),
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(export = it.export?.copy(servicesFailure = NzErrors.text(e)))
                }
            }
        }
    }

    /** « Exporter vers … » — one `POST`, then the job is followed every 800 ms. */
    fun sendExport() {
        val state = _state.value
        val export = state.export ?: return
        val serviceId = export.service ?: return
        if (export.sending || export.job?.running == true) return
        _state.update {
            it.copy(export = it.export?.copy(sending = true, job = null, missingShown = false))
        }
        scope.launch {
            try {
                val job = withContext(io) {
                    service.export(serviceId, "playlist", state.id, export.name.trim(), export.isPublic)
                }
                _state.update {
                    it.copy(export = it.export?.copy(sending = false, job = pendingJob(serviceId)))
                }
                followExport(job)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(export = it.export?.copy(sending = false)) }
                toast("L'export n'a pas pu démarrer : ${NzErrors.text(e)}") { sendExport() }
                // A `409` means the other service must be linked again.
                loadServices()
            }
        }
    }

    private fun followExport(job: String) {
        pollJob?.cancel()
        pollJob = scope.launch {
            while (true) {
                val answer = runCatching { withContext(io) { service.exportJob(job) } }.getOrNull()
                if (answer != null) {
                    _state.update { it.copy(export = it.export?.copy(job = answer)) }
                    if (!answer.running) {
                        if (answer.relink) loadServices()
                        return@launch
                    }
                }
                // One poll every 800 ms (the SPA's beat); a missed poll is not the
                // export's failure — the next one catches up.
                delay(POLL_MS)
            }
        }
    }

    fun resetExportJob() {
        pollJob?.cancel()
        _state.update { it.copy(export = it.export?.copy(job = null, missingShown = false)) }
    }

    fun toggleMissing() = _state.update {
        val export = it.export ?: return@update it
        it.copy(export = export.copy(missingShown = !export.missingShown))
    }

    /** « Télécharger la liste »: the CSV/XSPF text goes to the share sheet. */
    fun downloadList() {
        val state = _state.value
        val export = state.export ?: return
        scope.launch {
            try {
                val text = withContext(io) {
                    service.exportListFile("playlist", state.id, export.listFmt)
                }
                sharer.share(text, "${state.displayTitle}.${export.listFmt}")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("La liste n'a pas pu être exportée : ${NzErrors.text(e)}") { downloadList() }
            }
        }
    }

    fun copyMissing() {
        val job = _state.value.export?.job ?: return
        val text = job.missing.joinToString("\n") { missing ->
            listOf(missing.artist, missing.title).filter { it.isNotEmpty() }.joinToString(" - ")
        }
        sharer.copy(text, "Liste copiée")
    }

    private fun loadLink() {
        scope.launch {
            _state.update { it.copy(export = it.export?.copy(linkLoading = true)) }
            val link = runCatching { withContext(io) { service.shareLink(_state.value.id) } }
            _state.update {
                it.copy(
                    export = it.export?.copy(
                        link = link.getOrNull(),
                        linkLoading = false,
                        linkFailure = link.exceptionOrNull()?.let(NzErrors::text),
                    ),
                )
            }
        }
    }

    fun createLink() {
        val state = _state.value
        if (state.export?.linkBusy == true) return
        _state.update { it.copy(export = it.export?.copy(linkBusy = true, linkFailure = null)) }
        scope.launch {
            try {
                val link = withContext(io) { service.createShare(state.id) }
                _state.update { it.copy(export = it.export?.copy(link = link, linkBusy = false)) }
                toast("Lien public créé", null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update {
                    it.copy(
                        export = it.export?.copy(
                            linkBusy = false,
                            linkFailure = "Lien public indisponible : ${NzErrors.text(e)}",
                        ),
                    )
                }
            }
        }
    }

    /** The share sheet takes the whole URL, origin included. */
    fun shareLink() {
        val state = _state.value
        val link = state.export?.link ?: return
        val origin = runCatching {
            fr.neezer.mobile.bridge.NzBridge.core?.server()?.origin
        }.getOrNull()
        val url = if (origin.isNullOrEmpty()) link.url else origin + link.url
        sharer.share(url, state.displayTitle)
    }

    fun revokeLink(confirmed: Boolean = false) {
        if (!confirmed) {
            _state.update { it.copy(confirm = Confirm.RevokeLink) }
            return
        }
        val state = _state.value
        _state.update { it.copy(confirm = null, export = it.export?.copy(linkBusy = true)) }
        scope.launch {
            try {
                withContext(io) { service.revokeShare(state.id) }
                _state.update { it.copy(export = it.export?.copy(link = null, linkBusy = false)) }
                toast("Lien révoqué", null)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(export = it.export?.copy(linkBusy = false)) }
                toast("Le lien n'a pas été révoqué : ${NzErrors.text(e)}") { revokeLink(true) }
            }
        }
    }

    // -- the add-tracks sheet ---------------------------------------------------

    fun openAddTracks() {
        val rows = _state.value.rows
        val existing = if (rows == null) emptySet() else {
            buildSet {
                var start = 0
                while (start < rows.len()) {
                    val page = rows.source.window(start, 64)
                    if (page.isEmpty()) break
                    page.forEach { add(it.key) }
                    start += page.size
                }
            }
        }
        _state.update { it.copy(add = AddState(existing = existing)) }
        searchTracks("")
    }

    fun closeAddTracks() {
        addJob?.cancel()
        _state.update { it.copy(add = null) }
    }

    /** A key in the sheet's own box: one settled term, one request (MB6's search). */
    fun onAddTerm(text: String) {
        val add = _state.value.add ?: return
        _state.update { it.copy(add = add.copy(term = text)) }
        addJob?.cancel()
        addJob = scope.launch {
            delay(ADD_DEBOUNCE_MS)
            searchTracks(text)
        }
    }

    private fun searchTracks(term: String) {
        val add = _state.value.add ?: return
        if (term.isBlank()) {
            _state.update { it.copy(add = add.copy(rows = emptyList(), loading = false, failure = null)) }
            return
        }
        if (term.trim().length < 2) return
        addJob?.cancel()
        addJob = scope.launch {
            _state.update { current ->
                val sheet = current.add ?: return@update current
                current.copy(add = sheet.copy(loading = true, failure = null))
            }
            try {
                val rows = withContext(io) { service.searchTracks(term.trim()) }
                _state.update { current ->
                    val sheet = current.add ?: return@update current
                    if (sheet.term.trim() != term.trim()) return@update current
                    current.copy(add = sheet.copy(rows = rows, loading = false))
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { current ->
                    val sheet = current.add ?: return@update current
                    current.copy(add = sheet.copy(loading = false, failure = NzErrors.text(e)))
                }
            }
        }
    }

    /** The sheet's own « Ajouter »: the picked rows, then the sheet closes. */
    fun addChosen(rows: List<TrackRow>) {
        closeAddTracks()
        addTracks(rows)
    }

    // -- play -------------------------------------------------------------------

    fun playAll() {
        val state = _state.value
        val rows = state.rows ?: return
        val page = pageRows(rows)
        if (page.isEmpty()) return
        player.play(page, "playlist", state.id)
    }

    fun shufflePlay() {
        val state = _state.value
        val rows = state.rows ?: return
        val page = pageRows(rows)
        if (page.isEmpty()) return
        player.shuffle(page, "playlist", state.id)
    }

    /** The rows of the whole list, in pages: 4 000 rows cross 64 at a time. */
    private fun pageRows(rows: PlaylistRows): List<TrackRow> {
        val out = ArrayList<TrackRow>(rows.len())
        var start = 0
        while (start < rows.len()) {
            val page = rows.source.window(start, LIST_PAGE)
            if (page.isEmpty()) break
            out += page
            start += page.size
        }
        return out
    }

    // -- internals --------------------------------------------------------------

    /** A fresh page from a write: its rows replace the ones on screen. */
    private fun adopt(page: PlaylistPage) {
        _state.update {
            it.copy(
                page = page,
                rows = page.rows,
                wake = it.wake + 1,
                title = null,
                description = null,
                editing = it.editing && page.editable,
            )
        }
        applyProjection()
    }

    /** A `409`: the state above is the server's, and the person is told. */
    private fun conflicted(scope: PlaylistConflict) {
        val what = if (scope == PlaylistConflict.MEMBERS) "les collaborateurs" else "la liste"
        toast("La playlist a changé entre-temps : $what ont été rechargés.", null)
    }

    /** Re-reads the rows the core holds now (`0 requests`), after a failed write. */
    private fun refreshRows() {
        val id = _state.value.id
        scope.launch {
            val rows = withContext(io) { service.rows(id) } ?: return@launch
            _state.update { it.copy(rows = rows, wake = it.wake + 1) }
        }
    }

    private fun applyProjection() {
        val state = _state.value
        val rows = state.rows ?: return
        val query = state.query.trim()
        val sort = state.sort
        val descending = state.descending
        scope.launch {
            withContext(io) {
                rows.filter(query)
                // The natural order in the natural direction IS the list: asking
                // for it would only cost a re-sort of 4 000 rows.
                if (sort != PlaylistSort.Position || descending) rows.sort(sort, descending)
            }
            _state.update { it.copy(wake = it.wake + 1) }
        }
    }

    private fun enqueue(write: suspend () -> Unit) {
        writes.trySend(write)
    }

    /** The membership calls' common tail: adopt the answer, or say why. */
    private suspend fun members(call: suspend () -> PlaylistMembersWrite): PlaylistMembersWrite? {
        return try {
            val answer = withContext(io) { call() }
            _state.update { current ->
                val sheet = current.members
                current.copy(
                    page = current.page?.let { page ->
                        page.copy(
                            role = answer.role,
                            editable = answer.canEdit,
                            canManage = answer.canManage,
                            admin = answer.admin,
                            members = answer.members,
                            meta = page.meta.copy(membersVersion = answer.membersVersion),
                        )
                    },
                    members = sheet?.copy(
                        members = answer.members,
                        membersVersion = answer.membersVersion.toInt(),
                        role = answer.role,
                        canManage = answer.canManage,
                        admin = answer.admin,
                        busy = "",
                        error = null,
                    ),
                    editing = current.editing && answer.canEdit,
                )
            }
            val race = answer.conflict
            if (race != null) conflicted(race)
            if (answer.denied) {
                _state.update {
                    it.copy(
                        members = it.members?.copy(
                            denied = true,
                            busy = "",
                            error = "Vous n'avez plus le droit de modifier cette playlist.",
                        ),
                    )
                }
            }
            answer
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            _state.update {
                it.copy(
                    members = it.members?.copy(busy = "", error = "Impossible : ${NzErrors.text(e)}"),
                )
            }
            null
        }
    }

    /** The job the sheet shows before its first poll lands (stage « match »). */
    private fun pendingJob(serviceId: String): PlaylistExportJob = PlaylistExportJob(
        service = serviceId,
        stage = "match",
        running = true,
        total = 0u,
        done = 0u,
        matched = 0u,
        missingCount = 0u,
        missing = emptyList(),
        url = null,
        error = null,
        relink = false,
        percent = 0u,
    )

    /** The screen is going away: the order in the drag is sent, the polls stop. */
    fun leaveScreen() {
        flushOrder()
        pollJob?.cancel()
        loadJob?.cancel()
        recsJob?.cancel()
        addJob?.cancel()
    }

    companion object {
        /** `Playlist.svelte`'s coalescing window for a burst of drags. */
        const val ORDER_DEBOUNCE_MS = 400L

        /** The export job's poll period (`ExportSheet.svelte`: 800 ms). */
        const val POLL_MS = 800L

        /** `Playlist.svelte#loadRecommendations`'s limit. */
        const val RECOMMENDATIONS = 12

        /** One page of rows when a whole list has to cross (the queue's window). */
        private const val LIST_PAGE = 64

        /** The add-tracks box's own beat (the same 250 ms a search waits). */
        const val ADD_DEBOUNCE_MS = 250L

        /** The list formats the sheet offers when the server names none. */
        val DEFAULT_LIST_FORMATS = listOf(
            PlaylistExportFormat("csv", "CSV (TuneMyMusic, Soundiiz…)", "csv"),
            PlaylistExportFormat("xspf", "XSPF (VLC, lecteurs)", "xspf"),
            PlaylistExportFormat("txt", "Texte « Artiste - Titre »", "txt"),
        )
    }
}

/** `ListHandle::index_of` is not on the port; a page sweep is enough here. */
internal fun PlaylistRows.indexOfKey(key: String): Int {
    var start = 0
    while (start < len()) {
        val page = source.window(start, 64)
        if (page.isEmpty()) return -1
        val at = page.indexOfFirst { it.key == key }
        if (at >= 0) return start + at
        start += page.size
    }
    return -1
}
