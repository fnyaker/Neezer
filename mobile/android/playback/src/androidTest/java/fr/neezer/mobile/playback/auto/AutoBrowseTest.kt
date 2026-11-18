// MB25's acceptance, on a headless emulator: a MediaBrowser — the client a head
// unit is — browses the car's tree, opens a playlist and plays it, and every
// number is counted, never timed.
//
// Why a MediaBrowser and not the Desktop Head Unit: DHU drives the phone-side
// "Android Auto" app, which a `default` (non-Google) system image does not ship;
// docs/agents/mobile.md's MB25 acceptance allows the DHU "run headless, or
// Media3's `MediaBrowser` test client". This is that client, over the REAL
// service, the real Rust tree and the real player. Nothing touches the machine's
// display or speakers: the guest's audio goes to the emulator's null HAL
// (`-no-audio`).
//
// The server is `mobile/tools/perf-server.sh start --port 5595` (8 000 tracks,
// 4 000 favourites, 60 playlists, the first of them 4 000 titles), reached from
// the guest at `10.0.2.2:5595`. A host without it SKIPS these tests with the
// cause, as MB3c's own bench does.
package fr.neezer.mobile.playback.auto

import android.app.Application
import android.content.ComponentName
import android.os.Looper
import android.os.SystemClock
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.session.MediaBrowser
import androidx.media3.session.MediaLibraryService.LibraryParams
import androidx.media3.session.SessionToken
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.common.util.concurrent.ListenableFuture
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.playback.NzMediaService
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.NzCore
import uniffi.nz_ffi.PlayerApi

@RunWith(AndroidJUnit4::class)
class AutoBrowseTest {
    private lateinit var app: Application
    private lateinit var core: NzCore
    private lateinit var api: PlayerApi
    private var browser: MediaBrowser? = null

    /**
     * One latch per node, fed by the tree's own `notifyChildrenChanged`.
     *
     * Only a notification carrying ROWS releases it: Media3 also tells a freshly
     * subscribed browser that a parent has 0 children, and an empty answer from a
     * node the device has never seen is the same count — waiting on either would
     * end the wait before the read even left the device.
     */
    private val announced = ConcurrentHashMap<String, CountDownLatch>()

    @Before
    fun install() {
        app =
            InstrumentationRegistry.getInstrumentation()
                .targetContext.applicationContext as Application
        // `Application.onCreate` is a main-thread call, and `NzBridge.install`
        // reaches `ProcessLifecycleOwner`, which refuses another thread.
        if (NzBridge.core == null) {
            InstrumentationRegistry.getInstrumentation().runOnMainSync { NzBridge.install(app) }
        }
        core = NzBridge.core ?: error("le noyau Rust n'a pas démarré : ${NzBridge.failure}")
        api = core.player()
        // A COLD tree: the point of the acceptance is what a browse costs, and a
        // copy left by a previous run would turn "1 request" into "0".
        File(app.cacheDir, "neezer/auto").deleteRecursively()
        val reachable =
            runBlocking {
                runCatching { core.setServer(serverUrl()) }.isSuccess &&
                    runCatching { core.login("bench", "Bench1") }.isSuccess
            }
        assumeTrue(
            "serveur de bench injoignable à ${serverUrl()} : " +
                "mobile/tools/perf-server.sh start --port 5595",
            reachable,
        )
    }

    @After
    fun stop() {
        // Pause first: a released controller whose player still plays keeps the
        // service (and its warm tree) alive, and the next test would then browse
        // a node this one had already fetched.
        runCatching { onMain { browser?.pause(); browser?.release() } }
        browser = null
    }

