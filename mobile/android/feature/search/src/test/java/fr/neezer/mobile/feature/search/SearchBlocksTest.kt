package fr.neezer.mobile.feature.search

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.SearchScope

/**
 * The page's order, as a test reads it: `Search.svelte`'s structure, block by
 * block. This is what the screen draws, so an order that drifts here drifts on
 * the phone — and the window offset of a title is its block's own index.
 */
class SearchBlocksTest {

    private fun state(
        query: String = "",
        scope: SearchScope = SearchScope.LIBRARY,
        tab: SearchTab = SearchTab.All,
        answer: SearchAnswer? = null,
        youtube: SearchModel.YoutubeUi = SearchModel.YoutubeUi.Idle,
        failure: String? = null,
        stale: Boolean = false,
        importOffer: uniffi.nz_ffi.ImportOffer? = null,
        showsAvailable: Boolean = false,
    ) = SearchModel.State(
        query = query,
        scope = scope,
        tab = tab,
        answer = answer,
        youtube = youtube,
        failure = failure,
        stale = stale,
        importOffer = importOffer,
        showsAvailable = showsAvailable,
    )

    @Test
    fun anEmptyBoxIsTheLandingAndTheStyles() {
        assertEquals(
            listOf(SearchBlock.Header, SearchBlock.Landing, SearchBlock.Styles),
            searchBlocks(state(), 10),
        )
    }

    @Test
    fun theToutTabLeadsWithTheBestResultThenTenTitles() {
        val answer = SearchFixtures.answer(trackCount = 42)
        val blocks = searchBlocks(state(query = "daft punk", answer = answer), 10)
        assertEquals(SearchBlock.Header, blocks[0])
        assertEquals(SearchBlock.Tabs, blocks[1])
        assertEquals(SearchBlock.TopCard, blocks[2])
        assertEquals(SearchBlock.Section(SearchSection.Titles, more = 32), blocks[3])
        val tracks = blocks.filterIsInstance<SearchBlock.Track>()
        assertEquals("ten titles on « Tout »", 10, tracks.size)
        assertEquals(listOf(0, 1, 2, 3, 4, 5, 6, 7, 8, 9), tracks.map { it.index })
        assertTrue(
            "a family with more than one card offers « Tout afficher »",
            blocks.contains(SearchBlock.Section(SearchSection.Artists, more = 1)),
        )
        assertTrue(blocks.contains(SearchBlock.Shelf(SearchSection.Artists, grid = false)))
        assertTrue(blocks.contains(SearchBlock.Shelf(SearchSection.Albums, grid = false)))
        assertTrue(blocks.contains(SearchBlock.Shelf(SearchSection.Playlists, grid = false)))
    }

    @Test
    fun theTitresTabHoldsTheWholeList() {
        val answer = SearchFixtures.answer(trackCount = 42)
        val blocks = searchBlocks(
            state(query = "daft punk", tab = SearchTab.Tracks, answer = answer),
            10,
        )
        assertEquals(42, blocks.filterIsInstance<SearchBlock.Track>().size)
        assertTrue(blocks.none { it == SearchBlock.TopCard })
    }

    @Test
    fun aFamilyTabIsItsOwnGrid() {
        val answer = SearchFixtures.answer()
        val blocks = searchBlocks(
            state(query = "daft punk", tab = SearchTab.Albums, answer = answer),
            10,
        )
        assertEquals(
            listOf(
                SearchBlock.Header,
                SearchBlock.Tabs,
                SearchBlock.Section(SearchSection.Albums, more = 0),
                SearchBlock.Shelf(SearchSection.Albums, grid = true),
            ),
            blocks,
        )
    }

    @Test
    fun theFallbacksOwnLineSitsUnderTheList() {
        val answer = SearchFixtures.answer()
        val waiting = searchBlocks(
            state(query = "daft punk", answer = answer, youtube = SearchModel.YoutubeUi.Waiting),
            10,
        )
        assertTrue(waiting.any { it == SearchBlock.Fallback(FallbackStatus.Waiting) })
        val nothing = searchBlocks(
            state(query = "daft punk", answer = answer, youtube = SearchModel.YoutubeUi.Added(0)),
            10,
        )
        assertTrue(
            nothing.any { it == SearchBlock.Fallback(FallbackStatus.NothingMore("daft punk")) },
        )
    }

    @Test
    fun anEmptyAnswerIsOneQuietLineAndNoSkeleton() {
        val blocks = searchBlocks(
            state(query = "zzz", answer = emptyAnswer("zzz")),
            10,
        )
        assertEquals(
            listOf(SearchBlock.Header, SearchBlock.Tabs, SearchBlock.Note("Aucun résultat pour « zzz ».")),
            blocks,
        )
    }

    @Test
    fun aFailureIsTheCauseAndNothingElse() {
        val blocks = searchBlocks(state(query = "daft punk", failure = "Hors ligne"), 10)
        assertEquals(
            listOf(SearchBlock.Header, SearchBlock.Tabs, SearchBlock.Failure("Hors ligne")),
            blocks,
        )
    }

    @Test
    fun theStaleNoteSitsAboveWhatItExplains() {
        val answer = SearchFixtures.answer()
        val blocks = searchBlocks(state(query = "daft punk", answer = answer, stale = true), 10)
        assertEquals(SearchBlock.Stale, blocks[2])
    }

    @Test
    fun thePodcastsScopeIsItsSummaryAndItsPassages() {
        val answer = SearchFixtures.podcastsAnswer()
        val blocks = searchBlocks(
            state(
                query = "memoire",
                scope = SearchScope.PODCASTS,
                answer = answer,
                showsAvailable = true,
            ),
            10,
        )
        assertEquals(
            listOf(
                SearchBlock.Header,
                SearchBlock.PodcastTabs,
                SearchBlock.Note("1 passage · 1 épisode · 240 segments"),
                SearchBlock.Passage(0),
            ),
            blocks,
        )
    }

    @Test
    fun theShowsTabIsAGridOfShows() {
        val answer = SearchFixtures.showsAnswer()
        val blocks = searchBlocks(
            state(query = "daft", scope = SearchScope.SHOWS, answer = answer, showsAvailable = true),
            10,
        )
        assertEquals(
            listOf(
                SearchBlock.Header,
                SearchBlock.PodcastTabs,
                SearchBlock.Note("1 émission"),
                SearchBlock.Shelf(SearchSection.Shows, grid = true),
            ),
            blocks,
        )
    }

    @Test
    fun anImportOfferIsTheWholePage() {
        val blocks = searchBlocks(
            state(query = SearchFixtures.linkOffer.url, importOffer = SearchFixtures.linkOffer),
            10,
        )
        assertEquals(listOf(SearchBlock.Header), blocks)
    }
}
