package fr.neezer.mobile.feature.offline

import android.os.Looper
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Réglages → Téléchargements, rendered and VERIFIED at 360 dp (plan T3's
 * matrix, one representative state).
 *
 * T3 replaced MB15's write-only captures with this golden. The section is drawn
 * over a real [DownloadsModel] fed by [FakeOfflinePort]'s state (worker rule 7),
 * so the queued rows, the failures and the storage line are the ones a person
 * sees.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi")
class OfflineGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @After
    fun forget() {
        OfflineHost.reset()
    }

    @Test
    fun downloadsAt360() {
        val port = FakeOfflinePort()
        port.stateFlow.value = offlineState()
        OfflineHost.install(
            DownloadsModel(
                port = port,
                scope = CoroutineScope(Dispatchers.Unconfined),
                io = Dispatchers.Unconfined,
                freeSpace = { 8L * 1024 * 1024 * 1024 },
                toast = { _, _, _ -> },
                causeText = { it.message ?: "erreur" },
            ),
        )
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg).clipToBounds()) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
                        verticalArrangement = Arrangement.spacedBy(NzSpacing.l),
                    ) {
                        OfflineSettingsSection()
                    }
                }
            }
        }
        rule.composeRule.mainClock.advanceTimeBy(1_000)
        shadowOf(Looper.getMainLooper()).idle()
        rule.composeRule.waitForIdle()
        rule.capture("offline-telechargements-360dp-f1.0.png")
    }
}
