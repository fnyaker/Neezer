package fr.neezer.mobile.feature.nowplaying

import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
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
 * The covers of the player, counted (P1, rule 4: a click is measured).
 *
 * The player used to ask for **three decodes on every track change**: the strip
 * was a fixed 3-page pager keyed by position and a settle rewrote it to page 1,
 * so the swiped-to page was rebuilt as the centre, and `NzCoverRequest` was a
 * plain class, so even an unchanged neighbour restarted Coil's request on the
 * next recomposition.
 *
 * This test composes the real [PlayerSurface] with a fake [NzCoverLoader] that
 * records every `Image(url, sizePx)` the sleeve asks for. It proves:
 *
 *  * opening asks for exactly the three sleeves on screen (previous, current,
 *    next) at the stage's size, plus the backdrop's one small copy of the
 *    current cover (a cache hit, zero network — see `PlayerBackdrop`'s KDoc);
 *  * each `next()` asks for exactly ONE new sleeve — the new neighbour;
 *  * a state re-emit with no track change asks for nothing new.
 *
 * Run against the tree this lot fixes it fails on the second assertion (two new
 * sleeves per change: the rebuilt centre and the neighbour).
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

    /** What the composition asked for, in order. */
    private class Recorder : NzCoverLoader {
        val requests = mutableListOf<Pair<String, Int>>()
        val keys: Set<Pair<String, Int>> get() = requests.toSet()

        @Composable
        override fun Image(url: String, sizePx: Int) {
            requests += url to sizePx
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

    private fun model(port: FakePlayerPort): NowPlayingModel {
        val model = NowPlayingModel(
            player = port,
            service = FakeService(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        return model
    }

    /** The stage draws bigger than the backdrop's 120 dp copy. */
    private fun Recorder.stageSize(): Int =
        keys.map { it.second }.distinct().maxOrNull() ?: 0

    private fun Recorder.stageCovers(): Set<String> =
        keys.filter { it.second == stageSize() }.map { it.first }.toSet()

    @Test
    fun aTrackChangeAsksForExactlyTheNewNeighbour() {
        val port = port()
        val model = model(port)
        val recorder = Recorder()

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
        for (target in listOf(2, 3)) {
            model.next()
            port.snapshot = port.snapshot.copy(
                index = target.toUInt(),
                current = nowPlaying(target),
            )
            // The Rust push is what the feed delivers after the command.
            model.onSnapshot(port.snapshot)
            settle()

            val now = recorder.stageCovers()
            val added = now - seen
            println(
                "MEASURE p1 covers after next() to $target: ${now.size} distinct " +
                    "(+${added.size}: ${added.joinToString()})",
            )
            assertEquals("one new sleeve per track change (target $target)", 1, added.size)
            assertEquals(
                "the new sleeve is the new next",
                setOf("/api/cover/${target + 1}"),
                added,
            )
            seen = now
        }

        // A snapshot re-emit with the same track (a position tick, a pause) is
        // not a track change: the strip must not ask for anything new.
        val before = recorder.keys
        port.snapshot = port.snapshot.copy(version = port.snapshot.version + 1uL)
        model.onSnapshot(port.snapshot)
        settle()
        val after = recorder.keys
        println("MEASURE p1 covers on a re-emit: ${(after - before).size} new")
        assertTrue("a state re-emit asks for no new sleeve: ${after - before}", after == before)
    }

    private fun settle() {
        rule.mainClock.advanceTimeBy(1_200)
        rule.waitForIdle()
    }
}
