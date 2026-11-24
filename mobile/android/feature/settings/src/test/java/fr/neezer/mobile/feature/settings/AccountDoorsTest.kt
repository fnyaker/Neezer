package fr.neezer.mobile.feature.settings

import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.AppLink

/**
 * M2's two doors, on the real screen and a recording navigator
 * (`docs/agents/mobile.md`, section M2).
 *
 * Three facts, three tests: « Compte » offers both doors, a confirmed
 * « Déconnexion » really asks the shell for the login screen, and « Changer de
 * serveur » really asks it for the server screen. What a JVM test CANNOT hold is
 * the 401 itself: nothing produces a `Session.Ended` without the `.so`, so the
 * toast and the login screen after an expired cookie belong to the emulator —
 * the worker image has neither a JDK nor an emulator, so M2's PR says so instead
 * of pretending a capture exists.
 *
 * `NzBridge.core` is null here: `logout()` is a null-safe no-op, so the click
 * walks the `onSuccess` branch of the screen's `runCatching` — which is exactly
 * the line M2 added.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class AccountDoorsTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** The shell, recorded: a door that opens is a counter that moves. */
    private class RecordingNavigator : NzNavigator {
        var logins = 0
        var setups = 0

        override fun go(link: AppLink) {}
        override fun back() {}
        override fun openPlayer() {}
        override fun openQueue() {}
        override fun openLogin() {
            logins++
        }

        override fun openSetup() {
            setups++
        }
    }

    /** Drains what a click queued — an EVENT, never a duration (testing.md). */
    private fun settle() {
        rule.waitForIdle()
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
    }

    private fun render(nav: RecordingNavigator) {
        val service = FakeSettingsService()
        val model = SettingsModel(service, CoroutineScope(Dispatchers.Main))
        model.start()
        model.open("account")
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize()) {
                    AccountSection(model, model.state, nav)
                }
            }
        }
        settle()
    }

    /** « Compte » porte les deux portes, et « Changer de serveur » est CLIQUABLE. */
    @Test
    fun accountOffersBothDoors() {
        render(RecordingNavigator())
        rule.onNode(hasText("Déconnexion") and hasClickAction()).assertIsDisplayed()
        rule.onNode(hasText("Changer de serveur") and hasClickAction()).assertIsDisplayed()
    }

    /** Se déconnecter (après confirmation) atterrit sur la connexion, une fois. */
    @Test
    fun signingOutLandsOnTheLoginScreen() {
        val nav = RecordingNavigator()
        render(nav)
        rule.onNode(hasText("Déconnexion") and hasClickAction()).performClick()
        settle()
        rule.onNode(hasText("Confirmer") and hasClickAction()).performClick()
        settle()
        assertEquals(1, nav.logins)
    }

    /** « Changer de serveur » ouvre l'écran serveur — et ne déconnecte pas. */
    @Test
    fun changingTheServerOpensSetup() {
        val nav = RecordingNavigator()
        render(nav)
        rule.onNode(hasText("Changer de serveur") and hasClickAction()).performClick()
        settle()
        assertEquals(1, nav.setups)
        assertEquals(0, nav.logins)
    }
}
