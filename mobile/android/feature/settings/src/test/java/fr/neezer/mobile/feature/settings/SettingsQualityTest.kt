package fr.neezer.mobile.feature.settings

import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.Quality
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * La carte « Qualité demandée au serveur », telle qu'une personne la voit.
 *
 * The bug this holds: the card offered SIX rates, none of them `Auto` (the
 * player's own default), and two of them — `Opus 256`, `Opus 192` — that
 * `nz_audio::Quality` has no rung for, so choosing them played something else.
 * The tap also wrote `player.quality`, which the player never reads. The card
 * now offers the five rungs the engine loads, and a tap moves the PLAYER
 * (`player.audio`) through [PlaybackAudio].
 *
 * Robolectric with the NATIVE backend, on the real screen (`SettingsEntry` and
 * its route), over [FakeSettingsService] and [FakePlaybackAudio]: no `.so`, no
 * emulator, no window on the person's display.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class SettingsQualityTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val nav = object : fr.neezer.mobile.bridge.NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    /** Drains what a composition queued — an EVENT, never a duration (testing.md). */
    private fun settle() {
        rule.waitForIdle()
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
    }

    private fun render(playback: FakePlaybackAudio): FakeSettingsService {
        // A fresh device: the player's own default, `Auto` — not the SPA's `FLAC`.
        val service = FakeSettingsService(settings = SettingsFixtures.settings(quality = "AUTO"))
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    SettingsEntry(section = "playback", nav = nav, service = service, playback = playback)
                }
            }
        }
        settle()
        return service
    }

    /** The card offers exactly the five rungs the engine can load, `Auto` included. */
    @Test
    fun theCardOffersTheFiveRungsTheEngineLoads() {
        render(FakePlaybackAudio())
        rule.onNodeWithText("Qualité demandée au serveur").performScrollTo().assertIsDisplayed()

        for (label in listOf("Auto", "FLAC", "Opus 320", "Opus 128", "Opus 64")) {
            rule.onNodeWithText(label).assertIsDisplayed()
        }
        // The two rungs `nz_audio::Quality` cannot load: offering them is the bug.
        rule.onNodeWithText("Opus 256").assertDoesNotExist()
        rule.onNodeWithText("Opus 192").assertDoesNotExist()
    }

    /** A tap moves the PLAYER's value, once, and writes no `player.quality`. */
    @Test
    fun aTapMovesThePlayersQualityOnce() {
        val playback = FakePlaybackAudio()
        val service = render(playback)

        rule.onNodeWithText("Opus 128").performScrollTo().performClick()
        settle()

        assertEquals("one write, to the engine's own value", 1, playback.writes.size)
        assertEquals(Quality.OPUS128, playback.settings.quality)
        assertEquals(
            "the SPA's key is no longer a second truth: nothing else was written",
            emptyList<Pair<String, String>>(),
            service.writes,
        )
    }
}
