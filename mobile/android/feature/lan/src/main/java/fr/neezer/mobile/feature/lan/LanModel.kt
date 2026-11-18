package fr.neezer.mobile.feature.lan

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
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
import uniffi.nz_ffi.LanDeviceFfi
import uniffi.nz_ffi.LanFriendFfi
import uniffi.nz_ffi.LanFriendsFfi
import uniffi.nz_ffi.LanPlaylistFfi
import uniffi.nz_ffi.LanSettingsFfi
import uniffi.nz_ffi.LanSnapshotFfi
import uniffi.nz_ffi.LanStateFfi

/** The playlists a friend's scope may name (`/api/me/playlists`, the SPA's route). */
interface LanPlaylistSource {
    /** Disk only: what the library screen already fetched, or nothing. */
    fun cached(): List<LanPlaylistFfi>

    /** One `User` request; `GET /api/me/playlists`. */
    suspend fun fresh(): List<LanPlaylistFfi>
}

/**
 * « Appareils à proximité »'s state machine (lot LN14).
 *
 * Plain Kotlin over [LanPort] and [LanPlaylistSource], so a JVM test holds every
 * rule with a fake — no JNA, no `.so`, no device:
 *
 * - **The switches are optimistic** (rule 4: a click answers in the same frame):
 *   the state moves first, the write follows, a refusal puts it back and says
 *   why. The core persists `lan.*` in `nz-cache`'s `kv`, so what the phone
 *   stores is what every later screen reads.
 * - **The ceiling is the settings', not the UI's**: `toggleTolerated` never
 *   removes the last rung (the note: « at least one »), because an empty list
 *   means "never take anything from a peer".
 * - **The read is one clone of memory** (`snapshot()`, 2.3 µs): a change pushes a
 *   version and the model re-reads; nothing polls, nothing waits on the network
 *   to paint (rule 3).
 * - **A refusal is a cause, never a count** (rule 2): the server's own French
 *   sentence plus « Réessayer », on the row that failed.
 * - **The hotel line is an event, not a spinner** (rule 3): 10 s in `Searching`
 *   with another enrolled device and no peer shows [NO_PEER_HINT]; a peer
 *   arriving cancels it. The delay is a parameter, so the test drives it on the
 *   JVM's virtual clock and no wall clock is ever read.
 */
