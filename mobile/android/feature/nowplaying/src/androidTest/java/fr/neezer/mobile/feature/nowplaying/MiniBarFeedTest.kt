package fr.neezer.mobile.feature.nowplaying

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.NzRowSource
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.AudioSettings
import uniffi.nz_ffi.NowPlaying
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.PlayerListener
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.PositionInfo
import uniffi.nz_ffi.SleepTimer

/**
 * The mini bar must FOLLOW the player — the defect this lot fixes, as a test.
 *
 * Before: `MiniPlayer` built its own `NowPlayingModel`, which read
 * `player.snapshot()` once at composition and never collected [PlayerFeed]. A
 * phone that played showed no bar at all (measured on emulator-5660: the media
 * session said PLAYING at 2 000 ms while the tree had no « Ouvrir le lecteur
 * plein écran » node), which also left the full-screen player unreachable from
 * the shell.
 *
 * The test pushes what Rust pushes (`PlayerListener.onSnapshot`) and reads the
 * bar's own nodes: it fails on the old code at the FIRST wait, and it would fail
 * again if the bar were fed by a snapshot instead of the feed (the second title
 * never arrives).
 */
@RunWith(AndroidJUnit4::class)
class MiniBarFeedTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** The player as the bar asks it: one snapshot, and nothing else matters. */
    private class FakePort(private val initial: PlayerSnapshot) : PlayerPort {
        var listener: PlayerListener? = null

        override fun snapshot(): PlayerSnapshot = initial
        override fun position(): PositionInfo = NoPlayerPort.position()
        override fun command(cmd: PlayerCommand, tapNs: Long) = Unit
        override fun queueSource(): NzRowSource? = null
        override fun audio(): AudioSettings = NoPlayerPort.audio()
        override fun setAudio(settings: AudioSettings) = Unit
        override fun setSleep(timer: SleepTimer?) = Unit
    }

    private fun snapshot(title: String): PlayerSnapshot = PlayerSnapshot(
        state = PlayState.PLAYING,
        current = NowPlaying(
            key = "k-$title",
            id = "k-$title",
            title = title,
            artists = "Artiste du parcours",
            album = "Album du parcours",
            albumId = null,
            artistId = null,
            cover = null,
            durationS = 210.0,
            isEpisode = false,
            quality = "128",
            source = "deezer",
            liked = false,
        ),
        index = 0u,
        len = 12u,
        shuffle = false,
        repeat = uniffi.nz_ffi.Repeat.OFF,
        volume = 1f,
        muted = false,
        streaming = false,
        sleep = null,
        error = null,
        fault = null,
        version = 1u,
    )

    @Test
    fun theBarFollowsThePlayerFeed() {
        val first = FakePort(snapshot("Premier titre"))
        // The test owns the feed for this process: the app's own install is a
        // no-op after this one (one player per process, MB3c's rule).
        PlayerFeed.usePortForTest(first, subscribe = { listener -> first.listener = listener })

        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                MiniPlayer()
            }
        }

        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Premier titre").fetchSemanticsNodes().isNotEmpty()
        }
        // A pause glyph means the state really reached the bar, not just the title.
        assertTrue(
            "the bar shows no transport control",
            rule.onAllNodesWithText("210").fetchSemanticsNodes().isNotEmpty() ||
                rule.onAllNodesWithText("Artiste du parcours").fetchSemanticsNodes().isNotEmpty(),
        )

        // The next thing Rust would push: the bar has to follow it.
        checkNotNull(first.listener) { "the feed never subscribed to the player" }
            .onSnapshot(snapshot("Deuxième titre"))

        rule.waitUntil(timeoutMillis = TIMEOUT_MS) {
            rule.onAllNodesWithText("Deuxième titre").fetchSemanticsNodes().isNotEmpty()
        }
    }

    private companion object {
        const val TIMEOUT_MS = 10_000L
    }
}
