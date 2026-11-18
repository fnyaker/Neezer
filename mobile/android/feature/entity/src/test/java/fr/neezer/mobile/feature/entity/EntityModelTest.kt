package fr.neezer.mobile.feature.entity

import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzRowSource
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.ArtistData
import uniffi.nz_ffi.DiscographyData
import uniffi.nz_ffi.TrackCreditsData
import uniffi.nz_ffi.TrackRow

/**
 * The entity pages' logic, on the JVM (the screen standard: the data, the failure
 * and the click are the models', and fakes prove them without JNA, a `.so` or a
 * device).
 *
 * What each group is really about:
 *
 * - **the copy paints, the request replaces it** — and a failed refresh keeps the
 *   copy and says « Hors ligne · version mémorisée »;
 * - **one page open costs the SPA's requests and not one more**: the discography
 *   and the provider's half are asked once each (the double `Ready` emission must
 *   not double them), the lazy tabs ask nothing until they are opened, and a role
 *   chip asks nothing at all;
 * - **nothing to show is a cause and a working « Réessayer »**, never a spinner;
 * - **a click answers before its request** (the heart) and rolls back with a
 *   toast; **one action, one command** for the player.
 */
class EntityModelTest {

    // -- fakes -----------------------------------------------------------------

    private class FakeArtist(
        var cached: ArtistData? = EntityFixtures.artist(),
        var fresh: () -> ArtistData = { EntityFixtures.artist() },
        var groups: DiscographyData = EntityFixtures.discographyGroupsOnly(),
        var onDeezer: DiscographyData = EntityFixtures.discographyOnDeezer(),
        var cachedGroups: DiscographyData? = null,
        var tracks: List<TrackRow> = listOf(EntityFixtures.row("Ouverture", 0), EntityFixtures.row("Suite", 1)),
        var cachedTracks: List<TrackRow>? = null,
        var radio: List<TrackRow> = listOf(EntityFixtures.row("Radio 1", 0)),
    ) : ArtistService {
        var cachedCalls = 0
        var freshCalls = 0
        var discographyCalls = 0
        var onDeezerCalls = 0
        var trackCalls = 0
        var radioCalls = 0
        var favorites = mutableListOf<Triple<String, String, Boolean>>()
        var failFresh: Exception? = null
        var failOnDeezer: Exception? = null
        var failTracks: Exception? = null
        var failFavorite: Exception? = null

        override suspend fun cachedArtist(id: String): ArtistData? {
            cachedCalls++
            return cached
        }

        override suspend fun artist(id: String): ArtistData {
            freshCalls++
            failFresh?.let { throw it }
            return fresh()
        }

        override suspend fun cachedDiscography(id: String): DiscographyData? {
            return cachedGroups
        }

        override suspend fun discography(id: String, kind: String): DiscographyData {
            if (kind == ArtistModel.KIND_ON_DEEZER) {
                onDeezerCalls++
                failOnDeezer?.let { throw it }
                return onDeezer
            }
            discographyCalls++
            return groups
        }

        override suspend fun cachedArtistTracks(id: String): NzRowSource? =
            cachedTracks?.let { FakeRows(it) }

        override suspend fun artistTracks(id: String): NzRowSource {
            trackCalls++
            failTracks?.let { throw it }
            return FakeRows(tracks)
        }

        override suspend fun artistRadio(id: String): List<TrackRow> {
            radioCalls++
            return radio
        }

        override suspend fun setFavorite(
            kind: String,
            id: String,
            on: Boolean,
            title: String,
            artist: String?,
            artistId: String?,
        ) {
            failFavorite?.let { throw it }
            favorites += Triple(kind, id, on)
        }
    }

