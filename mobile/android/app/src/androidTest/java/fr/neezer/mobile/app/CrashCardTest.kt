package fr.neezer.mobile.app

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.bridge.CrashRecord
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * **Le chemin de LECTURE du dernier incident** (M14), sans tuer le processus : un
 * plantage réel ne se provoque pas depuis un test instrumenté (Android arrête
 * l'app, et le test avec elle), alors le fichier est écrit par la MÊME fonction
 * que `CrashLog` appelle — `CrashRecord.write` — puis Réglages → Assistance est
 * ouvert comme une personne l'ouvre.
 *
 * Ce que le test prouve, dans l'ordre où la personne le vit :
 *
 * 1. la carte « Dernier incident » apparaît avec la trace écrite ;
 * 2. « Effacer » la retire de l'écran ;
 * 3. en rouvrant Assistance, elle n'est pas revenue (le fichier est vide, pas
 *    seulement l'état Compose) — c'est la différence entre un état d'écran et ce
 *    qui survit à une navigation.
 *
 * Nettoyage dans `@After` : un test qui échoue au milieu ne doit pas laisser un
 * faux incident dans l'app de l'émulateur pour les tests suivants.
 */
@RunWith(AndroidJUnit4::class)
class CrashCardTest {

    @get:Rule
    val rule = createAndroidComposeRule<MainActivity>()

    private val context
        get() = InstrumentationRegistry.getInstrumentation().targetContext

    @After
    fun cleanUp() {
        CrashRecord.clear(context)
    }

    /** Réglages › Assistance, le chemin de `ShellSmokeTest` (le menu se déroule). */
    private fun openAssistance() {
        rule.onNode(hasText("Réglages") and hasClickAction()).performClick()
        rule.onNodeWithText("Chercher un réglage", substring = true).assertIsDisplayed()
        rule.onAllNodes(hasScrollAction()).onFirst().performScrollToNode(hasText("Assistance"))
        rule.onNode(hasText("Assistance") and hasClickAction()).performClick()
    }

    @Test
    fun theCardShowsTheLastCrashAndClearRemovesItForGood() {
        CrashRecord.write(
            context = context,
            thread = "main",
            stack = "java.lang.IllegalStateException: banc\n\tat fr.neezer.mobile.Foo.bar(Foo.kt:12)",
        )

        openAssistance()

        // `performScrollTo` avant chaque assertion : la carte est en bas de la
        // page « Assistance », sous « Versions » et « Mises à jour », et un
        // `assertIsDisplayed` sans défilement dépend de la hauteur de l'écran.
        rule.onNodeWithText("Dernier incident").performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("java.lang.IllegalStateException: banc", substring = true)
            .assertExists()
        rule.onNode(hasText("Partager l'incident") and hasClickAction())
            .performScrollTo()
            .assertIsDisplayed()

        rule.onNode(hasText("Effacer") and hasClickAction()).performScrollTo().performClick()

        rule.onNodeWithText("Dernier incident").assertDoesNotExist()

        // Rouvrir la section relit le FICHIER : si `clear` n'avait pas supprimé
        // l'entrée, la carte reviendrait ici et le test le dirait.
        rule.onNodeWithContentDescription("Retour aux réglages").performClick()
        rule.onAllNodes(hasScrollAction()).onFirst().performScrollToNode(hasText("Assistance"))
        rule.onNode(hasText("Assistance") and hasClickAction()).performClick()

        rule.onNodeWithText("Dernier incident").assertDoesNotExist()
    }
}
