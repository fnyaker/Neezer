package fr.neezer.mobile.feature.trackactions

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.MenuAction
import uniffi.nz_ffi.MenuEntry
import uniffi.nz_ffi.MenuContext
import uniffi.nz_ffi.MenuItem
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PlaylistChoice
import uniffi.nz_ffi.ReplaceCandidate
import uniffi.nz_ffi.TrackRow

/**
 * The track menu's state machine (lot MB10), `webapp/src/lib/trackmenu.js` +
 * `lib/actions.js` + `PlaylistPicker.svelte` + `ReplaceSheet.svelte` PORTED.
 *
 * - **The menu is built by the core**, never here: `open()` asks
 *   `Screens::track_menu` once (0 requests, 0 sockets) and draws what it is
 *   handed. A like flips the row the builder reads, so the next opening shows
 *   "Retirer des favoris" without a second request.
 * - **A write answers first, then talks.** The heart and the thumb set the state
 *   before their request and roll back with a toast whose "Réessayer" runs the
 *   same call again — `webapp.md`'s rule and the desktop's.
 * - **The picker paints the copy, then asks** (`cachedPlaylists()` then
 *   `playlists()`): a warm opening costs 0 requests, a dead link keeps the list
 *   with « Hors ligne · version mémorisée », and no copy is a `LoadFailed` with
 *   the cause and "Réessayer".
 * - **A ticket is an action, not a stored membership** (the SPA's note): the box
 *   shows what THIS opening added, and ticking twice never adds twice.
 * - **The replacement sheet listens before it commits**, and the rewrite is the
 *   server's job: the sheet polls [`MutationReport`] and names the playlist and
 *   favourites counts, or the cause.
 */