    /**
     * **The acceptance.** The tree browses (root → Favoris → Playlists), one
     * playlist opens, and the car's play button queues and PLAYS it — with the
     * tree's own counts: 0 for the root, 1 for each node's first browse, 0 for
     * the browsed node's play.
     */
    @Test
    fun the_tree_browses_and_a_playlist_plays() {
        val browser = connect()

        // --- the root: five sections, no request at all --------------------
        val beforeRoot = autoRequests()
        val root = call("la racine") { browser.getLibraryRoot(LIBRARY_PARAMS) }.value
        assertNotNull("la racine n'a pas de valeur", root)
        assertEquals(AutoNode.ROOT_TITLE, root!!.mediaMetadata.title.toString())
        val sections = children(browser, AutoNode.ROOT)
        assertEquals(
            listOf("Favoris", "Playlists", "Mon Flow", "Récents", "Téléchargements"),
            sections.map { it.mediaMetadata.title.toString() },
        )
        assertEquals("ouvrir l'arbre ne coûte aucune requête", beforeRoot, autoRequests())

        // --- Récents: THE cold node (no other test browses it) ------------
        // 1 request, one row per listen, and none once the device has it.
        val recent = browsed(browser, AutoNode.RECENT, expect = 30)
        assertEquals("le banc sème 30 écoutes", 30, recent.size)
        assertEquals(
            "une lecture froide = une requête (comptée : ${autoRequests() - beforeRoot})",
            1uL,
            autoRequests() - beforeRoot,
        )
        val warmBefore = autoRequests()
        assertEquals(30, children(browser, AutoNode.RECENT).size)
        assertEquals("un nœud chaud ne coûte rien", warmBefore, autoRequests())

        // --- Favoris: 200 titles, 100 rows --------------------------------
        val favorites = browsed(browser, AutoNode.FAVORITES, expect = 100)
        assertEquals("200 favoris se ramènent à 100 lignes", 100, favorites.size)

        // --- Playlists, then the big one opens ---------------------------
        val playlists = browsed(browser, AutoNode.PLAYLISTS, expect = 6)
        assertEquals("le banc sème 6 playlists", 6, playlists.size)
        val big = playlists.first { it.mediaMetadata.title.toString().startsWith("Playlist 0 ") }
        val tracks = browsed(browser, big.mediaId, expect = 100)
        assertEquals("une playlist de 120 titres montre 100 lignes", 100, tracks.size)
        assertTrue(
            "une ligne de playlist est un titre jouable",
            tracks.first().mediaMetadata.isPlayable == true &&
                tracks.first().mediaMetadata.isBrowsable != true,
        )

        // --- the tap: the browsed node plays, for free -------------------
        val requestsBeforePlay = autoRequests()
        // The tap instant, so the acceptance carries a tap → lecture number (an
        // elapsed time REPORTED, never a wait: the wait below is the controller's
        // own `isPlaying` event).
        val tappedAt = SystemClock.uptimeMillis()
        onMain {
            browser.setMediaItem(big)
            browser.prepare()
            browser.play()
        }
        val firstTitle = tracks.first().mediaMetadata.title.toString()
        assertTrue("la playlist ne joue pas", awaitPlaying(browser, firstTitle))
        val played = onMain { browser.currentMediaItem }
        assertNotNull("le lecteur n'a pas de titre courant", played)
        assertEquals(
            "un nœud déjà parcouru se joue sans requête d'arbre",
            requestsBeforePlay,
            autoRequests(),
        )
        assertEquals(
            "le premier titre joué est celui que la liste affichait",
            firstTitle,
            played!!.mediaMetadata.title.toString(),
        )
        assertEquals(
            "la file est exactement ce que la voiture a affiché",
            tracks.size,
            onMain { browser.mediaItemCount },
        )
        val tapMs = SystemClock.uptimeMillis() - tappedAt
        assertTrue("un tap de voiture doit répondre en moins de 5 s (mesuré $tapMs ms)", tapMs < 5_000)
        val myself = api.snapshot()
        println(
            "MB25 playlist : tap→lecture=${tapMs} ms, ${onMain { browser.mediaItemCount }} en file, " +
                "titre=${played.mediaMetadata.title}, ${soundReport()}; " +
                "noyau du test : file=${myself.len} état=${myself.state}",
        )
    }

    /**
     * A tap on ONE of the tree's titles keeps the app's own path: the session
     * hands the item to `NzPlayer`, which sends a one-title `PlayQueue`. Media3's
     * default callback would REFUSE that item (no `LocalConfiguration`), so it is
     * measured rather than assumed.
     */
    @Test
    fun a_title_tapped_in_the_car_plays_through_the_same_player() {
        val browser = connect()
        val favorites = browsed(browser, AutoNode.FAVORITES, expect = 100)
        val title = favorites[3]
        val before = autoRequests()
        val tappedAt = SystemClock.uptimeMillis()
        onMain {
            browser.setMediaItem(title)
            browser.prepare()
            browser.play()
        }
        val wanted = title.mediaMetadata.title.toString()
        assertTrue("le titre ne joue pas", awaitPlaying(browser, wanted))
        assertEquals("une file d'un titre", 1, onMain { browser.mediaItemCount })
        val played = onMain { browser.currentMediaItem }
        assertNotNull("le lecteur n'a pas de titre courant", played)
        assertEquals("le titre joué est celui de l'arbre", wanted, played!!.mediaMetadata.title.toString())
        assertEquals(
            "jouer un titre déjà affiché ne demande rien à l'arbre",
            before,
            autoRequests(),
        )
        val tapMs = SystemClock.uptimeMillis() - tappedAt
        println("MB25 titre : tap→lecture=${tapMs} ms, ${played.mediaMetadata.title}, ${soundReport()}")
    }

    // --- the harness -----------------------------------------------------

    /** A MediaBrowser, which IS the MediaController a head unit connects with. */
    private fun connect(): MediaBrowser {
        browser?.let { return it }
        val token = SessionToken(app, ComponentName(app, NzMediaService::class.java))
        val built =
            MediaBrowser.Builder(app, token)
                // The browser's application looper is the thread every call must
                // come from; the instrumentation thread is not it.
                .setApplicationLooper(Looper.getMainLooper())
                // The library listener is set on the BUILDER: `MediaBrowser`
                // only takes `Player.Listener`s afterwards, and this one carries
                // `onChildrenChanged` — how the tree says a node has more.
                .setListener(
                    object : MediaBrowser.Listener {
                        override fun onChildrenChanged(
                            browser: MediaBrowser,
                            parentId: String,
                            itemCount: Int,
                            params: LibraryParams?,
                        ) {
                            if (itemCount > 0) announced[parentId]?.countDown()
                        }
                    },
                )
                .buildAsync()
        val connected = built.get(30, TimeUnit.SECONDS)
        browser = connected
        return connected
    }

