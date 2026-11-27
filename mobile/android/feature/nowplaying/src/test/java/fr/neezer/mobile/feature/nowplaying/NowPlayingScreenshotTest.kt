package fr.neezer.mobile.feature.nowplaying

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.onAllNodesWithTag
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.AtfSuppressions
import fr.neezer.mobile.testkit.ScreenGoldenRule
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The player and its queue, rendered and VERIFIED: the long title and the
 * transport at 360, 412 and 600 dp and at the 1.0 and 1.3 font scales, plus the
 * queue's four rows with one playing (plan T3's matrix).
 *
 * T3 replaced MB11's and P1's write-only captures with goldens: the player's
 * blurred backdrop, its cover and its five-door thumb row are the surface the
 * owner looks at most, and a pixel that drifts there is a regression. Robolectric
 * draws it on the host with the real fonts (no emulator, no window — worker rule
 * 7); the motion tokens are advanced so the IMAGE is the settled state (an event,
 * not a sleep).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class NowPlayingGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    @Test
    fun playerAt360() = shotPlayer(360, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi", fontScale = 1.3f)
    fun playerAt360At130Percent() = shotPlayer(360, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h800dp-xhdpi")
    fun playerAt412() = shotPlayer(412, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h800dp-xhdpi", fontScale = 1.3f)
    fun playerAt412At130Percent() = shotPlayer(412, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
    fun playerAt600() = shotPlayer(600, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi", fontScale = 1.3f)
    fun playerAt600At130Percent() = shotPlayer(600, 1.3f)

    private fun shotPlayer(width: Int, scale: Float) {
        val model = model()
        val state = model.state.value
        rule.setContent(scale) {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    PlayerSurface(
                        state = state,
                        model = model,
                        nav = navigator,
                        lyrics = lyrics(),
                        about = null,
                        onClose = {},
                        liveReads = false,
                    )
                }
            }
        }
        settle()
        rule.capture("nowplaying-lecteur-${width}dp-f$scale.png")
    }

    private fun settle() {
        rule.composeRule.mainClock.advanceTimeBy(1_200)
        shadowOf(android.os.Looper.getMainLooper()).idle()
        rule.composeRule.waitForIdle()
        // JF : le fond est calculé hors du fil principal (flou en Kotlin). La
        // capture attend l'ÉVÉNEMENT « le champ est peint », jamais une horloge ;
        // la boucle est bornée et `waitUntil` seul ne voit pas un travail fini
        // sur `Dispatchers.Default`.
        val deadline = System.currentTimeMillis() + 10_000
        while (System.currentTimeMillis() < deadline) {
            rule.composeRule.waitForIdle()
            if (
                rule.composeRule.onAllNodesWithTag(PLAYER_BACKDROP_TAG)
                    .fetchSemanticsNodes().isNotEmpty()
            ) {
                return
            }
            Thread.sleep(10)
        }
        error("le champ du lecteur n'a jamais été peint en 10 s")
    }

    private fun model(): NowPlayingModel {
        val port = FakePlayerPort(
            snapshot = FakePlayerPort.snapshot(
                current = FakePlayerPort.track(
                    title = "Le chant des sirènes qui se brisent sur les rochers de la baie",
                ),
                index = 1u,
                len = 4_000u,
            ).copy(sleep = uniffi.nz_ffi.SleepState(30u, false)),
            queueRows = FakePlayerPort.queueOf(4_000),
        )
        val service = FakeService(cachedLyrics = FakeService.lyrics())
        val model = NowPlayingModel(
            player = port,
            service = service,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        return model
    }

    private fun lyrics(): LyricsModel {
        val model = LyricsModel(
            service = FakeService(cachedLyrics = FakeService.lyrics()),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.open("9001", "Fauve", "Le chant des sirènes")
        model.onPosition(25.0)
        return model
    }
}

/**
 * The queue sheet alone: its four rows with one playing, at 360 dp, 1.0.
 *
 * The sheet's veil produces the design system's one known ATF finding
 * (`AtfSuppressions.sheetVeil`), so only this class suppresses it — the player
 * goldens above run with none.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class NowPlayingQueueGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule(accessibilitySuppressions = AtfSuppressions.sheetVeil)

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    private fun model(): NowPlayingModel {
        val port = FakePlayerPort(
            snapshot = FakePlayerPort.snapshot(index = 1u, len = 4_000u),
            queueRows = FakePlayerPort.queueOf(4_000),
        )
        val model = NowPlayingModel(
            player = port,
            service = FakeService(cachedLyrics = FakeService.lyrics()),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        return model
    }

    @Test
    fun queueAt360() {
        val model = model()
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    QueueSheet(model = model, nav = navigator, lyrics = null, onClose = {})
                }
            }
        }
        rule.composeRule.mainClock.advanceTimeBy(1_200)
        shadowOf(android.os.Looper.getMainLooper()).idle()
        rule.composeRule.waitForIdle()
        rule.capture("nowplaying-file-360dp-f1.0.png")
    }
}
