package fr.neezer.mobile.car

import android.content.Context
import android.content.Intent
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.TextView
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * La porte du « mode voiture » sur l'appareil : l'écran s'ouvre, et CHAQUE
 * réglage prend.
 *
 * La porte est unique (`CarModeService.openSettings` : l'action « Réglages » de
 * la notification, la tuile des réglages rapides, la tuile fantôme du panneau et
 * le bouton de Réglages passent tous par elle) et elle finit toujours sur la
 * même intention, `CarSettingsActivity` + `FLAG_ACTIVITY_NEW_TASK`. Le test
 * lance EXACTEMENT cette intention, puis manœuvre chaque contrôle de l'écran une
 * fois : les trois interrupteurs, l'app GPS, la disposition du clavier, la limite
 * de lettres, la réinitialisation des positions et la barre d'opacité. Chaque
 * geste est vérifié dans [CarPrefs], donc « le clic ne fait rien » échoue ici
 * aussi.
 *
 * La régression qu'il ferme (mesurée le 2026-11-25) : l'écran héritait
 * d'`AppCompatActivity` sans thème `Theme.AppCompat` (l'app n'en livre aucun,
 * `:app`'s `themes.xml`), et `AppCompatDelegateImpl.createSubDecor` levait
 * `IllegalStateException: You need to use a Theme.AppCompat theme (or descendant)
 * with this activity.` au premier `setContentView` — l'écran mourait avant sa
 * première image, en debug comme en release. Sur le commit d'avant le correctif,
 * ce test échoue dans `ActivityScenario.launch` (le procès de l'app est tué par
 * l'exception) ; après, il passe.
 */
@RunWith(AndroidJUnit4::class)
class CarSettingsActivityDeviceTest {

    private val context: Context
        get() = ApplicationProvider.getApplicationContext()

    @Test
    fun theDoorOpensTheSettingsAndEverySettingTakes() {
        val prefs = CarPrefs(context)
        // L'intention que `CarModeService.launchSettingsActivity` construit, à la
        // lettre : le composant de l'activité, et NEW_TASK parce qu'on la quitte.
        val intent = Intent(context, CarSettingsActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        ActivityScenario.launch<CarSettingsActivity>(intent).use { scenario ->
            scenario.onActivity { activity ->
                assertFalse("l'écran s'est terminé tout seul", activity.isFinishing)

                // La première trame est peinte : le titre, puis une carte de chaque
                // section. C'est ce que le crash empêchait.
                val root = activity.window.decorView
                assertTrue("aucun ScrollView : l'écran n'a pas peint", firstOf<ScrollView>(root) != null)
                val texts = textsOf(root)
                for (title in listOf(
                    "Mode voiture",
                    "Déclenchement",
                    "Position",
                    "Clavier",
                    "Playlists du mode voiture",
                    "Opacité de la barre",
                )) {
                    assertTrue("« $title » manque à l'écran : $texts", texts.contains(title))
                }

                // Les trois interrupteurs : chacun inverse SON réglage.
                val gpsBefore = prefs.gpsTrigger
                toggle(activity, "Se placer sur l'app GPS au premier plan")
                assertEquals("le déclenchement GPS n'a pas suivi", !gpsBefore, prefs.gpsTrigger)

                val bluetoothBefore = prefs.bluetoothTrigger
                toggle(activity, "Aussi au Bluetooth de la voiture")
                assertEquals(
                    "le déclenchement Bluetooth n'a pas suivi",
                    !bluetoothBefore,
                    prefs.bluetoothTrigger,
                )

                val swipeBefore = prefs.invertSwipe
                toggle(activity, "Inverser gauche et droite")
                assertEquals("l'inversion des gestes n'a pas suivi", !swipeBefore, prefs.invertSwipe)

                // Une app GPS surveillée : la puce bascule son appartenance.
                val wazeBefore = "com.waze" in prefs.gpsPackages
                click(activity, "Waze")
                assertEquals(
                    "la puce Waze n'a pas suivi",
                    !wazeBefore,
                    "com.waze" in prefs.gpsPackages,
                )

                click(activity, "ABC")
                assertEquals("la disposition ABC n'a pas été retenue", "abc", prefs.keyboardLayout)

                click(activity, "4 lettres")
                assertEquals("la limite de lettres n'a pas été retenue", 4, prefs.maxLetters)

                prefs.setPosition("com.google.android.apps.maps", CarSize.FULL, CarSlot(10f, 20f))
                click(activity, "Réinitialiser la position")
                assertNull(
                    "la position n'a pas été effacée",
                    prefs.position("com.google.android.apps.maps", CarSize.FULL),
                )

                val bar = firstOf<SeekBar>(activity.window.decorView)
                assertTrue("aucune barre d'opacité", bar != null)
                bar!!.progress = 57
                assertEquals("l'opacité n'a pas suivi", 0.57f, prefs.overlayAlpha, 0.001f)
            }
        }
    }


    @Test
    fun theContentStartsBelowTheStatusBarInset() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val intent = Intent(context, CarSettingsActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        ActivityScenario.launch<CarSettingsActivity>(intent).use { scenario ->
            scenario.onActivity { activity ->
                val decor = activity.window.decorView
                val insets = ViewCompat.getRootWindowInsets(decor)
                val statusBarTop = insets?.getInsets(
                    WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
                )?.top ?: 0
                // The first child of the content LinearLayout is the first card/row.
                val content = activity::class.java.getDeclaredField("content")
                    .apply { isAccessible = true }.get(activity) as LinearLayout
                val firstChild = content.getChildAt(0)
                val loc = IntArray(2)
                firstChild.getLocationInWindow(loc)
                assertTrue(
                    "first child top ${loc[1]} < status bar inset $statusBarTop",
                    loc[1] >= statusBarTop,
                )
            }
        }
    }

    /** One of the drawn switches, found by the accessibility name it publishes. */
    private fun toggle(activity: CarSettingsActivity, description: String) {
        val view = find(activity.window.decorView) {
            it.isClickable && it.contentDescription?.toString() == description
        }
        assertTrue("aucun interrupteur « $description »", view != null)
        view!!.performClick()
    }

    /** One of the screen's chips, found by its exact text. */
    private fun click(activity: CarSettingsActivity, label: String) {
        val view = find(activity.window.decorView) {
            it.isClickable && it is TextView && it.text?.toString() == label
        }
        assertTrue("aucune puce « $label »", view != null)
        view!!.performClick()
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
