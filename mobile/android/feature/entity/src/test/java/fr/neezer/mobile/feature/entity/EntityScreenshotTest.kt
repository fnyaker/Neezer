package fr.neezer.mobile.feature.entity

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performScrollToNode
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Artiste, rendered and VERIFIED: **twelve albums in the grid with long
 * French names**, at 360, 412 and 600 dp and at the 1.0 and 1.3 font scales
 * (plan T3's golden matrix).
 *
 * T3 replaced MB7's write-only captures with goldens: the artist grid is one of
 * the three reported misalignments, so besides the picture this class asserts
 * the geometry a picture alone cannot state — the first row's three tiles have
 * equal widths and equal tops within 1 px, and the last tile's right edge stays
 * inside the page's 20 dp padding (no overflow). S2's band/tab/list-tail
 * assertions stay below; T3 does not duplicate them.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h1200dp-xhdpi")
class EntityGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun artistGridAt360() = shot(360, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w360dp-h1200dp-xhdpi", fontScale = 1.3f)
    fun artistGridAt360At130Percent() = shot(360, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h1200dp-xhdpi")
    fun artistGridAt412() = shot(412, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h1200dp-xhdpi", fontScale = 1.3f)
    fun artistGridAt412At130Percent() = shot(412, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h1200dp-xhdpi")
    fun artistGridAt600() = shot(600, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h1200dp-xhdpi", fontScale = 1.3f)
    fun artistGridAt600At130Percent() = shot(600, 1.3f)

    private fun shot(width: Int, scale: Float) {
        render(artistGridState(), scale)
        scrollToAlbums()
        rule.capture("entity-artiste-grille-${width}dp-f$scale.png")
    }

    private fun render(
        state: ArtistModel.State,
        fontScale: Float = 1f,
        content: @Composable () -> Unit = { ArtistScreen(state = state, actions = ArtistActions()) },
    ) {
        rule.setContent(fontScale) {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
            }
        }
    }

    private fun scrollToAlbums() {
        rule.composeRule.onNodeWithTag(ARTIST_LIST_TAG).performScrollToNode(hasText("Albums"))
        rule.composeRule.waitForIdle()
    }

    /** Twelve albums whose long names are the state the grid has to survive. */
    private fun twelveAlbums(): List<uniffi.nz_ffi.AlbumCardData> = (0 until 12).map { index ->
        EntityFixtures.albumCard(
            id = "alb-$index",
            title = "Les nuits blanches d'un orchestre qui refuse de dormir $index",
            year = "202${index % 6}",
        )
    }

    private fun artistGridState(): ArtistModel.State = ArtistModel.State(
        id = EntityFixtures.ARTIST_DEEZER,
        screen = ScreenState.Ready(EntityFixtures.artist()),
        tab = ArtistTab.Overview,
        discography = EntityFixtures.discography().copy(
            latest = emptyList(),
            albums = twelveAlbums(),
            singles = emptyList(),
            eps = emptyList(),
            featured = emptyList(),
            onDeezer = emptyList(),
        ),
        onDeezer = emptyList(),
        onDeezerFetched = true,
        tracks = ScreenState.Ready(FakeRows((0 until 40).map { EntityFixtures.row("Titre $it", it) })),
    )

    /**
     * T3's first reported misalignment, as numbers: the first row's three tiles
     * are equal, aligned and inside the page.
     */
    @Test
    fun theFirstGridRowIsEqualAlignedAndInsideThePage() {
        render(artistGridState())
        scrollToAlbums()
        val cards = twelveAlbums().take(3)
        val boxes = cards.map { card ->
            val description = "${card.title}, ${albumSubtitle(card)}"
            rule.composeRule.onNodeWithContentDescription(description).getUnclippedBoundsInRoot()
        }
        // `getUnclippedBoundsInRoot()` answers Dp, not px: the numbers are the
        // page's own units.
        val widths = boxes.map { (it.right - it.left).value }
        val tops = boxes.map { it.top.value }
        println("MEASURE t3 grille artiste: widths=$widths tops=$tops")
        widths.zipWithNext { a, b ->
            assertEquals("two tiles of the first row have the same width", a, b, 1f)
        }
        tops.zipWithNext { a, b ->
            assertEquals("two tiles of the first row share a top", a, b, 1f)
        }
        val screenDp = rule.composeRule.onRoot().getUnclippedBoundsInRoot().right.value
        val lastRight = boxes.maxOf { it.right.value }
        assertTrue(
            "the last tile stays inside the page's 20 dp padding " +
                "(right=$lastRight dp screen=$screenDp dp)",
            lastRight <= screenDp - 20f + 0.5f,
        )
    }

    /**
     * S2 : la bande annule la marge haute de la page (elle commence à 0), les
     * onglets sont à 20 dp sous elle, et la liste s'arrête 40 dp avant le bas.
     */
    @Test
    fun theArtistBandAndTheListFollowThePage() {
        render(artistTracksState())
        rule.composeRule.waitForIdle()
        val band = rule.composeRule.onNodeWithTag(ARTIST_BAND_TAG).getUnclippedBoundsInRoot()
        val tab = rule.composeRule.onNodeWithText("Aperçu").getUnclippedBoundsInRoot()
        println(
            "MEASURE s2 artiste: band.top=${band.top.value}dp tab.top=${tab.top.value}dp " +
                "band.bottom=${band.bottom.value}dp",
        )
        assertEquals("la bande commence au bord haut", 0f, band.top.value, 1f)
        assertEquals("les onglets sont à 20 dp sous la bande", 20f, tab.top.value - band.bottom.value, 1f)

        rule.composeRule.onNodeWithTag(ARTIST_LIST_TAG).performScrollToNode(hasText("Titre 39"))
        rule.composeRule.waitForIdle()
        val last = rule.composeRule
            .onNode(hasText("Titre 39") and hasClickAction())
            .getUnclippedBoundsInRoot()
        val root = rule.composeRule.onRoot().getUnclippedBoundsInRoot()
        println(
            "MEASURE s2 artiste tail: last.bottom=${last.bottom.value}dp " +
                "root.bottom=${root.bottom.value}dp",
        )
        assertEquals("la liste s'arrête 40 dp avant le bas", 40f, root.bottom.value - last.bottom.value, 1f)
    }

    private fun artistTracksState() = artistGridState().copy(tab = ArtistTab.Tracks)
}