    /**
     * One node's children, the way a head unit gets them: subscribe, ask, and if
     * the device had no copy, wait for the tree's OWN `notifyChildrenChanged` (an
     * event, never a sleep) before asking again.
     */
    private fun browsed(browser: MediaBrowser, parent: String, expect: Int): List<MediaItem> {
        val latch = CountDownLatch(1)
        announced[parent] = latch
        call("l'abonnement à $parent") { browser.subscribe(parent, LIBRARY_PARAMS) }
        var rows = children(browser, parent)
        while (rows.size < expect) {
            assertTrue(
                "le nœud $parent n'a jamais été annoncé avec des lignes " +
                    "(copie : ${rows.size} lignes)",
                latch.await(EVENT_SECONDS, TimeUnit.SECONDS),
            )
            rows = children(browser, parent)
            if (rows.size >= expect) break
            // Another read may still be in flight: wait for the next announcement.
            announced[parent] = CountDownLatch(1)
            assertTrue(
                "le nœud $parent n'a jamais rendu $expect lignes (${rows.size} reçues)",
                announced.getValue(parent).await(EVENT_SECONDS, TimeUnit.SECONDS),
            )
        }
        return rows
    }

    private fun children(browser: MediaBrowser, parent: String): List<MediaItem> =
        call("les enfants de $parent") {
            browser.getChildren(parent, 0, PAGE, LIBRARY_PARAMS)
        }.value.orEmpty()

    /**
     * One library call, made on the browser's OWN looper and awaited here.
     *
     * `MediaController` (and so `MediaBrowser`) refuses a call from any other
     * thread — measured: `IllegalStateException: MediaBrowser method is called
     * from a wrong thread` — while blocking the main thread on a future would
     * deadlock the answer it waits for.
     */
    private fun <T> call(what: String, block: () -> ListenableFuture<T>): T =
        await(what, onMain(block))

    /** One answer, waited on its future — an event, not a clock. */
    private fun <T> await(what: String, future: ListenableFuture<T>): T {
        try {
            return future.get(EVENT_SECONDS, TimeUnit.SECONDS)
        } catch (error: Exception) {
            throw AssertionError("$what : ${error.message}", error)
        }
    }

    /**
     * `true` once the SESSION's player plays [title] — awaited on the
     * controller's own events (`Player.Listener`), which is exactly what a head
     * unit sees. The title is part of the condition on purpose: a previous
     * test's music may still be playing, and "playing" alone would pass on it.
     */
    private fun awaitPlaying(browser: MediaBrowser, title: String): Boolean {
        fun reached(): Boolean =
            browser.isPlaying && browser.currentMediaItem?.mediaMetadata?.title?.toString() == title

        val latch = CountDownLatch(1)
        onMain {
            browser.addListener(
                object : Player.Listener {
                    override fun onEvents(player: Player, events: Player.Events) {
                        if (reached()) latch.countDown()
                    }
                },
            )
            if (reached()) latch.countDown()
        }
        return latch.await(EVENT_SECONDS, TimeUnit.SECONDS)
    }

    /**
     * The engine's own counters: the glitches the person would hear.
     *
     * NOT `lastFirstAudibleNs - lastTapNs`: MB3b's two fields are on different
     * clocks (the tap is Kotlin's `nanoTime`, the audible mark is
     * `Player::now_ms()` — milliseconds since the PLAYER started), so that
     * difference is a negative number (measured: −1 757 852,8 ms). The finding is
     * in docs/agents/mobile.md's MB25 section; the real tap → sound target
     * belongs to MQ2, which owns *Measures*.
     */
    private fun soundReport(): String {
        val stats = api.outputStats()
        return "sous-runs=%d, rebuffers=%d, tampon=%d ms"
            .format(stats.underruns.toLong(), stats.rebuffers.toLong(), stats.bufferMs.toLong())
    }

    /** The tree's own counted requests (`NzCore::stats`, tag `auto`). */
    private fun autoRequests(): ULong =
        core.stats().firstOrNull { it.tag == "auto" }?.requests ?: 0uL

    private fun <T> onMain(block: () -> T): T {
        var answer: T? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync { answer = block() }
        @Suppress("UNCHECKED_CAST")
        return answer as T
    }

    private fun serverUrl(): String =
        InstrumentationRegistry.getArguments().getString("nzServer") ?: DEFAULT_SERVER

    private companion object {
        /** `adb shell am instrument -e nzServer http://10.0.2.2:5595 …` overrides. */
        const val DEFAULT_SERVER = "http://10.0.2.2:5595"

        /** One page: the ceiling the tree answers (`AutoPage.MAX_PAGE`). */
        const val PAGE = 100

        /** A guard on a failure, never a wait: everything waits on an event. */
        const val EVENT_SECONDS = 60L

        val LIBRARY_PARAMS: LibraryParams? = null
    }
}
