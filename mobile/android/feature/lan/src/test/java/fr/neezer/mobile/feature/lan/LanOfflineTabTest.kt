package fr.neezer.mobile.feature.lan

import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.feature.offline.OfflineSettingsSection
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.PeerSetFfi

/**
 * LN14's one line in `:feature:offline`: the offline page's second tab.
 *
 * The tab strip and its state stay MB15's (that module owns the page); this lot
 * brings the content, so the test renders the REAL `OfflineSettingsSection` with
 * the slot the settings screen passes and proves what a person sees: two tabs,
 * and « Sur vos appareils » showing the peers' sets. A JVM render, so no
 * emulator is needed for it.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h2400dp-xhdpi")
class LanOfflineTabTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun the_offline_page_gains_sur_vos_appareils_as_its_second_tab() {
        val sets = PeerSetsModel(
            port = FakeLanPort(peerSetsValue = listOf(sampleSet(title = "Route"))),
            scope = CoroutineScope(Dispatchers.Main),
            io = Dispatchers.Main,
        )
        sets.reload()

        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, 1f)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg).padding(NzSpacing.pageX)) {
                        OfflineSettingsSection(peerSets = { PeerSetsSection(model = sets) })
                    }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()

        rule.onNodeWithText("Téléchargements").assertIsDisplayed()
        rule.onNodeWithText("Sur vos appareils").assertIsDisplayed()

        rule.onNodeWithText("Sur vos appareils").performClick()
        rule.waitForIdle()

        rule.onNodeWithText("Route", substring = true).assertIsDisplayed()
        rule.onNodeWithText("Télécharger").assertIsDisplayed()
    }

    /** A caller without the slot (an older build, a JVM fake) draws no tab. */
    @Test
    fun without_the_slot_the_page_is_exactly_as_before() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    OfflineSettingsSection()
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()

        val tabs = rule.onAllNodesWithTextSafe("Sur vos appareils")
        org.junit.Assert.assertEquals("no second tab is drawn", 0, tabs)
    }

    private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.onAllNodesWithTextSafe(text: String): Int =
        onAllNodes(androidx.compose.ui.test.hasText(text, substring = true)).fetchSemanticsNodes().size
}