    private class FakeAlbum(
        var cached: AlbumView? = null,
        var fresh: () -> AlbumView = { EntityFixtures.albumView() },
        var rest: List<TrackRow> = emptyList(),
    ) : AlbumService {
        var cachedCalls = 0
        var freshCalls = 0
        var restCalls = 0
        var failFresh: Exception? = null
        var failRest: Exception? = null
        var favorites = mutableListOf<Triple<String, String, Boolean>>()

        override suspend fun cachedAlbum(id: String): AlbumView? {
            cachedCalls++
            return cached
        }

        override suspend fun album(id: String): AlbumView {
            freshCalls++
            failFresh?.let { throw it }
            return fresh()
        }

        override suspend fun albumOnDeezer(id: String): List<TrackRow> {
            restCalls++
            failRest?.let { throw it }
            return rest
        }

        override suspend fun setFavorite(
            kind: String,
            id: String,
            on: Boolean,
            title: String,
            artist: String?,
            artistId: String?,
        ) {
            favorites += Triple(kind, id, on)
        }
    }

    private class FakeCredits(
        var cachedTrack: TrackCreditsData? = null,
        var freshTrack: () -> TrackCreditsData = { EntityFixtures.trackCredits() },
        var cachedPerson: PersonView? = null,
        var freshPerson: () -> PersonView = { EntityFixtures.person() },
        val roles: MutableMap<String, List<TrackRow>> = mutableMapOf(
            "composer" to listOf(EntityFixtures.row("Sinnerman", 0), EntityFixtures.row("Feeling Good", 1)),
            "author" to listOf(EntityFixtures.row("Wild Is the Wind", 2)),
        ),
    ) : CreditsService {
        var trackCalls = 0
        var personCalls = 0
        var roleCalls = 0
        var failTrack: Exception? = null
        var failPerson: Exception? = null

        override suspend fun cachedTrackCredits(id: String): TrackCreditsData? = cachedTrack

        override suspend fun trackCredits(id: String): TrackCreditsData {
            trackCalls++
            failTrack?.let { throw it }
            return freshTrack()
        }

        override suspend fun cachedPersonCredits(name: String): PersonView? = cachedPerson

        override suspend fun personCredits(name: String): PersonView {
            personCalls++
            failPerson?.let { throw it }
            return freshPerson()
        }

        override suspend fun personCreditsRole(name: String, role: String): NzRowSource? {
            roleCalls++
            return roles[role]?.let { FakeRows(it) }
        }
    }

    private class FakePlayer : EntityPlayer {
        data class Play(val keys: List<String>, val start: Int, val kind: String, val id: String)

        val plays = mutableListOf<Play>()
        val shuffled = mutableListOf<Play>()
        val queued = mutableListOf<String>()

        override fun play(
            tracks: List<TrackRow>,
            start: Int,
            contextKind: String,
            contextId: String,
            tapNs: Long,
        ) {
            plays += Play(tracks.map { it.key }, start, contextKind, contextId)
        }

        override fun playShuffled(
            tracks: List<TrackRow>,
            contextKind: String,
            contextId: String,
            tapNs: Long,
        ) {
            shuffled += Play(tracks.map { it.key }, 0, contextKind, contextId)
        }

        override fun playNext(row: TrackRow) {
            queued += row.key
        }
    }

    private class ArtistHarness(
        val service: FakeArtist = FakeArtist(),
        val credits: FakeCredits = FakeCredits(),
        val player: FakePlayer = FakePlayer(),
        val id: String = EntityFixtures.ARTIST_DEEZER,
    ) {
        val toasts = mutableListOf<String>()
        val model = ArtistModel(
            service = service,
            credits = credits,
            player = player,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = { toasts += it },
        )

        fun loaded(): ArtistModel {
            model.load(id)
            return model
        }
    }

    private class AlbumHarness(
        val service: FakeAlbum = FakeAlbum(),
        val player: FakePlayer = FakePlayer(),
    ) {
        val toasts = mutableListOf<String>()
        val model = AlbumModel(
            service = service,
            player = player,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = { toasts += it },
        )

        fun loaded(): AlbumModel {
            model.load(EntityFixtures.ALBUM_DEEZER)
            return model
        }
    }

    private class PersonHarness(
        val service: FakeCredits = FakeCredits(),
        val player: FakePlayer = FakePlayer(),
    ) {
        val toasts = mutableListOf<String>()
        val model = PersonModel(
            service = service,
            player = player,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = { toasts += it },
        )

        fun loaded(): PersonModel {
            model.load("Nina Simone")
            return model
        }
    }

    // -- the artist page -------------------------------------------------------

