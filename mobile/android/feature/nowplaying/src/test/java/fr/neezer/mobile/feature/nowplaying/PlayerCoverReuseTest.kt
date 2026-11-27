package fr.neezer.mobile.feature.nowplaying

import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.LocalNzCoverLoader
import fr.neezer.mobile.designsystem.NzCoverLoader
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.NowPlaying
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.TrackRow

/**
 * The covers of the player, counted at BOTH seams (P1 + T4, rule 4: a click is
 * measured).
 *
 * The player used to ask for **three decodes on every track change**: the strip
 * was a fixed 3-page pager keyed by position and a settle rewrote it to page 1,
 * so the swiped-to page was rebuilt as the centre, and `NzCoverRequest` was a
 * plain class, so even an unchanged neighbour restarted Coil's request on the
 * next recomposition.
 *
 * This test composes the real [PlayerSurface] with a recording [NzCoverLoader]
 * and counts two things:
 *
 *  * the **draw**, every `Image(url, sizePx)` the sleeve asks for;
 *  * the **core fetch**, the `coverPath(url, sizePx)` the loader asks the model's
 *    service for — the file in `nz-cache` that Coil then decodes. It is what
 *    catches a page that is *retained but re-composed*: the same sleeve drawn
 *    again is a decode, but a second `coverPath` for the same URL and size is a
 *    second network/disk ask, and that is the bug.
 *
 * It proves:
 *
 *  * opening asks for exactly the three sleeves on screen (previous, current,
 *    next) at the stage's size, plus the backdrop's one small copy of the
 *    current cover (a cache hit, zero network — see `PlayerBackdrop`'s KDoc);
 *  * each `next()` asks for exactly ONE new sleeve — the new neighbour — and the
 *    two sleeves that stay on the strip are **not fetched again** (0 core asks);
 *  * a state re-emit with no track change asks for nothing new;
 *  * across the whole run every sleeve the strip showed is fetched from the core
 *    exactly once (`cover asks: 0=1 1=1 2=1 3=1 4=1`), so a remount or a
 *    refetch fails.
 *
 * Run against the tree this lot gates it fails: `the shared sleeve /api/cover/1
 * is composited again, not refetched … expected:<0> but was:<1>` (measured on
 * 25b3551b, before P1).
 *
 * No assertion depends on Coil's cache (the JVM has none): both counters read
 * the app's own seams, which are the contract.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class PlayerCoverReuseTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    /**
     * What the composition asked for, and what the core behind it was asked for.
     *
     * Coil fetches a sleeve in a coroutine (`NzCoverFetcher`), so the loader asks
     * the service from a [LaunchedEffect] keyed exactly as Coil keys its request
     * (`url` + `sizePx`): a page that is only recomposed keeps its composition
     * and asks nothing, a page that is rebuilt is a NEW composition, its effect
     * runs again, and the core is asked a second time for a file it already
     * served — which is the bug this lot gates.
     */
    private class Recorder(private val service: NowPlayingService) : NzCoverLoader {
        val requests = mutableListOf<Pair<String, Int>>()
        val keys: Set<Pair<String, Int>> get() = requests.toSet()

        /** Le fond du lecteur demande des PIXELS (JF) : un décodage par pochette. */
        val decodes = mutableListOf<Pair<String, Int>>()

        @Composable
        override fun Image(url: String, sizePx: Int) {
            requests += url to sizePx
            LaunchedEffect(url, sizePx) { service.coverPath(url, sizePx) }
        }

        override suspend fun bitmap(url: String, sizePx: Int): ImageBitmap? {
            decodes += url to sizePx
            return null
        }
    }

    private fun covers(size: Int): List<TrackRow> =
        FakePlayerPort.queueOf(size).mapIndexed { index, row ->
            // The queue row and the player's `current` carry the SAME key in the
            // app (`entry.key()` in `nz-core`): the page key must not change when
            // the playhead lands on the page it already had.
            row.copy(key = "90$index", id = "90$index", cover = "/api/cover/$index")
        }

    private fun nowPlaying(index: Int): NowPlaying =
        FakePlayerPort.track(key = "90$index").copy(cover = "/api/cover/$index")

    private fun port(size: Int = 6, index: Int = 1): FakePlayerPort = FakePlayerPort(
        snapshot = FakePlayerPort.snapshot(
            state = PlayState.PLAYING,
            current = nowPlaying(index),
            index = index.toUInt(),
            len = size.toUInt(),
        ),
        queueRows = covers(size),
    )

    private fun model(port: FakePlayerPort, service: NowPlayingService): NowPlayingModel {
        val model = NowPlayingModel(
            player = port,
            service = service,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        return model
    }

    /**
     * The size the strip draws at: the one most sleeves share. The player also
     * asks the backdrop for a 96 px copy of the current cover (JF's blurred
     * field, on every API now), so "the biggest size" would name the backdrop on
     * a tree where the strip is smaller — the backdrop is one ask per track, the
     * strip three at open.
     */
    private fun Recorder.stageSize(): Int =
        keys.groupingBy { it.second }.eachCount().entries
            .maxWith(compareBy({ it.value }, { it.key })).key

    private fun Recorder.stageCovers(): Set<String> =
        keys.filter { it.second == stageSize() }.map { it.first }.toSet()

    @Test
    fun aTrackChangeAsksForExactlyTheNewNeighbour() {
        val port = port()
        val service = FakeService()
        val model = model(port, service)
        val recorder = Recorder(service)

        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                CompositionLocalProvider(LocalNzCoverLoader provides recorder) {
                    val state by model.state.collectAsState()
                    PlayerSurface(
                        state = state,
                        model = model,
                        nav = navigator,
                        lyrics = null,
                        about = null,
                        onClose = {},
                        liveReads = false,
                    )
                }
            }
        }
        settle()

        val stagePx = recorder.stageSize()
        val opened = recorder.stageCovers()
        println(
            "MEASURE p1 covers open: ${opened.size} distinct stage sleeves " +
                "(sizes ${recorder.keys.map { it.second }.distinct()}), ${recorder.requests.size} draw calls",
        )
        assertEquals(
            "the open asks for the previous, current and next sleeve",
            setOf("/api/cover/0", "/api/cover/1", "/api/cover/2"),
            opened,
        )

        var seen = opened
        var stageSleeves = opened
        for (target in listOf(2, 3)) {
            val askedFrom = service.coversAsked.size
            model.next()
            port.snapshot = port.snapshot.copy(
                index = target.toUInt(),
                current = nowPlaying(target),
            )
            // The Rust push is what the feed delivers after the command.
            model.onSnapshot(port.snapshot)
            settle()

            // The network/disk asks of THIS change, at the strip's size only:
            // the backdrop's small copy is a different Coil key and a cache hit.
            val fetched = service.coversAsked
                .subList(askedFrom, service.coversAsked.size)
                .filter { it.second == stagePx }

            val now = recorder.stageCovers()
            val added = now - seen
            val shared = seen intersect now
            println(
                "MEASURE p1 covers after next() to $target: ${now.size} distinct " +
                    "(+${added.size}: ${added.joinToString()}), core asks " +
                    fetched.joinToString { "${it.first}=${it.second}px" },
            )
            assertEquals("one new sleeve per track change (target $target)", 1, added.size)
            assertEquals(
                "the new sleeve is the new next",
                setOf("/api/cover/${target + 1}"),
                added,
            )
            for (url in shared) {
                assertEquals(
                    "the shared sleeve $url is composited again, not refetched (target $target)",
                    0,
                    fetched.count { it.first == url },
                )
            }
            for (url in added) {
                assertEquals(
                    "the new neighbour $url is fetched once (target $target)",
                    1,
                    fetched.count { it.first == url },
                )
            }
            seen = now
            stageSleeves = stageSleeves + now
        }

        // A snapshot re-emit with the same track (a position tick, a pause) is
        // not a track change: the strip must not ask for anything new.
        val before = recorder.keys
        val askedFrom = service.coversAsked.size
        port.snapshot = port.snapshot.copy(version = port.snapshot.version + 1uL)
        model.onSnapshot(port.snapshot)
        settle()
        val after = recorder.keys
        println("MEASURE p1 covers on a re-emit: ${(after - before).size} new")
        assertTrue("a state re-emit asks for no new sleeve: ${after - before}", after == before)
        assertEquals(
            "a state re-emit fetches nothing from the core",
            askedFrom,
            service.coversAsked.size,
        )

        // The whole run, per URL: one ask each. A remount, a refetch, or a page
        // rebuilt as its neighbour shows up here as a 2.
        val coreAsks = service.coversAsked
            .filter { it.second == stagePx }
            .groupingBy { it.first }
            .eachCount()
            .toSortedMap()
        println(
            "MEASURE t4 cover asks: " +
                coreAsks.entries.joinToString(" ") { "${it.key}=${it.value}" } +
                " (stage ${stagePx}px, ${service.coversAsked.size - coreAsks.values.sum()} backdrop asks)",
        )
        // Le fond (JF) : un décodage 96 px par pochette DISTINCTE, et pas un de
        // plus quand un snapshot se ré-émet sans changement de titre.
        val backdropAsks = recorder.decodes.filter { it.second != stagePx }
        println(
            "MEASURE jf backdrop decodes: " +
                backdropAsks.groupingBy { it }.eachCount().entries
                    .joinToString(" ") { "${it.key.first}=${it.key.second}px×${it.value}" },
        )
        assertTrue(
            "le fond ne décode qu'à 96 px : ${backdropAsks.map { it.second }.distinct()}",
            backdropAsks.all { it.second == 96 },
        )
        assertEquals(
            "un seul décodage par pochette distincte pour le fond : $backdropAsks",
            backdropAsks.toSet().size,
            backdropAsks.size,
        )
        assertEquals(
            "every sleeve the strip showed is fetched from the core exactly once",
            stageSleeves.associateWith { 1 },
            coreAsks,
        )
    }

    private fun settle() {
        rule.mainClock.advanceTimeBy(1_200)
        rule.waitForIdle()
    }
}
