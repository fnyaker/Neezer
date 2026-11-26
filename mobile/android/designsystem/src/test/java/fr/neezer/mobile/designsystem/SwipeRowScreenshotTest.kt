package fr.neezer.mobile.designsystem

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.TrackRow

/**
 * [`SwipeRow`] rendered and VERIFIED (G1's own captures, converted by T3): at
 * rest and **mid-swipe** (95 dp under the finger), 360 dp, Encre.
 *
 * The three rows are the three bodies the lot replaced — the entity pages'
 * numbered row without a sleeve, the library row with its 44 dp cover, and the
 * playlist's wrapping title — so the pictures show what the one component does
 * in each list. Robolectric's NATIVE graphics backend draws them on the host: no
 * window, no emulator (worker rule 7). The gesture's behaviour is
 * [`SwipeRowTest`]'s five counted cases; this file is its picture.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h640dp-xhdpi")
class SwipeRowGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    private fun track(title: String, artists: String) = TrackRow(
        key = title,
        id = title,
        title = title,
        artists = artists,
        album = "OutRun",
        albumId = null,
        durationS = 244u,
        cover = null,
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = 3u,
    )

    @Test
    fun atRest() = shot("designsystem-glisse-repos-360dp-f1.0.png", midSwipe = false)

    @Test
    fun midSwipe() = shot("designsystem-glisse-migeste-360dp-f1.0.png", midSwipe = true)

    private fun shot(name: String, midSwipe: Boolean) {
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Column(Modifier.fillMaxWidth()) {
                        NumberedRow(track("Ouverture", "Kavinsky, Phoenix"))
                        LibraryRow(track("Nuit blanche", "Kavinsky, Phoenix"))
                        PlaylistRow(track("La femme d'argent", "Air"))
                    }
                }
            }
        }
        if (midSwipe) {
            // The pointer stays down: the picture is the gesture, not its end.
            rule.composeRule.onNodeWithText("Nuit blanche").performTouchInput {
                down(center)
                advanceEventTime(16)
                moveBy(Offset(95.dp.toPx(), 0f))
            }
        }
        rule.capture(name)
        if (midSwipe) rule.composeRule.onNodeWithText("Nuit blanche").performTouchInput { cancel() }
    }

    @Composable
    private fun NumberedRow(row: TrackRow) {
        SwipeRow(onQueue = {}) { rowModifier ->
            Row(
                modifier = rowModifier
                    .heightIn(min = 56.dp)
                    .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                Box(Modifier.width(28.dp), contentAlignment = Alignment.Center) {
                    Text(row.position.toString(), style = Sillon.type.mono, color = Sillon.colors.dim)
                }
                TitleColumn(row, maxLines = 1)
                Duration(row)
                NzIconButton(NzGlyph.MoreVertical, "Plus d'actions pour ${row.title}", onClick = {})
            }
        }
    }

    @Composable
    private fun LibraryRow(row: TrackRow) {
        SwipeRow(onQueue = {}) { rowModifier ->
            Row(
                modifier = rowModifier
                    .heightIn(min = 56.dp)
                    .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                CoverImage(url = null, sizeDp = 44.dp, seed = row.id)
                TitleColumn(row, maxLines = 1)
                Duration(row)
                NzIconButton(NzGlyph.MoreVertical, "Plus d'actions pour ${row.title}", onClick = {})
            }
        }
    }

    @Composable
    private fun PlaylistRow(row: TrackRow) {
        SwipeRow(onQueue = {}) { rowModifier ->
            Row(
                modifier = rowModifier
                    .heightIn(min = 56.dp)
                    .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
            ) {
                CoverImage(url = null, sizeDp = 44.dp, seed = row.id)
                TitleColumn(row, maxLines = 2)
                Duration(row)
                NzIconButton(NzGlyph.MoreVertical, "Plus d'actions pour ${row.title}", onClick = {})
            }
        }
    }

    @Composable
    private fun RowScope.TitleColumn(row: TrackRow, maxLines: Int) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(NzSpacing.xxs),
        ) {
            Text(
                text = row.title,
                style = Sillon.type.body,
                color = Sillon.colors.tx,
                maxLines = maxLines,
            )
            Text(
                text = row.artists,
                style = Sillon.type.bodySmall,
                color = Sillon.colors.dim,
                maxLines = 1,
            )
        }
    }

    @Composable
    private fun Duration(row: TrackRow) {
        Text(
            text = nzDuration(row.durationS.toInt()),
            style = Sillon.type.mono,
            color = Sillon.colors.dim,
        )
    }
}
