package fr.neezer.mobile.feature.entity

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
import uniffi.nz_ffi.TrackCreditsData
import uniffi.nz_ffi.TrackRow

/**
 * `credits.py#PAGE_FROM`: a name links to its own credits page once it is
 * credited on this many titles — one is the title already on screen.
 */
const val PERSON_PAGE_FROM = 2

/** One row of a track's credits, as `TrackCredits.svelte` groups them. */
data class CreditsGroupView(
    val role: String,
    val label: String,
    val people: List<CreditsPersonView>,
)

/** One person, and where a tap on their name can go. */
data class CreditsPersonView(
    val name: String,
    /** The artist page to open, when the library holds one. */
    val artistId: String?,
    /** Titles credited: at two or more the name also opens its own credits page. */
    val credited: Int?,
) {
    /** `TrackCredits.svelte#href`: an artist page, else a credits page, else text. */
    val opensArtist: Boolean get() = !artistId.isNullOrBlank()

    val opensCredits: Boolean get() = !opensArtist && (credited ?: 0) >= PERSON_PAGE_FROM
}

/** A track's credits, as the sheet and the player's panel draw them. */
data class TrackCreditsView(
    val groups: List<CreditsGroupView>,
    val isrc: String?,
    val released: String?,
    val label: String?,
    val copyright: String?,
    val album: String?,
    /** Nothing at all: « Aucun crédit connu pour ce titre ». */
    val empty: Boolean,
) {
    /** The release line: album, date, label, copyright — joined, SPA order. */
    val releaseLine: List<String>
        get() = listOfNotNull(
            album?.takeIf { it.isNotBlank() },
            released?.takeIf { it.isNotBlank() }?.let(::frenchDate),
            label?.takeIf { it.isNotBlank() },
            copyright?.takeIf { it.isNotBlank() },
        )

    companion object {
        fun of(data: TrackCreditsData): TrackCreditsView = TrackCreditsView(
            groups = data.credits.map { group ->
                CreditsGroupView(
                    role = group.role,
                    label = group.label,
                    people = group.people.map { person ->
                        CreditsPersonView(
                            name = person.name,
                            artistId = person.artistId,
                            credited = person.credited?.toInt(),
                        )
                    },
                )
            },
            isrc = data.info.isrc,
            released = data.info.released,
            label = data.info.label,
            copyright = data.info.copyright,
            album = data.info.album,
            empty = data.empty,
        )
    }
}

/** `TrackCredits.svelte#formatDate`: « 2001-03-12 » → « 12 mars 2001 ». */
internal fun frenchDate(iso: String): String {
    val match = Regex("^(\\d{4})-(\\d{2})-(\\d{2})$").find(iso.trim()) ?: return iso
    val (year, month, day) = match.destructured
    val name = MONTHS.getOrNull(month.toIntOrNull()?.minus(1) ?: -1) ?: return iso
    return "${day.toIntOrNull() ?: day} $name $year"
}

private val MONTHS = listOf(
    "janvier", "février", "mars", "avril", "mai", "juin",
    "juillet", "août", "septembre", "octobre", "novembre", "décembre",
)

/**
 * One track's credits, for the sheet the track menu and the player's panel open
 * (`components/CreditsSheet.svelte`).
 *
 * The SAME request as the panel, kept on the device, and painted from the copy
 * first: opening the sheet on a title already looked at costs no request. A
 * failure is the sheet's own sentence with « Réessayer », never a spinner.
 */
class TrackCreditsModel(
    private val service: CreditsService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) {
    data class State(
        /** The title the sheet is about; `null` closes it. */
        val track: TrackRow? = null,
        val screen: ScreenState<TrackCreditsView> = ScreenState.Loading,
    ) {
        val data: TrackCreditsView? get() = (screen as? ScreenState.Ready)?.data
        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null

    /** Opens the sheet on [track] (the SPA's `$: if (creditsSheet && key !== key) load`). */
    fun load(track: TrackRow) {
        // The SAME title already on screen is the same answer: the SPA only
        // reloads when the KEY changes, so a menu opened twice on one row costs
        // nothing the second time.
        if (_state.value.track?.key == track.key) return
        loading?.cancel()
        _state.value = State(track = track)
        val id = track.id.ifBlank { track.key }
        loading = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedTrackCredits(id) }?.let(TrackCreditsView::of) },
                fresh = { withContext(io) { TrackCreditsView.of(service.trackCredits(id)) } },
            ).collect { screen -> _state.update { it.copy(screen = screen) } }
        }
    }

    /** Closes the sheet: the next open starts clean (the SPA's `$: if (!creditsSheet)`). */
    fun clear() {
        loading?.cancel()
        _state.value = State()
    }

    fun retry() {
        val failed = _state.value.screen as? ScreenState.Failed ?: return
        failed.retry()
    }
}