    @Test
    fun aWarmArtistPaintsTheCopyThenTheAnswerAndAsksEachPartOnce() {
        val harness = ArtistHarness(
            FakeArtist(cached = EntityFixtures.artist(), cachedGroups = EntityFixtures.discographyGroupsOnly()),
        )
        val model = harness.loaded()
        val state = model.state.value

        assertEquals("the copy is read once", 1, harness.service.cachedCalls)
        assertEquals("and refreshed once", 1, harness.service.freshCalls)
        assertEquals("Ascenseur", state.data?.name)
        assertFalse("the refresh succeeded", state.stale)
        assertEquals(
            "the discography is asked ONCE, not once per Ready emission",
            1,
            harness.service.discographyCalls,
        )
        assertEquals("and the provider's half once", 1, harness.service.onDeezerCalls)
        assertEquals("« Dernières sorties » is the discography's shelf", 2, state.latest.size)
        assertEquals("the provider's albums are there", 1, state.onDeezer.size)
        assertFalse(state.deezerDown)
    }

    @Test
    fun aFailedRefreshKeepsTheCopyAndSaysSo() {
        val service = FakeArtist(cached = EntityFixtures.artist())
        service.failFresh = IllegalStateException("Hors ligne")
        val model = ArtistHarness(service).loaded()
        val state = model.state.value

        assertTrue("the copy stays on screen, flagged", state.stale)
        assertEquals("Ascenseur", state.data?.name)
        assertEquals("the stale note is the screen's, not a toast", 0, ArtistHarness(service).toasts.size)
    }

    @Test
    fun nothingToShowIsACauseAndAWorkingRetry() {
        val service = FakeArtist(cached = null)
        service.failFresh = IllegalStateException("Hors ligne")
        val harness = ArtistHarness(service)
        val model = harness.loaded()

        val failed = model.state.value.screen as? ScreenState.Failed
        assertTrue("a cause, in French", failed?.cause.orEmpty().contains("Hors ligne"))
        assertNull(model.state.value.data)

        service.failFresh = null
        model.retry()
        assertEquals("Ascenseur", model.state.value.data?.name)
        assertFalse(model.state.value.stale)
    }

    @Test
    fun aCatalogueArtistIsNotAskedForTheProvidersPart() {
        val service = FakeArtist(
            cached = EntityFixtures.artist(library = false),
            fresh = { EntityFixtures.artist(library = false) },
        )
        ArtistHarness(service).loaded()
        assertEquals(
            "no library, nothing for Deezer to complete",
            0,
            service.onDeezerCalls,
        )
    }

    @Test
    fun aRefusedProviderLeavesThePageStandingWithItsSentence() {
        val service = FakeArtist()
        service.failOnDeezer = IllegalStateException("Deezer injoignable")
        val model = ArtistHarness(service).loaded()

        assertNotNull("the page is there", model.state.value.data)
        assertTrue("and says the provider is down", model.state.value.deezerDown)
        assertTrue(model.state.value.onDeezer.isEmpty())
    }

    @Test
    fun theTracksTabIsLazyAndItsCopyPaintsBeforeTheRequest() {
        val service = FakeArtist()
        val harness = ArtistHarness(service)
        val model = harness.loaded()
        assertEquals("nothing is asked before the tab opens", 0, service.trackCalls)

        model.selectTab(ArtistTab.Tracks)
        assertEquals("one request when it opens", 1, service.trackCalls)
        assertEquals(ArtistTab.Tracks, model.state.value.tab)
        assertEquals(2, model.state.value.trackSource?.len())
        assertEquals("the facts line prefers the title count once loaded", 2, model.state.value.trackCount)

        // A second open is the copy: no request, the same list.
        model.selectTab(ArtistTab.Overview)
        model.selectTab(ArtistTab.Tracks)
        assertEquals("re-opening the tab asks nothing", 1, service.trackCalls)
    }

    @Test
    fun aFailedTracksTabIsACauseAndARetry() {
        val service = FakeArtist()
        service.failTracks = IllegalStateException("Hors ligne")
        val model = ArtistHarness(service).loaded()
        model.selectTab(ArtistTab.Tracks)

        val failed = model.state.value.tracks as? ScreenState.Failed
        assertTrue(failed?.cause.orEmpty().contains("Hors ligne"))

        service.failTracks = null
        model.retryTracks()
        assertEquals(2, model.state.value.trackSource?.len())
    }

