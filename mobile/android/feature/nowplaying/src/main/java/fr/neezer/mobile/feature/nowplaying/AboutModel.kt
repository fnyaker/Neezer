package fr.neezer.mobile.feature.nowplaying

import fr.neezer.mobile.designsystem.NzRowSource
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
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.TrackCreditsData
import uniffi.nz_ffi.TrackRow

/**
 * What lives under the full-screen player — `NowPlayingMore.svelte`, for music.
 *
 * Two sections, each ONE request and each failing on its own (the SPA's rule:
 * a section that cannot load is simply absent, and the panel is never a failed
 * page):
 *
 * - **Crédits** — MB7's `/api/track/<id>/credits`. The panel shows the roles and
 *   their people; a name opens the artist page the library holds, else that
 *   person's own credits page (`TrackCredits.svelte#href`'s rule, which MB7
 *   already computed into `artistId`/`credited`).
 * - **Titres similaires** — `/api/radio/track/<id>`, the seed included as the
 *   server sends it. The playing row is filtered out of what is DRAWN (the SPA's
 *   own filter), and « Tout lire » starts the list as a radio.
 *
 * The artist's own card (`À propos de l'artiste`, its bio and its popular
 * titles) is NOT duplicated here: it is MB7's artist page, and the panel offers
 * the one tap that opens it. A card that repeats a page is how two screens start
 * disagreeing.
 *
 * An episode has no such panel yet: its chapters, description and transcript are
 * MB13's (`PodcastEpisodePanel`), and until that lot lands the player offers no
 * « À propos » for a podcast (a door that opens nothing is a bug).
 */
class AboutModel(
    private val service: NowPlayingService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) {
    data class State(
        val key: String? = null,
        val credits: TrackCreditsData? = null,
        /** The rows of the radio the server answered, minus the playing seed. */
        val similar: List<TrackRow> = emptyList(),
        /** « Titres similaires » failed: the section says so, with a retry. */
        val similarFailed: Boolean = false,
        val loading: Boolean = false,
        /** « Tout lire » is in flight (a second press is the same press). */
        val playing: Boolean = false,
    ) {
        val hasCredits: Boolean get() = credits?.let { !it.empty } == true

        /** Nothing at all to show: « Rien à afficher pour ce titre ». */
        val empty: Boolean get() = !hasCredits && similar.isEmpty() && !loading && !similarFailed
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var job: Job? = null

    /** The source the panel's windowed list reads (a page of 64 at a time). */
    var source: NzRowSource? = null
        private set

    /** Reads both sections of `key`'s panel. One call per opening. */
    fun load(key: String, seedKey: String) {
        if (key.isBlank()) return
        job?.cancel()
        _state.update {
            it.copy(
                key = key,
                credits = null,
                similar = emptyList(),
                similarFailed = false,
                loading = true,
            )
        }
        job = scope.launch {
            withContext(io) {
                // The credits: MB7's own cache first, then its one request.
                val cached = runCatching { service.cachedCredits(key) }.getOrNull()
                if (cached != null) {
                    _state.update { it.copy(credits = cached) }
                }
                runCatching { service.credits(key) }
                    .onSuccess { fresh -> _state.update { it.copy(credits = fresh) } }
            }
            val source = withContext(io) { runCatching { service.similarTracks(key) }.getOrNull() }
            if (source == null) {
                _state.update { it.copy(loading = false, similarFailed = true) }
                return@launch
            }
            // One page is the section: a radio is bounded by the server
            // (`_RADIO_SIZE`), so it crosses once, and the seed the server put
            // first is what the person is hearing — the SPA filters it out.
            val rows = withContext(io) { source.window(0, minOf(source.len(), 128)) }
                .filter { it.key != seedKey }
            this@AboutModel.source = source
            _state.update { it.copy(loading = false, similar = rows, similarFailed = false) }
        }
    }

    /** « Réessayer » on the similar section alone. */
    fun retry(key: String, seedKey: String) {
        load(key, seedKey)
    }

    fun clear() {
        job?.cancel()
        source = null
        _state.value = State()
    }

    /** « Tout lire »: the similar titles become the queue, as a radio of their own. */
    fun playAll(player: PlayerPort, contextId: String) {
        val rows = _state.value.similar
        if (rows.isEmpty() || _state.value.playing) return
        _state.update { it.copy(playing = true) }
        player.command(
            PlayerCommand.PlayQueue(
                tracks = rows.map(::ref),
                start = 0u,
                context = uniffi.nz_ffi.PlayContext(kind = "similar", id = contextId),
            ),
        )
        scope.launch {
            kotlinx.coroutines.delay(400)
            _state.update { it.copy(playing = false) }
        }
    }

    /** « Ajouter à la file »: the same rows, after what is playing. */
    fun queueNext(player: PlayerPort) {
        val rows = _state.value.similar
        if (rows.isEmpty()) return
        player.command(PlayerCommand.AddToQueue(tracks = rows.map(::ref)))
    }

    private fun ref(row: TrackRow) = uniffi.nz_ffi.TrackRef(
        key = row.key,
        id = row.id,
        title = row.title,
        artists = row.artists,
        album = row.album,
        albumId = row.albumId,
        artistId = null,
        cover = row.cover,
        durationS = row.durationS,
        episode = false,
        liked = row.liked,
    )
}
