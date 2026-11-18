package fr.neezer.mobile.feature.nowplaying

import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.designsystem.NzRowSource
import fr.neezer.mobile.designsystem.asRowSource
import uniffi.nz_ffi.AudioSettings
import uniffi.nz_ffi.CardLineData
import uniffi.nz_ffi.CardSelectionData
import uniffi.nz_ffi.LyricsData
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.PositionInfo
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.SleepTimer
import uniffi.nz_ffi.TrackCreditsData

/**
 * What the player's views ask of the ENGINE — MB3b's `PlayerApi`, behind a seam.
 *
 * Every rule of the transport (the queue, the advance, the crossfade, the sleep
 * timer's own deadline) is the player's and is not repeated here; a screen sends
 * a command and draws the snapshot it was handed. The seam exists so
 * `NowPlayingModelTest` proves every decision on the JVM with a fake — no JNA, no
 * `.so`, no device — and so the real one is the three thin lines at the bottom of
 * this file.
 */
interface PlayerPort {
    /** The whole picture, pushed on every CHANGE and never on a timer. */
    fun snapshot(): PlayerSnapshot

    /**
     * The HOT read: atomics only. Called once per visible frame by the progress
     * bar — and NEVER while nothing draws it (the ticker counts its calls; the
     * acceptance holds "0 FFI calls with the screen off").
     */
    fun position(): PositionInfo

    /**
     * Sends one command and returns at once. [tapNs] is the finger's instant
     * (`System.nanoTime()`), so "tap → sound" is read from `output_stats()`
     * instead of timed by a test.
     */
    fun command(cmd: PlayerCommand, tapNs: Long = System.nanoTime())

    /**
     * The queue as the design system's windowed SOURCE (`ListHandle.asRowSource`):
     * a fresh one per `onQueueChanged`, so 4 000 rows never cross whole and the
     * screen pages 64 at a time (`ListWindow`).
     */
    fun queueSource(): NzRowSource?

    fun audio(): AudioSettings

    fun setAudio(settings: AudioSettings)

    /** `null` cancels the timer; the player owns the deadline and the fade. */
    fun setSleep(timer: SleepTimer?)
}

/** A port that does nothing, for a process whose core could not be built. */
object NoPlayerPort : PlayerPort {
    override fun snapshot(): PlayerSnapshot = PlayerSnapshot(
        state = uniffi.nz_ffi.PlayState.STOPPED,
        current = null,
        index = null,
        len = 0u,
        shuffle = false,
        repeat = uniffi.nz_ffi.Repeat.OFF,
        volume = 1f,
        muted = false,
        streaming = false,
        sleep = null,
        error = null,
        fault = null,
        version = 0uL,
    )

    override fun position(): PositionInfo = PositionInfo(0.0, 0.0, 0.0, false)

    override fun command(cmd: PlayerCommand, tapNs: Long) = Unit

    override fun queueSource(): NzRowSource? = null

    override fun audio(): AudioSettings = AudioSettings(
        quality = uniffi.nz_ffi.Quality.AUTO,
        crossfadeS = 0f,
        trimSilence = false,
        normalization = uniffi.nz_ffi.Normalization.OFF,
        eqDb = List(10) { 0f },
        eqPreset = null,
        bassDb = 0f,
        mono = false,
        balance = 0f,
        speed = 1f,
        meteredCap = uniffi.nz_ffi.Quality.AUTO,
    )

    override fun setAudio(settings: AudioSettings) = Unit

    override fun setSleep(timer: SleepTimer?) = Unit
}

/** `PlayerCommand.Play`/`Pause` from a boolean, everywhere the glyph is drawn. */
fun PlayerPort.setPlaying(playing: Boolean, tapNs: Long = System.nanoTime()) {
    command(if (playing) PlayerCommand.Pause else PlayerCommand.Play, tapNs)
}

/**
 * What the views ask of the CORE's screens (MB11's own `nowplaying.rs`, plus the
 * two reads MB7 and the player already answer): the words, the card's rules, a
 * podcast's rate, a track's credits and « Titres similaires ».
 *
 * Same reason as [PlayerPort]: the models are plain Kotlin over this seam.
 */
interface NowPlayingService {
    /** The words as the device has them — disk only, 0 requests. */
    suspend fun cachedLyrics(key: String): LyricsData?

