package fr.neezer.mobile.feature.entity

import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.ListWindow
import fr.neezer.mobile.designsystem.NzRowSource
import uniffi.nz_ffi.AlbumCardData
import uniffi.nz_ffi.AlbumData
import uniffi.nz_ffi.ArtistData
import uniffi.nz_ffi.DiscographyData
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.PersonCreditsData
import uniffi.nz_ffi.PersonRoleData
import uniffi.nz_ffi.PlayContext
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.TrackCreditsData
import uniffi.nz_ffi.TrackRef
import uniffi.nz_ffi.TrackRow

/**
 * One album page, with its tracklist as a WINDOW source instead of a
 * `ListHandle`.
 *
 * The seam exists for the JVM tests: a UniFFI `ListHandle` is a native pointer
 * and cannot be built in a unit test, while [NzRowSource] is three lines of
 * Kotlin. `AlbumScreen` reads [tracks] exactly as it would read a handle — same
 * `ListWindow` policy (MB1), same pages of 64.
 */
data class AlbumView(
    val id: String,
    val deezerId: String,
    val title: String,
    val cover: String?,
    val year: String?,
    val releaseDate: String?,
    val artistName: String,
    /** What a tap opens (`entityRef`: the row's UUID when the server sent one). */
    val artistId: String,
    val favorite: Boolean,
    val library: Boolean,
    val nbTracks: Int,
    /** The whole tracklist's duration, in seconds. */
    val totalS: Int,
    val tracks: NzRowSource,
)

/** A person's credits page, its titles as a window source (see [AlbumView]). */
data class PersonView(
    val name: String,
    val roles: List<PersonRoleData>,
    val count: Int,
    val tracks: NzRowSource,
)

/**
 * What the artist page asks of the core (lot MB7).
 *
 * An interface, not the generated `Screens`, so the page's logic is plain Kotlin
 * that `EntityModelTest` runs on the JVM with a fake — no JNA, no `.so`, no
 * device. [NzArtistService] is the one implementation, a one-to-one delegation to
 * `nz-ffi/src/entity.rs`.
 */
interface ArtistService {
    /** The device's copy of the page: disk only, no request. */
    suspend fun cachedArtist(id: String): ArtistData?

    /** `GET /api/artist/<id>` — one request. */
    suspend fun artist(id: String): ArtistData

    /** The releases the device already knows, both halves merged. */
    suspend fun cachedDiscography(id: String): DiscographyData?

    /** `GET /api/artist/<id>/discography`, or `/on-deezer` for `kind` = `on-deezer`. */
    suspend fun discography(id: String, kind: String): DiscographyData

    /** The « Titres » tab's list as the device last had it. */
    suspend fun cachedArtistTracks(id: String): NzRowSource?

    /** `GET /api/artist/<id>/tracks` — asked once, when the tab first opens. */
    suspend fun artistTracks(id: String): NzRowSource

    /** `GET /api/radio/artist/<id>` — the « Radio » button. */
    suspend fun artistRadio(id: String): List<TrackRow>

    /** The heart: the device's copy first, then `POST /api/favorite/<kind>`. */
    suspend fun setFavorite(
        kind: String,
        id: String,
        on: Boolean,
        title: String,
        artist: String? = null,
        artistId: String? = null,
    )
}

/** What the album page asks of the core. */
interface AlbumService {
    suspend fun cachedAlbum(id: String): AlbumView?

    /** `GET /api/album/<id>` — one request. */
    suspend fun album(id: String): AlbumView

    /** `GET /api/album/<id>/on-deezer` — the rest of the tracklist, best effort. */
    suspend fun albumOnDeezer(id: String): List<TrackRow>

    suspend fun setFavorite(
        kind: String,
        id: String,
        on: Boolean,
        title: String,
        artist: String? = null,
        artistId: String? = null,
    )
}

/** What a credits sheet and a credits page ask of the core. */
interface CreditsService {
    suspend fun cachedTrackCredits(id: String): TrackCreditsData?

    /** `GET /api/track/<id>/credits` — one request. */
    suspend fun trackCredits(id: String): TrackCreditsData

    suspend fun cachedPersonCredits(name: String): PersonView?

    /** `GET /api/credits/person?name=` — one request. */
    suspend fun personCredits(name: String): PersonView

    /** A role chip: the titles the page already has, no request (null = never read). */
    suspend fun personCreditsRole(name: String, role: String): NzRowSource?
}