class TrackActionsModel(
    private val service: TrackActionsService,
    private val player: TrackActionPlayer,
    private val nav: NzNavigator,
    private val sharer: TrackActionSharer,
    private val context: MenuContext,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String, String?, (() -> Unit)?) -> Unit = { text, label, run ->
        NzToasts.show(text, label, run)
    },
    /**
     * The French sentence of a failure. `NzErrors.text` is the app's ONE register
     * (`nz_error_text` in Rust), and it is a port here because it is the only call
     * of the FFI surface a JVM test cannot make: `nzErrorText` reads the native
     * library, and a test that fails a fake's promise must still see its toast.
     */
    private val causeText: (Throwable) -> String = { NzErrors.text(it) },
    /** Whether the shell installed the credits sheet's door (MB7's component). */
    private val creditsDoor: Boolean = false,
    private val closeHost: () -> Unit = {},
    /**
     * The row this opening is about. The menu is built in the constructor, so the
     * sheet's very first frame is the menu — not a title with no entries.
     */
    initialRow: TrackRow? = null,
) {
    /** Which sheet the one host is drawing. */
    enum class Sheet { Menu, Picker, Replace }

    /** The picker's own state (`PlaylistPicker.svelte`). */
    data class Picker(
        /** The writable lists, `null` while the first answer is on its way. */
        val lists: List<PlaylistChoice>? = null,
        /** What is shown is the device's copy (offline, or a refresh that failed). */
        val stale: Boolean = false,
        /** Why the list could not be read at all. */
        val error: String? = null,
        val query: String = "",
        /** The ids this opening added: the tick, and the "never twice" rule. */
        val added: Set<String> = emptySet(),
        val creating: Boolean = false,
        val name: String = "",
        val busy: Boolean = false,
    ) {
        /** `editableList`: a viewer's list is not a target. */
        val targets: List<PlaylistChoice>
            get() = (lists ?: emptyList()).filter { it.editable }

        /** `filterList` + the « récente » row pinned first. */
        val shown: List<PlaylistChoice>
            get() {
                var list = targets
                val term = query.trim().lowercase()
                if (term.isNotEmpty()) {
                    list = list.filter { it.title.lowercase().contains(term) }
                }
                val at = list.indexOfFirst { it.recent }
                if (at > 0) {
                    list = listOf(list[at]) + list.take(at) + list.drop(at + 1)
                }
                return list
            }

        /** `manyPlaylists`: the filter only appears past six lists. */
        val filterShown: Boolean get() = targets.size > 6

        /** `suggestName`: "Ma playlist 8" for eight playlists, never an existing name. */
        val suggestion: String
            get() {
                val used = (lists ?: emptyList()).map { it.title.trim().lowercase() }.toSet()
                var n = (lists?.size ?: 0) + 1
                while (used.contains("ma playlist $n")) n++
                return "Ma playlist $n"
            }
    }

    /** The replacement sheet's own state (`ReplaceSheet.svelte`). */
    data class Replace(
        val candidates: List<ReplaceCandidate> = emptyList(),
        val loading: Boolean = true,
        /** Why the candidates could not be read (the sheet's line, not a toast). */
        val error: String? = null,
        /** The query the server actually searched. */
        val query: String = "",
        /** The search box's text. */
        val searched: String = "",
        /** The fallback source that answered unprompted. */
        val fallbackAuto: String? = null,
        val fallbacksAsked: Boolean = false,
        val fallbacksLoading: Boolean = false,
        val fallbacksFailed: Boolean = false,
        val busy: Boolean = false,
        /** "Supprimer ce titre" asks once before it deletes. */
        val confirming: Boolean = false,
    )

    data class State(
        val row: TrackRow,
        val entries: List<MenuEntry> = emptyList(),
        val liked: Boolean = false,
        val disliked: Boolean = false,
        val sheet: Sheet = Sheet.Menu,
        val picker: Picker? = null,
        val replace: Replace? = null,
        /** The credits sheet's subject, when the shell installed its door. */
        val credits: TrackRow? = null,
        /** The host draws nothing once this is true. */
        val closed: Boolean = false,
        /** The open accordion's label, if any (`openSub` in the SPA). */
        val expanded: String? = null,
    )

    private val _state = MutableStateFlow(
        State(row = TrackRow(
            key = "",
            id = "",
            title = "",
            artists = "",
            album = "",
            albumId = null,
            durationS = 0u,
            cover = null,
            explicit = false,
            liked = false,
            disliked = false,
            unavailable = false,
            downloaded = false,
            source = "",
            position = 0u,
        )),
    )
    val state: StateFlow<State> = _state.asStateFlow()

    init {
        if (initialRow != null) open(initialRow)
    }

    private var pickerJob: Job? = null
    private var replaceJob: Job? = null
    private var jobWatch: Job? = null

    /** Whether the credits sheet's door exists on this build (MB7's component). */
    val hasCreditsDoor: Boolean get() = creditsDoor

    // -- opening --------------------------------------------------------------

    /**
     * Opens the menu of [row]: one call to the core, no request, no socket.
     *
     * The row is put in the state FIRST, so the context the builder reads (a
     * downloaded title, a queue index) is this row's and not the default one's.
     */
    fun open(row: TrackRow) {
        _state.value = State(row = row, liked = row.liked, disliked = row.disliked)
        val merged = mergedContext()
        val entries = runCatching { service.menu(row, merged) }.getOrDefault(emptyList())
        _state.update { it.copy(entries = entries) }
    }

    /**
     * The context the builder reads, with the platform's facts filled in (the
     * admin flag from the restored session, the last playlist used, and whether
     * the credits sheet's door exists). Public so a test asserts what a surface's
     * facts became, and so a later lot can print it.
     */
    fun mergedContext(): MenuContext {
        val actor = runCatching { service.actor() }.getOrNull()
        val admin = actor?.admin ?: context.admin
        val last = actor?.lastPlaylist
        return MenuContext(
            surface = context.surface,
            index = context.index,
            currentIndex = context.currentIndex,
            canKeep = context.canKeep,
            canBrowse = context.canBrowse,
            admin = admin,
            remote = context.remote,
            downloaded = context.downloaded || _state.value.row.downloaded,
            lastPlaylistId = last?.id ?: context.lastPlaylistId,
            lastPlaylistTitle = last?.title ?: context.lastPlaylistTitle,
            lastPlaylistEditable = last?.editable ?: context.lastPlaylistEditable,
            artists = context.artists,
            episode = context.episode,
            episodePlaying = context.episodePlaying,
            channelId = context.channelId,
            credits = context.credits && hasCreditsDoor,
            offline = context.offline,
            genreTag = context.genreTag,
            markers = context.markers,
        )
    }

    /** Closes the whole host (Escape, the veil, or an entry that did its job). */
    fun close() {
        pickerJob?.cancel()
        replaceJob?.cancel()
        jobWatch?.cancel()
        _state.update { it.copy(closed = true) }
        closeHost()
    }

    /** The accordion: one submenu open at a time, and a second tap closes it. */
    fun toggleSub(item: MenuItem) {
        if (item.sub.isEmpty()) return
        _state.update { state ->
            state.copy(expanded = if (state.expanded == item.label) null else item.label)
        }
    }

    // -- running an entry -----------------------------------------------------

    /**
     * Runs one entry. The menu closes first — the SPA's `run()` — and an entry
     * that opens a sheet reopens the host on that sheet.
     */
    fun run(action: MenuAction) {
        val row = _state.value.row
        when (action) {
            is MenuAction.PlayNext -> {
                player.playNext(row)
                toast("Lira ensuite « ${row.title} »", null, null)
                close()
            }
            is MenuAction.AddToQueue -> {
                player.addToQueue(row)
                toast("Ajouté à la file", null, null)
                close()
            }
            is MenuAction.RemoveFromQueue -> {
                val at = context.index?.toInt() ?: row.position.toInt()
                player.removeFromQueue(action.key, at)
                close()
            }
            is MenuAction.AddToLastPlaylist ->
                addToPlaylist(action.playlistId, action.title)
            is MenuAction.OpenPlaylistPicker -> openPicker()
            is MenuAction.OpenReplace -> openReplace()
            is MenuAction.Share -> share()
            is MenuAction.StartRadio -> {
                player.startRadio(row)
                close()
            }
            is MenuAction.Favorite -> setFavorite(action.on)
            is MenuAction.Dislike -> setDislike(action.on)
            is MenuAction.OpenCredits -> {
                if (creditsDoor) {
                    // The host draws MB7's sheet over this screen; the menu steps
                    // aside until it is dismissed (`TrackCreditsHost`'s contract).
                    _state.update { it.copy(credits = row) }
                } else {
                    // The gate hides this entry; reaching here means a surface
                    // passed its own context, and a silent no-op would be a bug.
                    toast("Les crédits ne sont pas encore disponibles ici", null, null)
                    close()
                }
            }
            is MenuAction.GoToArtist -> {
                close()
                nav.go(AppLink.Artist(action.id))
            }
            is MenuAction.GoToAlbum -> {
                close()
                nav.go(AppLink.Album(action.id))
            }
            is MenuAction.OpenPodcast -> {
                close()
                nav.go(AppLink.Podcast(action.id))
            }
            is MenuAction.Download, is MenuAction.Undownload,
            is MenuAction.MarkPosition, is MenuAction.TagGenre,
            -> {
                // Gated off until its lot lands (MB15, MB13, MB18): a button
                // that does nothing is a bug, so saying so is the honest path.
                toast("Cette action arrivera dans une prochaine version", null, null)
                close()
            }
        }
    }

    // -- the heart and the thumb ---------------------------------------------

    /** `lib/actions.js#toggleFavorite`: state first, one request, rollback + « Réessayer ». */
    fun setFavorite(on: Boolean) {
        val row = _state.value.row
        _state.update { it.copy(liked = on) }
        scope.launch {
            try {
                withContext(io) { if (on) service.like(row.key) else service.unlike(row.key) }
                toast(if (on) "Ajouté aux favoris" else "Retiré des favoris", null, null)
                close()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(liked = !on) }
                toast(
                    "Échec de la mise à jour du favori : ${causeText(e)}",
                    "Réessayer",
                ) { setFavorite(on) }
            }
        }
    }

    /**
     * `lib/actions.js#toggleDislike`. Optimistic, with "Annuler" in the toast: a
     * third title of one artist offers to stop recommending the artist — that
     * hint comes back from the server (`artist_hint`), which this boundary does
     * not carry yet, so the sentence is the plain one.
     */
    fun setDislike(on: Boolean) {
        val row = _state.value.row
        _state.update { it.copy(disliked = on) }
        scope.launch {
            try {
                withContext(io) { if (on) service.dislike(row.key) else service.undislike(row.key) }
                if (on) {
                    toast("Ce titre ne sera plus proposé", "Annuler") { setDislike(false) }
                } else {
                    toast("Ce titre peut de nouveau être proposé", null, null)
                }
                close()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(disliked = !on) }
                toast("Échec : ${causeText(e)}", "Réessayer") { setDislike(on) }
            }
        }
    }

    // -- the playlist picker --------------------------------------------------

    /** Opens the picker: the copy paints at once, the answer replaces it. */
    fun openPicker() {
        _state.update { it.copy(sheet = Sheet.Picker, picker = Picker()) }
        val cached = runCatching { service.cachedPlaylists() }.getOrNull()
        if (cached != null) {
            _state.update { state ->
                state.copy(picker = state.picker?.copy(lists = cached, stale = true))
            }
        }
        loadPicker()
    }

    /** `PlaylistPicker.svelte#loadList` — one request, or the copy kept with a cause. */
    fun loadPicker() {
        pickerJob?.cancel()
        _state.update { state ->
            state.copy(picker = (state.picker ?: Picker()).copy(error = null))
        }
        pickerJob = scope.launch {
            try {
                val lists = withContext(io) { service.playlists() }
                _state.update { state ->
                    state.copy(picker = state.picker?.copy(lists = lists, stale = false, error = null))
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val kept = _state.value.picker?.lists
                _state.update { state ->
                    state.copy(
                        picker = state.picker?.copy(
                            lists = kept,
                            stale = kept != null,
                            error = if (kept == null) causeText(e) else null,
                        ),
                    )
                }
            }
        }
    }

    fun filter(query: String) {
        _state.update { it.copy(picker = it.picker?.copy(query = query)) }
    }

    /** The create form's field, and its two doors. */
    fun startCreating() {
        _state.update { it.copy(picker = it.picker?.copy(creating = true, name = "")) }
    }

    fun cancelCreating() {
        _state.update { it.copy(picker = it.picker?.copy(creating = false, name = "")) }
    }

    fun namePlaylist(name: String) {
        _state.update { it.copy(picker = it.picker?.copy(name = name)) }
    }

    /** A tick is the action: it adds NOW, and a refusal unticks. */
    fun addToPlaylist(playlistId: String, title: String) {
        val picker = _state.value.picker
        if (picker == null || picker.busy || picker.added.contains(playlistId)) return
        val key = _state.value.row.key
        _state.update { state ->
            state.copy(picker = state.picker?.copy(added = state.picker.added + playlistId))
        }
        scope.launch {
            try {
                withContext(io) { service.addToPlaylist(playlistId, title, listOf(key)) }
                toast("Ajouté à « $title »", null, null)
                close()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { state ->
                    state.copy(picker = state.picker?.copy(added = state.picker.added - playlistId))
                }
                toast("Échec de l'ajout à « $title » : ${causeText(e)}", "Réessayer") {
                    addToPlaylist(playlistId, title)
                }
            }
        }
    }

    /**
     * `createPlaylistFrom(title, [trackRef])` — one request, and only the answer
     * closes the sheet (the SPA's own rule: the server decides the rows).
     */
    fun createPlaylist() {
        val picker = _state.value.picker ?: return
        if (picker.busy) return
        val name = (picker.name.trim().ifEmpty { picker.suggestion }).trim()
        if (name.isEmpty()) return
        val key = _state.value.row.key
        _state.update { state -> state.copy(picker = state.picker?.copy(busy = true)) }
        scope.launch {
            try {
                withContext(io) { service.createPlaylist(name, listOf(key)) }
                toast("Ajouté à « $name »", null, null)
                close()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { state -> state.copy(picker = state.picker?.copy(busy = false)) }
                toast("Impossible de créer « $name » : ${causeText(e)}", "Réessayer") {
                    createPlaylist()
                }
            }
        }
    }

    // -- the replacement sheet ------------------------------------------------

    /** Opens the sheet and reads its candidates (ONE request). */
    fun openReplace() {
        _state.update { it.copy(sheet = Sheet.Replace, replace = Replace()) }
        loadCandidates("", false)
    }

    /** The search box's "Chercher": a typed query replaces the server's own. */
    fun searchReplace(query: String) {
        val text = query.trim().replace(Regex("\\s+"), " ")
        if (text.isEmpty()) return
        _state.update { it.copy(replace = it.replace?.copy(searched = text)) }
        loadCandidates(text, false)
    }

    /** "Chercher aussi sur …": the fallback sources, on request. */
    fun searchFallbacks() {
        val current = _state.value.replace ?: return
        if (current.fallbacksLoading || current.fallbacksAsked) return
        loadCandidates(current.searched, true)
    }

    /** "Réessayer" of a candidates read that failed. */
    fun retryCandidates() {
        val current = _state.value.replace ?: return
        _state.update { it.copy(replace = current.copy(error = null, loading = true)) }
        loadCandidates(current.searched, false)
    }

    private fun loadCandidates(query: String, fallbacks: Boolean) {
        replaceJob?.cancel()
        replaceGeneration += 1
        val generation = replaceGeneration
        val row = _state.value.row
        _state.update { state ->
            state.copy(
                replace = state.replace?.copy(
                    loading = !fallbacks,
                    error = null,
                    fallbacksLoading = fallbacks,
                ),
            )
        }
        replaceJob = scope.launch {
            try {
                val answer = withContext(io) {
                    service.replaceCandidates(row.key, query, fallbacks)
                }
                if (generation != replaceGeneration) return@launch
                _state.update { state ->
                    val current = state.replace ?: return@update state
                    val merged = if (fallbacks) {
                        val have = current.candidates.map { it.key }.toSet()
                        current.candidates + answer.candidates.filter { !have.contains(it.key) }
                    } else {
                        answer.candidates
                    }
                    state.copy(
                        replace = current.copy(
                            candidates = merged,
                            loading = false,
                            error = null,
                            query = answer.query,
                            searched = current.searched.ifEmpty { answer.query },
                            fallbackAuto = answer.fallbackAuto ?: current.fallbackAuto,
                            fallbacksAsked = current.fallbacksAsked || fallbacks,
                            fallbacksLoading = false,
                            fallbacksFailed = false,
                        ),
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (generation != replaceGeneration) return@launch
                _state.update { state ->
                    val current = state.replace ?: return@update state
                    state.copy(
                        replace = current.copy(
                            loading = false,
                            fallbacksLoading = false,
                            error = if (fallbacks) null else causeText(e),
                            fallbacksFailed = fallbacks,
                        ),
                    )
                }
            }
        }
    }

    private var replaceGeneration = 0

    /** "Remplacer" on one candidate: one request, then the server's job. */
    fun commitReplace(candidate: ReplaceCandidate) {
        val current = _state.value.replace ?: return
        if (current.busy) return
        val row = _state.value.row
        _state.update { it.copy(replace = current.copy(busy = true)) }
        scope.launch {
            try {
                val report = withContext(io) { service.replace(row.key, candidate.id) }
                val job = report.job
                if (job == null) {
                    toast("« ${candidate.title} » remplace « ${row.title} »", null, null)
                    close()
                    return@launch
                }
                // The rewrite touched playlists the person cannot see from here
                // yet: say it started, then what it changed (rule 2).
                toast("Remplacement en cours…", null, null)
                watchJob(job, "Le remplacement a échoué côté serveur")
                close()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { state -> state.copy(replace = state.replace?.copy(busy = false)) }
                toast("Le remplacement a échoué : ${causeText(e)}", "Réessayer") {
                    commitReplace(candidate)
                }
            }
        }
    }

    /**
     * "Supprimer ce titre" — the third answer, and the only one that asks twice.
     * The server re-checks both sources first and answers `409` when the title
     * turns out playable; that answer has its own sentence.
     */
    fun deleteTarget() {
        val current = _state.value.replace ?: return
        if (current.busy) return
        if (!current.confirming) {
            _state.update { it.copy(replace = current.copy(confirming = true)) }
            return
        }
        val row = _state.value.row
        _state.update { it.copy(replace = current.copy(busy = true, confirming = false)) }
        scope.launch {
            try {
                val report = withContext(io) { service.deleteTracks(listOf(row.key)) }
                toast("« ${row.title} » retiré de la bibliothèque", null, null)
                report.job?.let { watchJob(it, "La suppression a échoué côté serveur") }
                close()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { state -> state.copy(replace = state.replace?.copy(busy = false)) }
                toast(deleteCause(e), null, null)
            }
        }
    }

    /**
     * What a refused deletion says. The server re-checks BOTH sources and answers
     * `409` when the title turns out playable (`availability.py#verify_gone`);
     * that is the one refusal worth spelling out — the rest keeps the core's own
     * sentence.
     */
    private fun deleteCause(cause: Exception): String = when (cause) {
        is NzException.Conflict -> "Ce titre est en fait disponible — rien n'a été supprimé"
        else -> "Suppression impossible : ${NzErrors.text(cause)}"
    }

    /** The credits sheet was dismissed: back to the screen the menu came from. */
    fun dismissCredits() {
        _state.update { it.copy(credits = null, closed = true) }
        closeHost()
    }

    fun backToMenu() {
        _state.update { it.copy(sheet = Sheet.Menu, picker = null, replace = null) }
    }

    // -- sharing --------------------------------------------------------------

    /** "Partager…": the link comes from the core, the sheet from the platform. */
    fun share() {
        val row = _state.value.row
        close()
        scope.launch {
            try {
                val link = withContext(io) {
                    service.shareLink(if (row.source == "podcast") "episode" else "track", row.id)
                }
                sharer.share(link, "Partager « ${row.title} »")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Impossible de partager : ${causeText(e)}", "Réessayer") { share() }
            }
        }
    }

    // -- a server job ---------------------------------------------------------

    /**
     * Watches a rewrite's job until it ends, then says what it changed — or its
     * cause. Rule 2: a long job says where it is; nothing here is a bare count.
     */
    private fun watchJob(job: String, failure: String) {
        jobWatch?.cancel()
        jobWatch = scope.launch {
            repeat(40) {
                val report = try {
                    withContext(io) { service.replaceStatus(job) }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    return@launch
                }
                if (!report.running) {
                    if (report.ok) {
                        val bits = mutableListOf<String>()
                        if (report.playlists > 0u) {
                            val plural = if (report.playlists > 1u) "s" else ""
                            bits += "${report.playlists} playlist$plural"
                        }
                        if (report.favorites > 0u) bits += "favoris"
                        if (bits.isNotEmpty()) toast("Mis à jour dans " + bits.joinToString(" et "), null, null)
                    } else {
                        toast(failure, null, null)
                    }
                    return@launch
                }
                kotlinx.coroutines.delay(1000)
            }
        }
    }
}
