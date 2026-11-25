package fr.neezer.mobile.car

import android.view.View
import android.view.ViewGroup
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.ComponentActivity
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

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
        } finally {
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
}
