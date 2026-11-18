package fr.neezer.mobile.feature.entity

import fr.neezer.mobile.designsystem.NzRowSource
import uniffi.nz_ffi.AlbumCardData
import uniffi.nz_ffi.ArtistData
import uniffi.nz_ffi.ArtistTileData
import uniffi.nz_ffi.CreditGroupData
import uniffi.nz_ffi.CreditPersonData
import uniffi.nz_ffi.CreditsInfoData
import uniffi.nz_ffi.DiscographyData
import uniffi.nz_ffi.PersonRoleData
import uniffi.nz_ffi.TrackCreditsData
import uniffi.nz_ffi.TrackRow

/**
 * The rows a page draws, as a [NzRowSource] over a plain list.
 *
 * The real source is a `ListHandle` (a UniFFI object); this one is the same
 * three methods, which is what makes the window's policy and every screen
 * testable on the JVM. It COUNTS the windows it was asked for, so a test can
 * assert "the page asked for one page, not four thousand rows".
 */
class FakeRows(private val items: List<TrackRow>) : NzRowSource {
    var windows = 0
        private set

    override fun len(): Int = items.size

    override fun window(start: Int, count: Int): List<TrackRow> {
        windows++
        if (start >= items.size || count <= 0) return emptyList()
        return items.subList(start, minOf(start + count, items.size))
    }

    fun keys(): List<String> = items.map { it.key }
}

/** The fixtures the model tests and the screenshots share (the real records). */
object EntityFixtures {
    const val ARTIST_ID = "aaaaaaaa-1111-4111-8111-111111111111"
    const val ALBUM_ID = "bbbbbbbb-1111-4111-8111-111111111111"
    const val ARTIST_DEEZER = "42"
    const val ALBUM_DEEZER = "501"

    fun row(title: String, position: Int = 0, duration: Int = 180): TrackRow = TrackRow(
        key = if (position == 0) "row-0" else "row-$position",
        id = if (position == 0) "row-0" else "row-$position",
        title = title,
        artists = "Ascenseur",
        album = "Le matin",
        albumId = ALBUM_DEEZER,
        durationS = duration.toUInt(),
        cover = "/api/cover/501",
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = position.toUInt(),
    )

    fun albumCard(id: String, title: String, year: String? = "2024", type: String? = "album"): AlbumCardData =
        AlbumCardData(
            id = id,
            deezerId = id,
            title = title,
            cover = "/api/cover/$id",
            year = year,
            releaseDate = year?.let { "$it-03-01" },
            recordType = type,
            nbTracks = 2u,
            artistName = "Ascenseur",
            artistId = ARTIST_DEEZER,
        )

    fun artist(
        favorite: Boolean = false,
        credited: UInt = 2u,
        library: Boolean = true,
        fans: UInt? = null,
    ): ArtistData = ArtistData(
        id = ARTIST_ID,
        deezerId = ARTIST_DEEZER,
        name = "Ascenseur",
        picture = "/api/cover/$ARTIST_ID",
        fans = fans,
        favorite = favorite,
        credited = credited,
        library = library,
        bio = "Un groupe qui monte, lentement.",
        top = listOf(row("Ouverture", 0), row("Suite", 1, 200)),
        albums = listOf(albumCard(ALBUM_ID, "Le matin")),
        related = listOf(ArtistTileData(id = "cccccccc-1111-4111-8111-111111111111", name = "June", picture = null)),
    )

    fun discography(): DiscographyData = DiscographyData(
        latest = listOf(albumCard(ALBUM_ID, "Le matin", "2024"), albumCard("502", "Un titre", "2023", "single")),
        albums = listOf(albumCard(ALBUM_ID, "Le matin")),
        singles = listOf(albumCard("502", "Un titre", "2023", "single")),
        eps = emptyList(),
        featured = listOf(albumCard("503", "Chez June", "2022")),
        onDeezer = listOf(albumCard("777", "Seulement sur Deezer", "2019")),
        onDeezerLoaded = true,
        nextCursor = null,
    )

    fun discographyGroupsOnly(): DiscographyData = discography().copy(
        onDeezer = emptyList(),
        onDeezerLoaded = false,
    )

    fun discographyOnDeezer(): DiscographyData = DiscographyData(
        latest = emptyList(),
        albums = emptyList(),
        singles = emptyList(),
        eps = emptyList(),
        featured = emptyList(),
        onDeezer = listOf(albumCard("777", "Seulement sur Deezer", "2019")),
        onDeezerLoaded = true,
        nextCursor = null,
    )

    fun albumView(tracks: Int = 12, favorite: Boolean = false, library: Boolean = true): AlbumView {
        val rows = (0 until tracks).map { index ->
            row(if (index == 0) "Ouverture" else "Titre ${index + 1}", index, 180 + index)
        }
        return AlbumView(
            id = ALBUM_ID,
            deezerId = ALBUM_DEEZER,
            title = "Le matin",
            cover = "/api/cover/$ALBUM_DEEZER",
            year = "2024",
            releaseDate = "2024-03-01",
            artistName = "Ascenseur",
            artistId = ARTIST_ID,
            favorite = favorite,
            library = library,
            nbTracks = tracks,
            totalS = rows.sumOf { it.durationS.toInt() },
            tracks = FakeRows(rows),
        )
    }

    fun person(roles: List<PersonRoleData>? = null, rows: List<TrackRow>? = null): PersonView {
        val tracks = rows ?: listOf(
            row("Sinnerman", 0),
            row("Feeling Good", 1),
            row("Wild Is the Wind", 2),
        )
        return PersonView(
            name = "Nina Simone",
            roles = roles ?: listOf(
                PersonRoleData(role = "composer", label = "Compositeur", count = 2u),
                PersonRoleData(role = "author", label = "Auteur", count = 1u),
            ),
            count = tracks.size,
            tracks = FakeRows(tracks),
        )
    }

    fun trackCredits(): TrackCreditsData = TrackCreditsData(
        credits = listOf(
            CreditGroupData(
                role = "composer",
                label = "Compositeur",
                people = listOf(
                    CreditPersonData(
                        name = "Nina Simone",
                        artistId = "dddddddd-1111-4111-8111-111111111111",
                        credited = 3u,
                    ),
                ),
            ),
            CreditGroupData(
                role = "producer",
                label = "Producteur",
                people = listOf(CreditPersonData(name = "Bob Power", artistId = null, credited = 1u)),
            ),
        ),
        info = CreditsInfoData(
            isrc = "FR1234567890",
            released = "2001-03-12",
            label = "Blue Note",
            copyright = "(c) 2001",
            album = "Le matin",
        ),
        empty = false,
    )
}
