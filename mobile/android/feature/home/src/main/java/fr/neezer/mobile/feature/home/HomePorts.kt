package fr.neezer.mobile.feature.home

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.FlowPress
import uniffi.nz_ffi.FlowStartData
import uniffi.nz_ffi.FlowStateData
import uniffi.nz_ffi.FlowStyleData
import uniffi.nz_ffi.HomeData
import uniffi.nz_ffi.PlayContext
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow

/**
 * What the home asks of the core (lot MB5).
 *
 * An interface, not the generated `Screens`, so the screen's logic is plain
 * Kotlin that `HomeModelTest` runs on the JVM with a fake — no JNA, no `.so`, no
 * device. [NzHomeService] is the one implementation, a one-to-one delegation to
 * `nz-ffi/src/home.rs`.
 */
interface HomeService {
    /** The device's copy of the home: disk only, no request. */
    suspend fun cachedHome(): HomeData?

    /** The seven requests of `Home.svelte#loadAll`, on the `User` lane. */
    suspend fun home(): HomeData

    /** The style chips' genres, from the device (no request). */
    suspend fun cachedFlowStyles(): List<FlowStyleData>

    /** `GET /flow/styles` — asked once per session, then kept. */
    suspend fun flowStyles(): List<FlowStyleData>

    /** The Flow's state for the controls and "up next": no request. */
    fun flowState(): FlowStateData

    /** The styles the Flow rests on — the device's own, no request. */
    fun setFlowStyles(styles: List<String>)

    /** The discovery level the device paints first. */
    fun discoveryLevel(): Double

    /** One notch up or down, locally (the caller re-aims a live Flow). */
    fun nudgeEnergy(direction: Int): Double?

    /** The device first, then `PUT /api/discover/prefs` (ONE request). */
    suspend fun setEnergy(notch: Double?): Double?

    /** The device first, then `PUT /api/discover/prefs` (ONE request). */
    suspend fun setDiscoveryLevel(level: Double): Double

    /** "Lancer le Flow": one `POST /flow/next` carrying the notch, the level and the styles. */
    suspend fun startFlow(): FlowStartData

    /** One press of the controls; answers the title to queue next. */
    suspend fun flowFeedback(press: FlowPress): TrackRow?

    /** `GET /api/moods/<id>` — one request per tap. */
    suspend fun moodTracks(id: String): List<TrackRow>

    /** "Autres suggestions": `/recommend` again, new seed. */
    suspend fun moreSuggestions(): List<TrackRow>
}

/** The one real implementation: this lot's `nz-ffi` methods. */
class NzHomeService(private val screens: Screens) : HomeService {
    override suspend fun cachedHome(): HomeData? = screens.cachedHome()

    override suspend fun home(): HomeData = screens.home()

    override suspend fun cachedFlowStyles(): List<FlowStyleData> = screens.cachedFlowStyles()

    override suspend fun flowStyles(): List<FlowStyleData> = screens.flowStyles()

    override fun flowState(): FlowStateData = screens.homeFlowState()

    override fun setFlowStyles(styles: List<String>) = screens.setFlowStyles(styles)

    override fun discoveryLevel(): Double = screens.discoveryLevel()

    override fun nudgeEnergy(direction: Int): Double? = screens.nudgeEnergy(direction)

    override suspend fun setEnergy(notch: Double?): Double? = screens.setEnergy(notch)

    override suspend fun setDiscoveryLevel(level: Double): Double = screens.setDiscoveryLevel(level)

    override suspend fun startFlow(): FlowStartData = screens.startFlow()

    override suspend fun flowFeedback(press: FlowPress): TrackRow? = screens.flowFeedback(press)

    override suspend fun moodTracks(id: String): List<TrackRow> = screens.moodTracks(id)

    override suspend fun moreSuggestions(): List<TrackRow> = screens.moreSuggestions()
}

/**
 * What the home asks of the player (MB3b's `PlayerApi`).
 *
 * The home never holds a `PlayerApi` itself: it hands the rows to this seam, so
 * a JVM test asserts the COMMAND a tap sends with a fake, and the real one is one
 * thin file. The play rules (the queue, the Flow's refill) are the player's and
 * are not repeated here.
 */
interface PlayerPort {
    /** Replaces the queue with [tracks] and starts at [start]. */
    fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String)

    /** Inserts one row right after the playing title. */
    fun playNext(row: TrackRow)

    /** Drops everything after the playing title. */
    fun clearUpcoming()

    /** Skips to the next row (what "moins comme ça" does after re-choosing). */
    fun skipToNext()
}

/** A port that does nothing, for a process whose core could not be built. */
object NoPlayer : PlayerPort {
    override fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String) = Unit

    override fun playNext(row: TrackRow) = Unit

    override fun clearUpcoming() = Unit

    override fun skipToNext() = Unit
}

/** The real port: `PlayerCommand`, on the player's own worker. */
class NzPlayerPort(private val player: PlayerApi) : PlayerPort {
    override fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String) {
        player.command(
            PlayerCommand.PlayQueue(
                tracks = tracks.map(::ref),
                start = start.coerceAtLeast(0).toUInt(),
                context = PlayContext(kind = contextKind, id = contextId),
            ),
            0uL,
        )
    }

    override fun playNext(row: TrackRow) {
        player.command(PlayerCommand.PlayNext(tracks = listOf(ref(row))), 0uL)
    }

    override fun clearUpcoming() {
        player.command(PlayerCommand.ClearUpcoming, 0uL)
    }

    override fun skipToNext() {
        player.command(PlayerCommand.Next, 0uL)
    }

    /** A screen row, as the queue's own reference (what a playlist row hands it). */
    private fun ref(row: TrackRow): TrackRef = TrackRef(
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

/**
 * The app's player, asked of the core for every command.
 *
 * **No cache here on purpose.** `NzCore::player()` used to build a NEW player —
 * its own queue, its own two runtime workers, its own engine — on every call
 * (a contract bug MB3c found: two players would be two queues over one
 * database), so this lot held one instance. The core now owns the ONE instance
 * (a `OnceLock`, PR #313), which makes asking per command correct AND cheap;
 * a cache here would keep an object the core could replace.
 *
 * The [PlayerPort] seam stays: `:playback` (MB3c) exposes the app's facade over
 * the same object, and nothing on this screen changes the day it does.
 */
object AppPlayer {
    /** The app's player, or `null` when the native core could not be built. */
    fun api(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()
}
