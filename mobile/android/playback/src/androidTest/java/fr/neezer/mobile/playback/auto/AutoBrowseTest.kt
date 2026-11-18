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
import androidx.media3.common.MediaItem
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
import java.util.concurrent.atomic.AtomicReference
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
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerApi
import uniffi.nz_ffi.PlayerListener
import uniffi.nz_ffi.PlayerSnapshot

@RunWith(AndroidJUnit4::class)
class AutoBrowseTest {
    private lateinit var app: Application
    private lateinit var core: NzCore
    private lateinit var api: PlayerApi
    private var browser: MediaBrowser? = null

    /** One latch per node, fed by the tree's own `notifyChildrenChanged`. */
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
        runCatching { onMain { browser?.release() } }
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
        val root = await("la racine", browser.getLibraryRoot(LIBRARY_PARAMS)).value
        assertNotNull("la racine n'a pas de valeur", root)
        assertEquals(AutoNode.ROOT_TITLE, root!!.mediaMetadata.title.toString())
        val sections = children(browser, AutoNode.ROOT)
        assertEquals(
            listOf("Favoris", "Playlists", "Mon Flow", "Récents", "Téléchargements"),
            sections.map { it.mediaMetadata.title.toString() },
        )
        assertEquals("ouvrir l'arbre ne coûte aucune requête", beforeRoot, autoRequests())

        // --- Favoris: 1 request cold, 100 rows, then 0 ---------------------
        val favorites = browsed(browser, AutoNode.FAVORITES, expect = 100)
        assertEquals("4 000 favoris se ramènent à 100 lignes", 100, favorites.size)
        assertTrue(
            "une lecture froide = une requête (comptée : ${autoRequests() - beforeRoot})",
            autoRequests() - beforeRoot == 1uL,
        )
        val warmBefore = autoRequests()
        assertEquals(100, children(browser, AutoNode.FAVORITES).size)
        assertEquals("un nœud chaud ne coûte rien", warmBefore, autoRequests())

        // --- Playlists, then the big one opens ---------------------------
        val playlists = browsed(browser, AutoNode.PLAYLISTS, expect = 60)
        assertEquals("le banc sème 60 playlists", 60, playlists.size)
        val big = playlists.first { it.mediaMetadata.title.toString().startsWith("Playlist 0 ") }
        val tracks = browsed(browser, big.mediaId, expect = 100)
        assertEquals("une playlist de 4 000 titres montre 100 lignes", 100, tracks.size)
        assertTrue(
            "une ligne de playlist est un titre jouable",
            tracks.first().mediaMetadata.isPlayable == true &&
                tracks.first().mediaMetadata.isBrowsable != true,
        )

        // --- the tap: the browsed node plays, for free -------------------
        val requestsBeforePlay = autoRequests()
        onMain {
            browser.setMediaItem(big)
            browser.prepare()
            browser.play()
        }
        val playing =
            awaitPlayer("la playlist joue") {
                it.state == PlayState.PLAYING && it.len >= 4_000u && it.current != null
            }
        assertEquals(
            "un nœud déjà parcouru se joue sans requête d'arbre",
            requestsBeforePlay,
            autoRequests(),
        )
        assertEquals(
            "le premier titre joué est celui que la liste affichait",
            tracks.first().mediaMetadata.title.toString(),
            playing.current?.title,
        )
        val stats = api.outputStats()
        assertTrue("le tap n'a pas été vu par le moteur", stats.lastTapNs > 0uL)
        assertTrue("aucune trame audible", stats.lastFirstAudibleNs > 0uL)
        val tapToSoundMs =
            (stats.lastFirstAudibleNs.toDouble() - stats.lastTapNs.toDouble()) / 1_000_000.0
        println(
            "MB25 playlist tap→son = %.1f ms (file=%d, titre=%s)"
                .format(tapToSoundMs, playing.len, playing.current?.title),
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
        onMain {
            browser.setMediaItem(title)
            browser.prepare()
            browser.play()
        }
        val playing = awaitPlayer("le titre joue") { it.len == 1u && it.state == PlayState.PLAYING }
        assertEquals("une file d'un titre", 1u, playing.len)
        assertEquals(
            "le titre joué est celui de l'arbre",
            title.mediaMetadata.title.toString(),
            playing.current?.title,
        )
        assertEquals(
            "jouer un titre déjà affiché ne demande rien à l'arbre",
            before,
            autoRequests(),
        )
        val stats = api.outputStats()
        println(
            "MB25 titre tap→son = %.1f ms (titre=%s)"
                .format(
                    (stats.lastFirstAudibleNs.toDouble() - stats.lastTapNs.toDouble()) / 1_000_000.0,
                    playing.current?.title,
                ),
        )
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
                            announced[parentId]?.countDown()
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
        val latch = announced.computeIfAbsent(parent) { CountDownLatch(1) }
        await("l'abonnement à $parent", browser.subscribe(parent, LIBRARY_PARAMS))
        val cold = children(browser, parent)
        if (cold.size >= expect) return cold
        assertTrue(
            "le nœud $parent n'a jamais été annoncé (copie froide : ${cold.size} lignes)",
            latch.await(EVENT_SECONDS, TimeUnit.SECONDS),
        )
        val warm = children(browser, parent)
        assertTrue(
            "après l'annonce, $parent ne rend que ${warm.size} lignes (attendu $expect)",
            warm.size >= expect,
        )
        return warm
    }

    private fun children(browser: MediaBrowser, parent: String): List<MediaItem> =
        await("les enfants de $parent", browser.getChildren(parent, 0, PAGE, LIBRARY_PARAMS))
            .value
            .orEmpty()

    /** One answer, waited on its future — an event, not a clock. */
    private fun <T> await(what: String, future: ListenableFuture<T>): T {
        try {
            return future.get(EVENT_SECONDS, TimeUnit.SECONDS)
        } catch (error: Exception) {
            throw AssertionError("$what : ${error.message}", error)
        }
    }

    /** The player's own events, so a state is awaited and never polled. */
    private fun awaitPlayer(what: String, want: (PlayerSnapshot) -> Boolean): PlayerSnapshot {
        val reached = AtomicReference<PlayerSnapshot?>(null)
        val latch = CountDownLatch(1)
        val snapshot = api.snapshot()
        if (want(snapshot) && reached.compareAndSet(null, snapshot)) latch.countDown()
        api.subscribe(
            object : PlayerListener {
                override fun onSnapshot(snapshot: PlayerSnapshot) {
                    if (want(snapshot) && reached.compareAndSet(null, snapshot)) latch.countDown()
                }

                override fun onQueueChanged(version: ULong) {}

                override fun onFault(cause: String) {
                    println("MB25 défaut du lecteur : $cause")
                }
            },
        )
        assertTrue(
            "$what : l'état n'est jamais venu (état=${api.snapshot().state}, " +
                "file=${api.snapshot().len}, défaut=${api.snapshot().fault})",
            latch.await(EVENT_SECONDS, TimeUnit.SECONDS),
        )
        return reached.get()!!
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
