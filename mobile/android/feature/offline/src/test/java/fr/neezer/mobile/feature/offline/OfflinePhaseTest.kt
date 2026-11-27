package fr.neezer.mobile.feature.offline

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Réglages → Téléchargements change de phase en fondu (lot X6) : la section
 * (les cartes, la page des titres, l'onglet des pairs) et la page des titres
 * (échec, vide, liste) se remplacent **dans la même image** en éco.
 *
 * The port fails where the plan's case needs it: an EMPTY state (version 0) then
 * a `state()` that throws, which is exactly the "nothing to draw and no cause"
 * shape the LoadFailed branch exists for. The curve is `inPlaceSpecs`'s; the
 * wiring is what a Compose test can hold.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class OfflinePhaseTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @After
    fun forget() {
        OfflineHost.reset()
    }

    private class FlakyPort(private val inner: FakeOfflinePort = FakeOfflinePort()) : OfflinePort by inner {
        var failing = false
        override suspend fun state(): uniffi.nz_ffi.OfflineState {
            if (failing) throw IllegalStateException("le magasin local est illisible")
            return inner.state()
        }
    }

    @Test
    fun theSectionPhaseReplacesInTheSameFrameAtEco() {
        val port = FlakyPort()
        val model = DownloadsModel(
            port = port,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            freeSpace = { 0L },
            toast = { _, _, _ -> },
            causeText = { it.message ?: "erreur" },
        )
        OfflineHost.install(model)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    OfflineSettingsSection()
                }
            }
        }
        rule.waitForIdle()
        // Nothing read yet: the cards are the phase, and the cause is not.
        rule.onNodeWithText("Qualité de téléchargement par défaut").assertExists()
        rule.onNodeWithText("Téléchargements indisponibles").assertDoesNotExist()

        port.failing = true
        model.refresh()
        rule.waitForIdle()
        // The same frame at eco: the cause is there, the cards are out.
        rule.onNodeWithText("Téléchargements indisponibles").assertExists()
        rule.onNodeWithText("Qualité de téléchargement par défaut").assertDoesNotExist()

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }

    @Test
    fun theTwoPhaseFunctionsReadTheFourStates() {
        assertEquals(OFFLINE_FAILED, offlinePhase("illisible", 0uL, downloadsOpen = false, tab = 0, hasPeers = false))
        assertEquals(OFFLINE_DOWNLOADS, offlinePhase(null, 3uL, downloadsOpen = true, tab = 0, hasPeers = false))
        assertEquals(OFFLINE_PEERS, offlinePhase(null, 3uL, downloadsOpen = false, tab = 1, hasPeers = true))
        assertEquals(OFFLINE_SETTINGS, offlinePhase(null, 3uL, downloadsOpen = false, tab = 0, hasPeers = true))

        assertEquals(OFFLINE_DOWNLOADS_FAILED, downloadsPagePhase("illisible", null))
        assertEquals(OFFLINE_DOWNLOADS_EMPTY, downloadsPagePhase(null, null))
    }
}
