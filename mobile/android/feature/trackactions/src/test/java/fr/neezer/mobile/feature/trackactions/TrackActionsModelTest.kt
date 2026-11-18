package fr.neezer.mobile.feature.trackactions

import fr.neezer.mobile.bridge.NzNavigator
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.MenuAction
import uniffi.nz_ffi.MenuActor
import uniffi.nz_ffi.MenuContext
import uniffi.nz_ffi.MenuEntry
import uniffi.nz_ffi.MutationReport
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PlaylistChoice
import uniffi.nz_ffi.ReplaceAnswer
import uniffi.nz_ffi.TrackRow

/**
 * The track menu's logic, on the JVM: the menu's own spec (0 requests), the
 * optimistic writes with their rollback, the picker's copy-then-answer, the
 * replacement's one read and one commit, and every player command a tap sends.
 *
 * Fakes stand in for the core and the player, so "the menu costs no request" is a
 * COUNT of calls, not a hope — the screen standard's own measure
 * (`webapp/test/menu/run.mjs` holds the SPA to the same 0).
 */
private fun menuContextOf(context: TrackActionsContext): MenuContext = menuContext(
    surface = context.surface,
    index = context.index,
    currentIndex = context.currentIndex,
    canKeep = context.canKeep,
    canBrowse = context.canBrowse,
    remote = context.remote,
    downloaded = context.downloaded,
    episode = context.episode,
    episodePlaying = context.episodePlaying,
    channelId = context.channelId,
    offline = context.offline,
    genreTag = context.genreTag,
    markers = context.markers,
)

class TrackActionsModelTest {

    // -- fakes -----------------------------------------------------------------

    private class FakeService : TrackActionsService {
        var menuCalls = 0
        var playlistCalls = 0
        var cachedPlaylistCalls = 0
        var entries: List<MenuEntry> = TrackActionsFixtures.entries()
        var cached: List<PlaylistChoice>? = null
        var lists: List<PlaylistChoice> = listOf(
            TrackActionsFixtures.choice(),
            TrackActionsFixtures.choice(
                id = TrackActionsFixtures.OTHER_PLAYLIST_ID,
                title = "Le matin",
                nbTracks = 1,
            ),
            TrackActionsFixtures.choice(
                id = "33333333-3333-4333-8333-333333333333",
                title = "Celle de June",
                editable = false,
                role = "viewer",
                owner = "June",
            ),
        )
        var failLists: Exception? = null
        var failLike: Exception? = null
        var failDislike: Exception? = null
        var failAdd: Exception? = null
        var failCreate: Exception? = null
        var failCandidates: Exception? = null
        var failReplace: Exception? = null
        var failDelete: Exception? = null
        var failShare: Exception? = null
        var admin = false
        var lastPlaylist: uniffi.nz_ffi.LastPlaylist? = null
        val likes = mutableListOf<Pair<String, Boolean>>()
        val dislikes = mutableListOf<Pair<String, Boolean>>()
        val adds = mutableListOf<Triple<String, String, List<String>>>()
        val creates = mutableListOf<Pair<String, List<String>>>()
        val candidateQueries = mutableListOf<Triple<String, String, Boolean>>()
        val replaces = mutableListOf<Pair<String, String>>()
        val statuses = mutableListOf<String>()
        val deletes = mutableListOf<List<String>>()
        var candidates = listOf(TrackActionsFixtures.candidate())
        var fallbackCandidates = listOf(
            TrackActionsFixtures.candidate(
                key = "youtube:abc",
                id = "abc",
                title = "Ouverture (live)",
                source = "youtube",
                local = false,
            ),
        )
        var fallbackAuto: String? = null
        var query = "Ascenseur Ouverture"
        var jobsLeft = 1

        override fun actor(): MenuActor = MenuActor(admin = admin, lastPlaylist = lastPlaylist)

        override fun menu(row: TrackRow, ctx: MenuContext): List<MenuEntry> {
            menuCalls++
            return if (row.liked) TrackActionsFixtures.entries(liked = true) else entries
        }

        override fun cachedPlaylists(): List<PlaylistChoice>? {
            cachedPlaylistCalls++
            return cached
        }

        override suspend fun playlists(): List<PlaylistChoice> {
            playlistCalls++
            failLists?.let { throw it }
            return lists
        }

        override suspend fun addToPlaylist(
            playlistId: String,
            title: String,
            keys: List<String>,
        ): Int {
            failAdd?.let { throw it }
            adds += Triple(playlistId, title, keys)
            return keys.size
        }

