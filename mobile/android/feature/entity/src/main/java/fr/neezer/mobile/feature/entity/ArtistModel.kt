package fr.neezer.mobile.feature.entity

import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import fr.neezer.mobile.designsystem.NzRowSource
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
import uniffi.nz_ffi.AlbumCardData
import uniffi.nz_ffi.ArtistData
import uniffi.nz_ffi.DiscographyData
import uniffi.nz_ffi.TrackRow

/** The three tabs of `routes/Artist.svelte`. */
enum class ArtistTab { Overview, Tracks, Credits }

/**
 * The artist page's state machine (lot MB7).
 *
 * Every rule `routes/Artist.svelte` taught is here, in the order a person meets
 * them, and the whole thing is plain Kotlin over [ArtistService],
 * [CreditsService] and [EntityPlayer] — `EntityModelTest` proves each one on the
 * JVM with a fake: no JNA, no `.so`, no device.
 *
 * - **The copy paints, the request replaces it** (`screenFlow`): a warm open
 *   shows the device's page in the first frame; a dead link keeps it and the
 *   screen says « Hors ligne · version mémorisée »; with nothing, the cause and
 *   « Réessayer ».
 * - **Each part is asked ONCE, after the page paints**: the discography and the
 *   provider's half are separate requests the SPA fires after `data` is on
 *   screen (`#load`), so a slow provider never delays the header — and the
 *   double `Ready` emission of `screenFlow` (copy, then fresh) must not ask
 *   twice (`Artist.svelte`'s own trap, re-paid by MB5's home).
 * - **The « Titres » tab is lazy** (`#loadTracks`): one request the first time it
 *   opens, never on the page's own load.
 * - **The heart answers before its request** and rolls back with a toast
 *   (`toggleEntityFavorite`).
 * - **A tap plays**: the popular shelf is the title then its radio
 *   (`playThenRadio`), the « Titres » list plays from the tapped row, and the
 *   queue itself is the player's.
 */
