package fr.neezer.mobile.car

import android.view.View
import android.view.ViewGroup
import kotlin.math.roundToInt
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.TrackRow

/**
 * « Le mode voiture touche 48 dp partout » — les contrôles que le parcours du
 * juge ne voit PAS (C48, JF).
 *
 * `precheck` ne lit que la capture `07-mode-voiture`, c'est-à-dire l'Activity de
 * réglages. Le lecteur voiture et la recherche n'y sont jamais photographiés :
 * leur poignée de déplacement (44 dp), leur touche passager (30 dp), leur champ
 * et leurs touches de clavier compactes (42 dp) restaient donc invisibles à
 * toute porte. Ce test lit l'arbre ENTIER des deux vues, comme
 * `CarSettingsActivityLaunchTest` le fait pour les réglages, et nomme chaque
 * contrôle fautif avec sa taille en dp.
 *
 * La règle de `precheck.py` pour les nœuds nommés est transposée aux Views :
 * une cible qui n'est pas annoncée (`IMPORTANT_FOR_ACCESSIBILITY_NO`) n'est pas
 * un contrôle pour une personne, donc on ne la mesure pas.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CarTargetsTest {

    /** Le plancher tactile de Material, sur la taille de l'émulateur du juge. */
    @Test
    @Config(qualifiers = "w411dp-h866dp-xxhdpi")
    // Robolectric mesure un pixel par caractère en mode LEGACY : les libellés
    // des pastilles paraîtraient minuscules. NATIVE mesure la vraie police.
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun everyClickableControlOfTheCarPlayerIsAtLeast48Dp() {
        val root = CarPlayerView(RuntimeEnvironment.getApplication(), FakeCarHost())
        val visited = assertEveryTargetIsAtLeast48Dp(root, "lecteur voiture")
        assertTrue("le lecteur voiture n'a aucune cible : $visited", visited > 0)
    }

    /**
     * Le cas de l'écran compact (640 dp de haut, la plus petite taille que le
     * fichier nomme) : c'est là que les touches tombaient à 42 dp.
     *
     * La largeur est 640 dp, pas les 411 dp de l'émulateur du juge : dix touches
     * par rangée ne peuvent pas faire 48 dp chacune dans 411 dp, et l'en-tête de
     * `CarSearchView` dit lui-même « a 640 dp car screen ». Mesurer la recherche
     * sur la largeur d'un téléphone condamnerait le test, pas le code.
     */
    @Test
    @Config(qualifiers = "w640dp-h640dp-xxhdpi")
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun everyClickableControlOfTheCompactCarSearchIsAtLeast48Dp() {
        // En roulant : la touche « Je suis passager » est VISIBLE, donc mesurée.
        val host = FakeCarHost(driving = true)
        val root = CarSearchView(RuntimeEnvironment.getApplication(), host)
        val visited = assertEveryTargetIsAtLeast48Dp(root, "recherche voiture compacte", widthDp = 640)
        // Le clavier + le champ + les trois boutons : bien plus que zéro.
        assertTrue("la recherche compacte n'a visité que $visited cibles", visited >= 20)
    }

    /** Le clavier non compact (touches de 50 dp) et la même marche. */
    @Test
    @Config(qualifiers = "w640dp-h866dp-xxhdpi")
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun everyClickableControlOfTheCarSearchIsAtLeast48Dp() {
        val root = CarSearchView(RuntimeEnvironment.getApplication(), FakeCarHost())
        val visited = assertEveryTargetIsAtLeast48Dp(root, "recherche voiture", widthDp = 640)
        assertTrue("la recherche n'a visité que $visited cibles", visited >= 20)
    }

    /**
     * Mesure la vue à 411 × 866 dp (la taille de l'émulateur du juge), puis
     * parcourt toutes les cibles visibles et rend le nombre visité. Le message
     * d'un échec dit « quoi » et « où », comme le test des réglages.
     */
    private fun assertEveryTargetIsAtLeast48Dp(
        root: View,
        name: String,
        widthDp: Int = 411,
        heightDp: Int = 866,
    ): Int {
        val density = root.resources.displayMetrics.density
        val width = (widthDp * density).roundToInt()
        val height = (heightDp * density).roundToInt()
        root.measure(
            View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY),
        )
        root.layout(0, 0, width, height)

        val floor = 48f * density
        val offenders = mutableListOf<String>()
        var visited = 0
        fun walk(view: View) {
            if (view.visibility != View.VISIBLE) return
            if (
                view.isClickable &&
                view.importantForAccessibility != View.IMPORTANT_FOR_ACCESSIBILITY_NO
            ) {
                visited++
                if (view.width < floor || view.height < floor) {
                    val w = (view.width / density).roundToInt()
                    val h = (view.height / density).roundToInt()
                    offenders.add("${labelOf(view)} : ${w}x$h dp")
                }
            }
            if (view is ViewGroup) {
                for (i in 0 until view.childCount) walk(view.getChildAt(i))
            }
        }
        walk(root)

        println("MEASURE car targets ($name): $visited vues, ${offenders.size} sous 48 dp")
        assertTrue(
            "cibles tactiles sous 48 dp dans $name (${offenders.size}) : " +
                offenders.joinToString(", "),
            offenders.isEmpty(),
        )
        return visited
    }

    /** Ce que TalkBack annonce, puis la classe réelle : « quoi » et « où ». */
    private fun labelOf(view: View): String {
        val desc = view.contentDescription?.toString().orEmpty()
        val named = desc.ifBlank { "(sans nom)" }
        return "$named ${view.javaClass.simpleName}"
    }

    /**
     * Le seul contrat que les deux vues lisent : l'état de conduite et les
     * préférences. Aucune méthode de lecture audio, de recherche ou de fenêtre
     * n'est appelée à la construction ; elles comptent un appel si jamais.
     */
    private class FakeCarHost(
        private val driving: Boolean = false,
        private var passenger: Boolean = false,
    ) : CarHost {
        override val prefs: CarPrefs = CarPrefs(RuntimeEnvironment.getApplication())

        override fun now(): CarNow? = null
        override fun isDriving(): Boolean = driving
        override fun speedKmh(): Double = if (driving) 50.0 else 0.0
        override fun isPassenger(): Boolean = passenger
        override fun setPassenger(value: Boolean) {
            passenger = value
        }

        override fun togglePlay() = Unit
        override fun next() = Unit
        override fun previous() = Unit
        override fun seekTo(seconds: Double) = Unit
        override fun toggleFavorite() = Unit
        override fun setSize(size: CarSize) = Unit
        override fun setScreen(screen: CarScreen) = Unit
        override fun closeOverlay() = Unit
        override fun moveOverlayBy(dxPx: Int, dyPx: Int) = Unit
        override fun endOverlayMove() = Unit
        override fun launch(kind: String, id: String, title: String, track: TrackRow?) = Unit
        override fun openSettings() = Unit
        override fun toast(message: String) = Unit
    }
}
