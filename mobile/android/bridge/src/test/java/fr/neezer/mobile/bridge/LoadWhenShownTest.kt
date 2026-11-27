package fr.neezer.mobile.bridge

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * La porte de chargement du lot SB2 (`LoadWhenShown`) : ce que les quatre écrans
 * d'onglet appellent, maintenant qu'ils restent composés.
 *
 * Quatre cas, ceux du contrat : invisible ne charge pas ; devenir visible charge
 * UNE fois ; revenir après un aller-retour ne recharge pas (le modèle et son état
 * ont survécu) ; changer de clé pendant que la surface est visible recharge.
 *
 * L'assertion lit un compteur après `waitForIdle`, jamais une horloge : la porte
 * n'a pas de délai, elle a une transition d'état.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = android.app.Application::class)
class LoadWhenShownTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun invisibleLoadsNothing() {
        var loads = 0
        rule.setContent { LoadWhenShown(visible = false) { loads++ } }
        rule.waitForIdle()
        assertEquals("une surface non visible a chargé", 0, loads)
    }

    @Test
    fun becomingVisibleLoadsOnce() {
        var visible by mutableStateOf(false)
        var loads = 0
        rule.setContent { LoadWhenShown(visible) { loads++ } }
        rule.waitForIdle()
        assertEquals(0, loads)

        visible = true
        rule.waitForIdle()
        assertEquals("devenir visible n'a pas chargé une fois", 1, loads)
    }

    @Test
    fun comingBackDoesNotLoadAgain() {
        var visible by mutableStateOf(true)
        var loads = 0
        rule.setContent { LoadWhenShown(visible) { loads++ } }
        rule.waitForIdle()
        assertEquals(1, loads)

        visible = false
        rule.waitForIdle()
        assertEquals("quitter la surface a rechargé", 1, loads)

        visible = true
        rule.waitForIdle()
        assertEquals("revenir sur une surface déjà chargée l'a rechargée", 1, loads)
    }

    @Test
    fun aKeyChangeWhileVisibleLoadsAgain() {
        var key by mutableStateOf<Any?>("a")
        var loads = 0
        rule.setContent { LoadWhenShown(visible = true, key = key) { loads++ } }
        rule.waitForIdle()
        assertEquals(1, loads)

        key = "b"
        rule.waitForIdle()
        assertEquals("un changement de clé visible n'a pas rechargé", 2, loads)

        // `null` est une clé valide (la recherche sans terme) : elle compte comme
        // un changement, et seulement une fois.
        key = null
        rule.waitForIdle()
        assertEquals("la clé nulle n'a pas compté comme un changement", 3, loads)
        key = null
        rule.waitForIdle()
        assertEquals("la même clé nulle a rechargé deux fois", 3, loads)
    }
}