        override suspend fun createPlaylist(title: String, keys: List<String>): PlaylistChoice {
            failCreate?.let { throw it }
            creates += title to keys
            return TrackActionsFixtures.choice(id = "55555555-5555-4555-8555-555555555555", title = title)
        }

        override suspend fun like(key: String) {
            failLike?.let { throw it }
            likes += key to true
        }

        override suspend fun unlike(key: String) {
            failLike?.let { throw it }
            likes += key to false
        }

        override suspend fun dislike(key: String) {
            failDislike?.let { throw it }
            dislikes += key to true
        }

        override suspend fun undislike(key: String) {
            failDislike?.let { throw it }
            dislikes += key to false
        }

        override suspend fun replaceCandidates(
            key: String,
            query: String,
            fallbacks: Boolean,
        ): ReplaceAnswer {
            candidateQueries += Triple(key, query, fallbacks)
            failCandidates?.let { throw it }
            return if (fallbacks) {
                ReplaceAnswer(
                    query = this.query,
                    candidates = fallbackCandidates,
                    fallbackAuto = "youtube",
                )
            } else {
                ReplaceAnswer(
                    query = if (query.isEmpty()) this.query else query,
                    candidates = if (query.isEmpty()) candidates else listOf(TrackActionsFixtures.candidate(title = "Version de $query")),
                    fallbackAuto = fallbackAuto,
                )
            }
        }

        override suspend fun replace(key: String, candidate: String): MutationReport {
            failReplace?.let { throw it }
            replaces += key to candidate
            return MutationReport(
                job = "job-replace",
                running = true,
                ok = false,
                playlists = 0u,
                favorites = 0u,
                deleted = emptyList(),
                kept = emptyList(),
                missing = emptyList(),
                done = 0u,
                total = 0u,
                error = null,
            )
        }

        override suspend fun replaceStatus(job: String): MutationReport {
            statuses += job
            jobsLeft--
            return MutationReport(
                job = job,
                running = jobsLeft > 0,
                ok = true,
                playlists = 2u,
                favorites = 1u,
                deleted = emptyList(),
                kept = emptyList(),
                missing = emptyList(),
                done = 0u,
                total = 0u,
                error = null,
            )
        }

        override suspend fun deleteTracks(keys: List<String>): MutationReport {
            failDelete?.let { throw it }
            deletes += keys
            return MutationReport(
                job = "job-delete",
                running = true,
                ok = false,
                playlists = 0u,
                favorites = 0u,
                deleted = emptyList(),
                kept = emptyList(),
                missing = emptyList(),
                done = 0u,
                total = 1u,
                error = null,
            )
        }

        override suspend fun deleteStatus(job: String): MutationReport = replaceStatus(job)

        override suspend fun deleteUploads(keys: List<String>): MutationReport = deleteTracks(keys)

        override suspend fun shareLink(kind: String, id: String): String {
            failShare?.let { throw it }
            return "https://neezer.local/api/share/file/$id"
        }
    }

    private class FakePlayer : TrackActionPlayer {
        val next = mutableListOf<String>()
        val queued = mutableListOf<String>()
        val removed = mutableListOf<Pair<String, Int>>()
        val radios = mutableListOf<String>()

        override fun playNext(row: TrackRow) {
            next += row.key
        }

        override fun addToQueue(row: TrackRow) {
            queued += row.key
        }

        override fun removeFromQueue(key: String, index: Int) {
            removed += key to index
        }

        override fun startRadio(row: TrackRow) {
            radios += row.key
        }
    }

    private class RecordingNav : NzNavigator {
        val visited = mutableListOf<AppLink>()

        override fun go(link: AppLink) {
            visited += link
        }

        override fun back() = Unit

        override fun openPlayer() = Unit

        override fun openQueue() = Unit
    }

    /** One toast, as a test reads it: its sentence, its action, and the action's body. */
    private class ToastLog {
        data class Entry(val text: String, val action: String?, val run: (() -> Unit)?)

        val entries = mutableListOf<Entry>()

        fun last(): Entry = entries.last()

        fun texts(): List<String> = entries.map { it.text }
    }

