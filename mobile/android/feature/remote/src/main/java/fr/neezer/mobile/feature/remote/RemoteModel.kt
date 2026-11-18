package fr.neezer.mobile.feature.remote

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.RemoteCommand
import uniffi.nz_ffi.RemoteDuration
import uniffi.nz_ffi.RemoteLevel
import uniffi.nz_ffi.RemoteListener
import uniffi.nz_ffi.RemotePhase
import uniffi.nz_ffi.RemoteStatus

/** Where a claim stands (`routes/RemoteClaim.svelte`). */
enum class ClaimPhase { Idle, Claiming, Connected, Invalid, Offline }

/** A failure the sheet draws with « Réessayer ». */
data class RemoteFailure(val cause: String, val retry: () -> Unit)

/**
 * Télécommande (lot MB19), the SPA's `RemoteSheet.svelte`, `RemoteBanner.svelte`,
 * `RemoteHostChip.svelte` and `routes/RemoteClaim.svelte`, ported.
 *
 * One state machine for both roles, because they are one service:
 *
 * - **the owner's side** — [`RemoteModel.create`] mints a link (level, duration,
 *   name), shows its URL to share, [`RemoteModel.cut`] cuts one,
 *   [`RemoteModel.cutAll`] cuts every link onto this device (armed once, like the
 *   SPA's `confirmAll`), and the links are re-read while the sheet is open
 *   (`RemoteSheet.svelte`'s 4 s cadence — a light in-memory `GET`, only while the
 *   sheet is on screen);
 * - **the controller side** — [`RemoteModel.claim`] claims `/rc/<token>` then
 *   starts driving, [`RemoteModel.send`] sends one transport command (optimistic:
 *   the core's view changed before the request), [`RemoteModel.leave`] hands the
 *   loan back, and [`RemoteModel.refreshControl`] reads the owner's player while
 *   the control screen is visible — a cheap read, four times a second, exactly
 *   like the SPA's `tick`.
 *
 * The status arrives from Rust on CHANGE (`RemoteListener`), never on a timer: a
 * controller appearing, a tap being acked and a loan ending are the three events
 * the chip and the banner paint.
 *
 * Everything is plain Kotlin over [RemoteService], so `RemoteModelTest` proves
 * the rules on the JVM with a fake and a virtual clock.
 */