class ArtistModel(
    private val service: ArtistService,
    private val credits: CreditsService,
    private val player: EntityPlayer,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
) {
    data class State(
        /** The id the page was opened with — the SPA's route argument, and the
         * key its copy is kept under. */
        val id: String = "",
        val screen: ScreenState<ArtistData> = ScreenState.Loading,
        val tab: ArtistTab = ArtistTab.Overview,
        /** The releases, once the discography (or the copy of it) answered. */
        val discography: DiscographyData? = null,
        /** What only the provider has, and whether it refused. */
        val onDeezer: List<AlbumCardData> = emptyList(),
        val onDeezerBusy: Boolean = false,
        val onDeezerFetched: Boolean = false,
        val deezerDown: Boolean = false,
        /** The « Titres » tab: lazy, and its own failure with « Réessayer ». */
        val tracks: ScreenState<NzRowSource> = ScreenState.Loading,
        /** The « Crédits » tab: the person's page, and the role shown. */
        val credits: ScreenState<PersonView> = ScreenState.Loading,
        val creditsRole: String = "",
        val creditsTracks: NzRowSource? = null,
        /** The heart, flipped before its request (`null`: the server's answer). */
        val favorite: Boolean? = null,
        val radioBusy: Boolean = false,
    ) {
        /** The page, when there is one to draw. */
        val data: ArtistData? get() = (screen as? ScreenState.Ready)?.data

        /** What is drawn came from the device and the refresh failed. */
        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true

        /** The heart's state: the optimistic flip wins over the server's answer. */
        val isFavorite: Boolean get() = favorite ?: data?.favorite ?: false

        /** The « Crédits » tab exists only when the artist is credited somewhere. */
        val hasCredits: Boolean get() = (data?.credited ?: 0u) > 0u

        /** The albums grid: the discography's group, else the page's own albums. */
        val albums: List<AlbumCardData>
            get() = discography?.albums?.takeIf { it.isNotEmpty() } ?: data?.albums.orEmpty()

        /** « Dernières sorties »: five, newest first. */
        val latest: List<AlbumCardData> get() = discography?.latest.orEmpty()

        val singles: List<AlbumCardData> get() = discography?.singles.orEmpty()
        val featured: List<AlbumCardData> get() = discography?.featured.orEmpty()

        /** The « Titres » tab's list, once it answered. */
        val trackSource: NzRowSource? get() = (tracks as? ScreenState.Ready)?.data

        /** How many titles the artist has, once the tab has been opened (the
         * facts line prefers it over the fan count, `Artist.svelte#facts`). */
        val trackCount: Int? get() = trackSource?.len()

        /** The credits page, when there is one to draw. */
        val creditsPage: PersonView? get() = (credits as? ScreenState.Ready)?.data

        /** The titles the credits tab shows, for the role chosen. */
        val creditsSource: NzRowSource? get() = creditsTracks ?: creditsPage?.tracks
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null
    private var discographyJob: Job? = null
    private var deezerJob: Job? = null
    private var tracksJob: Job? = null
    private var creditsJob: Job? = null

    /** Reads the page: the device's copy first, then the SPA's requests. */
    fun load(id: String) {
        if (id.isBlank()) return
        if (loading?.isActive == true && _state.value.id == id) return
        loading?.cancel()
        discographyJob?.cancel()
        deezerJob?.cancel()
        tracksJob?.cancel()
        creditsJob?.cancel()
        _state.value = State(id = id)
        loading = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedArtist(id) } },
                fresh = { withContext(io) { service.artist(id) } },
            ).collect { screen ->
                _state.update { it.copy(screen = screen) }
                if (screen is ScreenState.Ready) {
                    loadDiscography(id)
                    if (screen.data.library) loadOnDeezer(id)
                }
            }
        }
    }

    /** « Réessayer », from a failed page. */
    fun retry() {
        val failed = _state.value.screen as? ScreenState.Failed ?: return
        failed.retry()
    }

    /**
     * The releases: the copy paints, then `/discography` answers. Guarded on the
     * JOB and on what is already there, or the two `Ready` emissions of a warm
     * open would ask it twice.
     */
    private fun loadDiscography(id: String) {
        if (_state.value.discography != null || discographyJob?.isActive == true) return
        discographyJob = scope.launch {
            runCatching { withContext(io) { service.cachedDiscography(id) } }
                .getOrNull()
                ?.let { cached -> _state.update { it.copy(discography = cached) } }
            runCatching { withContext(io) { service.discography(id, KIND_GROUPS) } }
                .onSuccess { fresh -> _state.update { it.copy(discography = fresh) } }
                .onFailure { cause ->
                    // The page's own albums stand in: the SPA's `disco` stays null
                    // and nothing on screen apologises (the shelf simply falls
                    // back to `data.albums`).
                    if (cause is CancellationException) throw cause
                }
        }
    }

    /**
     * « Disponible sur Deezer »: asked once, after the page is on screen, and
     * only for a library artist. A refusal is a sentence on the page, never a
     * failed page (the provider is optional, CLAUDE.md).
     */
    private fun loadOnDeezer(id: String) {
        if (_state.value.onDeezerFetched || deezerJob?.isActive == true) return
        _state.update { it.copy(onDeezerBusy = true) }
        deezerJob = scope.launch {
            try {
                val groups = withContext(io) { service.discography(id, KIND_ON_DEEZER) }
                _state.update {
                    it.copy(
                        onDeezer = groups.onDeezer,
                        onDeezerBusy = false,
                        onDeezerFetched = true,
                        deezerDown = false,
                    )
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

    /** A tab of the page. The list tabs load on their first open, never before. */
    fun selectTab(tab: ArtistTab) {
        _state.update { it.copy(tab = tab) }
        when (tab) {
            ArtistTab.Tracks -> loadTracks()
            ArtistTab.Credits -> loadCredits("")
            ArtistTab.Overview -> Unit
        }
    }

    /** The « Titres » tab: the copy paints, then `/artist/<id>/tracks` (once). */
    private fun loadTracks() {
        val id = _state.value.id
        if (id.isBlank()) return
        if (tracksJob?.isActive == true || _state.value.trackSource != null) return
        tracksJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedArtistTracks(id) } },
                fresh = { withContext(io) { service.artistTracks(id) } },
            ).collect { screen -> _state.update { it.copy(tracks = screen) } }
        }
    }

    /** « Réessayer » under the « Titres » tab. */
    fun retryTracks() {
        val failed = _state.value.tracks as? ScreenState.Failed ?: return
        failed.retry()
    }

    /**
     * The « Crédits » tab: the person's page (ONE request), then a chip narrows
     * the list it already has — no request (`PersonCredits.svelte#shown`).
     */
    fun loadCredits(role: String) {
        val name = _state.value.data?.name?.takeIf { it.isNotBlank() } ?: return
        if (role.isNotEmpty()) {
            scope.launch {
                val source = withContext(io) {
                    runCatching { credits.personCreditsRole(name, role) }.getOrNull()
                }
                _state.update { it.copy(creditsRole = role, creditsTracks = source ?: it.creditsTracks) }
            }
            return
        }
        if (creditsJob?.isActive == true || _state.value.creditsPage != null) return
        creditsJob = scope.launch {
            screenFlow(
                cached = { withContext(io) { credits.cachedPersonCredits(name) } },
                fresh = { withContext(io) { credits.personCredits(name) } },
            ).collect { screen ->
                _state.update {
                    it.copy(
                        credits = screen,
                        creditsTracks = (screen as? ScreenState.Ready)?.data?.tracks
                            ?: it.creditsTracks,
                    )
                }
            }
        }
    }

    /** « Réessayer » under the « Crédits » tab. */
    fun retryCredits() {
        val failed = _state.value.credits as? ScreenState.Failed ?: return
        failed.retry()
    }

    /** « Tout » on the credits tab: every role again, no request. */
    fun clearCreditsRole() {
        _state.update { it.copy(creditsRole = "", creditsTracks = it.creditsPage?.tracks) }
    }

    /** The heart: the icon answers now, the request follows, a refusal rolls back. */
    fun toggleFavorite() {
        val data = _state.value.data ?: return
        val id = _state.value.id
        val next = !_state.value.isFavorite
        _state.update { it.copy(favorite = next) }
        scope.launch {
            try {
                withContext(io) { service.setFavorite("artist", id, next, data.name) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(favorite = !next) }
                toast("Impossible de mettre à jour le suivi : ${NzErrors.text(e)}. Réessayer.")
            }
        }
    }

    // -- what the person taps --------------------------------------------------

    /** The popular shelf's row: the title alone, then its radio. */
    fun playTop(row: TrackRow) {
        player.play(listOf(row), 0, CONTEXT_RADIO, row.id, System.nanoTime())
    }

    /** « Lire »: the popular titles, in their order. */
    fun playTopAll() {
        val tap = System.nanoTime()
        val rows = _state.value.data?.top.orEmpty()
        if (rows.isNotEmpty()) player.play(rows, 0, CONTEXT_ARTIST, _state.value.id, tap)
    }

    /** « Tout lire » under the « Titres » tab: every title, from the first. */
    fun playAllTracks() {
        val tap = System.nanoTime()
        val source = _state.value.trackSource ?: return
        val id = _state.value.id
        scope.launch {
            val rows = withContext(io) { source.allRows() }
            if (rows.isNotEmpty()) player.play(rows, 0, CONTEXT_ARTIST, id, tap)
        }
    }

    /** A row of the « Titres » tab: the whole list, from there (`TrackList`'s rule). */
    fun playFromTracks(row: TrackRow) {
        val tap = System.nanoTime()
        val source = _state.value.trackSource ?: return
        val id = _state.value.id
        scope.launch {
            val rows = withContext(io) { source.allRows() }
            if (rows.isEmpty()) return@launch
            val start = rows.indexOfFirst { it.key == row.key }.coerceAtLeast(0)
            player.play(rows, start, CONTEXT_ARTIST, id, tap)
        }
    }

    /** Swipe right on a row: « Lire ensuite » (the queue is the player's). */
    fun queueNext(row: TrackRow) {
        player.playNext(row)
    }

    /**
     * « Radio »: ONE request for the first titles, then a context the player's own
     * refill continues (`radio:artist:<id>`).
     */
    fun startRadio() {
        val tap = System.nanoTime()
        val id = _state.value.id
        if (id.isBlank() || _state.value.radioBusy) return
        _state.update { it.copy(radioBusy = true) }
        scope.launch {
            try {
                val rows = withContext(io) { service.artistRadio(id) }
                if (rows.isEmpty()) {
                    toast("Rien à proposer pour cette radio.")
                } else {
                    player.play(rows, 0, CONTEXT_RADIO, "$CONTEXT_ARTIST:$id", tap)
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("La radio n'a pas pu démarrer : ${NzErrors.text(e)}. Réessayer.")
            } finally {
                _state.update { it.copy(radioBusy = false) }
            }
        }
    }

    /** A play of the credits tab's list: the titles it shows, from the first. */
    fun playCredits() {
        val tap = System.nanoTime()
        val source = _state.value.creditsSource ?: return
        val name = _state.value.data?.name.orEmpty()
        scope.launch {
            val rows = withContext(io) { source.allRows() }
            if (rows.isNotEmpty()) player.play(rows, 0, CONTEXT_CREDITS, name, tap)
        }
    }

    /** A row of the credits list: the list that is shown, from there. */
    fun playFromCredits(row: TrackRow) {
        val tap = System.nanoTime()
        val source = _state.value.creditsSource ?: return
        val name = _state.value.data?.name.orEmpty()
        scope.launch {
            val rows = withContext(io) { source.allRows() }
            if (rows.isEmpty()) return@launch
            val start = rows.indexOfFirst { it.key == row.key }.coerceAtLeast(0)
            player.play(rows, start, CONTEXT_CREDITS, name, tap)
        }
    }

    companion object {
        /** The `kind` `nz-ffi` reads as the release groups. */
        const val KIND_GROUPS = "all"

        /** The `kind` that asks the provider's half (`/artist/<id>/on-deezer`). */
        const val KIND_ON_DEEZER = "on-deezer"

        /** `Player.svelte`'s context kinds: what the queue's refill reads. */
        const val CONTEXT_ARTIST = "artist"
        const val CONTEXT_RADIO = "radio"
        const val CONTEXT_CREDITS = "credits"
    }
}
