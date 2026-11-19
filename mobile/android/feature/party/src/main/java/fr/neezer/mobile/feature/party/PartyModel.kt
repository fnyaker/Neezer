package fr.neezer.mobile.feature.party

import fr.neezer.mobile.bridge.NzErrors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.nz_ffi.PartyPhase
import uniffi.nz_ffi.PartyRole
import uniffi.nz_ffi.PartyStateData

/** What handing the link out needs from the platform (the SPA's `share`/`copy`). */
interface PartyShare {
    /** Copies `text`; answers `false` when the platform refused. */
    fun copy(text: String): Boolean

    /**
     * The system share sheet, if there is one. Answers `false` when the app must
     * fall back to [copy] (no share sheet, or the person cancelled).
     */
    fun share(title: String, text: String, url: String): Boolean
}

/**
 * The HOST's side of the listen party (lot MB20): `components/PartySheet.svelte`
 * ported — start it, hand the link out (a QR and the share sheet), see who is
 * listening and how well they are locked, end it.
 *
 * Three rules it keeps, all the SPA's:
 *
 * - **Every click answers at once.** Starting shows « Démarrage… » in the same
 *   frame, ending flips to its armed sentence, and the state the sheet draws is
 *   whatever Rust pushed last (`PartyListener`), so a change never waits for a
 *   poll.
 * - **A failure is a sentence with its cause.** « Impossible de démarrer la
 *   soirée » plus `nz_error_text` (rule 3) — never a spinner and never a code.
 * - **The party outlives the sheet.** Closing it changes nothing; the loops are
 *   Rust's and the server keeps the link. Re-opening reads the state, which is
 *   what makes a party started on another screen show up here too.
 */