/**
 * A `ListHandle` as a window source.
 *
 * NOT MB1's `asRowSource()`: that extension's methods call `len()`/`window()` on
 * the anonymous object itself instead of on the handle, so the first call is a
 * `StackOverflowError` — measured on the emulator by this lot's device test
 * (`ListWindow.kt:25`, `at ...asRowSource$1.len`). It never showed before because
 * no list lot had landed. The fix belongs to `:designsystem` (MB1's file, one
 * writer per file); this adapter is the same three lines with the receiver made
 * explicit, and it goes away when that file is fixed.
 */
internal fun ListHandle.asEntitySource(): NzRowSource = object : NzRowSource {
    override fun len(): Int = this@asEntitySource.len().toInt()

    override fun window(start: Int, count: Int): List<TrackRow> =
        this@asEntitySource.window(start.coerceAtLeast(0).toUInt(), count.coerceAtLeast(0).toUInt())
}

/** The one real implementation: this lot's `nz-ffi` methods. */
class NzArtistService(private val screens: Screens) : ArtistService {
    override suspend fun cachedArtist(id: String): ArtistData? = screens.cachedArtist(id)

    override suspend fun artist(id: String): ArtistData = screens.artist(id)

    override suspend fun cachedDiscography(id: String): DiscographyData? = screens.cachedDiscography(id)

    override suspend fun discography(id: String, kind: String): DiscographyData =
        screens.discography(id, kind, null)

    override suspend fun cachedArtistTracks(id: String): NzRowSource? =
        screens.cachedArtistTracks(id)?.asEntitySource()

    override suspend fun artistTracks(id: String): NzRowSource = screens.artistTracks(id).asEntitySource()

    override suspend fun artistRadio(id: String): List<TrackRow> = screens.artistRadio(id)

    override suspend fun setFavorite(
        kind: String,
        id: String,
        on: Boolean,
        title: String,
        artist: String?,
        artistId: String?,
    ) = screens.setEntityFavorite(kind, id, on, title, artist, artistId)
}

class NzAlbumService(private val screens: Screens) : AlbumService {
    override suspend fun cachedAlbum(id: String): AlbumView? = screens.cachedAlbum(id)?.let(::view)

    override suspend fun album(id: String): AlbumView = view(screens.album(id))

    override suspend fun albumOnDeezer(id: String): List<TrackRow> = screens.albumOnDeezer(id)

    override suspend fun setFavorite(
        kind: String,
        id: String,
        on: Boolean,
        title: String,
        artist: String?,
        artistId: String?,
    ) = screens.setEntityFavorite(kind, id, on, title, artist, artistId)

    private fun view(page: AlbumData) = AlbumView(
        id = page.id,
        deezerId = page.deezerId,
        title = page.title,
        cover = page.cover,
        year = page.year,
        releaseDate = page.releaseDate,
        artistName = page.artistName,
        artistId = page.artistId,
        favorite = page.favorite,
        library = page.library,
        nbTracks = page.nbTracks.toInt(),
        totalS = page.totalS.toInt(),
        tracks = page.tracks.asEntitySource(),
    )
}

class NzCreditsService(private val screens: Screens) : CreditsService {
    override suspend fun cachedTrackCredits(id: String): TrackCreditsData? =
        screens.cachedTrackCredits(id)

    override suspend fun trackCredits(id: String): TrackCreditsData = screens.trackCredits(id)

    override suspend fun cachedPersonCredits(name: String): PersonView? =
        screens.cachedPersonCredits(name)?.let(::view)

    override suspend fun personCredits(name: String): PersonView = view(screens.personCredits(name))

    override suspend fun personCreditsRole(name: String, role: String): NzRowSource? =
        screens.personCreditsRole(name, role)?.asEntitySource()

    private fun view(page: PersonCreditsData) = PersonView(
        name = page.name,
        roles = page.roles,
        count = page.count.toInt(),
        tracks = page.tracks.asEntitySource(),
    )
}

/**
 * What an entity page asks of the player (MB3b's `PlayerApi`).
 *
 * The page never holds a `PlayerApi` itself: it hands the rows to this seam, so a
 * JVM test asserts the COMMAND a tap sends with a fake, and the real one is one
 * thin file. The play rules (the queue, a radio's refill) stay the player's.
 */
interface EntityPlayer {
    /**
     * Replaces the queue with [tracks] and starts at [start].
     *
     * [tapNs] is the instant the finger landed (`System.nanoTime()`): the player
     * keeps it beside the first rendered frame, so "tap → sound" is READ from
     * `output_stats()` (MB3b) instead of timed with a clock by a test.
     */
    fun play(
        tracks: List<TrackRow>,
        start: Int,
        contextKind: String,
        contextId: String,
        tapNs: Long = System.nanoTime(),
    )

