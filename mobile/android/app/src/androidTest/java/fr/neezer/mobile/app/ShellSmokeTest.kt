package fr.neezer.mobile.app

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import fr.neezer.mobile.feature.nowplaying.NowPlayingState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The skeleton's instrumented smoke (docs/agents/mobile.md, MQ1): the app really
 * starts on a device, its shell paints, a route opens, and the Rust core answers
 * through JNA. Four facts, four tests — each one is what a lot after MB0 can
 * break without noticing, and each failure names the layer it broke.
 *
 * **Semantics, never coordinates** (the note's rule): every node is found by its
 * French text, which is the same tree TalkBack reads, so a re-laid-out screen
 * cannot make a test pass by accident the way a screenshot coordinate can.
 *
 * Text uniqueness: a tab's label is on screen twice (the top bar's title and the
 * bar's own item), and the placeholder titles repeat it a third time. So no test
 * asserts on "Accueil" alone: they assert on a sentence only that screen draws,
 * and they click the node that is clickable ([hasClickAction]) rather than the
 * first text match. For the home — the first screen a lot FILLED — the sentence
 * is now a stable marker instead: `HomeScreen.kt#HOME_MARKER`, a fixed
 * `contentDescription` on the screen's root, present in every state (hero,
 * skeletons, a failed load with no server) and independent of the clock (the
 * greeting above the hero names the weekday, so it can never be a needle).
 *
 * Runs on the emulator `mobile/tools/emu.sh` starts, through
 * `mobile/tools/ci-emulator.sh` (which discovers every module with an
 * `src/androidTest` and runs `connectedDebugAndroidTest` on it).
 */
@RunWith(AndroidJUnit4::class)
class ShellSmokeTest {

    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    /** M1 : une installation neuve n'a pas de serveur — elle ouvre l'écran serveur. */
    @Test
    fun freshInstallOpensServerSetup() {
        rule.onNodeWithText("Adresse du serveur").assertIsDisplayed()
        rule.onNode(hasText("Vérifier le serveur") and hasClickAction()).assertIsDisplayed()
    }

    /** La coquille reste utilisable depuis l'écran serveur (les onglets sont vivants). */
    @Test
    fun theHomeTabOpensFromTheSetupScreen() {
        rule.onNode(hasText("Accueil") and hasClickAction()).performClick()
        rule.onNodeWithContentDescription("Accueil Neezer").assertIsDisplayed()
    }

    /**
     * A tab opens its own destination. MB8 filled the library, so its stub's
     * sentence is gone and the test reads the screen's own marker
     * (`LibraryScreen.kt#LIBRARY_MARKER`, a fixed `contentDescription` present in
     * every state) — the same rule the home's test took when MB5 filled it.
     */
    @Test
    fun libraryTabOpensItsRoute() {
        // Le mot de la barre est « Biblio » depuis L1 (le label du SPA, 72 dp sur
        // 360 dp) ; le marqueur de l'écran garde son nom complet.
        rule.onNode(hasText("Biblio") and hasClickAction()).performClick()
        rule.onNodeWithContentDescription("Bibliothèque Neezer").assertIsDisplayed()
    }

    /** L'onglet Podcasts ouvre l'écran MB13, plus le placeholder MB0 (audit, bloqueur 2). */
    @Test
    fun podcastsTabOpensTheRealScreen() {
        rule.onNode(hasText("Podcasts") and hasClickAction()).performClick()
        rule.onNodeWithContentDescription("Podcasts Neezer").assertIsDisplayed()
    }

    /**
     * The whole chain in one assertion: Réglages › Aide prints the version the
     * RUST crate reports (`nz_version()`), read through JNA from
     * `libnz_ffi.so` — not a Kotlin constant. If the `.so` were missing from the
     * APK, or the JNA AAR absent, this is the line that fails.
     */
    @Test
    fun settingsShowsTheRustVersion() {
        // MB16 filled the screen: the skeleton's placeholder (« Choisis une
        // section. ») and its « Aide » row are gone, and Réglages is two levels
        // now — the menu, then a category. The proof is unchanged: the version
        // comes from `nz_version()`, read through JNA, never from a Kotlin
        // constant. *Assistance* is the last row, so the menu is scrolled to it.
        rule.onNode(hasText("Réglages") and hasClickAction()).performClick()
        rule.onNodeWithText("Chercher un réglage", substring = true).assertIsDisplayed()
        rule.onAllNodes(hasScrollAction()).onFirst().performScrollToNode(hasText("Assistance"))
        rule.onNode(hasText("Assistance") and hasClickAction()).performClick()
        rule.onNodeWithText("Noyau (Rust) :", substring = true).assertIsDisplayed()
    }

    /**
     * **SB3 : le lecteur plein écran couvre la barre et se cale sous l'inset.**
     *
     * Le lecteur était monté dans le contenu de la `Scaffold` : sa couche
     * s'arrêtait au haut de la barre (mini-lecteur 64 dp + onglets 64 dp + inset
     * bas), et son `statusBarsPadding()` comptait l'inset une seconde fois
     * puisque les `PaddingValues` de la `Scaffold` l'avaient déjà dépensé.
     *
     * Ici les insets sont les VRAIS (émulateur, job `emulator`), et les bornes
     * sont mesurées : `assertIsNotDisplayed` ne voit pas l'occlusion.
     */
    @Test
    fun thePlayerSitsUnderTheStatusBarAndCoversTheBar() {
        try {
            NowPlayingState.show()
            rule.waitForIdle()
            val surface = rule.onNodeWithContentDescription("Lecteur plein écran", substring = true)
                .getUnclippedBoundsInRoot()
            val root = rule.onRoot().getUnclippedBoundsInRoot()
            println(
                "MEASURE sb3 device: surface.bottom=${surface.bottom.value}dp " +
                    "root.bottom=${root.bottom.value}dp",
            )
            assertEquals(
                "le lecteur ne couvre pas la barre",
                root.bottom.value,
                surface.bottom.value,
                1f,
            )
            val inset = ViewCompat.getRootWindowInsets(rule.activity.window.decorView)!!
                .getInsets(WindowInsetsCompat.Type.statusBars()).top /
                rule.activity.resources.displayMetrics.density
            val collapse = rule.onNodeWithContentDescription("Réduire le lecteur")
                .getUnclippedBoundsInRoot()
            println("MEASURE sb3 device: collapse.top=${collapse.top.value}dp inset=${inset}dp")
            // Le nœud que lit ce test est la CIBLE de 48 dp de `NzIconButton`, pas
            // le glyphe de 24 : il commence AU padding de la rangée. Mesure JVM
            // (`PlayerInsetsTest`, inset 0) : 2 dp, hauteur 48 dp. L'en-tête est
            // donc à `inset + 2`, et `2 × inset + 4` sur le code d'avant — les
            // 12 dp de centrage du glyphe que la spec ajoutait ne sont pas dans
            // ces bornes.
            assertTrue(
                "l'en-tête n'est pas à 2 dp sous l'inset (${collapse.top.value} dp, inset $inset dp)",
                collapse.top.value in (inset + 1f)..(inset + 3f),
            )
        } finally {
            // L'état du lecteur est un objet du process : un test qui le laisse
            // ouvert ferait échouer les autres cas de cette classe.
            NowPlayingState.hide()
        }
    }
}
