package fr.neezer.mobile.car

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.InsetDrawable
import android.view.View
import android.view.ViewGroup
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.ComponentActivity
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.math.roundToInt

/**
 * « Réglages → Mode voiture » s'ouvre (MB24, porte de M13) — la première trame,
 * sur le JVM, sans émulateur.
 *
 * Ce test existe pour une régression précise, mesurée : l'écran héritait
 * d'`AppCompatActivity` alors que l'app ne livre AUCUN thème `Theme.AppCompat`
 * (`:app`'s `themes.xml` : « No appcompat and no Material Views theme »), et
 * `AppCompatDelegateImpl.createSubDecor` lève
 * `IllegalStateException: You need to use a Theme.AppCompat theme (or descendant)
 * with this activity.` au PREMIER `setContentView` — la totalité de l'écran
 * mourait avant sa première image, en debug comme en release. Sur le commit
 * d'avant le correctif, ce test échoue à
 * `AppCompatDelegateImpl.java:902 → CarSettingsActivity.kt:137`
 * (`:car:testDebugUnitTest`).
 *
 * Il tourne dans le job `apk` de `mobile.yaml` (`testDebugUnitTest`), donc une
 * régression est vue en minutes, pas après le tour d'émulateur.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CarSettingsActivityLaunchTest {

    @Test
    fun theCarSettingsPaintTheirFirstFrameUnderTheAppWindowTheme() {
        val controller = Robolectric.buildActivity(CarSettingsActivity::class.java).setup()
        try {
            val activity = controller.get()
            // Le thème de la fenêtre est celui du manifeste (`Theme.Material.NoActionBar`),
            // pas un AppCompat : c'est exactement ce que `ComponentActivity` accepte et
            // ce que `AppCompatActivity` refusait.
            assertTrue(activity is ComponentActivity)

            val root = activity.window.decorView
            assertTrue("l'écran n'a pas de ScrollView", firstOf<ScrollView>(root) != null)

            val texts = textsOf(root)
            assertTrue("le titre de l'écran manque : $texts", texts.contains("Mode voiture"))
            assertTrue("la carte « Déclenchement » manque : $texts", texts.contains("Déclenchement"))
            // Une ligne d'aide sous un réglage est du « texte inutile » (C5,
            // décision du propriétaire 2026-11-27). La garde nomme celles que
            // l'écran portait, pour qu'elles ne reviennent pas.
            for (line in HELP_LINES) {
                assertFalse("ligne d'aide revenue : $line", texts.any { it.contains(line) })
            }
        } finally {
            controller.close()
        }
    }

    /**
     * Le sol tactile de Material : 48 × 48 dp sur CHAQUE contrôle nommé
     * (https://m3.material.io/foundations/designing/structure — « Touch
     * targets », https://developer.android.com/guide/topics/ui/accessibility/apps#large-controls).
     *
     * Le precheck du juge comptait 13 cibles sous 48 dp sur la capture
     * `07-mode-voiture` (run 45810671271 : retour 44×44, les deux
     * interrupteurs 52×30, les puces à 40 dp, « Réinitialiser la position »
     * 33 dp) — dont la moitié seulement tenait dans la fenêtre de la capture.
     * Ce test lit l'arbre ENTIER de l'écran, à la taille d'un téléphone, et
     * nomme chaque contrôle fautif avec sa taille en dp, pour qu'une
     * régression dise quoi et où.
     */
    @Test
    @Config(qualifiers = "w411dp-h866dp-xxhdpi")
    // Les mesures de texte de Robolectric en mode LEGACY valent un pixel par
    // caractère (« Google Maps » 11 px) : une puce paraîtrait alors large de
    // 30 dp sur le JVM et de 111 dp sur l'émulateur. Le mode NATIVE mesure la
    // vraie police, comme l'appareil.
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun everyClickableControlOfTheCarSettingsIsAtLeast48Dp() {
        val app = RuntimeEnvironment.getApplication()
        // Une liste vide de playlists ne montre aucune ligne à bouton
        // (épingler/monter) : on en pose deux pour que l'arbre contienne aussi
        // ces deux contrôles-là, et pas seulement les puces et les
        // interrupteurs de la première fenêtre.
        CarPlaylistSource.forget()
        CarPrefs(app).pins = listOf(
            PinnedPlaylist("bench-a", "Liste de test A", 3),
            PinnedPlaylist("bench-b", "Liste de test B", 2),
        )
        val controller = Robolectric.buildActivity(CarSettingsActivity::class.java).setup()
        try {
            val activity = controller.get()
            val density = activity.resources.displayMetrics.density
            // La taille de l'émulateur du juge : 411 × 866 dp.
            val width = (411f * density).roundToInt()
            val height = (866f * density).roundToInt()
            val root = activity.window.decorView
            root.measure(
                View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY),
            )
            root.layout(0, 0, width, height)

            val floor = 48f * density
            val offenders = mutableListOf<String>()
            fun walk(view: View) {
                if (view.visibility != View.VISIBLE) return
                if (view.isClickable && (view.width < floor || view.height < floor)) {
                    val w = (view.width / density).roundToInt()
                    val h = (view.height / density).roundToInt()
                    offenders.add("${labelOf(view)} ${w}x${h} dp")
                }
                if (view is ViewGroup) {
                    for (i in 0 until view.childCount) walk(view.getChildAt(i))
                }
            }
            walk(root)

            assertTrue(
                "cibles tactiles sous 48 dp (${offenders.size}) : " + offenders.joinToString(", "),
                offenders.isEmpty(),
            )

            // Seule la boîte grandit : le rond de 44 dp et la pastille de
            // 40 dp gardent leur taille dessinée, centrés dans les 48 dp.
            // Une première passe de dessin fixe les bornes des Drawable.
            root.draw(Canvas(Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)))

            val back = find(root) { it.contentDescription == "Retour" }!!
            val circle = (back.background as InsetDrawable).drawable!!.bounds
            assertTrue(
                "le rond du retour doit rester 44x44 dp, mesuré " +
                    "${(circle.width() / density).roundToInt()}x${(circle.height() / density).roundToInt()} dp",
                circle.width() == (44f * density).roundToInt() &&
                    circle.height() == (44f * density).roundToInt(),
            )
            assertTrue(
                "le rond du retour doit être centré dans sa boîte de 48 dp, left=${circle.left}px",
                circle.left == (2f * density).roundToInt(),
            )

            val chip = find(root) { it is TextView && it.text == "Google Maps" }!!
            val pill = (chip.background as InsetDrawable).drawable!!.bounds
            assertTrue(
                "la pastille doit rester 40 dp de haut, mesurée ${(pill.height() / density).roundToInt()} dp",
                pill.height() == (40f * density).roundToInt(),
            )
            assertTrue(
                "la pastille garde sa largeur visible (${pill.width()}px pour ${chip.width}px de boîte)",
                pill.width() == chip.width,
            )
        } finally {
            CarPrefs(app).pins = emptyList()
            controller.close()
        }
    }

    private fun find(root: View, predicate: (View) -> Boolean): View? {
        if (predicate(root)) return root
        if (root is ViewGroup) {
            for (i in 0 until root.childCount) {
                find(root.getChildAt(i), predicate)?.let { return it }
            }
        }
        return null
    }

    private inline fun <reified T : View> firstOf(root: View): T? = find(root) { it is T } as T?

    private fun textsOf(root: View): List<String> {
        val out = mutableListOf<String>()
        fun walk(view: View) {
            if (view is TextView) view.text?.toString()?.let(out::add)
            if (view is ViewGroup) {
                for (i in 0 until view.childCount) walk(view.getChildAt(i))
            }
        }
        walk(root)
        return out
    }

    /** Ce que TalkBack annonce, puis la classe réelle : « quoi » et « où ». */
    private fun labelOf(view: View): String {
        val desc = view.contentDescription?.toString().orEmpty()
        val text = (view as? TextView)?.text?.toString().orEmpty()
        val named = listOf(desc, text).firstOrNull { it.isNotBlank() }
        val name = if (named != null) "\"$named\"" else "(sans nom)"
        return "$name ${view.javaClass.simpleName}"
    }

    private companion object {
        /** Les phrases d'aide que l'écran portait, retirées le 2026-11-27. */
        val HELP_LINES = listOf(
            "Le mode apparaît quand une app GPS passe devant",
            "Sans GPS ou sans statistiques d'usage",
            "Fais glisser la carte en Grand pour la placer",
            "Pour que la barre apparaisse sans quitter le GPS.",
            "Pour détecter l'app GPS au premier plan.",
            "Pour savoir si tu roules.",
            "Pour réagir à la connexion de la voiture.",
            "Pour la recherche vocale en roulant.",
            "Pour les habitudes RTL",
            "Épingle celles que tu veux voir en roulant.",
            "Plus la barre est opaque, moins on voit la route derrière.",
            "Suit le clavier du téléphone",
        )
    }
}
