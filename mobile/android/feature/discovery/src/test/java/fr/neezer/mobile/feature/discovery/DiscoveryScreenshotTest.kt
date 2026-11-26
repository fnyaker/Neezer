package fr.neezer.mobile.feature.discovery

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Découverte, rendered and VERIFIED: the releases page with one group open at
 * 360 dp (plan T3's matrix, one representative state).
 *
 * T3 replaced MB14's write-only captures with this golden. The real
 * [ReleasesScreen] is drawn over [DiscoveryFixtures] (worker rule 7: Robolectric's
 * NATIVE backend, no emulator, no window); a shelf header or a row that drifts
 * fails the comparison.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class DiscoveryGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun releasesAt360() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    // No outer scroll: the screen scrolls itself (a LazyColumn
                    // cannot be measured inside one).
                    Column(Modifier.fillMaxSize()) {
                        ReleasesScreen(
                            state = ReleasesModel.State(
                                screen = ScreenState.Ready(DiscoveryFixtures.releases()),
                                showAll = true,
                                opened = setOf("501"),
                                groupRows = mapOf("501" to DiscoveryFixtures.rows().take(2)),
                            ),
                            actions = ReleasesActions(),
                        )
                    }
                }
            }
        }
        rule.capture("discovery-nouveautes-360dp-f1.0.png")
    }
}
