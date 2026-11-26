package fr.neezer.mobile.feature.playlist

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.PlaylistImportStatus

/**
 * L'import répond au doigt et laisse ses trois actions sur l'écran (lot X6).
 *
 * **The defect the lot closes.** At 360 dp the « Lire », « Mélanger » and
 * « Enregistrer » buttons were a plain `Row`: ≈ 380 dp of labelled buttons in a
 * 320 dp column, so the third was drawn past the window's edge and could not be
 * reached. A `FlowRow` wraps it (the repo's own answer, `AdminParts.kt`), and the
 * buttons still share one line at 600 dp.
 *
 * **What is measured, and where the number comes from.** The import body's own
 * content padding is `NzSpacing.l` (16 dp), so the right edge a button may reach
 * is `360 − 16 = 344 dp`; the plan's "360 − 20 dp" assumed the page inset
 * (`pageX`), which this screen does not use, and a spacing change is out of X6's
 * scope. The hard requirement — every button INSIDE the window — is asserted
 * too (`right ≤ 360 dp`).
 *
 * A Compose test cannot read a running animation's spec (X0's note): the curve
 * is `inPlaceSpecs`'s, and this file proves the WIRING — at eco the new phase is
 * there in the frame it was asked for, and a re-emit of the same phase adds no
 * node (M6).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class ImportScreenMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun doneState() = ImportModel.State(
        link = "https://open.spotify.com/playlist/abc",
        job = PlaylistImportStatus(
            id = "job-1",
            state = "done",
            source = "spotify",
            sourceLabel = "Spotify",
            kind = "playlist",
            url = "https://open.spotify.com/playlist/abc",
            name = "Soirée",
            cover = null,
            total = 12u,
            truncated = false,
            done = 12u,
            matched = 12u,
            error = null,
            playlistId = null,
            counts = emptyList(),
            items = emptyList(),
            running = false,
            percent = 100u,
        ),
    )

    private fun host(state: ImportModel.State, level: NzMotionLevel) {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = level) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ImportPhases(state = state, modifier = Modifier.fillMaxSize())
                }
            }
        }
    }

    private fun actionBounds(): List<androidx.compose.ui.unit.DpRect> =
        listOf("Lire", "Mélanger", "Enregistrer").map { label ->
            rule.onNode(hasText(label) and hasClickAction()).getUnclippedBoundsInRoot()
        }

    @Test
    fun theThreeActionsAreReadableAndOnScreenAt360Dp() {
        host(doneState(), NzMotionLevel.Full)
        rule.waitForIdle()

        val bounds = actionBounds()
        val window = 360f
        val inset = NzSpacing.l.value
        bounds.forEach { b ->
            assertTrue("an action is past the window's edge: ${b.right.value} > $window", b.right.value <= window)
            assertTrue(
                "an action is past the body's content edge: ${b.right.value} > ${window - inset} (measured 360 − ${inset} dp)",
                b.right.value <= window - inset + 0.5f,
            )
        }
        // The defect this lot closes is the THIRD label, not a button off the
        // screen: a plain Row shrank it to 111 dp and « Enregistrer » ellipsized.
        // Measured floors (FlowRow, 360 dp, NATIVE text): 82 / 118.5 / 130.5 dp.
        val floors = listOf(80f, 112f, 128f)
        bounds.forEachIndexed { index, b ->
            assertTrue(
                "action $index is squeezed: width=${(b.right - b.left).value} < ${floors[index]} dp",
                (b.right - b.left).value >= floors[index],
            )
        }
    }

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
    fun theThreeActionsStayOnOneLineAt600Dp() {
        host(doneState(), NzMotionLevel.Full)
        rule.waitForIdle()

        val tops = actionBounds().map { it.top.value }
        assertEquals("one line at 600 dp: $tops", 1, tops.distinct().size)
    }

    @Test
    fun thePhaseReplacesInTheSameFrameAtEco() {
        var state by mutableStateOf(ImportModel.State())
        var tick by mutableIntStateOf(0)
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    tick
                    ImportPhases(state = state, modifier = Modifier.fillMaxSize())
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("Importer une playlist").assertExists()
        rule.onNodeWithText("Lire").assertDoesNotExist()

        state = doneState()
        rule.waitForIdle()
        rule.onNodeWithText("Lire").assertExists()
        rule.onNodeWithText("Importer une playlist").assertDoesNotExist()

        // M6: the same phase re-emitted plays nothing and adds no node.
        tick++
        rule.waitForIdle()
        rule.onAllNodesWithText("Lire").assertCountEquals(1)

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }

    @Test
    fun theOldPhaseDoesNotLingerForTheWholeFade() {
        var state by mutableStateOf(ImportModel.State())
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    ImportPhases(state = state, modifier = Modifier.fillMaxSize())
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("Importer une playlist").assertExists()

        rule.mainClock.autoAdvance = false
        state = doneState()
        rule.waitForIdle()
        // M7: the outgoing phase is not held for the 140 ms fade (a Compose frame
        // boundary is the granularity; measured: gone by 50 ms, present at 1 ms).
        rule.mainClock.advanceTimeBy(50)
        rule.onNodeWithText("Importer une playlist").assertDoesNotExist()
        rule.onNodeWithText("Lire").assertExists()
        rule.mainClock.autoAdvance = true
    }

    @Test
    fun theThreePhasesAreTheScreensOwnStates() {
        assertEquals(IMPORT_INTRO, importPhase(ImportModel.State()))
        assertEquals(
            IMPORT_FAILED,
            importPhase(ImportModel.State(failure = "Le serveur n'a pas répondu.")),
        )
        assertEquals(IMPORT_BODY, importPhase(doneState()))
        // A job that carries a failure is still the body: the cause is drawn in
        // the body (the job's own line), not as a whole-screen failure.
        assertEquals(IMPORT_BODY, importPhase(doneState().copy(failure = "une ligne")))
    }
}
