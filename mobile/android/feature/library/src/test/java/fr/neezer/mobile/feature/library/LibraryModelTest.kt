package fr.neezer.mobile.feature.library

import fr.neezer.mobile.bridge.ScreenState
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.FavoriteIdsData
import uniffi.nz_ffi.LibraryAlbum
import uniffi.nz_ffi.LibraryArtist
import uniffi.nz_ffi.LibraryTree
import uniffi.nz_ffi.PlaylistRow
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.UploadUsageData

/**
 * The library's logic, on the JVM (the screen standard: the data, the failure and
 * the click are the models', and fakes prove them without JNA, a `.so` or a
 * device).
 *
 * What each group is really about:
 *
 * - **one leaf is one request**, and a warm open paints the device's copy before
 *   it asks — the double `Ready` emission must not double the route (MB5's and
 *   MB7's trap);
 * - **a leaf that was never opened costs nothing**;
 * - **a folder write moves the tree first**, sends ONE order, and rolls back with
 *   a toast when it is refused — offline included;
 * - **nothing to show is a cause and a working « Réessayer »**, never a spinner;
 * - **one action, one command** for the player.
 */
class LibraryModelTest {

    // -- fakes -----------------------------------------------------------------

    private class FakeLibrary(
        var cachedFavorites: LibraryRows? = FakeRows(LibraryFixtures.rows(40)),
        var freshFavorites: LibraryRows = FakeRows(LibraryFixtures.rows(40)),
        var cachedTree: LibraryTree? = LibraryFixtures.tree(),
        var freshTree: LibraryTree = LibraryFixtures.tree(),
        var cachedArtists: List<LibraryArtist>? = LibraryFixtures.artists(),
        var freshArtists: List<LibraryArtist> = LibraryFixtures.artists(),
        var cachedAlbums: List<LibraryAlbum>? = LibraryFixtures.albums(),
        var freshAlbums: List<LibraryAlbum> = LibraryFixtures.albums(),
        var cachedLocal: LibraryRows? = FakeRows(listOf(LibraryFixtures.row("Ma prise", 0, "Nadia"))),
        var freshLocal: LibraryRows = FakeRows(listOf(LibraryFixtures.row("Ma prise", 0, "Nadia"))),
        var cachedUsage: UploadUsageData? = LibraryFixtures.usage(),
        var freshUsage: UploadUsageData = LibraryFixtures.usage(),
        var cachedDislikes: LibraryRows? = FakeRows(listOf(LibraryFixtures.row("Trop fort", 0, "Bruit"))),
        var freshDislikes: LibraryRows = FakeRows(listOf(LibraryFixtures.row("Trop fort", 0, "Bruit"))),
        var cachedUnavailable: LibraryRows? = FakeRows(emptyList()),
        var freshUnavailable: LibraryRows = FakeRows(emptyList()),
        var supported: Boolean? = true,
    ) : LibraryService {
        var favoritesCalls = 0
        var cachedFavoritesCalls = 0
        var treeCalls = 0
        var cachedTreeCalls = 0
        var artistCalls = 0
        var albumCalls = 0
        var localCalls = 0
        var usageCalls = 0
        var dislikeCalls = 0
        var unavailableCalls = 0
        var probeCalls = 0
        var removed = mutableListOf<String>()
        val orders = mutableListOf<List<String>>()
        /** Les noms des playlists créées, dans l'ordre. */
        val created = mutableListOf<String>()
        var failFavorites: Exception? = null
        var failTree: Exception? = null
        var failCreate: Exception? = null
        var failCreatePlaylist: Exception? = null
        var failRename: Exception? = null
        var failDelete: Exception? = null
        var failMove: Exception? = null
        var failRemoveDislike: Exception? = null
        var gateTree: CompletableDeferred<Unit>? = null
        var gateOrder: CompletableDeferred<Unit>? = null

        override suspend fun cachedTree(): LibraryTree? {
            cachedTreeCalls++
            return cachedTree
        }

        override suspend fun tree(): LibraryTree {
            treeCalls++
            gateTree?.await()
            failTree?.let { throw it }
            return freshTree
        }

        override suspend fun foldersSupported(): Boolean? {
            probeCalls++
            return supported
        }

        override suspend fun createPlaylist(title: String): String {
            failCreatePlaylist?.let { throw it }
            created += title
            return "41"
        }

        override suspend fun createFolder(title: String): LibraryTree {
            failCreate?.let { throw it }
            val folder = LibraryFixtures.folder(
                "ffffffff-1111-4111-8111-0000000000aa",
                title,
                2,
                0u,
            )
            freshTree = freshTree.copy(folders = freshTree.folders + folder)
            return freshTree
        }

        override suspend fun renameFolder(id: String, title: String): LibraryTree {
            failRename?.let { throw it }
            freshTree = freshTree.copy(
                folders = freshTree.folders.map { if (it.id == id) it.copy(title = title) else it },
            )
            return freshTree
        }

        override suspend fun deleteFolder(id: String): LibraryTree {
            failDelete?.let { throw it }
            freshTree = freshTree.copy(folders = freshTree.folders.filter { it.id != id })
            return freshTree
        }

        override suspend fun reorderFolders(ids: List<String>): LibraryTree {
            orders += ids
            gateOrder?.await()
            val byId = freshTree.folders.associateBy { it.id }
            freshTree = freshTree.copy(folders = ids.mapNotNull { byId[it] })
            return freshTree
        }

        override suspend fun reorderContainer(folder: String?, ids: List<String>): LibraryTree {
            orders += ids
            gateOrder?.await()
            return freshTree
        }

        override suspend fun movePlaylistTo(playlistId: String, folder: String?): LibraryTree {
            failMove?.let { throw it }
            freshTree = moveLocally(freshTree, playlistId, folder, listOf(playlistId))
            return freshTree
        }

        override suspend fun cachedFavorites(): LibraryRows? {
            cachedFavoritesCalls++
            return cachedFavorites
        }

        override suspend fun favorites(): LibraryRows {
            favoritesCalls++
            failFavorites?.let { throw it }
            return freshFavorites
        }

        override suspend fun cachedArtists(): List<LibraryArtist>? = cachedArtists

        override suspend fun artists(): List<LibraryArtist> {
            artistCalls++
            return freshArtists
        }

        override suspend fun cachedAlbums(): List<LibraryAlbum>? = cachedAlbums

        override suspend fun albums(): List<LibraryAlbum> {
            albumCalls++
            return freshAlbums
        }

        override suspend fun cachedLocal(): LibraryRows? = cachedLocal

        override suspend fun local(): LibraryRows {
            localCalls++
            return freshLocal
        }

        override suspend fun cachedUsage(): UploadUsageData? = cachedUsage

        override suspend fun usage(): UploadUsageData {
            usageCalls++
            return freshUsage
        }

        override suspend fun cachedDislikes(): LibraryRows? = cachedDislikes

        override suspend fun dislikes(): LibraryRows {
            dislikeCalls++
            return freshDislikes
        }

        override suspend fun removeDislike(reference: String) {
            failRemoveDislike?.let { throw it }
            removed += reference
        }

        override suspend fun cachedUnavailable(): LibraryRows? = cachedUnavailable

        override suspend fun unavailable(): LibraryRows {
            unavailableCalls++
            return freshUnavailable
        }

        override suspend fun favoriteIds(): FavoriteIdsData = LibraryFixtures.favoriteIds()
    }

