package fr.neezer.mobile.feature.library

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Les deux portes de la Bibliothèque — lot M9 (`routes/Library.svelte:531`, `:545`,
 * `:611`).
 *
 * The screen is the REAL [LibraryScreen] over [LibraryFixtures], the same records
 * the MB8 screenshots draw, so what is asserted is what a person taps: the head's
 * link icon, the « + » menu, and the name sheet behind it. No network, no `.so`,
 * no device: the doors are pure callbacks here, and the model's own consequences
 * (the creation, the reload, the open) are `LibraryModelTest`'s.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class LibraryPlaylistDoorTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** The « Playlists » leaf, loaded: the door belongs to every tab's head. */
    private fun playlistsState() = LibraryModel.State(
        leaf = LibraryLeaf.Playlists,
        tree = ScreenState.Ready(LibraryFixtures.tree()),
        foldersSupported = true,
    )

    /** The head's link icon opens « Importer une playlist partagée » (the SPA's
     * `<a href="#/import">`, `Library.svelte:611`). */
    @Test
    fun theHeaderOffersTheImportDoor() {
        var opened = 0
        rule.setContent {
            NzTheme {
                LibraryScreen(
                    state = playlistsState(),
                    actions = LibraryActions(openImport = { opened++ }),
                )
            }
        }

        rule.onNodeWithContentDescription("Importer une playlist partagée").performClick()

        assertEquals("le lien de l'en-tête ouvre l'import", 1, opened)
    }

    /**
     * « + » → « Nouvelle playlist » → un nom → « Créer » : the sheet asks the name
     * the SPA's `window.prompt` asks, and the action receives it trimmed by the
     * model.
     */
    @Test
    fun theAddSheetCreatesAPlaylist() {
        val created = mutableListOf<String>()
        rule.setContent {
            NzTheme {
                LibraryScreen(
                    state = playlistsState(),
                    actions = LibraryActions(newPlaylist = { created += it }),
                )
            }
        }

        rule.onNodeWithContentDescription("Ajouter à la bibliothèque").performClick()
        rule.onNode(hasText("Nouvelle playlist") and hasClickAction()).performClick()

        rule.onNodeWithContentDescription("Nom de la playlist").performClick()
        rule.onNodeWithContentDescription("Nom de la playlist").performTextInput("Test")
        rule.onNode(hasText("Créer") and hasClickAction()).performClick()

        assertEquals(listOf("Test"), created)
    }
}