    private class Harness(
        val service: FakeService = FakeService(),
        val player: FakePlayer = FakePlayer(),
        val nav: RecordingNav = RecordingNav(),
        val sharer: RecordingSharer = RecordingSharer(),
        val context: TrackActionsContext = TrackActionsContext(),
        val creditsDoor: Boolean = true,
        val closeHost: () -> Unit = {},
    ) {
        val toasts = ToastLog()
        var closed = 0
        val model = TrackActionsModel(
            service = service,
            player = player,
            nav = nav,
            sharer = sharer,
            context = menuContextOf(context),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            toast = { text, label, run -> toasts.entries += ToastLog.Entry(text, label, run) },
            // The real one is `NzErrors.text` → `nz_error_text`, which reads the
            // native library; a JVM test cannot load it, so the cause's sentence
            // is the fake's own (the SHAPE of a failure is what is under test).
            causeText = { it.message ?: it::class.java.simpleName },
            creditsDoor = creditsDoor,
            closeHost = {
                closed++
                closeHost()
            },
        )

        init {
            model.open(TrackActionsFixtures.row())
        }
    }

    // -- the menu itself -------------------------------------------------------

    /// The sheet draws what the CORE built, exactly once, and opening it costs no
    /// request (`menu/run.mjs`'s own measure).
    @Test
    fun opening_the_menu_asks_the_core_once_and_no_request() {
        val harness = Harness()
        assertEquals(1, harness.service.menuCalls)
        assertEquals(0, harness.service.playlistCalls)
        assertEquals(0, harness.service.candidateQueries.size)
        assertTrue(harness.model.state.value.entries.contains(MenuEntry.Divider))
        assertEquals(
            "Lire ensuite",
            (harness.model.state.value.entries.first() as MenuEntry.Item).item.label,
        )
    }

    /// A like flips the ROW the builder reads, so the next opening says "Retirer
    /// des favoris" — without a second call into the builder being a request.
    @Test
    fun the_heart_reads_the_rows_own_flag() {
        val harness = Harness()
        harness.model.setFavorite(true)
        assertEquals(listOf(TrackActionsFixtures.KEY to true), harness.service.likes)
        assertTrue(harness.model.state.value.liked)
        // One toast says it worked; the sheet is closed (an entry did its job).
        assertEquals("Ajouté aux favoris", harness.toasts.last().text)
        assertEquals(1, harness.closed)
    }

    /// A refused like rolls back and offers ONE « Réessayer », which runs the very
    /// same call again (one click = one request).
    @Test
    fun a_refused_like_rolls_back_and_its_retry_asks_again() {
        val service = FakeService()
        service.failLike = NzException.Offline()
        val harness = Harness(service = service)
        harness.model.setFavorite(true)
        assertFalse("rolled back", harness.model.state.value.liked)
        val toast = harness.toasts.last()
        assertEquals("Réessayer", toast.action)
        assertTrue(toast.text.contains("Échec de la mise à jour du favori"))

        service.failLike = null
        toast.run?.invoke()
        assertTrue(harness.model.state.value.liked)
        assertEquals(1, service.likes.size)
    }

    /// "Je n'aime pas" is a different store: it never touches the heart, and its
    /// toast offers to undo it (`toggleDislike`'s "Annuler").
    @Test
    fun the_thumb_is_its_own_write_with_an_undo() {
        val harness = Harness()
        harness.model.setDislike(true)
        assertEquals(listOf(TrackActionsFixtures.KEY to true), harness.service.dislikes)
        assertTrue(harness.model.state.value.disliked)
        assertFalse(harness.model.state.value.liked)
        val undo = harness.toasts.last()
        assertEquals("Annuler", undo.action)
        undo.run?.invoke()
        assertEquals(
            "the undo is the opposite write, not a second dislike",
            listOf(true, false),
            harness.service.dislikes.map { it.second },
        )
        assertFalse(harness.model.state.value.disliked)
    }

    /// The transport verbs are PLAYER commands: no request of this lot at all.
    @Test
    fun the_transport_verbs_are_player_commands() {
        val harness = Harness()
        harness.model.run(MenuAction.PlayNext)
        harness.model.run(MenuAction.AddToQueue)
        harness.model.run(MenuAction.StartRadio)
        assertEquals(listOf(TrackActionsFixtures.KEY), harness.player.next)
        assertEquals(listOf(TrackActionsFixtures.KEY), harness.player.queued)
        assertEquals(listOf(TrackActionsFixtures.KEY), harness.player.radios)
        assertEquals(0, harness.service.playlistCalls)
        assertEquals(0, harness.service.candidateQueries.size)
    }