    private class FakePlayer : LibraryPlayer {
        data class Play(
            val keys: List<String>,
            val start: Int,
            val contextKind: String,
            val contextId: String,
        )

        val plays = mutableListOf<Play>()
        val shuffled = mutableListOf<Play>()
        val queued = mutableListOf<String>()

        override fun play(tracks: List<TrackRow>, start: Int, contextKind: String, contextId: String) {
            plays += Play(tracks.map { it.key }, start, contextKind, contextId)
        }

        override fun playShuffled(tracks: List<TrackRow>, contextKind: String, contextId: String) {
            shuffled += Play(tracks.map { it.key }, 0, contextKind, contextId)
        }

        override fun playNext(row: TrackRow) {
            queued += row.key
        }
    }

    private class Harness(
        val service: FakeLibrary = FakeLibrary(),
        val player: FakePlayer = FakePlayer(),
        val leaf: LibraryLeaf = LibraryLeaf.Favorites,
    ) {
        val toasts = mutableListOf<String>()
        val model = LibraryModel(
            service = service,
            player = player,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = { toasts += it },
        )

        fun loaded(): Harness {
            model.load(leaf)
            return this
        }

        val state get() = model.state.value
    }

    // -- the data --------------------------------------------------------------

    /**
     * 8 000 favourites cost ONE request, and the copy is what paints first: the
     * cold open asks `/me/favorites` once, the warm one reads the device's copy
     * (`cached`) before its single background refresh — never twice (the two
     * `Ready` emissions of `screenFlow`).
     */
    @Test
    fun eightThousandFavoritesAreOneRequestAndPaintFromTheCopy() {
        val eightThousand = FakeRows(LibraryFixtures.rows(8_000))
        val service = FakeLibrary(cachedFavorites = eightThousand, freshFavorites = eightThousand)
        val harness = Harness(service = service).loaded()

        assertEquals("one request for the whole list", 1, service.favoritesCalls)
        assertEquals("the copy is read once", 1, service.cachedFavoritesCalls)
        val rows = harness.state.favoriteRows
        assertTrue(rows != null)
        assertEquals(8_000, rows!!.len())
        assertEquals("the other leaves are not opened", 0, service.artistCalls)
        assertEquals(0, service.treeCalls)
        assertTrue(harness.state.counts, harness.state.counts.startsWith("8000 titres"))
    }

