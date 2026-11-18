package fr.neezer.mobile.feature.nowplaying

import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
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
import uniffi.nz_ffi.CardLineData
import uniffi.nz_ffi.LyricsData

/**
 * The words of the playing title, and the lyric card a person builds from them —
 * `webapp/src/lib/lyrics.js` and `lib/lyriccard.js`, ported.
 *
 * # When the words are asked for
 *
 * **Not on the track change: when the audio is under way.** `lib/lyrics.js`
 * subscribes to `playable`, not to `current`: nobody reads along before the song
 * starts, and a lyrics request racing its own stream spends the first second of a
 * mobile link on words the person is not looking at yet. [open] is therefore
 * called by the view from the player's own `PLAYING` state, once per title.
 *
 * # What the panel shows, in order
 *
 * 1. the device's copy (`cached_lyrics`, 0 requests) — a revisit paints at once;
 * 2. the fresh answer, replacing it;
 * 3. on a failure with a copy: the copy, marked « version mémorisée »;
 * 4. on a failure with nothing: the cause and « Réessayer » (`LoadFailed`).
 *
 * An answer with no words is a STATE, not a failure: « Paroles indisponibles
 * pour ce titre » — the same sentence the SPA prints once the track is playable.
 *
 * # The card's rules are Rust's
 *
 * Which lines it can offer, which one it opens on, what a tap does at the limit,
 * the order of the words and the file's name all come from
 * `nz_core::nowplaying::card` (the port of `lyriccard.js`, held by Rust tests).
 * This model only keeps the selection between two taps and turns a refusal into
 * the sentence the person reads.
 */
class LyricsModel(
    private val service: NowPlayingService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) {
    data class State(
        /** The title these words belong to (`null`: nothing asked yet). */
        val key: String? = null,
        val screen: ScreenState<LyricsData> = ScreenState.Loading,
        /** The synced line being HEARD, `-1` when nothing is synced or playing. */
        val activeLine: Int = -1,
        /** The card sheet is up. */
        val cardOpen: Boolean = false,
        val lines: List<CardLineData> = emptyList(),
        val selected: List<Int> = emptyList(),
        /** The card's refusal sentence (« Trois lignes au maximum… »). */
        val notice: String = "",
        val filename: String = "",
        val artist: String = "",
        val title: String = "",
    ) {
        val lyrics: LyricsData? get() = (screen as? ScreenState.Ready)?.data

        /** Nothing to read on this title: the panel says so instead of spinning. */
        val none: Boolean get() = lyrics?.empty == true

        /** What is drawn came off the device and the refresh failed. */
        val stale: Boolean get() = (screen as? ScreenState.Ready)?.stale == true

        /** The line the card would open on (an index in [lines], `-1` for none). */
        val cardDefault: Int get() = lines.indexOfFirst { it.source.toInt() == activeLine }
            .takeIf { it >= 0 } ?: if (lines.isEmpty()) -1 else 0

        val texts: List<String> get() = selected.sorted().mapNotNull { lines.getOrNull(it)?.text }
        val canCreate: Boolean get() = lines.isNotEmpty()

        /** The line being heard, as the player's own overlay shows it. */
        val activeText: String? get() {
            val list = lyrics?.synced ?: return null
            val line = list.getOrNull(activeLine) ?: return null
            return line.text.takeIf { it.isNotBlank() }
        }
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var loading: Job? = null

    /** The words of [key], once. A second call for the same title does nothing. */
    fun open(key: String, artist: String, title: String) {
        if (key.isBlank()) return
        if (_state.value.key == key && loading?.isActive == true) return
        if (_state.value.key == key && _state.value.screen is ScreenState.Ready) {
            // A warm panel: the words are already there, the card's list too.
            return
        }
        loading?.cancel()
        _state.update {
            it.copy(
                key = key,
                screen = ScreenState.Loading,
                activeLine = -1,
                cardOpen = false,
                lines = emptyList(),
                selected = emptyList(),
                notice = "",
                artist = artist,
                title = title,
            )
        }
        loading = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedLyrics(key) } },
                fresh = { withContext(io) { service.lyrics(key) } },
            ).collect { screen ->
                _state.update { it.copy(screen = screen) }
                if (screen is ScreenState.Ready) {
                    val lines = withContext(io) { service.cardLines(screen.data) }
                    val selected = withContext(io) {
                        val open = service.cardDefaultLine(screen.data, _state.value.activeLine.toLong())
                        if (open >= 0) listOf(open) else emptyList()
                    }
                    val filename = service.cardFilename(artist, title)
                    _state.update {
                        it.copy(lines = lines, selected = selected, filename = filename)
                    }
                }
            }
        }
    }

    /** The track changed: the previous song's words must not stay under it. */
    fun clear() {
        loading?.cancel()
        _state.value = State()
    }

    /** Resets the words when the title changes, keeping the card state closed. */
    fun onTrack(key: String?, artist: String, title: String, playable: Boolean) {
        if (key == null) {
            clear()
            return
        }
        if (key != _state.value.key) {
            _state.value = State(key = null, artist = artist, title = title)
            if (playable) open(key, artist, title) else Unit
        } else if (playable && _state.value.screen is ScreenState.Loading && loading?.isActive != true) {
            open(key, artist, title)
        }
    }

    /**
     * The line being heard, from the player's own position (the engine's clock is
     * what is RENDERED, so no look-ahead is subtracted here — the SPA had to,
     * because its clock was the `<audio>` element's, which its graph delayed).
     */
    fun onPosition(positionS: Double) {
        val lyrics = _state.value.lyrics ?: return
        if (lyrics.synced.isEmpty()) return
        val ms = positionS * 1000.0
        var index = -1
        for ((i, line) in lyrics.synced.withIndex()) {
            if (line.timeMs.toDouble() <= ms) index = i else break
        }
        if (index != _state.value.activeLine) {
            _state.update { it.copy(activeLine = index) }
        }
    }

    /** The card's own door: a tap opens it on the line being heard. */
    fun openCard() {
        val lines = _state.value.lines
        if (lines.isEmpty()) return
        val default = _state.value.cardDefault
        _state.update {
            it.copy(
                cardOpen = true,
                selected = if (default >= 0) listOf(default) else it.selected,
                notice = "",
            )
        }
    }

    fun closeCard() {
        _state.update { it.copy(cardOpen = false, notice = "") }
    }

    /** One tap on a line: Rust decides, this turns a refusal into a sentence. */
    fun toggleLine(index: Int) {
        val answer = service.cardToggle(_state.value.selected, index)
        val notice = when (answer.reason) {
            "max" -> "Trois lignes au maximum : retirez-en une."
            "min" -> "Gardez au moins une ligne."
            else -> ""
        }
        if (answer.reason != null) {
            _state.update { it.copy(notice = notice) }
            return
        }
        _state.update { it.copy(selected = answer.indices, notice = "") }
    }

    /** The card's texts, from Rust's own order rule (lyric order, at most three). */
    fun cardTexts(): List<String> {
        val lyrics = _state.value.lyrics ?: return emptyList()
        return service.cardTexts(lyrics, _state.value.selected)
    }
}
