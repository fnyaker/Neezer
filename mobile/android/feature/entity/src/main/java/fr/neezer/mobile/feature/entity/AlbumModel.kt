package fr.neezer.mobile.feature.entity

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
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
import uniffi.nz_ffi.TrackRow

/**
 * The album page's state machine (lot MB7), `routes/Album.svelte` ported.
 *
 * - **The copy paints, the request replaces it** (`screenFlow`): the sleeve and
 *   the tracklist are there in the first frame on a revisit, and a dead link
 *   keeps them with « Hors ligne · version mémorisée ».
 * - **The provider's remainder is asked after the page**, once, and only for a
 *   library album (`#load`): its refusal is the page's own sentence
 *   (« Deezer est injoignable : la suite de l'album n'est pas affichée. »), never
 *   a failed album.
 * - **« Lire » plays the rows the page already has** — no request, exactly what
 *   `playQueue(data.tracks, 0, …)` does — and « Mélanger » is the same queue
 *   shuffled once (`shufflePlay`).
 * - **The heart answers before its request** and rolls back with a toast.
 */
class AlbumModel(
    private val service: AlbumService,
    private val player: EntityPlayer,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
) {
    data class State(
        /** The id the page was opened with (the key of its copy). */
        val id: String = "",
        val screen: ScreenState<AlbumView> = ScreenState.Loading,
        /** What the provider has and the library does not (« Disponible sur Deezer »). */
        val onDeezer: List<TrackRow> = emptyList(),
        val onDeezerBusy: Boolean = false,
        val onDeezerFetched: Boolean = false,
        val deezerDown: Boolean = false,
        /** The heart, flipped before its request. */
        val favorite: Boolean? = null,
    ) {
        val data: AlbumView? get() = (screen as? ScreenState.Ready)?.data

        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true

        val isFavorite: Boolean get() = favorite ?: data?.favorite ?: false

        /** The band's facts, as `Album.svelte#facts` builds them. */
        val facts: String
            get() {
                val page = data ?: return ""
                val parts = mutableListOf("Album")
                page.year?.takeIf { it.isNotBlank() }?.let(parts::add)
                parts += "${page.nbTracks} titre" + if (page.nbTracks > 1) "s" else ""
                parts += minutesSeconds(page.totalS)
                return parts.joinToString(" · ")
            }
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null
    private var deezerJob: Job? = null

    fun load(id: String) {
        if (id.isBlank()) return
        if (loading?.isActive == true && _state.value.id == id) return
        loading?.cancel()
        deezerJob?.cancel()
        _state.value = State(id = id)
        loading = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedAlbum(id) } },
                fresh = { withContext(io) { service.album(id) } },
            ).collect { screen ->
                _state.update { it.copy(screen = screen) }
                if (screen is ScreenState.Ready && screen.data.library) loadOnDeezer(id)
            }
        }
    }

    fun retry() {
        val failed = _state.value.screen as? ScreenState.Failed ?: return
        failed.retry()
    }

    private fun loadOnDeezer(id: String) {
        if (_state.value.onDeezerFetched || deezerJob?.isActive == true) return
        _state.update { it.copy(onDeezerBusy = true) }
        deezerJob = scope.launch {
            try {
                val rows = withContext(io) { service.albumOnDeezer(id) }
                _state.update {
                    it.copy(onDeezer = rows, onDeezerBusy = false, onDeezerFetched = true)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                _state.update {
                    it.copy(onDeezerBusy = false, onDeezerFetched = true, deezerDown = true)
                }
            }
        }
    }

    /** The heart: the icon answers now, the request follows, a refusal rolls back. */
    fun toggleFavorite() {
        val page = _state.value.data ?: return
        val next = !_state.value.isFavorite
        _state.update { it.copy(favorite = next) }
        val id = _state.value.id
        val artist = page.artistName.ifBlank { null }
        val artistId = page.artistId.ifBlank { null }
        scope.launch {
            try {
                withContext(io) {
                    service.setFavorite("album", id, next, page.title, artist, artistId)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(favorite = !next) }
                toast("Impossible de mettre à jour le favori : ${NzErrors.text(e)}. Réessayer.")
            }
        }
    }

    /** « Lire »: the album, from its first title. No request. */
    fun playAll() {
        val tap = System.nanoTime()
        val page = _state.value.data ?: return
        val id = _state.value.id
        scope.launch {
            val rows = withContext(io) { page.tracks.allRows() }
            if (rows.isNotEmpty()) player.play(rows, 0, CONTEXT_ALBUM, id, tap)
        }
    }

    /** « Mélanger »: the same queue, shuffled once (`stores.js#shufflePlay`). */
    fun playShuffled() {
        val tap = System.nanoTime()
        val page = _state.value.data ?: return
        val id = _state.value.id
        scope.launch {
            val rows = withContext(io) { page.tracks.allRows() }
            if (rows.isNotEmpty()) player.playShuffled(rows, CONTEXT_ALBUM, id, tap)
        }
    }

    /** A row: the album, from there (`TrackList`'s context rule). */
    fun playFrom(row: TrackRow) {
        val tap = System.nanoTime()
        val page = _state.value.data ?: return
        val id = _state.value.id
        scope.launch {
            val rows = withContext(io) { page.tracks.allRows() }
            if (rows.isEmpty()) return@launch
            val start = rows.indexOfFirst { it.key == row.key }.coerceAtLeast(0)
            player.play(rows, start, CONTEXT_ALBUM, id, tap)
        }
    }

    /** A row of the provider's remainder: the title alone, then its radio. */
    fun playMissing(row: TrackRow) {
        player.play(listOf(row), 0, CONTEXT_RADIO, row.id, System.nanoTime())
    }

    /** Swipe right on a row: « Lire ensuite ». */
    fun queueNext(row: TrackRow) {
        player.playNext(row)
    }

    companion object {
        const val CONTEXT_ALBUM = "album"
        const val CONTEXT_RADIO = "radio"
    }
}
