package fr.neezer.mobile.app

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * **Le cadre des coquilles : les surfaces plein écran sont AU-DESSUS des barres**
 * (SB3, `spec-mobile-shell-bugs.md` §SB3).
 *
 * Le lecteur plein écran était monté DANS le contenu de la `Scaffold` : il
 * remplissait la zone entre l'inset haut et la `bottomBar`, et le mini-lecteur
 * plus la barre d'onglets restaient visibles en dessous. Ce test mesure le cadre
 * seul (`ShellFrame`), avec des boîtes nues — aucun noyau Rust, aucune route.
 *
 * Robolectric, `w360dp-h800dp-xhdpi` : le lecteur téléphone et le lecteur large.
 */
@RunWith(RobolectricTestRunner::class)
@Config(
    sdk = [34],
    qualifiers = "w360dp-h800dp-xhdpi",
    // Le VRAI `Application` de l'app charge `libnz_ffi.so` dans `onCreate` : sur
    // la JVM il n'y a pas de `.so`, et le cadre n'appelle rien du noyau.
    application = android.app.Application::class,
)
class ShellChromeTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /**
     * Téléphone : la couche plein écran touche le bas de la fenêtre ET son haut.
     *
     * Le `bottomBar` de 64 dp est le repère : la couche doit PASSER dessous, pas
     * s'arrêter à son haut (ce que le montage dans le contenu faisait).
     */
    @Test
    fun theOverlayCoversTheBar() {
        frame(wide = false, rail = {})

        val root = rule.onRoot().getUnclippedBoundsInRoot()
        val overlay = rule.onNodeWithTag("overlay").getUnclippedBoundsInRoot()
        println(
            "MEASURE sb3 overlay phone: top=${overlay.top.value}dp " +
                "bottom=${overlay.bottom.value}dp root.bottom=${root.bottom.value}dp",
        )
        assertEquals("la couche ne part pas du haut de la fenêtre", 0.dp, overlay.top)
        assertEquals(
            "la couche s'arrête au-dessus de la barre (bottom ${overlay.bottom.value} dp, " +
                "racine ${root.bottom.value} dp)",
            root.bottom.value,
            overlay.bottom.value,
            0.5f,
        )
    }

    /**
     * Large : la couche commence au bord GAUCHE, donc au-dessus du rail.
     *
     * Aujourd'hui le contenu large est la colonne à droite du rail : la couche y
     * commençait à `rail.width` (80 dp) et laissait le rail visible à côté du
     * lecteur.
     */
    @Test
    fun theOverlayCoversTheRailOnWide() {
        frame(wide = true, rail = { Box(Modifier.width(80.dp).fillMaxHeight().testTag("rail")) })

        val overlay = rule.onNodeWithTag("overlay").getUnclippedBoundsInRoot()
        println("MEASURE sb3 overlay wide: left=${overlay.left.value}dp")
        assertEquals("la couche commence au bord du rail, pas à celui de la fenêtre", 0.dp, overlay.left)
    }

    private fun frame(wide: Boolean, rail: @androidx.compose.runtime.Composable () -> Unit) {
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                ShellFrame(
                    wide = wide,
                    bottomBar = {
                        Box(Modifier.fillMaxWidth().height(64.dp).testTag("bar"))
                    },
                    rail = rail,
                    backRow = {},
                    content = { Box(Modifier.fillMaxSize().testTag("content")) },
                    overlay = { Box(Modifier.fillMaxSize().testTag("overlay")) },
                )
            }
        }
        rule.waitForIdle()
    }
}