    /// A queue row is removed BY KEY at its own index, so a playhead that moved
    /// under an open menu cannot remove the wrong row.
    @Test
    fun a_queue_row_is_removed_by_key_at_its_index() {
        val harness = Harness(context = TrackActionsContext(surface = "queue", index = 3, currentIndex = 1))
        harness.model.run(MenuAction.RemoveFromQueue(TrackActionsFixtures.KEY))
        assertEquals(listOf(TrackActionsFixtures.KEY to 3), harness.player.removed)
    }

    /// The links are the shell's routes: artist, album, show.
    @Test
    fun the_links_go_through_the_shells_routes() {
        val harness = Harness()
        harness.model.run(MenuAction.GoToArtist("aaaaaaaa-1111-4111-8111-111111111111"))
        harness.model.run(MenuAction.GoToAlbum("501"))
        harness.model.run(MenuAction.OpenPodcast("dddddddd-1111-4111-8111-111111111111"))
        assertEquals(
            listOf(
                AppLink.Artist("aaaaaaaa-1111-4111-8111-111111111111"),
                AppLink.Album("501"),
                AppLink.Podcast("dddddddd-1111-4111-8111-111111111111"),
            ),
            harness.nav.visited,
        )
    }

    /// "Voir les crédits" sets the subject MB7's sheet draws; without its door the
    /// entry says so instead of opening nothing.
    @Test
    fun the_credits_entry_needs_its_door() {
        val withDoor = Harness()
        withDoor.model.run(MenuAction.OpenCredits)
        assertEquals(TrackActionsFixtures.KEY, withDoor.model.state.value.credits?.key)
        assertEquals(0, withDoor.closed)

        val without = Harness(creditsDoor = false)
        without.model.run(MenuAction.OpenCredits)
        assertNull(without.model.state.value.credits)
        assertTrue(without.toasts.last().text.contains("crédits"))
    }

    /// The accordion: one submenu open at a time, a second tap closes it.
    @Test
    fun a_submenu_opens_and_closes_in_place() {
        val harness = Harness()
        val parent = TrackActionsFixtures.item(
            "Aller à l'artiste",
            MenuAction.GoToArtist("a"),
            sub = listOf(TrackActionsFixtures.item("June", MenuAction.GoToArtist("b"))),
        )
        harness.model.toggleSub(parent)
        assertEquals("Aller à l'artiste", harness.model.state.value.expanded)
        harness.model.toggleSub(parent)
        assertNull(harness.model.state.value.expanded)
    }

    // -- the picker ------------------------------------------------------------

    /// The copy paints first, the answer replaces it: a warm opening costs the
    /// copy's read and ONE request, and the sheet says « mémorisée » until then.
    @Test
    fun the_picker_paints_the_copy_then_the_answer() {
        val service = FakeService()
        service.cached = listOf(TrackActionsFixtures.choice(title = "Route du soir (copie)"))
        val harness = Harness(service = service)
        harness.model.openPicker()
        assertEquals(1, service.cachedPlaylistCalls)
        assertEquals(1, service.playlistCalls)
        assertEquals(TrackActionsModel.Sheet.Picker, harness.model.state.value.sheet)
        val picker = harness.model.state.value.picker!!
        assertEquals(3, picker.lists?.size)
        assertFalse(picker.stale)
    }

    /// A dead link with a copy keeps it and says so; with NO copy it is a cause and
    /// a working « Réessayer » (never a spinner).
    @Test
    fun a_dead_link_keeps_the_copy_or_names_the_cause() {
        val service = FakeService()
        service.cached = listOf(TrackActionsFixtures.choice())
        service.failLists = TrackActionsFixtures.offline()
        val kept = Harness(service = service)
        kept.model.openPicker()
        val picker = kept.model.state.value.picker!!
        assertEquals(1, picker.lists?.size)
        assertTrue(picker.stale)
        assertNull(picker.error)

        val empty = FakeService()
        empty.failLists = TrackActionsFixtures.offline()
        val failed = Harness(service = empty)
        failed.model.openPicker()
        val broken = failed.model.state.value.picker!!
        assertNull(broken.lists)
        assertNotNull(broken.error)
        assertTrue(broken.error!!.contains("Hors ligne"))

        empty.failLists = null
        failed.model.loadPicker()
        assertEquals(3, failed.model.state.value.picker?.lists?.size)
        assertEquals(2, empty.playlistCalls)
    }

