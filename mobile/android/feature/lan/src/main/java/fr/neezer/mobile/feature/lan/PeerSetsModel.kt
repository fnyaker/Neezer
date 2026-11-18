package fr.neezer.mobile.feature.lan

import fr.neezer.mobile.bridge.NzErrors
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.PeerSetFfi

/**
 * « Sur vos appareils »'s state machine (lot LN14): the sets the connected peers
 * expose, multi-selectable, and « Télécharger » (rule 2: it becomes an ordinary
 * `nz-offline` job, with its progress and its causes on the downloads screen).
 *
 * The list is a MEMORY read (`Lan::sets()` is a short lock, 0 requests): a peer's
 * sets are replicated when the connection authenticates, so opening this tab
 * costs nothing on the wire and paints in one frame.
 *
 * A key is `"<peer>:<kind>:<ident>"`, the exact shape LN13's FFI validates before
 * anything is asked of a peer — nothing a screen sends becomes a path or a
 * request line.
 */
class PeerSetsModel(
    private val port: LanPort,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = {},
) {
    data class State(
        val sets: List<PeerSetFfi> = emptyList(),
        /** The keys the person ticked. */
        val selected: Set<String> = emptySet(),
        /** How many peers answered (the empty state's sentence). */
        val peers: Int = 0,
        val busy: Boolean = false,
        /** A refusal's cause, with « Réessayer » (rule 3). */
        val notice: String? = null,
        /** What the last « Télécharger » did, in one sentence (rule 2). */
        val note: String? = null,
    ) {
        /** The peers, in the order their first set appears. */
        val groups: List<Pair<PeerSetFfi, List<PeerSetFfi>>>
            get() = sets.groupBy { it.peer to it.peerName }
                .map { (peer, rows) -> rows.first() to rows }

        val canDownload: Boolean
            get() = selected.isNotEmpty() && !busy
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** Reads the replica; called when the tab appears and on every node change. */
    fun reload() {
        val rows = runCatching { port.peerSets() }.getOrDefault(emptyList())
        _state.update { current ->
            current.copy(
                sets = rows,
                peers = rows.map(PeerSetFfi::peer).distinct().size,
                // A set a peer no longer exposes leaves the selection with it.
                selected = current.selected.intersect(rows.map(::keyOf).toSet()),
            )
        }
    }

    /** One tap: tick or untick. Optimistic by nature (nothing leaves the phone). */
    fun toggle(key: String) {
        _state.update {
            it.copy(selected = if (key in it.selected) it.selected - key else it.selected + key)
        }
    }

    /** Every set of one peer, or none of it (the group's own control). */
    fun togglePeer(peer: String) {
        val keys = _state.value.sets.filter { it.peer == peer }.map(::keyOf)
        val all = keys.all { it in _state.value.selected }
        _state.update {
            it.copy(selected = if (all) it.selected - keys.toSet() else it.selected + keys)
        }
    }

    /** « Télécharger »: queues the ticked sets as ordinary downloads. */
    fun download() {
        val keys = _state.value.selected.toList()
        if (keys.isEmpty() || _state.value.busy) return
        _state.update { it.copy(busy = true, notice = null, note = null) }
        scope.launch {
            try {
                withContext(io) { port.downloadSets(keys) }
                val sentence = if (keys.size == 1) {
                    "1 liste ajoutée aux téléchargements"
                } else {
                    "${keys.size} listes ajoutées aux téléchargements"
                }
                _state.update { it.copy(selected = emptySet(), note = sentence) }
                toast(sentence)
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) throw e
                val cause = NzErrors.text(e)
                _state.update { it.copy(notice = cause) }
                toast(cause)
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }

    fun dismissNotice() {
        _state.update { it.copy(notice = null, note = null) }
    }

    companion object {
        /** The one spelling `LanApi::download_sets` takes. */
        fun keyOf(set: PeerSetFfi): String = "${set.peer}:${set.kind}:${set.ident}"
    }
}