    @Test
    fun theCreditsTabIsOneRequestAndARoleChipIsNone() {
        val service = FakeArtist()
        val credits = FakeCredits()
        val harness = ArtistHarness(service, credits)
        val model = harness.loaded()

        model.selectTab(ArtistTab.Credits)
        assertEquals("the person's page: one request", 1, credits.personCalls)
        assertEquals("Nina Simone", model.state.value.creditsPage?.name)
        assertEquals("every role is shown first", 3, model.state.value.creditsSource?.len())

        model.loadCredits("author")
        assertEquals("a chip asks nothing", 1, credits.personCalls)
        assertEquals(1, credits.roleCalls)
        assertEquals("author", model.state.value.creditsRole)
        assertEquals(1, model.state.value.creditsSource?.len())

        model.clearCreditsRole()
        assertEquals("Tout is the list again", 3, model.state.value.creditsSource?.len())
    }

    @Test
    fun theHeartFlipsBeforeItsRequestAndRollsBackWhenRefused() {
        val service = FakeArtist()
        val harness = ArtistHarness(service)
        val model = harness.loaded()
        assertFalse(model.state.value.isFavorite)

        model.toggleFavorite()
        assertTrue("the heart answered before the request", model.state.value.isFavorite)
        assertEquals(
            "the copy's own key is the route argument (the SPA's `routeId`)",
            EntityFixtures.ARTIST_DEEZER,
            service.favorites.single().second,
        )
        assertTrue("the request said follow", service.favorites.single().third)

        // A refusal puts the heart back where it was and says what to retry.
        service.failFavorite = IllegalStateException("Serveur en erreur")
        model.toggleFavorite()
        assertTrue("the refused flip rolled back to « suivi »", model.state.value.isFavorite)
        assertTrue(harness.toasts.single().contains("Réessayer"))
    }

    @Test
    fun aRowOfTheShelvesPlaysTheTitleThenItsRadioAndTheListPlaysFromTheTappedRow() {
        val harness = ArtistHarness()
        val model = harness.loaded()
        val top = EntityFixtures.row("Ouverture", 0)

        model.playTop(top)
        assertEquals("the title alone, then its radio", listOf("row-0"), harness.player.plays.single().keys)
        assertEquals("radio", harness.player.plays.single().kind)
        assertEquals("row-0", harness.player.plays.single().id)

        model.selectTab(ArtistTab.Tracks)
        model.playFromTracks(EntityFixtures.row("Suite", 1))
        val play = harness.player.plays.last()
        assertEquals("the whole list", 2, play.keys.size)
        assertEquals("from the tapped row", 1, play.start)
        assertEquals("artist", play.kind)

        model.playAllTracks()
        assertEquals(0, harness.player.plays.last().start)
    }

    @Test
    fun theRadioButtonIsOneRequestThenAStationThePlayerContinues() {
        val service = FakeArtist()
        val harness = ArtistHarness(service)
        val model = harness.loaded()

        model.startRadio()
        assertEquals(1, service.radioCalls)
        val play = harness.player.plays.single()
        assertEquals("radio", play.kind)
        assertEquals(
            "the context names the seed, which is what the refill reads",
            "artist:${EntityFixtures.ARTIST_DEEZER}",
            play.id,
        )
    }

    // -- the album page --------------------------------------------------------

    @Test
    fun aColdAlbumAsksThePageAndTheProvidersPartOnceAndAWarmOnePaintsTheCopy() {
        val service = FakeAlbum(
            cached = EntityFixtures.albumView(),
            rest = listOf(EntityFixtures.row("Bonus", 12)),
        )
        val harness = AlbumHarness(service)
        val model = harness.loaded()
        val state = model.state.value

        assertEquals("the copy is read once", 1, service.cachedCalls)
        assertEquals("the page is refreshed once", 1, service.freshCalls)
        assertEquals("the provider's remainder once", 1, service.restCalls)
        assertEquals("Le matin", state.data?.title)
        assertEquals(12, state.data?.nbTracks)
        assertEquals("the rest of the tracklist", 1, state.onDeezer.size)
        assertEquals(
            "the facts line is the SPA's own sentence",
            "Album · 2024 · 12 titres · 37:06",
            state.facts,
        )
    }

