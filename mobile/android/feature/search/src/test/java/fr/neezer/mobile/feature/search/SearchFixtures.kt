package fr.neezer.mobile.feature.search

import fr.neezer.mobile.designsystem.NzRowSource
import uniffi.nz_ffi.EntityKind
import uniffi.nz_ffi.ImportOffer
import uniffi.nz_ffi.RadioPlan
import uniffi.nz_ffi.SearchEntity
import uniffi.nz_ffi.SearchFallback
import uniffi.nz_ffi.SearchScope
import uniffi.nz_ffi.TopResult
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.TranscriptHit
import uniffi.nz_ffi.TranscriptPart

/**
 * What this lot's tests draw and count: the answers, the rows and the fakes.
 *
 * The rows are the REAL `TrackRow` records and the answers the REAL
 * [SearchAnswer]; only the two seams ([SearchService], [SearchPlayer]) are fakes,
 * so a test proves the model's rules, not a mock's.
 */
object SearchFixtures {

    /** A page of rows a pixel-free test can window: `ListWindow` only asks len/window. */
    class FakeRows(private val rows: List<TrackRow>) : NzRowSource {
        override fun len(): Int = rows.size

        override fun window(start: Int, count: Int): List<TrackRow> =
            rows.drop(start).take(count)
    }

    fun rows(count: Int, youtubeAt: Int? = null): List<TrackRow> = (0 until count).map { number ->
        TrackRow(
            key = "row:$number",
            id = "id-$number",
            title = "Titre $number",
            artists = "Daft Punk",
            album = "Random Access Memories",
            albumId = "501",
            durationS = (180 + number).toUInt(),
            cover = "/api/cover/501",
            explicit = false,
            liked = false,
            disliked = false,
            unavailable = false,
            downloaded = false,
            source = if (number == youtubeAt) "youtube" else "deezer",
            position = number.toUInt(),
        )
    }

    fun entity(
        id: String,
        title: String,
        kind: EntityKind = EntityKind.ARTIST,
        subtitle: String = "",
    ): SearchEntity = SearchEntity(
        id = id,
        deezerId = id,
        title = title,
        subtitle = subtitle,
        cover = null,
        kind = kind,
    )

    fun answer(
        term: String = "daft punk",
        scope: SearchScope = SearchScope.LIBRARY,
        trackCount: Int = 4,
        /** Which row is the fallback's, if any (the woven list's own shape). */
        youtubeAt: Int? = null,
        artists: List<SearchEntity> = listOf(
            entity("daftside", "Daftside"),
            entity("justice", "Justice"),
            entity("air", "Air"),
        ),
        albums: List<SearchEntity> = listOf(
            entity("501", "Random Access Memories", EntityKind.ALBUM, "Daft Punk"),
            entity("502", "Discovery", EntityKind.ALBUM, "Daft Punk"),
            entity("503", "Homework", EntityKind.ALBUM, "Daft Punk"),
        ),
        playlists: List<SearchEntity> = listOf(
            entity("7001", "French Touch", EntityKind.PLAYLIST, "Nadia"),
            entity("7002", "Nuit blanche", EntityKind.PLAYLIST, "Nadia"),
        ),
        podcasts: List<SearchEntity> = emptyList(),
        top: TopResult? = TopResult(EntityKind.ARTIST, entity("daftp", "Daft Punk")),
        transcripts: List<TranscriptHit> = emptyList(),
        youtubeAnswered: Boolean = youtubeAt != null,
        youtubeAvailable: Boolean = true,
        youtubeFailure: SearchFallback? = null,
        radioOffered: Boolean = false,
    ): SearchAnswer = SearchAnswer(
        term = term,
        scope = scope,
        top = top,
        tracks = FakeRows(rows(trackCount, youtubeAt)),
        trackCount = trackCount,
        artists = artists,
        albums = albums,
        playlists = playlists,
        podcasts = podcasts,
        transcripts = transcripts,
        transcriptTotal = transcripts.size,
        transcriptMore = false,
        transcriptEpisodes = 1,
        transcriptSegments = 240,
        transcriptTruncated = false,
        youtube = if (youtubeAt != null) 1 else 0,
        youtubeAnswered = youtubeAnswered,
        youtubeAvailable = youtubeAvailable,
        youtubeFailure = youtubeFailure,
        radioTotal = if (radioOffered) 36 else trackCount + artists.size + albums.size,
        radioOffered = radioOffered,
    )

    fun transcript(term: String = "memoire"): TranscriptHit = TranscriptHit(
        key = "ep:0",
        episodeId = "ep-1",
        title = "Épisode 1",
        channel = "Les Histoires",
        startS = 600u,
        text = listOf(
            TranscriptPart("la ", false),
            TranscriptPart("memoire", true),
            TranscriptPart(" de la ville", false),
        ),
        cover = null,
    )

    fun podcastsAnswer(term: String = "memoire"): SearchAnswer = answer(
        term = term,
        scope = SearchScope.PODCASTS,
        trackCount = 0,
        artists = emptyList(),
        albums = emptyList(),
        playlists = emptyList(),
        top = null,
        transcripts = listOf(transcript(term)),
    )

    fun showsAnswer(term: String = "daft"): SearchAnswer = answer(
        term = term,
        scope = SearchScope.SHOWS,
        trackCount = 0,
        artists = emptyList(),
        albums = emptyList(),
        playlists = emptyList(),
        podcasts = listOf(entity("8001", "Daft Punk, une histoire", EntityKind.SHOW)),
        top = null,
    )

    fun plan(): RadioPlan = RadioPlan(
        tracks = rows(6),
        contextKind = "radio",
        contextId = "track:id-0",
        contextTitle = "Radio de patate",
        seedKind = "track",
        seedArtist = "27",
        total = 36u,
    )

    fun emptyPlan(): RadioPlan = RadioPlan(
        tracks = emptyList(),
        contextKind = "",
        contextId = "",
        contextTitle = "",
        seedKind = "",
        seedArtist = null,
        total = 0u,
    )

    val linkOffer: ImportOffer = ImportOffer(
        url = "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
        source = "spotify",
        label = "Spotify",
    )
}