/**
 * A person's credits page (`routes/Credits.svelte` + `components/PersonCredits.svelte`).
 *
 * ONE request, whatever the number of roles; a chip then narrows the list the
 * page already has (0 requests). « Tout lire » plays what is shown.
 */
class PersonModel(
    private val service: CreditsService,
    private val player: EntityPlayer,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
) {
    data class State(
        val name: String = "",
        val screen: ScreenState<PersonView> = ScreenState.Loading,
        /** The role the chips have chosen ("" = every role). */
        val role: String = "",
        /** The list the chosen role shows. */
        val shown: NzRowSource? = null,
    ) {
        val data: PersonView? get() = (screen as? ScreenState.Ready)?.data
        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true
        val tracks: NzRowSource? get() = shown ?: data?.tracks

        /** The header's subtitle: « 3 titres de la bibliothèque ». */
        val countLabel: String
            get() {
                val count = data?.count ?: 0
                if (count <= 0) return ""
                return "$count titre" + (if (count > 1) "s" else "")
            }
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null
    private var roleJob: Job? = null

    fun load(name: String) {
        if (name.isBlank()) return
        if (loading?.isActive == true && _state.value.name == name) return
        loading?.cancel()
        roleJob?.cancel()
        _state.value = State(name = name)
        loading = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedPersonCredits(name) } },
                fresh = { withContext(io) { service.personCredits(name) } },
            ).collect { screen ->
                _state.update {
                    // A fresh answer keeps the role the person chose: the chips do
                    // not jump back to « Tout » when the copy is replaced.
                    it.copy(
                        screen = screen,
                        shown = if (it.role.isEmpty()) {
                            (screen as? ScreenState.Ready)?.data?.tracks
                        } else {
                            it.shown ?: (screen as? ScreenState.Ready)?.data?.tracks
                        },
                    )
                }
            }
        }
    }

    fun retry() {
        val failed = _state.value.screen as? ScreenState.Failed ?: return
        failed.retry()
    }

    /** A role chip: the list the page already has, narrowed — no request. */
    fun selectRole(role: String) {
        val name = _state.value.name
        if (role == _state.value.role) return
        if (role.isEmpty()) {
            _state.update { it.copy(role = "", shown = it.data?.tracks) }
            return
        }
        _state.update { it.copy(role = role) }
        roleJob?.cancel()
        roleJob = scope.launch {
            val source = withContext(io) {
                runCatching { service.personCreditsRole(name, role) }.getOrNull()
            }
            _state.update { if (it.role == role) it.copy(shown = source ?: it.shown) else it }
        }
    }

    /** « Tout lire »: what is shown, from the first title. */
    fun playAll() {
        val tap = System.nanoTime()
        val source = _state.value.tracks ?: return
        val name = _state.value.name
        scope.launch {
            val rows = withContext(io) { source.allRows() }
            if (rows.isNotEmpty()) player.play(rows, 0, CONTEXT_CREDITS, name, tap)
        }
    }

    /** A row: what is shown, from there. */
    fun playFrom(row: TrackRow) {
        val tap = System.nanoTime()
        val source = _state.value.tracks ?: return
        val name = _state.value.name
        scope.launch {
            val rows = withContext(io) { source.allRows() }
            if (rows.isEmpty()) return@launch
            val start = rows.indexOfFirst { it.key == row.key }.coerceAtLeast(0)
            player.play(rows, start, CONTEXT_CREDITS, name, tap)
        }
    }

    /** Swipe right on a row: « Lire ensuite ». */
    fun queueNext(row: TrackRow) {
        player.playNext(row)
    }

    companion object {
        const val CONTEXT_CREDITS = "credits"
    }
}
