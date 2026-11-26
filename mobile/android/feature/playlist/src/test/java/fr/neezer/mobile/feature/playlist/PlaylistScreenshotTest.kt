package fr.neezer.mobile.feature.playlist

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

/**
 * The playlist, rendered and VERIFIED: twenty rows at 360 dp (plan T3's matrix).
 *
 * T3 replaced MB9's write-only captures with this golden: the row is the unit
 * the owner's playlist page is made of, and a title that grows or a duration
 * that drifts is a regression the comparison catches at 1 dp. Robolectric draws
 * the real [PlaylistScreen] over [PlaylistFixtures]'s rows (worker rule 7).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi")
class PlaylistGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun twentyRowsAt360() {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    PlaylistScreen(
                        state = twentyRows(),
                        actions = PlaylistActions(),
                        wide = false,
                    )
                }
            }
        }
        rule.capture("playlist-liste-20-360dp-f1.0.png")
    }

    /** Twenty rows, editable, with the owner's name in the band. */
    private fun twentyRows(): PlaylistModel.State {
        val page = PlaylistFixtures.page(
            rows = FakeRows(PlaylistFixtures.tracks(20)),
            members = listOf(
                PlaylistFixtures.owner(),
                uniffi.nz_ffi.PlaylistMember("u2", "camille", uniffi.nz_ffi.PlaylistRole.EDITOR, null),
            ),
        )
        return PlaylistModel.State(
            id = "pl-1",
            page = page,
            rows = page.rows,
            recs = RecsUi.Ready(listOf(PlaylistFixtures.track(9_000), PlaylistFixtures.track(9_001))),
        )
    }
}