class LanModel(
    private val port: LanPort,
    private val scope: CoroutineScope,
    private val playlists: LanPlaylistSource,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = {},
    private val hintAfterMs: Long = NO_PEER_HINT_AFTER_MS,
) {
    data class State(
        /** The node, cloned from memory; `null` before the first read. */
        val snapshot: LanSnapshotFfi? = null,
        /** This device's settings (the stored ones answer before the node exists). */
        val settings: LanSettingsFfi? = null,
        val devices: ScreenState<List<LanDeviceFfi>> = ScreenState.Loading,
        val friends: ScreenState<LanFriendsFfi> = ScreenState.Loading,
        /** The key of the row whose request is in flight (a second tap is ignored). */
        val busy: String? = null,
        /** A refusal's cause, with its retry, on the row that failed. */
        val notice: String? = null,
        /** The advanced port field, while the person types. */
        val portDraft: String = "",
        /** The 10 s « aucun appareil » line (rule 3). */
        val showNoPeerHint: Boolean = false,
        /** The friend whose playlist scope is open (a picker, not a route). */
        val pickerFor: String? = null,
        val playlistChoices: List<LanPlaylistFfi> = emptyList(),
    ) {
        /** My devices, as the note's screen wants them ("what a person owns"). */
        val devicesSeen: List<LanDeviceFfi>
            get() = (devices as? ScreenState.Ready)?.data.orEmpty()

        /** The other enrolled devices: the ones a `Searching` state is waiting for. */
        val hasOtherDevice: Boolean
            get() = devicesSeen.any { !it.current && it.revoked == null }

        /** Whether the node is off, so the page says so and the lists rest. */
        val off: Boolean
            get() = settings?.enabled == false || snapshot?.state == LanStateFfi.Off

        val unsupported: Boolean
            get() = snapshot?.state == LanStateFfi.Unsupported
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var started = false
    private var hintJob: Job? = null

    /**
     * Reads what is already in memory, then starts the node in the background.
     *
     * The node's start is a server probe on the core's runtime (LN13 made it
     * async for exactly this: no blocking request on the main thread), so the
     * screen paints from the settings and the snapshot at once and the probe
     * lands later, on a change.
     */
    fun start() {
        if (started) return
        started = true
        _state.update {
            it.copy(
                snapshot = runCatching { port.snapshot() }.getOrNull(),
                settings = runCatching { port.settings() }.getOrNull(),
                portDraft = runCatching { port.settings() }.getOrNull()?.port?.toString().orEmpty(),
            )
        }
        port.subscribe(LanFeed.listener)
        scope.launch {
            LanFeed.version.collect { publish() }
        }
        refresh()
        scope.launch {
            runCatching { port.start() }
            publish()
        }
    }

    /** Reads the account's devices and friends again (the section's « Réessayer »). */
    fun refresh() {
        scope.launch { loadDevices() }
        scope.launch { loadFriends() }
    }

    private fun publish() {
        val snapshot = runCatching { port.snapshot() }.getOrNull() ?: return
        _state.update { it.copy(snapshot = snapshot) }
        choreographHint(snapshot)
    }

    private suspend fun loadDevices() {
        _state.update { it.copy(devices = ScreenState.Loading) }
        val result: ScreenState<List<LanDeviceFfi>> = try {
            ScreenState.Ready(withContext(io) { port.devices() })
        } catch (e: Exception) {
            if (e is kotlinx.coroutines.CancellationException) throw e
            ScreenState.Failed(NzErrors.text(e)) { refresh() }
        }
        _state.update { it.copy(devices = result) }
        _state.value.snapshot?.let(::choreographHint)
    }

    private suspend fun loadFriends() {
        _state.update { it.copy(friends = ScreenState.Loading) }
        val result: ScreenState<LanFriendsFfi> = try {
            ScreenState.Ready(withContext(io) { port.friends() })
        } catch (e: Exception) {
            if (e is kotlinx.coroutines.CancellationException) throw e
            ScreenState.Failed(NzErrors.text(e)) { refresh() }
        }
        _state.update { it.copy(friends = result) }
    }

    // -- the two switches, and the tolerated rungs --------------------------

    /** « Échanger avec mes appareils à proximité » (`lan.enabled`). */
    fun setEnabled(on: Boolean) {
        write(current().copy(enabled = on))
        if (on) scope.launch { runCatching { port.start() }; publish() }
        else publish()
    }

    /** « Partager mes sons » (`lan.serve`). */
    fun setServe(on: Boolean) = write(current().copy(serve = on))

    /** One tolerated rung; the last one cannot be removed (`lan.tolerated`). */
    fun toggleTolerated(id: String) {
        val now = current()
        write(now.copy(tolerated = toggleTolerated(now.tolerated, id)))
    }

    /** The advanced port field: `0` = any, anything else must be a port. */
    fun setPortDraft(text: String) {
        _state.update { it.copy(portDraft = text.filter { ch -> ch.isDigit() }.take(5)) }
    }

    /** Commits the port; an empty or out-of-range field keeps the previous value. */
    fun commitPort() {
        val parsed = parsePort(_state.value.portDraft) ?: run {
            _state.update { it.copy(portDraft = current().port.toString()) }
            return
        }
        write(current().copy(port = parsed.toUShort()))
    }

    private fun current(): LanSettingsFfi = _state.value.settings ?: port.settings()

    /** Optimistic settings write: the state moves, then the core persists it. */
    private fun write(next: LanSettingsFfi) {
        _state.update { it.copy(settings = next) }
        runCatching { port.setSettings(next) }
            .onFailure { failure ->
                val previous = currentOrNull()
                val cause = NzErrors.text(failure)
                _state.update { it.copy(settings = previous, notice = cause) }
                toast(cause)
            }
    }

    private fun currentOrNull(): LanSettingsFfi? = runCatching { port.settings() }.getOrNull()

    // -- my devices ---------------------------------------------------------

    /** Renames one of my devices; the row moves first, a refusal puts it back. */
    fun renameDevice(id: String, name: String) {
        val trimmed = name.trim()
        if (trimmed.isEmpty() || trimmed.length > 64) return
        val before = _state.value.devicesSeen
        optimisticDevices(before.map { if (it.id == id) it.copy(name = trimmed) else it })
        mutate("device:$id", onFailure = { optimisticDevices(before) }) {
            port.rename(id, trimmed)
        }
    }

    /** « Révoquer »: the row leaves at once; every peer refuses it at its next sync. */
    fun revokeDevice(id: String) {
        val before = _state.value.devicesSeen
        optimisticDevices(before.filterNot { it.id == id })
        mutate("device:$id", onFailure = { optimisticDevices(before) }) {
            port.revoke(id)
        }
    }

    private fun optimisticDevices(rows: List<LanDeviceFfi>) {
        _state.update { it.copy(devices = ScreenState.Ready(rows)) }
    }

    // -- trusted friends ----------------------------------------------------

    fun addFriend(name: String) {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) return
        mutate("friend:$trimmed") {
            port.addFriend(trimmed)
            loadFriendsNow()
        }
    }

    fun openPicker(userId: String) {
        _state.update { it.copy(pickerFor = userId) }
        _state.update { it.copy(playlistChoices = playlists.cached()) }
        scope.launch {
            val fresh = runCatching { withContext(io) { playlists.fresh() } }.getOrNull()
            if (fresh != null) _state.update { it.copy(playlistChoices = fresh) }
        }
    }

    fun closePicker() {
        _state.update { it.copy(pickerFor = null) }
    }

    /** « Tout partager » for one friend; off keeps only the picked playlists. */
    fun setFriendAll(userId: String, all: Boolean) {
        val friend = friendOf(userId) ?: return
        val rows = friendsRows()
        optimisticFriends(
            rows.map {
                if (it.userId == userId) it.copy(all = all, playlists = if (all) emptyList() else it.playlists) else it
            },
        )
        mutate(
            "friend:$userId",
            onFailure = { optimisticFriends(rows) },
        ) {
            port.setFriendScope(userId, all, if (all) emptyList() else friend.playlists.map(LanPlaylistFfi::id))
        }
    }

    /** One playlist in a friend's scope (only when « Tout partager » is off). */
    fun toggleFriendPlaylist(userId: String, playlistId: String) {
        val rows = friendsRows()
        val friend = rows.firstOrNull { it.userId == userId } ?: return
        val next = if (friend.playlists.any { it.id == playlistId }) {
            friend.playlists.filterNot { it.id == playlistId }
        } else {
            val title = _state.value.playlistChoices.firstOrNull { it.id == playlistId }?.title ?: playlistId
            friend.playlists + LanPlaylistFfi(id = playlistId, title = title)
        }
        val updated = friend.copy(all = false, playlists = next)
        optimisticFriends(rows.map { if (it.userId == userId) updated else it })
        mutate(
            "friend:$userId",
            onFailure = { optimisticFriends(rows) },
        ) {
            port.setFriendScope(userId, false, next.map(LanPlaylistFfi::id))
        }
    }

    /** « Retirer »: the friend leaves both lists at once. */
    fun removeFriend(userId: String) {
        val before = _state.value.friends
        val friend = friendOf(userId) ?: return
        optimisticFriends(friendsRows().filterNot { it.userId == userId }, trustedBy = trustedByRows().filterNot { it.userId == userId })
        mutate(
            "friend:$userId",
            onFailure = { _state.update { state -> state.copy(friends = before) } },
        ) {
            port.removeFriend(userId)
        }
    }

    private fun friendOf(userId: String): LanFriendFfi? =
        friendsRows().firstOrNull { it.userId == userId }
            ?: trustedByRows().firstOrNull { it.userId == userId }

    private fun friendsRows(): List<LanFriendFfi> =
        (_state.value.friends as? ScreenState.Ready)?.data?.sharesTo.orEmpty()

    private fun trustedByRows(): List<LanFriendFfi> =
        (_state.value.friends as? ScreenState.Ready)?.data?.trustedBy.orEmpty()

    private fun optimisticFriends(sharesTo: List<LanFriendFfi>, trustedBy: List<LanFriendFfi> = trustedByRows()) {
        _state.update {
            it.copy(friends = ScreenState.Ready(LanFriendsFfi(sharesTo = sharesTo, trustedBy = trustedBy)))
        }
    }

    private suspend fun loadFriendsNow() {
        loadFriends()
    }

    // -- the one mutation wrapper ------------------------------------------

    /**
     * One request at a time per row (`busy`), its cause on the row, and a
     * `Réessayer` closure that runs the same lambda again. Nothing retries by
     * itself (`docs/agents/mobile.md`: mutations are never retried).
     */
    private fun mutate(key: String, onFailure: () -> Unit = {}, block: suspend () -> Unit) {
        if (_state.value.busy == key) return
        _state.update { it.copy(busy = key, notice = null) }
        scope.launch {
            try {
                withContext(io) { block() }
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) throw e
                onFailure()
                val cause = NzErrors.text(e)
                _state.update { it.copy(notice = cause) }
                toast(cause)
            } finally {
                _state.update { it.copy(busy = null) }
            }
        }
    }

    // -- the hotel line ----------------------------------------------------

    /**
     * Shows [NO_PEER_HINT] after [hintAfterMs] in `Searching` with another
     * enrolled device and no peer — and takes it back as soon as a peer answers
     * or the node stops looking. A job, not a timer: the test drives it on
     * `runTest`'s virtual clock, and the person never sees a spinner.
     */
    private fun choreographHint(snapshot: LanSnapshotFfi) {
        val waiting = snapshot.peers.isEmpty() &&
            snapshot.state == LanStateFfi.Searching &&
            _state.value.settings?.enabled != false &&
            _state.value.hasOtherDevice
        if (!waiting) {
            hintJob?.cancel()
            hintJob = null
            if (_state.value.showNoPeerHint) _state.update { it.copy(showNoPeerHint = false) }
            return
        }
        if (hintJob != null) return
        hintJob = scope.launch {
            delay(hintAfterMs)
            _state.update { it.copy(showNoPeerHint = true) }
        }
    }

    /** Clears the refusal line (the row's own dismiss). */
    fun dismissNotice() {
        _state.update { it.copy(notice = null) }
    }
}
