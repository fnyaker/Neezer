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
import uniffi.nz_ffi.PartyStateData

/**
 * The GUEST's side (lot MB20): `routes/Party.svelte` ported — the landing card
 * (who invites you, what is playing, one tap to join), then the live view.
 *
 * The four rules the page keeps, and this keeps:
 *
 * - **The card paints from the device first.** A warm open shows the remembered
 *   card at once and refreshes it (`stale`), so a link opened twice costs no
 *   blocking request; with no copy and a failure, the cause and « Réessayer »,
 *   never a spinner (rule 3).
 * - **The tap is the permission.** Nothing is fetched or played before it: the
 *   card is one read, and `join` is the seat.
 * - **The playhead is the ROOM's**, extrapolated by Rust between two polls
 *   (`positionNow`, a hot read) — the screen reads it each frame it draws, and
 *   never asks the network.
 * - **The four values are the listener's**: the name, the party's volume and the
 *   *Décalage* they tuned by ear are kept by the core and re-used next time.
 */
class PartyGuestModel(
    private val service: PartyService,
    private val scope: CoroutineScope,
    /** The party the link named. */
    val id: String,
    /** The SPA's debounce for the *Décalage* card; a test passes 0. */
    private val trimDebounceMs: Long = 350,
    /** The French sentence for a failure (`NzErrors.text` in the app). */
    private val errorText: (Throwable) -> String = { NzErrors.text(it) },
) {
    data class State(
        val phase: PartyPhase = PartyPhase.IDLE,
        val data: PartyStateData? = null,
        /** What is shown is the card the device remembered. */
        val stale: Boolean = false,
        /** The card could not be read and nothing was remembered: the cause. */
        val failure: String? = null,
        val loading: Boolean = true,
        /** The seat is being taken (the tap is answered, the request is out). */
        val joining: Boolean = false,
        /** The name field's text (the person's own, kept by the core). */
        val name: String = "",
        /** The live view's volume, while a finger is on the slider. */
        val volume: Double = 1.0,
        /** The *Décalage* card is open. */
        val showTrim: Boolean = false,
        /** The trim the chip prints, milliseconds. */
        val trimMs: Int = 0,
        /** A one-line message to toast, consumed by the screen. */
        val message: String? = null,
    ) {
        /** The landing card, with or without a copy behind it. */
        val landing: Boolean get() = phase == PartyPhase.LANDING
        val live: Boolean get() = phase == PartyPhase.LIVE || phase == PartyPhase.JOINING
        /** The two dead-link pages and the failure page own the whole screen. */
        val dead: Boolean
            get() = phase == PartyPhase.MISSING ||
                phase == PartyPhase.ENDED ||
                phase == PartyPhase.LEFT ||
                phase == PartyPhase.FAILED
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var subscribeJob: Job? = null
    private var trimJob: Job? = null
    private var trimDraft: Int = 0
    private var subscribed = false

    /** The card: the device's copy, then the server's answer. */
    fun load() {
        val prefs = service.prefs()
        trimDraft = 0
        _state.value = _state.value.copy(
            name = prefs.name,
            volume = prefs.volume,
            trimMs = prefs.trimMs.toInt(),
            loading = true,
            failure = null,
        )
        scope.launch {
            var kept: PartyStateData? = null
            try {
                kept = service.cachedPeek(id)
            } catch (_: Exception) {
                // A copy that cannot be read is not a failure the person sees:
                // the fresh read decides (MB2's `screenFlow` rule).
            }
            if (kept != null && _state.value.data == null) {
                _state.value = _state.value.copy(
                    phase = PartyPhase.LANDING,
                    data = kept,
                    stale = true,
                    loading = false,
                )
            }
            try {
                val card = service.peek(id)
                if (card == null) {
                    _state.value = _state.value.copy(
                        phase = PartyPhase.MISSING,
                        data = null,
                        stale = false,
                        loading = false,
                    )
                } else {
                    _state.value = _state.value.copy(
                        phase = PartyPhase.LANDING,
                        data = card,
                        stale = false,
                        loading = false,
                        failure = null,
                    )
                }
            } catch (error: Exception) {
                val cause = errorText(error)
                _state.value = if (kept != null) {
                    _state.value.copy(stale = true, loading = false)
                } else {
                    _state.value.copy(
                        phase = PartyPhase.FAILED,
                        failure = cause,
                        loading = false,
                    )
                }
            }
        }
    }

    fun retry() = load()

    fun setName(value: String) {
        val trimmed = value.take(32)
        _state.value = _state.value.copy(name = trimmed)
        service.setName(trimmed)
    }

    /** The tap: one seat, then the room's sound. */
    fun join() {
        if (_state.value.joining) return
        _state.value = _state.value.copy(joining = true, failure = null)
        scope.launch {
            try {
                val data = service.join(id, _state.value.name.trim())
                subscribe()
                _state.value = _state.value.copy(
                    phase = if (data.phase == PartyPhase.IDLE) PartyPhase.JOINING else data.phase,
                    data = data,
                    joining = false,
                    stale = false,
                    volume = data.volume,
                    trimMs = data.trimMs.toInt(),
                )
            } catch (error: Exception) {
                _state.value = _state.value.copy(
                    joining = false,
                    phase = PartyPhase.LANDING,
                    message = errorText(error),
                )
            }
        }
    }

    /** « Quitter »: the seat goes, the screen says so, the sound stops. */
    fun leave() {
        scope.launch {
            try {
                service.leave()
            } catch (_: Exception) {
                // The sound is already off (Rust stops the scheduler first); a
                // leave that never reached the server costs a stale name in the
                // host's list until its own timeout, and nothing else.
            }
            trimJob?.cancel()
            _state.value = _state.value.copy(phase = PartyPhase.LEFT, joining = false)
        }
    }

    /** « Rejoindre à nouveau »: the card again, then the tap. */
    fun rejoin() {
        unsubscribe()
        _state.value = _state.value.copy(
            phase = PartyPhase.IDLE,
            data = null,
            stale = false,
            failure = null,
        )
        load()
    }

    fun setVolume(volume: Double) {
        val clamped = volume.coerceIn(0.0, 1.0)
        _state.value = _state.value.copy(volume = clamped)
        service.setVolume(clamped)
    }

    fun toggleTrim() {
        _state.value = _state.value.copy(showTrim = !_state.value.showTrim)
    }

    /**
     * The *Décalage*: added to the draft, applied 350 ms after the LAST tap — a
     * run of taps is one re-alignment (`Party.svelte#nudge`).
     */
    fun nudge(deltaMs: Int) {
        val base = _state.value.trimMs + trimDraft
        trimDraft = (base + deltaMs).coerceIn(-500, 1000) - _state.value.trimMs
        _state.value = _state.value.copy(trimMs = service.prefs().trimMs.toInt() + trimDraft)
        trimJob?.cancel()
        trimJob = scope.launch {
            if (trimDebounceMs > 0) delay(trimDebounceMs)
            service.setTrim(service.prefs().trimMs + trimDraft.toDouble())
            trimDraft = 0
            _state.value = _state.value.copy(trimMs = service.prefs().trimMs.toInt())
        }
    }

    /** The playhead, seconds: a HOT read for the bar (`positionNow` in Rust). */
    fun positionNow(): Double = service.positionNow()

    fun consumeMessage() {
        if (_state.value.message != null) _state.value = _state.value.copy(message = null)
    }

    /**
     * The screen is going away. The guest KEEPS playing (the note's rule: a party
     * outlives its screen, like the player's queue) — only an explicit [leave]
     * ends it. What stops here is this screen's own work, and a *Décalage* the
     * person tapped just before leaving is APPLIED rather than dropped: a value
     * they set is a setting, and a debounce is not a reason to lose it
     * (`Party.svelte#nudge` applies on the last tap, never on unload).
     */
    fun close() {
        unsubscribe()
        trimJob?.cancel()
        if (trimDraft != 0) {
            service.setTrim(service.prefs().trimMs + trimDraft.toDouble())
            trimDraft = 0
            _state.value = _state.value.copy(trimMs = service.prefs().trimMs.toInt())
        }
    }

    private fun subscribe() {
        if (subscribed) return
        subscribed = true
        subscribeJob = scope.launch {
            service.subscribe { data -> onPush(data) }
        }
    }

    private fun unsubscribe() {
        subscribed = false
        subscribeJob?.cancel()
        subscribeJob = null
    }

    private fun onPush(data: PartyStateData) {
        // The push never moves the playhead (a screen reads it hot), and it never
        // pulls a landed trim back: only what the room says changes.
        val phase = when (data.phase) {
            PartyPhase.IDLE, PartyPhase.LANDING -> _state.value.phase
            else -> data.phase
        }
        _state.value = _state.value.copy(
            data = data,
            phase = phase,
            volume = data.volume,
            trimMs = data.trimMs.toInt() + trimDraft,
        )
    }
}