    /** The same, with the queue already shuffled (`Album.svelte#shufflePlay`). */
    fun playShuffled(
        tracks: List<TrackRow>,
        contextKind: String,
        contextId: String,
        tapNs: Long = System.nanoTime(),
    )

    /** Inserts one row right after the playing title. */
    fun playNext(row: TrackRow)
}

/** A port that does nothing, for a process whose core could not be built. */
object NoEntityPlayer : EntityPlayer {
    override fun play(
        tracks: List<TrackRow>,
        start: Int,
        contextKind: String,
        contextId: String,
        tapNs: Long,
    ) = Unit

    override fun playShuffled(
        tracks: List<TrackRow>,
        contextKind: String,
        contextId: String,
        tapNs: Long,
    ) = Unit

    override fun playNext(row: TrackRow) = Unit
}

/** The real port: `PlayerCommand`, on the player's own worker. */
class NzEntityPlayer(private val player: PlayerApi) : EntityPlayer {
    override fun play(
        tracks: List<TrackRow>,
        start: Int,
        contextKind: String,
        contextId: String,
        tapNs: Long,
    ) {
        if (tracks.isEmpty()) return
        player.command(
            PlayerCommand.PlayQueue(
                tracks = tracks.map(::ref),
                start = start.coerceIn(0, tracks.lastIndex).toUInt(),
                context = PlayContext(kind = contextKind, id = contextId),
            ),
            tapNs.toULong(),
        )
    }

    override fun playShuffled(
        tracks: List<TrackRow>,
        contextKind: String,
        contextId: String,
        tapNs: Long,
    ) {
        if (tracks.isEmpty()) return
        // `stores.js#shufflePlay`: the list is shuffled ONCE, then the queue is
        // replaced — the player's shuffle flag then keeps the rest in its own
        // order, which is what the SPA's state does too.
        player.command(
            PlayerCommand.PlayQueue(
                tracks = tracks.shuffled().map(::ref),
                start = 0u,
                context = PlayContext(kind = contextKind, id = contextId),
            ),
            0uL,
        )
        player.command(PlayerCommand.SetShuffle(on = true), 0uL)
    }

    override fun playNext(row: TrackRow) {
        player.command(PlayerCommand.PlayNext(tracks = listOf(ref(row))), 0uL)
    }

    /** A screen row, as the queue's own reference. */
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
 * `NzCore::player()` is ONE per process (MB3c's measurement): asking it per
 * command is correct and cheap, and a cache here would keep an object the core
 * could replace. The same holder shape MB5's home uses, in this module's own
 * file because a feature lot owns its module.
 */
object EntityPlayerHolder {
    fun api(): PlayerApi? = runCatching { NzBridge.core?.player() }.getOrNull()
}

/**
 * The window a `LazyColumn` over an [NzRowSource] reads.
 *
 * `rememberListWindow` takes a `ListHandle`; the entity pages hold a source
 * (see [AlbumView]), and `ListWindow` itself takes the interface. Same policy,
 * same pages of 64, same fling rule (MB1's `ListWindowTest` holds it).
 */
internal fun windowFor(source: NzRowSource, pageSize: Int = 64): ListWindow =
    ListWindow(source, pageSize)

/** The whole list, for a "play all" that must not cost a request: pages of 128. */
internal fun NzRowSource.allRows(): List<TrackRow> {
    val total = len()
    if (total <= 0) return emptyList()
    val rows = ArrayList<TrackRow>(total)
    var start = 0
    while (start < total) {
        val page = window(start, minOf(128, total - start))
        if (page.isEmpty()) break
        rows += page
        start += page.size
    }
    return rows
}

/** `Album.svelte#releaseSubtitle`: « Album · 1998 », « Sortie » when unknown. */
internal fun albumSubtitle(card: AlbumCardData): String {
    val type = when (card.recordType?.lowercase()) {
        "album" -> "Album"
        "single" -> "Single"
        "ep" -> "EP"
        "compile", "compilation" -> "Compilation"
        else -> "Sortie"
    }
    return card.year?.let { "$type · $it" } ?: type
}

/** `format.js#duration`: `m:ss` for the page's facts line. */
internal fun minutesSeconds(seconds: Int): String {
    val total = seconds.coerceAtLeast(0)
    return "${total / 60}:${(total % 60).toString().padStart(2, '0')}"
}

/** A count as the SPA writes it: `1 204` (a narrow no-break space). */
internal fun frenchCount(value: Int): String {
    val digits = value.coerceAtLeast(0).toString()
    val out = StringBuilder()
    digits.forEachIndexed { index, ch ->
        if (index > 0 && (digits.length - index) % 3 == 0) out.append('\u202F')
        out.append(ch)
    }
    return out.toString()
}
