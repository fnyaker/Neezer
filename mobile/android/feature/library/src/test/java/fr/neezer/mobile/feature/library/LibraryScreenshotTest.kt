package fr.neezer.mobile.feature.library

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performScrollToNode
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The library list, rendered and VERIFIED: a 60-character title and a long
 * artist line, at 360 and 412 dp and at the 1.0 and 1.3 font scales (plan T3's
 * matrix).
 *
 * T3 replaced MB8's write-only captures with these goldens: the long title is
 * the state a golden has to freeze, because that is where a row clips. S2's own
 * page-margin and list-tail assertions stay below — T3 adds the pixels, it does
 * not duplicate S2's geometry.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class LibraryGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun longTitlesAt360() = shot(360, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi", fontScale = 1.3f)
    fun longTitlesAt360At130Percent() = shot(360, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h900dp-xhdpi")
    fun longTitlesAt412() = shot(412, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h900dp-xhdpi", fontScale = 1.3f)
    fun longTitlesAt412At130Percent() = shot(412, 1.3f)

    private fun shot(width: Int, scale: Float) {
        render(longTitlesState(), scale)
        rule.capture("library-bibliotheque-liste-${width}dp-f$scale.png")
    }

    private fun render(
        state: LibraryModel.State,
        fontScale: Float = 1f,
        content: @Composable () -> Unit = { LibraryScreen(state = state, actions = LibraryActions()) },
    ) {
        rule.setContent(fontScale) {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
            }
        }
    }

    /**
     * The matrix's state: twelve rows, the first with a 60-character title and a
     * four-name artist line — the worst case an 8 000-row library puts in a row.
     */
    private fun longTitlesState(): LibraryModel.State {
        val rows = listOf(
            LibraryFixtures.row(
                title = "L'homme qui marchait sur les toits de la ville endormie avant l'aube",
                position = 0,
                artists = "Ascenseur, June, Nina Simone, Orchestre de la nuit",
            ),
        ) + (1 until 12).map { index ->
            LibraryFixtures.row(
                title = if (index % 2 == 0) "Ouverture $index" else "Sinnerman $index",
                position = index,
                artists = "Ascenseur",
            )
        }
        return LibraryModel.State(
            leaf = LibraryLeaf.Favorites,
            favorites = ScreenState.Ready(FakeRows(rows)),
            artists = ScreenState.Ready(LibraryFixtures.artists()),
            albums = ScreenState.Ready(LibraryFixtures.albums()),
            tree = ScreenState.Ready(LibraryFixtures.tree()),
            local = ScreenState.Ready(FakeRows(rows.take(3))),
            unavailable = ScreenState.Ready(FakeRows(emptyList())),
            dislikes = ScreenState.Ready(FakeRows(emptyList())),
            usage = LibraryFixtures.usage(),
            foldersSupported = true,
        )
    }

    /**
     * S2 : la Bibliothèque commence à 20 dp et sa liste s'arrête 40 dp avant le
     * bas (la coquille fournit l'inset ; la liste n'ajoute plus 128 dp de vide).
     */
    @Test
    fun thePageMarginAndTheListTail() {
        render(favoritesState(count = 60))
        rule.composeRule.waitForIdle()
        val head = rule.composeRule.onNodeWithTag(LIBRARY_HEAD_TAG).getUnclippedBoundsInRoot()
        println("MEASURE s2 bibliotheque: head.top=${head.top.value}dp")
        assertEquals("la page commence à 20 dp", 20f, head.top.value, 1f)

        rule.composeRule.onNodeWithTag(LIBRARY_LIST_TAG).performScrollToNode(hasText("Sinnerman 59"))
        rule.composeRule.waitForIdle()
        val last = rule.composeRule
            .onNode(hasText("Sinnerman 59") and hasClickAction())
            .getUnclippedBoundsInRoot()
        val root = rule.composeRule.onRoot().getUnclippedBoundsInRoot()
        println(
            "MEASURE s2 bibliotheque tail: last.bottom=${last.bottom.value}dp " +
                "root.bottom=${root.bottom.value}dp",
        )
        assertEquals(
            "la liste s'arrête 40 dp au-dessus du bas",
            40f,
            root.bottom.value - last.bottom.value,
            1f,
        )
    }

    private fun favoritesState(count: Int = 12) = LibraryModel.State(
        leaf = LibraryLeaf.Favorites,
        favorites = ScreenState.Ready(FakeRows(LibraryFixtures.rows(count))),
        artists = ScreenState.Ready(LibraryFixtures.artists()),
        albums = ScreenState.Ready(LibraryFixtures.albums()),
        tree = ScreenState.Ready(LibraryFixtures.tree()),
        local = ScreenState.Ready(FakeRows(LibraryFixtures.rows(3))),
        unavailable = ScreenState.Ready(FakeRows(emptyList())),
        dislikes = ScreenState.Ready(FakeRows(listOf(LibraryFixtures.row("Trop fort", 0, "Bruit")))),
        usage = LibraryFixtures.usage(),
        foldersSupported = true,
    )
}