    /** A leaf reads once, the first time it opens — and never again. */
    @Test
    fun aLeafIsAskedOnceAndOnlyWhenItOpens() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        assertEquals(0, service.artistCalls)

        harness.model.selectLeaf(LibraryLeaf.Artists)
        harness.model.selectLeaf(LibraryLeaf.Artists)
        assertEquals(1, service.artistCalls)
        assertEquals(2, harness.state.shownArtists.size)

        harness.model.selectLeaf(LibraryLeaf.Albums)
        assertEquals(1, service.albumCalls)

        harness.model.selectLeaf(LibraryLeaf.Playlists)
        assertEquals(1, service.treeCalls)
        assertEquals("every section, empty folders included", 5, harness.state.sections.size)

        harness.model.selectLeaf(LibraryLeaf.Local)
        assertEquals(1, service.localCalls)
        assertEquals("the quota is this leaf's own request", 1, service.usageCalls)
        assertEquals(20.0, harness.state.usage?.percent ?: 0.0, 0.001)

        harness.model.selectLeaf(LibraryLeaf.Unavailable)
        assertEquals(1, service.unavailableCalls)

        harness.model.selectLeaf(LibraryLeaf.Dislikes)
        assertEquals(1, service.dislikeCalls)

        // Back to the favourites: the handle it already has, no second request.
        harness.model.selectLeaf(LibraryLeaf.Favorites)
        assertEquals(1, service.favoritesCalls)
    }

    /** A refusal with nothing to draw is a CAUSE, and « Réessayer » works. */
    @Test
    fun aRefusedLeafNamesItsCauseAndRetries() {
        val service = FakeLibrary(cachedFavorites = null)
        service.failFavorites = IllegalStateException("Hors ligne")
        val harness = Harness(service = service).loaded()

        val failure = harness.state.failure
        assertTrue(failure != null)
        assertEquals("Hors ligne", failure!!.cause)
        assertNull(harness.state.favoriteRows)

        service.failFavorites = null
        harness.model.retry()
        assertEquals("one click, one more attempt", 2, service.favoritesCalls)
        assertTrue(harness.state.favoriteRows != null)
    }

    /** A stale copy stays on screen, flagged, when the refresh is refused. */
    @Test
    fun aRefusedRefreshKeepsTheCopyAndSaysSo() {
        val service = FakeLibrary()
        service.failFavorites = IllegalStateException("Hors ligne")
        val harness = Harness(service = service).loaded()

        assertTrue(harness.state.stale)
        assertEquals(40, harness.state.favoriteRows?.len())
        assertNull("a copy is not a failure", harness.state.failure)
    }

    // -- the tools -------------------------------------------------------------

    /** The search box and the sorts go through the handle, not a copy of the list. */
    @Test
    fun theSearchAndTheSortsGoThroughTheHandle() {
        val rows = FakeRows(LibraryFixtures.rows(12))
        val harness = Harness(service = FakeLibrary(cachedFavorites = rows, freshFavorites = rows)).loaded()

        val filtersBefore = rows.filters
        val sortsBefore = rows.sorts
        harness.model.filterFavorites("sinnerman")
        assertTrue("the filter ran through the handle", rows.filters > filtersBefore)
        assertEquals("the len follows the filter", 6, harness.state.favoriteRows?.len())

        harness.model.sortFavorites(FavoriteSort.Title)
        val afterSort = rows.sorts
        assertTrue("the sort ran through the handle", afterSort > sortsBefore)
        harness.model.reverseFavorites()
        assertTrue(harness.state.favoriteDescending)
        assertTrue("inverting the order is a sort of its own", rows.sorts > afterSort)
        assertEquals("Ordre d'origine", FavoriteSort.Position.label)
    }

    /** The artists' grid filters and sorts locally: no request for a keystroke. */
    @Test
    fun theArtistsGridSortsWithoutARequest() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Artists)

        harness.model.filterArtists("zaz")
        assertEquals(1, harness.state.shownArtists.size)
        harness.model.filterArtists("")
        harness.model.sortArtists(ArtistSort.Name)
        assertEquals(listOf("Ascenseur", "Zazie"), harness.state.shownArtists.map { it.name })
        harness.model.sortArtists(ArtistSort.Recent)
        assertEquals(listOf("Zazie", "Ascenseur"), harness.state.shownArtists.map { it.name })
        assertEquals("the server's own order is « Les plus écoutés »", 1, service.artistCalls)
    }

    // -- what a tap does -------------------------------------------------------

    /** « Tout lire » hands the whole list to the player, from the first row. */
    @Test
    fun playAllHandsTheWholeListToThePlayer() {
        val player = FakePlayer()
        val harness = Harness(player = player).loaded()
        harness.model.playFavorites()

        assertEquals(1, player.plays.size)
        val play = player.plays.single()
        assertEquals("favorites", play.contextKind)
        assertEquals(0, play.start)
        assertEquals("the whole list, pages and all", 40, play.keys.size)
        assertTrue(player.shuffled.isEmpty())
    }

    @Test
    fun shuffleIsItsOwnCommand() {
        val player = FakePlayer()
        val harness = Harness(player = player).loaded()
        harness.model.shuffleFavorites()

        assertEquals(1, player.shuffled.size)
        assertEquals("favorites", player.shuffled.single().contextKind)
        assertEquals(40, player.shuffled.single().keys.size)
        assertTrue(player.plays.isEmpty())
    }

    /** A row's tap plays ITS list, from that row. */
    @Test
    fun aTapPlaysFromThatRow() {
        val player = FakePlayer()
        val harness = Harness(player = player).loaded()
        val third = LibraryFixtures.row("Sinnerman 2", 2)

        harness.model.playFrom(third, LibraryLeaf.Favorites)
        val play = player.plays.single()
        assertEquals(2, play.start)
        assertEquals(40, play.keys.size)
        assertEquals("favorites", play.contextId)
    }

    @Test
    fun aSwipeQueuesTheRowNext() {
        val player = FakePlayer()
        val harness = Harness(player = player).loaded()
        harness.model.queueNext(LibraryFixtures.row("Ouverture 0", 0))
        assertEquals(listOf("k-0"), player.queued)
        assertEquals("and nothing else is played", 0, player.plays.size + player.shuffled.size)
    }

    // -- the folder writes -----------------------------------------------------

    /** « Nouveau dossier »: the tree answers now, ONE request follows. */
    @Test
    fun aFolderIsCreatedAndTheTreeAnswersFirst() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)
        val before = harness.state.treeData!!.folders.size

        harness.model.openFolderSheet(null, "")
        harness.model.editFolderTitle("Route de nuit")
        harness.model.submitFolderSheet()

        assertNull("the sheet closes on success", harness.state.folderSheet)
        assertEquals(before + 1, harness.state.treeData!!.folders.size)
        assertEquals("Route de nuit", harness.state.treeData!!.folders.last().title)
    }

    /** A refused creation keeps the sheet open with the cause and « Réessayer ». */
    @Test
    fun aRefusedFolderKeepsTheSheetAndItsCause() {
        val service = FakeLibrary()
        service.failCreate = IllegalStateException("Hors ligne")
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)
        val before = harness.state.treeData!!.folders.size

        harness.model.openFolderSheet(null, "")
        harness.model.editFolderTitle("Route de nuit")
        harness.model.submitFolderSheet()

        assertEquals("nothing was invented", before, harness.state.treeData!!.folders.size)
        val sheet = harness.state.folderSheet
        assertTrue(sheet != null)
        assertEquals("Hors ligne. Réessayer.", sheet!!.error)
        assertFalse(sheet.busy)
    }

    /** An empty name never reaches the server, and the sheet says why. */
    @Test
    fun aBlankFolderNameNeverAsksTheServer() {
        val harness = Harness().loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)
        val before = harness.state.treeData!!.folders.size
        harness.model.openFolderSheet(null, "")
        harness.model.editFolderTitle("   ")
        harness.model.submitFolderSheet()

        assertEquals("Le nom du dossier ne peut pas être vide.", harness.state.folderSheet?.error)
        assertEquals(before, harness.state.treeData!!.folders.size)
    }

    /** A rename answers in the tree and the section's heading follows it. */
    @Test
    fun aRenameAnswersInTheTree() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)

        harness.model.openFolderSheet(LibraryFixtures.VOYAGE, "Voyage")
        harness.model.editFolderTitle("Voyages")
        harness.model.submitFolderSheet()

        assertEquals("Voyages", harness.state.treeData!!.folders.first().title)
        assertNull(harness.state.folderSheet)
    }

    /**
     * The acceptance's own sentence: a folder move refused (offline) puts the row
     * back and offers « Réessayer ».
     */
    @Test
    fun aRefusedMoveRollsBackWithRetry() {
        val service = FakeLibrary()
        service.failMove = IllegalStateException("Hors ligne")
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)
        val before = harness.state.treeData!!

        harness.model.openMoveSheet(LibraryFixtures.MATIN)
        assertEquals("Matin", harness.state.moveTarget?.title)
        harness.model.movePlaylist(LibraryFixtures.VOYAGE)

        val after = harness.state.treeData!!
        assertEquals(
            "the row is back where it was",
            before.playlists.first { it.id == LibraryFixtures.MATIN }.folderId,
            after.playlists.first { it.id == LibraryFixtures.MATIN }.folderId,
        )
        assertTrue("${harness.toasts}", harness.toasts.any { it.contains("Impossible de déplacer la playlist") })
        assertTrue(harness.toasts.any { it.contains("Réessayer") })
        assertEquals("the sheet stays open for the retry", false, harness.state.moveTarget?.busy)
    }

    /** A successful move is optimistic, then the server's own tree replaces it. */
    @Test
    fun aMoveIsOptimisticThenAuthoritative() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)

        harness.model.openMoveSheet(LibraryFixtures.MATIN)
        harness.model.movePlaylist(LibraryFixtures.VOYAGE)

        val row = harness.state.treeData!!.playlists.first { it.id == LibraryFixtures.MATIN }
        assertEquals(LibraryFixtures.VOYAGE, row.folderId)
        assertNull(harness.state.moveTarget)
    }

    /** Moving a playlist to where it already is asks nothing. */
    @Test
    fun aMoveToTheSameFolderIsNoRequest() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)
        val treeCalls = service.treeCalls

        harness.model.openMoveSheet(LibraryFixtures.MATIN)
        harness.model.movePlaylist(null)
        assertNull(harness.state.moveTarget)
        assertEquals(treeCalls, service.treeCalls)
    }

    /** A deleted folder's toast states how many playlists it released. */
    @Test
    fun aDeletedFolderSaysWhatItReleased() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)

        harness.model.deleteFolder(LibraryFixtures.VOYAGE)
        assertTrue(
            "the toast states the release: ${harness.toasts}",
            harness.toasts.any { it.contains("1 playlist déplacée") },
        )
        assertFalse(harness.state.treeData!!.folders.any { it.id == LibraryFixtures.VOYAGE })
    }

    /** A refused deletion brings the folder back with its cause. */
    @Test
    fun aRefusedDeletionBringsTheFolderBack() {
        val service = FakeLibrary()
        service.failDelete = IllegalStateException("Hors ligne")
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)

        harness.model.deleteFolder(LibraryFixtures.VOYAGE)
        assertTrue(harness.state.treeData!!.folders.any { it.id == LibraryFixtures.VOYAGE })
        assertTrue(harness.toasts.any { it.contains("Impossible de supprimer le dossier") })
    }

    /**
     * Three presses on « Monter » while ONE write is in flight are TWO requests:
     * the SPA's 400 ms debounce, restated as one write in flight plus the latest
     * order — never a storm of PUTs.
     */
    @Test
    fun threeOrdersInARowAreTwoRequests() {
        val service = FakeLibrary()
        val gate = CompletableDeferred<Unit>()
        service.gateOrder = gate
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)
        harness.model.openOrganizer()

        // The first press starts a write that is held; the next two only move the
        // tree (the SPA's debounce).
        harness.model.organizeMove(1, -1)
        harness.model.organizeMove(1, -1)
        harness.model.organizeMove(1, -1)
        assertEquals("one write in flight", 1, service.orders.size)
        assertTrue(harness.state.organizeStatus, harness.state.organizeStatus.contains("position 1 sur 2"))

        // The gate opens: the pending order goes out once, with the LAST order.
        service.gateOrder = null
        gate.complete(Unit)
        assertEquals(2, service.orders.size)
        assertEquals(
            "the latest order wins",
            harness.state.treeData!!.folders.map { it.id },
            service.orders.last(),
        )
    }

    /** « Organiser » shows the folders, then a container's playlists. */
    @Test
    fun theOrganizerMovesBetweenItsContainers() {
        val harness = Harness().loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)
        harness.model.openOrganizer()
        assertTrue(harness.state.organizing)
        assertEquals(2, harness.state.organizeRows.size)
        assertEquals("Voyage", harness.state.organizeRows.first().title)
        assertEquals("1 playlist", harness.state.organizeRows.first().detail)

        harness.model.pickContainer(LibraryModel.CONTAINER_ROOT)
        assertEquals("every unfiled playlist, shared ones included", 3, harness.state.organizeRows.size)
        assertEquals("Matin", harness.state.organizeRows.first().title)

        harness.model.closeOrganizer()
        assertFalse(harness.state.organizing)
    }

    /** A section's chevron folds it, and the query filters the cards. */
    @Test
    fun aSectionFoldsAndTheQueryFilters() {
        val harness = Harness().loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)

        harness.model.toggleSection("folder:${LibraryFixtures.VOYAGE}")
        assertTrue(harness.state.sections.first().folded)
        harness.model.toggleSection("folder:${LibraryFixtures.VOYAGE}")
        assertFalse(harness.state.sections.first().folded)

        harness.model.filterPlaylists("import")
        assertEquals(1, harness.state.sections.size)
        assertEquals("Import Deezer", harness.state.sections.single().rows.single().title)
    }

    /** « Je n'aime pas »: one removal, then the list is read again. */
    @Test
    fun removingADislikeAsksOneMutationThenReadsTheList() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Dislikes)
        assertEquals(1, service.dislikeCalls)

        harness.model.removeDislike(LibraryFixtures.row("Trop fort", 0, "Bruit"))
        assertEquals(listOf("row-0"), service.removed)
        assertEquals("the list is read again after the mutation", 2, service.dislikeCalls)
    }

    @Test
    fun aRefusedRemovalSaysSoAndReadsTheListAgain() {
        val service = FakeLibrary()
        service.failRemoveDislike = IllegalStateException("Hors ligne")
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Dislikes)

        harness.model.removeDislike(LibraryFixtures.row("Trop fort", 0, "Bruit"))
        assertTrue(harness.toasts.any { it.contains("Impossible de retirer ce titre") })
        assertEquals(2, service.dislikeCalls)
    }

    /** Rule 7: an older server hides the folder affordances, and a doubt hides
     * nothing. */
    @Test
    fun anOlderServerIsRememberedAndADoubtIsNot() {
        val older = FakeLibrary(supported = false)
        val first = Harness(service = older).loaded()
        assertEquals(false, first.state.foldersSupported)

        val doubt = FakeLibrary(supported = null)
        val second = Harness(service = doubt).loaded()
        assertNull(second.state.foldersSupported)
        assertEquals("the probe is one call, never one per leaf", 1, doubt.probeCalls)
    }

    /** The empty states the SPA states, in the same places. */
    @Test
    fun theEmptyStatesAreTheSpas() {
        val empty = FakeLibrary(
            cachedFavorites = FakeRows(emptyList()),
            freshFavorites = FakeRows(emptyList()),
        )
        val harness = Harness(service = empty).loaded()
        assertTrue(harness.state.favoriteRows != null)
        assertEquals(0, harness.state.favoriteRows!!.len())
        assertEquals("an empty loaded list states its zero", "0 titre", harness.state.counts)
        harness.model.filterFavorites("rien")
        assertEquals(0, harness.state.favoriteRows!!.len())
        assertEquals("rien", harness.state.favoriteFilter)
    }

    // -- la création d'une playlist --------------------------------------------

    /**
     * « Nouvelle playlist » : UNE création, l'arbre relu (le `userPlaylists(true)`
     * du SPA), et l'identifiant rendu à la coquille qui ouvre la liste.
     */
    @Test
    fun creatingAPlaylistOpensItAndReloads() {
        val service = FakeLibrary()
        val harness = Harness(service = service).loaded()
        harness.model.selectLeaf(LibraryLeaf.Playlists)
        assertEquals("the leaf's own read", 1, service.treeCalls)

        harness.model.newPlaylist("   ")
        assertEquals("a blank name asks nothing", emptyList<String>(), service.created)

        harness.model.newPlaylist("  Test  ")
        assertEquals(listOf("Test"), service.created)
        assertEquals("the tree is read again, not guessed", 2, service.treeCalls)
        assertEquals("41", harness.state.openPlaylist)
        assertTrue("the reloaded tree is on screen", harness.state.treeData != null)

        harness.model.consumeOpenPlaylist()
        assertNull("the shell followed it once", harness.state.openPlaylist)
    }

    /** Une création refusée dit sa cause et n'ouvre rien. */
    @Test
    fun aRefusedCreationNamesItsCauseAndOpensNothing() {
        val service = FakeLibrary()
        service.failCreatePlaylist = IllegalStateException("Hors ligne")
        val harness = Harness(service = service).loaded()

        harness.model.newPlaylist("Test")

        assertTrue(
            "the toast names the cause: ${harness.toasts}",
            harness.toasts.any {
                it.startsWith("Impossible de créer la playlist : ") && it.contains("Hors ligne")
            },
        )
        assertNull(harness.state.openPlaylist)
    }
}