    @Test
    fun aFailedAlbumRefreshKeepsTheCopy() {
        val service = FakeAlbum(cached = EntityFixtures.albumView())
        service.failFresh = IllegalStateException("Hors ligne")
        val model = AlbumHarness(service).loaded()

        assertTrue(model.state.value.stale)
        assertEquals("Le matin", model.state.value.data?.title)
    }

    @Test
    fun anAlbumWithNoPageIsACauseAndARetry() {
        val service = FakeAlbum()
        service.failFresh = IllegalStateException("Le serveur ne répond pas")
        val model = AlbumHarness(service).loaded()

        val failed = model.state.value.screen as? ScreenState.Failed
        assertTrue(failed?.cause.orEmpty().contains("Le serveur ne répond pas"))

        service.failFresh = null
        model.retry()
        assertEquals("Le matin", model.state.value.data?.title)
    }

    @Test
    fun aRefusedProviderLeavesTheAlbumWhole() {
        val service = FakeAlbum()
        service.failRest = IllegalStateException("Deezer injoignable")
        val model = AlbumHarness(service).loaded()

        assertNotNull(model.state.value.data)
        assertTrue(model.state.value.deezerDown)
    }

    @Test
    fun lirePlaysTheAlbumFromItsFirstTitleAndMelangerShufflesOnce() {
        val harness = AlbumHarness(FakeAlbum())
        val model = harness.loaded()

        model.playAll()
        val play = harness.player.plays.first()
        assertEquals(12, play.keys.size)
        assertEquals(0, play.start)
        assertEquals("album", play.kind)

        model.playFrom(EntityFixtures.row("Titre 5", 4))
        assertEquals(4, harness.player.plays.last().start)

        model.playShuffled()
        assertEquals(12, harness.player.shuffled.single().keys.size)
        assertEquals(harness.player.plays.first().keys.toSet(), harness.player.shuffled.single().keys.toSet())

        model.queueNext(EntityFixtures.row("Titre 2", 1))
        assertEquals(listOf("row-1"), harness.player.queued)
    }

    @Test
    fun theAlbumHeartFlipsBeforeItsRequest() {
        val service = FakeAlbum()
        val harness = AlbumHarness(service)
        val model = harness.loaded()

        model.toggleFavorite()
        assertTrue(model.state.value.isFavorite)
        val (kind, id, on) = service.favorites.single()
        assertEquals("album", kind)
        assertEquals(EntityFixtures.ALBUM_DEEZER, id)
        assertTrue(on)
    }

    // -- the credits pages -----------------------------------------------------

    @Test
    fun aPersonPageIsOneRequestAndAChapterChipIsLocal() {
        val service = FakeCredits()
        val harness = PersonHarness(service)
        val model = harness.loaded()

        assertEquals("one request", 1, service.personCalls)
        assertEquals("Nina Simone", model.state.value.data?.name)
        assertEquals("3 titres", model.state.value.countLabel)
        assertEquals(3, model.state.value.tracks?.len())

        model.selectRole("composer")
        assertEquals("a chip asks nothing", 1, service.personCalls)
        assertEquals(2, model.state.value.tracks?.len())
        assertEquals("composer", model.state.value.role)

        model.selectRole("")
        assertEquals("Tout is the whole list again", 3, model.state.value.tracks?.len())

        model.playAll()
        assertEquals(3, harness.player.plays.single().keys.size)
        assertEquals("credits", harness.player.plays.single().kind)
        assertEquals("Nina Simone", harness.player.plays.single().id)
    }

    @Test
    fun aFailedPersonPageIsACauseAndARetry() {
        val service = FakeCredits()
        service.failPerson = IllegalStateException("Hors ligne")
        val model = PersonHarness(service).loaded()

        val failed = model.state.value.screen as? ScreenState.Failed
        assertTrue(failed?.cause.orEmpty().contains("Hors ligne"))

        service.failPerson = null
        model.retry()
        assertEquals("Nina Simone", model.state.value.data?.name)
    }

