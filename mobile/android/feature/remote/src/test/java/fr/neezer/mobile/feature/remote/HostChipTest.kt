package fr.neezer.mobile.feature.remote

import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.RemotePhase

/**
 * La barre hôte (lot M6) : `HostChip` est pur (il prend un `RemoteStatusFi`), donc
 * deux assertions suffisent à tenir la règle du lot — la barre NOMME qui pilote
 * (« Léa »), et « Couper » part au PREMIER geste (une révocation, sans la
 * confirmation en deux temps de la feuille). Un rendu JVM : ni émulateur ni
 * fenêtre sur l'écran de la personne (worker rule 7).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-xhdpi")
class HostChipTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** Un appareil piloté : `driven`, un contrôleur, la phase « en direct ». */
    private val driven = RemoteStatusFi(
        controlling = false,
        driven = true,
        hostName = "Léa",
        who = "Léa",
        controllers = listOf(
            RemoteControllerRow(link = "link-1", level = "full", levelLabel = "Complet", label = "Léa"),
        ),
        hostLevel = "Complet",
        hostError = null,
        applied = 0,
        phase = RemotePhase.LIVE,
        online = true,
        grant = null,
        line = "",
        error = null,
        restored = null,
    )

    private fun show(onCut: () -> Unit) {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxWidth().background(Sillon.colors.bg).padding(NzSpacing.m)) {
                    HostChip(status = driven, onCut = onCut, modifier = Modifier.fillMaxWidth())
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
    }

    @Test
    fun the_chip_names_the_driver() {
        show(onCut = {})
        rule.onNodeWithText("Léa").assertIsDisplayed()
    }

    @Test
    fun cutting_is_one_tap() {
        var cuts = 0
        show(onCut = { cuts++ })
        rule.onNode(hasText("Couper") and hasClickAction()).performClick()
        rule.waitForIdle()
        assertEquals("un seul geste coupe", 1, cuts)
    }
}
