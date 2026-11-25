package fr.neezer.mobile.designsystem

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * La barre d'onglets de L1 seule, aux quatre coins du plan (§4 L1, « Tests to
 * add ») : 360 et 600 dp × Encre et Papier × `fontScale` 1 et 2, plus une image
 * de coquille qui montre ce que le lot enlève (aucun bandeau au-dessus du
 * contenu, une seule rangée de retour sur un écran de détail).
 *
 * Le rendu est Robolectric sur son backend graphique NATIF (Skia sur l'hôte) —
 * la recette du dépôt (worker rule 7 : pas de fenêtre, pas d'émulateur, rien sur
 * l'écran de la personne). Paparazzi est refusé par ce dépôt pour les raisons
 * mesurées dans `:designsystem`'s `build.gradle.kts`.
 *
 * `fontScale` est posé par `LocalDensity` plutôt que par un qualifier : c'est le
 * même chemin que le composant lit (`LocalDensity.current.fontScale`). La largeur,
 * elle, est un qualifier — le renderer d'un test rend une fenêtre à la taille de
 * l'écran configuré, donc 600 dp est une seconde classe.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class NavBarScreenshotTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun encreAtFontScaleOne() = shot("l1-barre-360-encre-1.png", NzThemeKind.Encre, 1f)

    @Test
    fun encreAtFontScaleTwo() = shot("l1-barre-360-encre-2.png", NzThemeKind.Encre, 2f)

    @Test
    fun papierAtFontScaleOne() = shot("l1-barre-360-papier-1.png", NzThemeKind.Papier, 1f)

    @Test
    fun papierAtFontScaleTwo() = shot("l1-barre-360-papier-2.png", NzThemeKind.Papier, 2f)

    /** Ce que le bandeau laisse: aucune barre au-dessus du contenu, une rangée. */
    @Test
    fun aDetailRowAndTheBar() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Column(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Row(
                        modifier = Modifier.fillMaxWidth().height(56.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.Start,
                    ) {
                        NzIconButton(glyph = NzGlyph.ChevronLeft, contentDescription = "Retour", onClick = {})
                    }
                    Spacer(Modifier.weight(1f))
                    NzTabBar(items = TAB_ITEMS_FOR_TEST, selected = -1, onPick = {})
                }
            }
        }
        write("l1-coquille-detail-360.png")
    }

    private fun shot(name: String, kind: NzThemeKind, fontScale: Float) {
        rule.setContent {
            NzTheme(kind = kind, motion = NzMotionLevel.Full) {
                Column(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Spacer(Modifier.weight(1f))
                    val density = LocalDensity.current
                    CompositionLocalProvider(
                        LocalDensity provides Density(density = density.density, fontScale = fontScale),
                    ) {
                        NzTabBar(items = TAB_ITEMS_FOR_TEST, selected = 1, onPick = {})
                    }
                }
            }
        }
        write(name)
    }

    private fun write(name: String) {
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        outputDir.mkdirs()
        val file = File(outputDir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    private val outputDir: File
        get() = File(
            System.getProperty("nz.repoRoot")
                ?: error("nz.repoRoot is not set: the screenshot test writes the lot's images"),
            "docs/agents/screenshots/mobile/L1",
        )
}

/**
 * The same four 600 dp renders: a tablet window is where the bar's items are the
 * widest they ever get (120 dp each), and where a label at `fontScale = 2` has to
 * fit without ellipsis.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
class NavBarWideScreenshotTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun encreAtFontScaleOne() = shot("l1-barre-600-encre-1.png", NzThemeKind.Encre, 1f)

    @Test
    fun encreAtFontScaleTwo() = shot("l1-barre-600-encre-2.png", NzThemeKind.Encre, 2f)

    @Test
    fun papierAtFontScaleOne() = shot("l1-barre-600-papier-1.png", NzThemeKind.Papier, 1f)

    @Test
    fun papierAtFontScaleTwo() = shot("l1-barre-600-papier-2.png", NzThemeKind.Papier, 2f)

    private fun shot(name: String, kind: NzThemeKind, fontScale: Float) {
        rule.setContent {
            NzTheme(kind = kind, motion = NzMotionLevel.Full) {
                Column(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Spacer(Modifier.weight(1f))
                    val density = LocalDensity.current
                    CompositionLocalProvider(
                        LocalDensity provides Density(density = density.density, fontScale = fontScale),
                    ) {
                        NzTabBar(items = TAB_ITEMS_FOR_TEST, selected = 1, onPick = {})
                    }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        outputDir.mkdirs()
        val file = File(outputDir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    private val outputDir: File
        get() = File(
            System.getProperty("nz.repoRoot")
                ?: error("nz.repoRoot is not set: the screenshot test writes the lot's images"),
            "docs/agents/screenshots/mobile/L1",
        )
}

/**
 * The same five items the shell builds (`TAB_ITEMS` lives in `:app`, which the
 * design system cannot see): the bar's own picture must not depend on the shell.
 */
private val TAB_ITEMS_FOR_TEST = listOf(
    NzTabItem(NzGlyph.Home, "Accueil"),
    NzTabItem(NzGlyph.Compass, "Rechercher"),
    NzTabItem(NzGlyph.Library, "Biblio"),
    NzTabItem(NzGlyph.Mic, "Podcasts"),
    NzTabItem(NzGlyph.Settings, "Réglages"),
)