    /// Only a writable list is a target, the filter narrows them, and the last one
    /// used is pinned first.
    @Test
    fun the_picker_offers_only_writable_lists_and_pins_the_last_one() {
        val service = FakeService()
        service.lists = listOf(
            TrackActionsFixtures.choice(id = "a", title = "Zèbre"),
            TrackActionsFixtures.choice(id = "b", title = "Ascenseur", recent = true),
            TrackActionsFixtures.choice(id = "c", title = "Celle de June", editable = false, role = "viewer"),
        )
        val harness = Harness(service = service)
        harness.model.openPicker()
        val picker = harness.model.state.value.picker!!
        assertEquals(listOf("b", "a"), picker.shown.map { it.id })
        harness.model.filter("asc")
        assertEquals(listOf("b"), harness.model.state.value.picker!!.shown.map { it.id })
        // Six lists is the threshold of the SPA's filter box.
        assertFalse(picker.filterShown)
    }

    /// A tick adds ONCE, and a refusal unticks it with « Réessayer ».
    @Test
    fun a_tick_adds_once_and_a_refusal_unticks_it() {
        val service = FakeService()
        val harness = Harness(service = service)
        harness.model.openPicker()
        harness.model.addToPlaylist(TrackActionsFixtures.PLAYLIST_ID, "Route du soir")
        harness.model.addToPlaylist(TrackActionsFixtures.PLAYLIST_ID, "Route du soir")
        assertEquals(1, service.adds.size)
        assertEquals(listOf(TrackActionsFixtures.KEY), service.adds[0].third)
        assertEquals("Ajouté à « Route du soir »", harness.toasts.last().text)

        val failing = FakeService()
        failing.failAdd = TrackActionsFixtures.offline()
        val refused = Harness(service = failing)
        refused.model.openPicker()
        refused.model.addToPlaylist(TrackActionsFixtures.OTHER_PLAYLIST_ID, "Le matin")
        assertFalse(
            refused.model.state.value.picker!!.added.contains(TrackActionsFixtures.OTHER_PLAYLIST_ID),
        )
        assertEquals("Réessayer", refused.toasts.last().action)

        failing.failAdd = null
        refused.toasts.last().run?.invoke()
        assertTrue(
            refused.model.state.value.picker!!.added.contains(TrackActionsFixtures.OTHER_PLAYLIST_ID),
        )
    }

    /// The create form: the suggested name is the next "Ma playlist N", and only
    /// the server's answer closes the sheet.
    @Test
    fun creating_a_playlist_waits_for_the_servers_answer() {
        val service = FakeService()
        service.lists = listOf(
            TrackActionsFixtures.choice(id = "a", title = "Ma playlist 1"),
            TrackActionsFixtures.choice(id = "b", title = "Ma playlist 2"),
        )
        val harness = Harness(service = service)
        harness.model.openPicker()
        assertEquals("Ma playlist 3", harness.model.state.value.picker!!.suggestion)
        harness.model.startCreating()
        harness.model.createPlaylist()
        assertEquals("Ma playlist 3", service.creates.single().first)
        assertEquals(listOf(TrackActionsFixtures.KEY), service.creates.single().second)
        assertEquals("Ajouté à « Ma playlist 3 »", harness.toasts.last().text)
        assertEquals(1, harness.closed)
    }

    // -- the replacement sheet -------------------------------------------------

    /// One read for the candidates, and a typed query REPLACES the list rather
    /// than appending to it (`ReplaceSheet.svelte#load`).
    @Test
    fun the_replacement_sheet_reads_once_and_a_typed_search_replaces_it() {
        val service = FakeService()
        val harness = Harness(service = service)
        harness.model.openReplace()
        assertEquals(1, service.candidateQueries.size)
        assertEquals(TrackActionsFixtures.KEY, service.candidateQueries[0].first)
        assertEquals("", service.candidateQueries[0].second)
        assertEquals("Ascenseur Ouverture", harness.model.state.value.replace!!.query)
        assertEquals(1, harness.model.state.value.replace!!.candidates.size)

        harness.model.searchReplace("June")
        assertEquals(Triple(TrackActionsFixtures.KEY, "June", false), service.candidateQueries[1])
        assertEquals(
            listOf("Version de June"),
            harness.model.state.value.replace!!.candidates.map { it.title },
        )
    }