    @Test
    fun aTrackSheetIsOneRequestPaintedFromTheCopyFirst() {
        val service = FakeCredits()
        val model = TrackCreditsModel(service, CoroutineScope(Dispatchers.Unconfined), Dispatchers.Unconfined)
        model.load(EntityFixtures.row("Sinnerman", 0))

        assertEquals(1, service.trackCalls)
        val data = model.state.value.data
        assertEquals(2, data?.groups?.size)
        assertEquals("Compositeur", data?.groups?.first()?.label)
        assertTrue("a name the library holds opens its artist page", data!!.groups[0].people[0].opensArtist)
        assertFalse("one title credited is not a page", data.groups[1].people[0].opensArtist)
        assertEquals(
            "the release line, in the SPA's order and its French date",
            listOf("Le matin", "12 mars 2001", "Blue Note", "(c) 2001"),
            data.releaseLine,
        )
        assertEquals("FR1234567890", data.isrc)

        // The same title again: the copy, no request.
        model.load(EntityFixtures.row("Sinnerman", 0))
        assertEquals("a warm sheet asks nothing", 1, service.trackCalls)
    }

    @Test
    fun anEmptyCreditSheetSaysSoAndAFailureOffersRéessayer() {
        val service = FakeCredits()
        service.freshTrack = { EntityFixtures.trackCredits().copy(credits = emptyList(), info = uniffi.nz_ffi.CreditsInfoData(null, null, null, null, null), empty = true) }
        val model = TrackCreditsModel(service, CoroutineScope(Dispatchers.Unconfined), Dispatchers.Unconfined)
        model.load(EntityFixtures.row("Sinnerman", 0))
        assertTrue(model.state.value.data?.empty == true)

        val failing = FakeCredits()
        failing.failTrack = IllegalStateException("Hors ligne")
        val second = TrackCreditsModel(failing, CoroutineScope(Dispatchers.Unconfined), Dispatchers.Unconfined)
        second.load(EntityFixtures.row("Sinnerman", 0))
        val failed = second.state.value.screen as? ScreenState.Failed
        assertTrue(failed?.cause.orEmpty().contains("Hors ligne"))
    }

    // -- the copy the screens draw --------------------------------------------

    @Test
    fun aRoleFiltersTheRowsItAlreadyHasAndTheWindowReadsPages() {
        val rows = FakeRows((0 until 200).map { EntityFixtures.row("Titre $it", it) })
        assertEquals(200, rows.len())
        assertEquals(64, rows.window(0, 64).size)
        assertEquals(10, rows.window(190, 64).size)
        assertEquals("two pages read, not 200 rows", 2, rows.windows)
    }

    @Test
    fun aSubtitleSaysTheReleaseTypeAndTheYear() {
        assertEquals("Album · 2024", albumSubtitle(EntityFixtures.albumCard("1", "Le matin", "2024", "album")))
        assertEquals("Single · 2023", albumSubtitle(EntityFixtures.albumCard("2", "Un titre", "2023", "single")))
        assertEquals("EP · 2023", albumSubtitle(EntityFixtures.albumCard("3", "Un EP", "2023", "ep")))
        assertEquals("Compilation", albumSubtitle(EntityFixtures.albumCard("4", "Compil", null, "compile")))
        assertEquals("Sortie", albumSubtitle(EntityFixtures.albumCard("5", "Inconnu", null, null)))
    }

    @Test
    fun aDurationAndACountAreWrittenLikeTheSpa() {
        assertEquals("0:00", minutesSeconds(0))
        assertEquals("6:20", minutesSeconds(380))
        assertEquals("1\u202F204", frenchCount(1204))
        assertEquals("43\u202F120", frenchCount(43120))
        assertEquals("999", frenchCount(999))
    }

    @Test
    fun anIsoDateBecomesTheFrenchSentenceAndAnythingElseIsLeftAlone() {
        assertEquals("12 mars 2001", frenchDate("2001-03-12"))
        assertEquals("1 janvier 2024", frenchDate("2024-01-01"))
        assertEquals("2001", frenchDate("2001"))
        assertEquals("printemps 2001", frenchDate("printemps 2001"))
    }
}
