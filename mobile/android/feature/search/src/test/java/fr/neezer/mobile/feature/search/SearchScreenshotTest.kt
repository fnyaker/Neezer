package fr.neezer.mobile.feature.search

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.TrackRow

/**
 * The search results, rendered and VERIFIED: a long highlight at 360 and 412 dp
 * (plan T3's matrix).
 *
 * T3 replaced MB6's write-only captures with these goldens. The state is the one
 * that breaks a result row: the typed word sits inside a 60-character title, so
 * the highlight, its ellipsis and the row's height are frozen in the image.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class SearchGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun longHighlightAt360() = shot(360, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h900dp-xhdpi")
    fun longHighlightAt412() = shot(412, 1.0f)

    private fun shot(width: Int, scale: Float) {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    SearchScreen(state = longHighlightState(), actions = SearchActions())
                }
            }
        }
        rule.capture("search-resultats-${width}dp-f$scale.png")
    }

    /** The typed word `sirènes` inside a title long enough to ellipsize. */
    private fun longHighlightState(): SearchModel.State {
        val base = SearchModel.State(
            query = "sirènes",
            answer = SearchFixtures.answer(term = "sirènes", trackCount = 3),
        )
        val answer = base.answer ?: return base
        val rows = listOf(
            row("Le chant des sirènes qui se brisent sur les rochers de la baie", 0),
            row("Sirènes, dernière partie : le retour des voix perdues", 1),
            row("Titre 2", 2),
        )
        return base.copy(answer = answer.copy(tracks = SearchFixtures.FakeRows(rows), trackCount = rows.size))
    }

    private fun row(title: String, position: Int) = TrackRow(
        key = "row:$position",
        id = "id-$position",
        title = title,
        artists = "Fauve, Ascenseur, June, Nina Simone",
        album = "Vieux frères, partie 2 (édition deluxe)",
        albumId = "501",
        durationS = (215 + position).toUInt(),
        cover = "/api/cover/501",
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = position.toUInt(),
    )
}
