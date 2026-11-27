package fr.neezer.mobile.feature.library

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Le tri des favoris, tel qu'une personne le voit et le touche : UN contrôle qui
 * nomme l'ordre courant, une feuille de six lignes plus l'interrupteur de
 * direction.
 *
 * La même règle que la porte de la soirée (MB20) : ce qui se peint dans une case
 * de 48 dp est mesuré là où la personne le reçoit. Ici, la porte remplace sept
 * chips qui débordaient à 360 dp : avant tout tap, « Date d'ajout » n'existe
 * nulle part ; après, la feuille le montre et le choix remonte à l'action.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class LibrarySortSheetTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun favoritesState() = LibraryModel.State(
        leaf = LibraryLeaf.Favorites,
        favorites = ScreenState.Ready(FakeRows(LibraryFixtures.rows(40))),
    )

    @Test
    fun theDoorNamesTheOrderAndOpensTheSheet() {
        val picked = mutableListOf<FavoriteSort>()
        rule.setContent {
            NzTheme {
                LibraryScreen(
                    state = favoritesState(),
                    actions = LibraryActions(sortFavorites = { picked += it }),
                )
            }
        }

        val door = "Trier les favoris — Ordre d'origine, croissant"
        rule.onNodeWithContentDescription(door).assertIsDisplayed()
        rule.onNodeWithText("Date d'ajout").assertDoesNotExist()

        rule.onNodeWithContentDescription(door).performClick()

        rule.onNodeWithText("Trier les favoris").assertIsDisplayed()
        listOf("Ordre d'origine", "Titre", "Artiste", "Album", "Durée", "Date d'ajout")
            .forEach { label -> rule.onNodeWithText(label).assertIsDisplayed() }

        rule.onNode(hasText("Date d'ajout") and hasClickAction()).performClick()

        assertEquals(listOf(FavoriteSort.Added), picked)
        rule.onNodeWithText("Trier les favoris").assertDoesNotExist()
    }
}