class RemoteModel(
    private val service: RemoteService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String, (() -> Unit)?) -> Unit = { text, retry ->
        NzToasts.show(text, if (retry != null) "Réessayer" else null, retry)
    },
) {
    data class State(
        /** The links onto this device, as last read. */
        val links: List<RemoteLinkRow> = emptyList(),
        val linksLoaded: Boolean = false,
        /** What is drawn is the copy this device kept in memory, not a fresh read. */
        val linksStale: Boolean = false,
        /** The cause of a read that failed with nothing to draw. */
        val failure: RemoteFailure? = null,
        /** A link is being minted (the sheet's spinner). */
        val creating: Boolean = false,
        /** The link the sheet is inviting with, once made. */
        val invite: RemoteLinkRow? = null,
        /** « Tout couper » was pressed once and waits for its confirmation. */
        val armedCut: Boolean = false,
        val levels: List<RemoteLevel> = emptyList(),
        val durations: List<RemoteDuration> = emptyList(),
        /** The radio choices the sheet holds while it is open. */
        val level: String = "queue",
        val duration: String = "1d",
        val label: String = "",
        /** The chip and the banner, pushed by Rust on change. */
        val status: RemoteStatusFi? = null,
        /** The claim screen, and WHY it could not connect (rule 3: a cause). */
        val claim: ClaimPhase = ClaimPhase.Idle,
        val claimCause: String? = null,
        /** The grant a claim produced, painted while the banner catches up. */
        val grant: RemoteGrantFi? = null,
        /** The loan ended; `restored` names the account the server gave back. */
        val ended: String? = null,
        /** The owner's player, while the control screen is visible. */
        val control: RemoteNowFi? = null,
        val rows: RemoteRows? = null,
        /** A command is in flight (a second tap on a refused link still answers). */
        val sending: Boolean = false,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var refreshJob: Job? = null
    private var subscribed = false

    // --- the sheet ---------------------------------------------------------

    /**
     * The sheet opened: subscribe once, read the levels and durations, then the
     * links — the cached one first (it is this device's own token list, kept in
     * MEMORY only), the fresh one replacing it.
     */
    fun open() {
        subscribe()
        if (_state.value.levels.isEmpty()) {
            runCatching { service.levels() }.onSuccess { levels ->
                _state.update { it.copy(levels = levels, level = levels.firstOrNull()?.id ?: it.level) }
            }
            runCatching { service.durations() }.onSuccess { durations ->
                _state.update { it.copy(durations = durations) }
            }
        }
        val cached = runCatching { service.cachedLinks() }.getOrDefault(emptyList()).map { it.toRow() }
        if (cached.isNotEmpty()) {
            _state.update { it.copy(links = cached, linksStale = true) }
        }
        load()
        // Who is connected changes while the sheet is open: a light re-read, the
        // SPA's own 4 s.
        refreshJob?.cancel()
        refreshJob = scope.launch {
            while (true) {
                delay(REFRESH_MS)
                load(quiet = true)
            }
        }
    }

    /** The sheet closed: the 4 s read stops, and the invite with it. */
    fun close() {
        refreshJob?.cancel()
        refreshJob = null
        _state.update { it.copy(invite = null, armedCut = false) }
    }

    fun setLevel(level: String) = _state.update { it.copy(level = level) }

    fun setDuration(duration: String) = _state.update { it.copy(duration = duration) }

    fun setLabel(label: String) = _state.update { it.copy(label = label) }

    /** `GET /api/remote/links?device=` — one request, on the `User` lane. */
    fun load(quiet: Boolean = false) {
        scope.launch {
            try {
                val fresh = withContext(io) { service.links() }.map { it.toRow() }
                _state.update {
                    it.copy(
                        links = fresh,
                        linksLoaded = true,
                        linksStale = false,
                        failure = null,
                        invite = it.invite?.let { held -> fresh.firstOrNull { row -> row.id == held.id } },
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (!quiet) {
                    _state.update {
                        if (it.links.isNotEmpty()) {
                            it.copy(linksStale = true, linksLoaded = true, failure = null)
                        } else {
                            it.copy(
                                linksLoaded = true,
                                failure = RemoteFailure(NzErrors.text(e)) { load() },
                            )
                        }
                    }
                }
            }
        }
    }

    /**
     * « Créer le lien » — the only write of this sheet that is not optimistic, and
     * for a reason: the TOKEN comes from the server (`<id>.<mac>`), so there is
     * nothing honest to show before it answers.
     */
    fun create() {
        val state = _state.value
        if (state.creating) return
        _state.update { it.copy(creating = true) }
        scope.launch {
            try {
                val label = state.label.trim().ifEmpty { null }
                val link = withContext(io) { service.createLink(state.level, state.duration, label) }
                _state.update {
                    it.copy(
                        creating = false,
                        invite = link.toRow(),
                        links = listOf(link.toRow()) + it.links.filterNot { held -> held.id == link.id },
                        label = "",
                    )
                }
                // This device is lent from now on: its player answers at once,
                // which is what the SPA's `startHosting()` does after a create.
                withContext(io) { service.startHost() }
                subscribe()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(creating = false) }
                toast(NzErrors.text(e)) { create() }
            }
        }
    }

    fun dismissInvite() = _state.update { it.copy(invite = null) }

    /** « Couper » on one link: the session it opened ends on its next request. */
    fun cut(id: String) {
        scope.launch {
            try {
                withContext(io) { service.revokeLink(id) }
                _state.update {
                    it.copy(
                        links = it.links.filterNot { row -> row.id == id },
                        invite = it.invite?.takeIf { row -> row.id != id },
                    )
                }
                toast("Lien coupé") { cut(id) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Impossible de couper ce lien : ${NzErrors.text(e)}") { cut(id) }
            }
        }
    }

    /**
     * « Tout couper » — armed once (`RemoteSheet.svelte#cutEverything`): the first
     * press asks, the second cuts. The core stops its loops BEFORE the request, so
     * nothing is sent on a link that is being cut.
     */
    fun cutAll() {
        if (!_state.value.armedCut) {
            _state.update { it.copy(armedCut = true) }
            return
        }
        scope.launch {
            try {
                withContext(io) { service.revokeAll() }
                _state.update { it.copy(links = emptyList(), invite = null, armedCut = false) }
                toast("Contrôle à distance coupé") { cutAll() }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(armedCut = false) }
                toast("Impossible de tout couper : ${NzErrors.text(e)}") { cutAll() }
            }
        }
    }

    /** The text « Partager » and « Copier » hand out. */
    fun inviteText(row: RemoteLinkRow): String =
        "Tu peux piloter ma musique (${row.levelLabel.lowercase()}) : ${row.url}"

    // --- the controller ----------------------------------------------------

    /**
     * Claim `/rc/<token>` (`routes/RemoteClaim.svelte#claim`).
     *
     * An EMPTY token means "this session is already a loan": the control screen
     * opened from the button after a restart has no link to paste, and the grant
     * `/api/me` reports is the truth.
     */
    fun claim(token: String) {
        val trimmed = token.trim()
        if (trimmed.isEmpty()) {
            val grant = runCatching { service.status() }.getOrNull()?.grant?.toFi()
            if (grant != null) {
                _state.update { it.copy(claim = ClaimPhase.Connected, grant = grant, claimCause = null) }
                startLoan()
            } else {
                _state.update { it.copy(claim = ClaimPhase.Invalid) }
            }
            return
        }
        _state.update { it.copy(claim = ClaimPhase.Claiming) }
        scope.launch {
            try {
                val grant = withContext(io) { service.claim(trimmed) }
                _state.update { it.copy(claim = ClaimPhase.Connected, grant = grant.toFi(), claimCause = null) }
                startLoan()
                subscribe()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val cause = NzErrors.text(e)
                val offline = cause.contains("réseau", ignoreCase = true) ||
                    cause.contains("hors ligne", ignoreCase = true)
                _state.update {
                    it.copy(
                        claim = if (offline) ClaimPhase.Offline else ClaimPhase.Invalid,
                        claimCause = cause,
                    )
                }
            }
        }
    }

    private fun startLoan() {
        scope.launch {
            runCatching { withContext(io) { service.startLoan() } }
            refreshControl()
        }
    }

    /**
     * One read of the owner's player (the "Contrôle distant" screen, four times a
     * second while it is visible). The queue's rows are rebuilt only when the
     * owner's queue version moved.
     */
    fun refreshControl() {
        val now = try {
            service.now().toFi()
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // The banner already names the cause (`status().error`); a screen that
            // cannot read one step keeps the picture it has.
            return
        }
        // The transport is painted before the queue: a queue that cannot be read
        // (a level, a failure) must not blank the play/pause button.
        _state.update { it.copy(control = now) }
        // Re-read the owner's queue when its version moved, when this screen has
        // none yet, AND while what it has is empty: a screen opened before the
        // crate had read `/remote/queue` would otherwise keep an empty list for
        // ever (the version it captured is already the current one).
        val stale = _state.value.rows?.let { it.source.len() == 0 } ?: true
        if (stale || now.queueVersion != _state.value.control?.queueVersion) {
            try {
                _state.update { it.copy(rows = RemoteRows.of(service.queue())) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // The rows keep whatever they had.
            }
        }
    }

    /**
     * One transport command. Optimistic: the core's view changes in the same
     * frame, and a refusal (a level that does not allow it, a link pressed while
     * offline) changes NOTHING and says why, with « Réessayer ».
     */
    fun send(command: RemoteCommand) {
        if (_state.value.sending) return
        _state.update { it.copy(sending = true) }
        scope.launch {
            try {
                withContext(io) { service.send(command) }
                refreshControl()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast(NzErrors.text(e)) { send(command) }
            } finally {
                _state.update { it.copy(sending = false) }
            }
        }
    }

    /** « Quitter » — the loan ends here, and the account comes back. */
    fun leave() {
        scope.launch {
            val restored = runCatching { withContext(io) { service.leave() } }.getOrNull()
            _state.update {
                it.copy(claim = ClaimPhase.Idle, control = null, rows = null, ended = restored ?: "")
            }
            toast(
                if (restored.isNullOrEmpty()) "Contrôle terminé" else "De retour sur le compte de $restored",
                null,
            )
        }
    }

    // --- what Rust pushes --------------------------------------------------

    private fun subscribe() {
        if (subscribed) return
        subscribed = true
        runCatching {
            service.subscribe(object : RemoteListener {
                override fun onStatus(status: RemoteStatus) {
                    _state.update { it.copy(status = status.toFi()) }
                }

                override fun onEnded(restored: String?) {
                    _state.update {
                        it.copy(
                            status = it.status?.copy(phase = RemotePhase.ENDED),
                            ended = restored ?: "",
                            control = null,
                        )
                    }
                }
            })
        }.onFailure { subscribed = false }
    }

    companion object {
        /** `RemoteSheet.svelte`'s `setInterval(load, 4000)`, only while open. */
        const val REFRESH_MS = 4_000L

        /** The control screen's own beat (`controller.js#tick`). */
        const val CONTROL_MS = 250L
    }
}
