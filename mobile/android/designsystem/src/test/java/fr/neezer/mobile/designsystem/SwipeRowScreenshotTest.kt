package fr.neezer.mobile.designsystem

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
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
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.TrackRow

/**
 * [`SwipeRow`] rendered at 360 dp: at rest, and **mid-swipe** (95 dp under the
 * finger), committed under `docs/agents/screenshots/mobile/G1/`.
 *
 * The three rows are the three bodies the lot replaced — the entity pages'
 * numbered row without a sleeve, the library row with its 44 dp cover, and the
 * playlist's wrapping title — so the pictures show what the one component does
 * in each list. The renderer is Robolectric's NATIVE graphics backend (Skia on
 * the host), the repository's own recipe (worker rule 7: no window, no
 * emulator). The real screens' at-rest renders stay the MB7/MB8/MB9 screenshot
 * suites, which this lot re-runs unchanged.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h640dp-xhdpi")
class SwipeRowScreenshotTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val outputDir: File
        get() = File(
            System.getProperty("nz.repoRoot")
                ?: error("nz.repoRoot is not set: the screenshot test writes the lot's images"),
            "docs/agents/screenshots/mobile/G1",
        )

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
    fun atRest() = shot("g1-rangees-repos.png", midSwipe = false)

    @Test
    fun midSwipe() = shot("g1-rangees-glisse.png", midSwipe = true)

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
            rule.onNodeWithText("Nuit blanche").performTouchInput {
                down(center)
                advanceEventTime(16)
                moveBy(Offset(95.dp.toPx(), 0f))
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        outputDir.mkdirs()
        val file = File(outputDir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
        if (midSwipe) rule.onNodeWithText("Nuit blanche").performTouchInput { cancel() }
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