    /// The fallback search APPENDS what is new and never duplicates a candidate.
    @Test
    fun the_fallback_search_appends_and_never_duplicates() {
        val service = FakeService()
        val harness = Harness(service = service)
        harness.model.openReplace()
        harness.model.searchFallbacks()
        assertEquals(true, service.candidateQueries.last().third)
        val replace = harness.model.state.value.replace!!
        assertEquals(2, replace.candidates.size)
        assertEquals("youtube", replace.fallbackAuto)
        // A second press is not a second search.
        harness.model.searchFallbacks()
        assertEquals(2, service.candidateQueries.size)
    }

    /// A candidates read that failed names its cause and retries from the sheet.
    @Test
    fun a_failed_candidates_read_retries_from_the_sheet() {
        val service = FakeService()
        service.failCandidates = TrackActionsFixtures.offline()
        val harness = Harness(service = service)
        harness.model.openReplace()
        val broken = harness.model.state.value.replace!!
        assertNotNull(broken.error)
        assertFalse(broken.loading)

        service.failCandidates = null
        harness.model.retryCandidates()
        assertNull(harness.model.state.value.replace!!.error)
        assertEquals(1, harness.model.state.value.replace!!.candidates.size)
    }

    /// "Remplacer" is ONE request, then the server's job says what it changed.
    @Test
    fun replacing_commits_one_request_then_watches_the_job() {
        val service = FakeService()
        service.jobsLeft = 1
        val harness = Harness(service = service)
        harness.model.openReplace()
        val candidate = TrackActionsFixtures.candidate()
        harness.model.commitReplace(candidate)
        assertEquals(listOf(TrackActionsFixtures.KEY to candidate.id), service.replaces)
        assertEquals(1, harness.closed)
        assertEquals("Remplacement en cours…", harness.toasts.texts().first())
    }

    /// "Supprimer ce titre" asks twice, and a `409` is the sentence that says the
    /// title turned out playable.
    @Test
    fun deleting_asks_twice_and_names_a_playable_title() {
        val service = FakeService()
        val harness = Harness(service = service)
        harness.model.openReplace()
        harness.model.deleteTarget()
        assertTrue(harness.model.state.value.replace!!.confirming)
        assertEquals(0, service.deletes.size)
        harness.model.deleteTarget()
        assertEquals(listOf(listOf(TrackActionsFixtures.KEY)), service.deletes)

        val refused = FakeService()
        refused.failDelete = NzException.Conflict("track is available")
        val refusedHarness = Harness(service = refused)
        refusedHarness.model.openReplace()
        refusedHarness.model.deleteTarget()
        refusedHarness.model.deleteTarget()
        assertEquals(
            "Ce titre est en fait disponible — rien n'a été supprimé",
            refusedHarness.toasts.last().text,
        )
    }

    // -- sharing ---------------------------------------------------------------

    /// "Partager…" asks the core for the address and hands it to the platform's
    /// own sheet — the link is the person's to send.
    @Test
    fun sharing_hands_the_core_link_to_the_platform() {
        val harness = Harness()
        harness.model.share()
        assertEquals(1, harness.sharer.shared.size)
        val (link, title) = harness.sharer.shared.single()
        assertTrue(link.endsWith("/api/share/file/${TrackActionsFixtures.ID}"))
        assertTrue(title.contains("Ouverture"))
        assertEquals(1, harness.closed)
    }

    @Test
    fun a_failed_share_offers_its_retry() {
        val service = FakeService()
        service.failShare = TrackActionsFixtures.offline()
        val harness = Harness(service = service)
        harness.model.share()
        assertEquals(0, harness.sharer.shared.size)
        assertEquals("Réessayer", harness.toasts.last().action)
        service.failShare = null
        harness.toasts.last().run?.invoke()
        assertEquals(1, harness.sharer.shared.size)
    }

    // -- what the host does ----------------------------------------------------

    /// Closing the menu closes the HOST, once, whatever the path.
    @Test
    fun closing_the_sheet_tells_the_host_once() {
        val harness = Harness()
        harness.model.close()
        assertTrue(harness.model.state.value.closed)
        assertEquals(1, harness.closed)
    }

    /// A surface's own facts reach the builder: a queue passes its surface and
    /// its playhead, so the menu is the queue's own.
    @Test
    fun the_surfaces_facts_reach_the_builder() {
        val harness = Harness(
            context = TrackActionsContext(surface = "queue", index = 4, currentIndex = 2),
        )
        val built = harness.model.mergedContext()
        assertEquals("queue", built.surface)
        assertEquals(4, built.index?.toInt())
    }
}
