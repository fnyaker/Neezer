package fr.neezer.mobile.feature.party

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.PartyPhase

/**
 * L'écran d'invité change d'état en fondu (lot X6) : la carte de chargement
 * laisse l'accueil **dans la même image** en éco, et les cinq autres visages
 * (échec, expirée, terminée, quittée, en direct) sont des phases du même
 * `NzSwap`. Le `NzSheet` de la soirée garde sa propre entrée — ce lot ne le
 * touche pas.
 *
 * `liveReads = false`: a still test observes nothing (the playhead's frame loop
 * would never let the composition go idle).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class PartyGuestPhaseTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val nav = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    @Test
    fun theGuestPhaseReplacesInTheSameFrameAtEco() {
        val gate = CompletableDeferred<Unit>()
        val port = FakePartyService(card = partyState(phase = PartyPhase.LANDING))
        port.onPeek = { gate.await() }
        val model = PartyGuestModel(port, CoroutineScope(Dispatchers.Unconfined), id = "p1")
        model.load()

        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    PartyGuestScreen(model = model, nav = nav, liveReads = false)
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("Chargement…").assertExists()
        rule.onNodeWithText(PartyCopy.JOIN).assertDoesNotExist()

        gate.complete(Unit)
        rule.waitForIdle()
        rule.onNodeWithText(PartyCopy.JOIN).assertExists()
        rule.onNodeWithText("Chargement…").assertDoesNotExist()

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }

    @Test
    fun theGuestPhasesAreTheModelsOwnStates() {
        assertEquals(PARTY_LOADING, partyGuestPhase(PartyGuestModel.State(loading = true)))
        assertEquals(PARTY_FAILED, partyGuestPhase(PartyGuestModel.State(phase = PartyPhase.FAILED, loading = false)))
        assertEquals(PARTY_MISSING, partyGuestPhase(PartyGuestModel.State(phase = PartyPhase.MISSING, loading = false)))
        assertEquals(PARTY_ENDED, partyGuestPhase(PartyGuestModel.State(phase = PartyPhase.ENDED, loading = false)))
        assertEquals(PARTY_LEFT, partyGuestPhase(PartyGuestModel.State(phase = PartyPhase.LEFT, loading = false)))
        assertEquals(PARTY_LANDING, partyGuestPhase(PartyGuestModel.State(phase = PartyPhase.LANDING, loading = false)))
        assertEquals(PARTY_LIVE, partyGuestPhase(PartyGuestModel.State(phase = PartyPhase.LIVE, loading = false)))
    }
}