class PartyModel(
    private val service: PartyService,
    private val scope: CoroutineScope,
    private val share: PartyShare,
    /** The SPA's debounce for the *Décalage* chip; a test passes 0. */
    private val trimDebounceMs: Long = 350,
    /**
     * The French sentence for a failure: `NzErrors.text` in the app, which is
     * `nz_error_text` over JNI — a JVM test passes its own, because the native
     * library is not loaded there (MB13's rule for a testable model).
     */
    private val errorText: (Throwable) -> String = { NzErrors.text(it) },
) {
    data class State(
        val open: Boolean = false,
        /** The party as Rust paints it; `null` before the first push. */
        val data: PartyStateData? = null,
        /** The server has no party routes (rule 7): the door is hidden. */
        val unsupported: Boolean = false,
        val busy: Boolean = false,
        /** « Terminer » asked once: the second press really ends it. */
        val confirmEnd: Boolean = false,
        /** A one-line message to toast, consumed by the screen. */
        val message: String? = null,
        /** A party exists on this device (hosted or joined). */
        val hosting: Boolean = false,
    ) {
        /** Whether the sheet shows the live side instead of its three arguments. */
        val live: Boolean get() = hosting
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** The last trim a run of taps asked for, applied once they stop. */
    private var trimJob: Job? = null
    private var trimDraft: Int = 0
    private var subscribed = false

    /**
     * The door's own question, asked once when it is first drawn (rule 7): one
     * `OPTIONS`, on the `Prefetch` lane, and a DOUBT never hides the door.
     */
    fun probe() {
        scope.launch { refreshSupport() }
    }

    /** The sheet was opened: the door's tap. */
    fun open() {
        _state.value = _state.value.copy(open = true, message = null)
        subscribe()
        scope.launch { refreshSupport() }
        scope.launch { refresh() }
    }

    fun close() {
        _state.value = _state.value.copy(open = false, confirmEnd = false)
        trimJob?.cancel()
        // A draft that never landed is not a setting: apply it now, so the chip's
        // number and the device agree when the sheet closes.
        if (trimDraft != 0) {
            service.setTrim(service.prefs().trimMs + trimDraft.toDouble())
            trimDraft = 0
        }
    }

    /** `startParty()`: one request, and the first line goes out with it. */
    fun start() {
        if (_state.value.busy) return
        _state.value = _state.value.copy(busy = true, message = null)
        scope.launch {
            try {
                service.host()
                push(service.state())
                _state.value = _state.value.copy(
                    busy = false,
                    hosting = true,
                    data = service.state(),
                    confirmEnd = false,
                )
            } catch (error: Exception) {
                _state.value = _state.value.copy(
                    busy = false,
                    message = PartyCopy.START_FAILED + " · " + errorText(error),
                )
            }
        }
    }

    /** `endParty()`: the first press arms, the second ends it. */
    fun end() {
        if (_state.value.busy) return
        if (!_state.value.confirmEnd) {
            _state.value = _state.value.copy(confirmEnd = true)
            return
        }
        _state.value = _state.value.copy(busy = true)
        scope.launch {
            try {
                service.leave()
                _state.value = _state.value.copy(
                    busy = false,
                    confirmEnd = false,
                    hosting = false,
                    data = service.state(),
                    message = PartyCopy.ENDED,
                    open = false,
                )
            } catch (error: Exception) {
                _state.value = _state.value.copy(
                    busy = false,
                    confirmEnd = false,
                    message = errorText(error),
                )
            }
        }
    }

    fun dismissConfirm() {
        if (_state.value.confirmEnd) _state.value = _state.value.copy(confirmEnd = false)
    }

    fun copyLink() {
        val link = _state.value.data?.link.orEmpty()
        if (link.isEmpty()) return
        val copied = share.copy(link)
        _state.value = _state.value.copy(
            message = if (copied) PartyCopy.COPIED else PartyCopy.COPY_FAILED,
        )
    }

    fun shareLink() {
        val link = _state.value.data?.link.orEmpty()
        if (link.isEmpty()) return
        if (!share.share(PartyCopy.INVITE_TITLE, PartyCopy.INVITE_TEXT, link)) copyLink()
    }

    fun consumeMessage() {
        if (_state.value.message != null) _state.value = _state.value.copy(message = null)
    }

    /**
     * The *Décalage* chip: `deltaMs` is added to the draft, which is applied
     * `trimDebounceMs` after the LAST tap — a run of taps is one re-alignment
     * (`Party.svelte#nudge`), not ten.
     */
    fun nudge(deltaMs: Int) {
        val base = service.prefs().trimMs.toInt() + trimDraft
        trimDraft = (base + deltaMs).coerceIn(-500, 1000) - service.prefs().trimMs.toInt()
        push(service.state())
        trimJob?.cancel()
        trimJob = scope.launch {
            if (trimDebounceMs > 0) delay(trimDebounceMs)
            val target = service.prefs().trimMs + trimDraft.toDouble()
            service.setTrim(target)
            trimDraft = 0
            push(service.state())
        }
    }

    /** The trim the chip prints: what the device has, plus an unlanded draft. */
    fun trimShown(): Int = service.prefs().trimMs.toInt() + trimDraft

    private suspend fun refreshSupport() {
        val supported = try {
            service.supported()
        } catch (_: Exception) {
            null
        }
        // A DOUBT never hides the door: only a server that really said "no".
        if (supported == false) {
            _state.value = _state.value.copy(unsupported = true, open = false)
        }
    }

    /** Re-open a party this device was hosting before the app was killed. */
    private suspend fun refresh() {
        val resumed = try {
            service.resume()
        } catch (_: Exception) {
            null
        }
        push(service.state())
        if (resumed != null) {
            _state.value = _state.value.copy(hosting = true, data = service.state())
        }
    }

    private fun subscribe() {
        if (subscribed) return
        subscribed = true
        service.subscribe { data -> push(data) }
    }

    private fun push(data: PartyStateData) {
        val hosting = data.role is PartyRole.Host
        val ended = data.phase == PartyPhase.ENDED || data.phase == PartyPhase.IDLE
        _state.value = _state.value.copy(
            data = data,
            hosting = hosting && !ended,
        )
    }
}