    /** `GET /api/lyrics/<id>` — one request on the `User` lane. */
    suspend fun lyrics(key: String): LyricsData

    /** `GET /api/radio/track/<id>` — « Titres similaires », one request. */
    suspend fun similarTracks(key: String): NzRowSource?

    /** The lines a lyric card can offer. */
    fun cardLines(lyrics: LyricsData): List<CardLineData>

    /** Which line the card opens on (`-1` for none). */
    fun cardDefaultLine(lyrics: LyricsData, activeSource: Long): Int

    /** One tap on a line. */
    fun cardToggle(selected: List<Int>, index: Int): CardSelection

    /** The selected lines' text, in lyric order. */
    fun cardTexts(lyrics: LyricsData, selected: List<Int>): List<String>

    /** `neezer-<artist>-<title>.png`. */
    fun cardFilename(artist: String, title: String): String

    /** The rates the episode's speed menu offers. */
    fun speedChoices(): List<Float>

    /** The rate a show is played at (1 when nobody chose one). */
    fun speed(show: String): Float

    /** Remembers a rate for a show; 1 forgets it. */
    fun setSpeed(show: String, rate: Float)

    /**
     * The cached file of a sleeve, at most [sizePx]: what the lyric card draws,
     * and what its colours are read from. MB2 fetches it on the lane the screen's
     * own visibility deserves and returns a PATH — a card made offline is the
     * same card (rule 3).
     */
    suspend fun coverPath(url: String, sizePx: Int): String?

    /** MB7's `/api/track/<id>/credits` — the « À propos » panel's first section. */
    suspend fun cachedCredits(key: String): TrackCreditsData?

    suspend fun credits(key: String): TrackCreditsData
}

/** One tap's answer, in the model's own words (`min`/`max` become French copy). */
data class CardSelection(val indices: List<Int>, val reason: String?)

/** The real service: MB11's FFI, and MB7's credits read. */
class NzNowPlayingService(private val screens: Screens) : NowPlayingService {
    override suspend fun cachedLyrics(key: String): LyricsData? = screens.cachedLyrics(key)

    override suspend fun lyrics(key: String): LyricsData = screens.lyrics(key)

    override suspend fun similarTracks(key: String): NzRowSource? =
        screens.similarTracks(key).asRowSource()

    override fun cardLines(lyrics: LyricsData): List<CardLineData> = screens.cardLines(lyrics)

    override fun cardDefaultLine(lyrics: LyricsData, activeSource: Long): Int =
        screens.cardDefaultLine(lyrics, activeSource).toInt()

    override fun cardToggle(selected: List<Int>, index: Int): CardSelection {
        val answer: CardSelectionData = screens.cardToggle(
            selected.map { it.toUInt() },
            index.toUInt(),
        )
        return CardSelection(answer.indices.map { it.toInt() }, answer.reason)
    }

    override fun cardTexts(lyrics: LyricsData, selected: List<Int>): List<String> =
        screens.cardTexts(lyrics, selected.map { it.toUInt() })

    override fun cardFilename(artist: String, title: String): String =
        screens.cardFilename(artist, title)

    override fun speedChoices(): List<Float> = screens.podcastSpeedChoices()

    override fun speed(show: String): Float = screens.podcastSpeed(show)

    override fun setSpeed(show: String, rate: Float) {
        // A store that cannot be written is not a failed tap: the rate still
        // applies for this session, and the next pick tries again.
        runCatching { screens.setPodcastSpeed(show, rate) }
    }

    override suspend fun coverPath(url: String, sizePx: Int): String? =
        runCatching { screens.coverPath(url, sizePx.toUInt()) }.getOrNull()

    override suspend fun cachedCredits(key: String): TrackCreditsData? =
        screens.cachedTrackCredits(key)

    override suspend fun credits(key: String): TrackCreditsData = screens.trackCredits(key)
}

/** The core's screens, or `null` when the native library could not be loaded. */
object NowPlayingCore {
    fun screens(): Screens? = runCatching { NzBridge.core?.screens() }.getOrNull()

    fun player(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()

    fun service(): NowPlayingService? = screens()?.let(::NzNowPlayingService)

    /** The French sentence for a failure: `nz_error_text`, never a code. */
    fun cause(e: Throwable): String = NzErrors.text(e)
}
