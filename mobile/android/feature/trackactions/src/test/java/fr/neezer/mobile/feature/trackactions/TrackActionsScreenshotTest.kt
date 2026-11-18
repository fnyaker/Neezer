package fr.neezer.mobile.feature.trackactions

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.TrackRowView
import java.io.File
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The track actions, rendered: the menu (and one of its accordions), the playlist
 * picker (its copy, its failure), and the replacement sheet, in Encre and Papier
 * at 360 dp and 600 dp plus the 200 % font scale the acceptance asks about —
 * committed under `docs/agents/screenshots/mobile/MB10/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The same recipe
 * MB1 measured and MB5/MB7 use.
 *
 * **A sheet is drawn over a LIST**, because that is what a person sees: the
 * captures of the menu sit on rows, so the veil, the sheet's material and the
 * rows behind it can be judged together (and the crop keeps the page, never an
 * empty display).
 */
abstract class TrackActionsScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    private fun capture(
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        ListBehind()
                        content()
                    }
                }
            }
        }
        // A sheet's veil fades in and its pane slides: the IMAGE must be the
        // settled state, so the test advances the main clock past both tokens
        // (an EVENT, not a sleep) before it reads a pixel.
        rule.mainClock.advanceTimeBy(1_000)
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    /** The ground under a sheet: rows, so the veil has something to veil. */
    @Composable
    private fun ListBehind() {
        Column(modifier = Modifier.fillMaxSize()) {
            repeat(12) { index ->
                TrackRowView(
                    row = TrackActionsFixtures.row(
                        title = listOf(
                            "Ouverture",
                            "Suite",
                            "Le matin se lève",
                            "Sinnerman",
                            "Wild Is the Wind",
                            "Feeling Good",
                        )[index % 6] + if (index >= 6) " (live)" else "",
                        key = "deezer:${9000 + index}",
                        id = "${9000 + index}",
                        liked = index == 2,
                        position = index,
                    ),
                    playing = index == 0,
                    onClick = {},
                    onMore = {},
                    onSwipeQueue = {},
                )
            }
        }
    }

    /** The window is taller than the page: crop the ground away (MB5's recipe). */
    private fun cropToContent(full: Bitmap, theme: NzThemeKind): Bitmap {
        val background = Sillon.theme(theme).bg.toArgb()
        var last = full.height - 1
        val row = IntArray(full.width)
        while (last > 0) {
            full.getPixels(row, 0, full.width, 0, last, full.width, 1)
            if (row.any { it != background }) break
            last--
        }
        val height = (last + 24).coerceIn(1, full.height)
        if (height <= 24) return full
        return Bitmap.createBitmap(full, 0, 0, full.width, height)
    }

    private fun suffix(fontScale: Float) = if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else ""

    private fun name(theme: NzThemeKind, screen: String, fontScale: Float = 1f) =
        "mb10-%s-%ddp-%s%s.png".format(theme.name.lowercase(), width, screen, suffix(fontScale))

    private fun menuState(liked: Boolean = false, expanded: String? = null) = TrackActionsModel.State(
        row = TrackActionsFixtures.row(liked = liked),
        entries = TrackActionsFixtures.entries(liked = liked),
        liked = liked,
        expanded = expanded,
    )

    private fun episodeState() = TrackActionsModel.State(
        row = TrackActionsFixtures.row(title = "Épisode 12 · Les histoires du banc"),
        entries = TrackActionsFixtures.episodeEntries(),
    )

    private fun pickerState(): TrackActionsModel.State {
        val lists = listOf(
            TrackActionsFixtures.choice(recent = true),
            TrackActionsFixtures.choice(
                id = TrackActionsFixtures.OTHER_PLAYLIST_ID,
                title = "Le matin",
                nbTracks = 1,
            ),
            TrackActionsFixtures.choice(
                id = "33333333-3333-4333-8333-333333333333",
                title = "Chez June",
                editable = false,
                role = "viewer",
                owner = "June",
                nbTracks = 9,
            ),
            TrackActionsFixtures.choice(id = "44444444-4444-4444-8444-444444444444", title = "Route de nuit"),
            TrackActionsFixtures.choice(id = "55555555-5555-4555-8555-555555555555", title = "Vélo"),
        )
        return TrackActionsModel.State(
            row = TrackActionsFixtures.row(),
            sheet = TrackActionsModel.Sheet.Picker,
            picker = TrackActionsModel.Picker(
                lists = lists,
                added = setOf(TrackActionsFixtures.OTHER_PLAYLIST_ID),
            ),
        )
    }

    private fun pickerOfflineState(): TrackActionsModel.State = pickerState().let { state ->
        state.copy(
            picker = state.picker?.copy(
                lists = state.picker.lists?.take(2),
                stale = true,
            ),
        )
    }

    private fun pickerFailureState(): TrackActionsModel.State = TrackActionsModel.State(
        row = TrackActionsFixtures.row(),
        sheet = TrackActionsModel.Sheet.Picker,
        picker = TrackActionsModel.Picker(error = "Hors ligne"),
    )

    private fun replaceState(): TrackActionsModel.State = TrackActionsModel.State(
        row = TrackActionsFixtures.row(),
        sheet = TrackActionsModel.Sheet.Replace,
        replace = TrackActionsModel.Replace(
            loading = false,
            query = "Ascenseur Ouverture",
            searched = "Ascenseur Ouverture",
            candidates = listOf(
                TrackActionsFixtures.candidate(),
                TrackActionsFixtures.candidate(
                    key = "youtube:abc",
                    id = "abc",
                    title = "Ouverture (live)",
                    source = "youtube",
                    local = false,
                ),
                TrackActionsFixtures.candidate(
                    key = "deezer:9001b",
                    id = "9001b",
                    title = "Ouverture (remasterisée)",
                    source = "deezer",
                    local = false,
                ),
            ),
        ),
    )

    private fun replaceEmptyState(): TrackActionsModel.State = TrackActionsModel.State(
        row = TrackActionsFixtures.row(),
        sheet = TrackActionsModel.Sheet.Replace,
        replace = TrackActionsModel.Replace(
            loading = false,
            query = "Ascenseur Ouverture",
            searched = "Ascenseur Ouverture",
        ),
    )

    private fun replaceFailureState(): TrackActionsModel.State = TrackActionsModel.State(
        row = TrackActionsFixtures.row(),
        sheet = TrackActionsModel.Sheet.Replace,
        replace = TrackActionsModel.Replace(
            loading = false,
            error = "Hors ligne",
            searched = "",
        ),
    )

    private fun menu(state: TrackActionsModel.State, theme: NzThemeKind) = capture(
        name(theme, "menu"),
        theme,
    ) {
        TrackMenuSheet(state = state, onRun = {}, onToggleSub = {}, onDismiss = {})
    }

    @Test
    fun menuEncre() = menu(menuState(), NzThemeKind.Encre)

    @Test
    fun menuPapier() = menu(menuState(liked = true), NzThemeKind.Papier)

    @Test
    fun menuWithASubmenu() = capture(name(NzThemeKind.Encre, "menu-sous-menu"), NzThemeKind.Encre) {
        TrackMenuSheet(
            state = menuState(expanded = "Aller à l'artiste"),
            onRun = {},
            onToggleSub = {},
            onDismiss = {},
        )
    }

    @Test
    fun episodeMenu() = capture(name(NzThemeKind.Papier, "menu-episode"), NzThemeKind.Papier) {
        TrackMenuSheet(state = episodeState(), onRun = {}, onToggleSub = {}, onDismiss = {})
    }

    @Test
    fun menuAtTwoHundredPercent() = capture(
        name(NzThemeKind.Encre, "menu", fontScale = 2f),
        NzThemeKind.Encre,
        fontScale = 2f,
    ) {
        TrackMenuSheet(state = menuState(), onRun = {}, onToggleSub = {}, onDismiss = {})
    }

    @Test
    fun pickerEncre() = capture(name(NzThemeKind.Encre, "picker"), NzThemeKind.Encre) {
        PlaylistPickerSheet(
            state = pickerState(),
            onFilter = {},
            onPick = {},
            onCreate = {},
            onStartCreating = {},
            onCancelCreating = {},
            onName = {},
            onRetry = {},
            onDismiss = {},
        )
    }

    @Test
    fun pickerPapierOffline() = capture(name(NzThemeKind.Papier, "picker-hors-ligne"), NzThemeKind.Papier) {
        PlaylistPickerSheet(
            state = pickerOfflineState(),
            onFilter = {},
            onPick = {},
            onCreate = {},
            onStartCreating = {},
            onCancelCreating = {},
            onName = {},
            onRetry = {},
            onDismiss = {},
        )
    }

    @Test
    fun pickerFailure() = capture(name(NzThemeKind.Encre, "picker-echec"), NzThemeKind.Encre) {
        PlaylistPickerSheet(
            state = pickerFailureState(),
            onFilter = {},
            onPick = {},
            onCreate = {},
            onStartCreating = {},
            onCancelCreating = {},
            onName = {},
            onRetry = {},
            onDismiss = {},
        )
    }

    @Test
    fun pickerAtTwoHundredPercent() = capture(
        name(NzThemeKind.Papier, "picker", fontScale = 2f),
        NzThemeKind.Papier,
        fontScale = 2f,
    ) {
        PlaylistPickerSheet(
            state = pickerState(),
            onFilter = {},
            onPick = {},
            onCreate = {},
            onStartCreating = {},
            onCancelCreating = {},
            onName = {},
            onRetry = {},
            onDismiss = {},
        )
    }

    @Test
    fun replaceEncre() = capture(name(NzThemeKind.Encre, "remplacer"), NzThemeKind.Encre) {
        ReplaceSheet(
            state = replaceState(),
            onSearch = {},
            onFallbacks = {},
            onCommit = {},
            onDelete = {},
            onRetry = {},
            onDismiss = {},
        )
    }

    @Test
    fun replacePapier() = capture(name(NzThemeKind.Papier, "remplacer-vide"), NzThemeKind.Papier) {
        ReplaceSheet(
            state = replaceEmptyState(),
            onSearch = {},
            onFallbacks = {},
            onCommit = {},
            onDelete = {},
            onRetry = {},
            onDismiss = {},
        )
    }

    @Test
    fun replaceFailure() = capture(name(NzThemeKind.Encre, "remplacer-echec"), NzThemeKind.Encre) {
        ReplaceSheet(
            state = replaceFailureState(),
            onSearch = {},
            onFallbacks = {},
            onCommit = {},
            onDelete = {},
            onRetry = {},
            onDismiss = {},
        )
    }

    @Test
    fun replaceAtTwoHundredPercent() = capture(
        name(NzThemeKind.Encre, "remplacer", fontScale = 2f),
        NzThemeKind.Encre,
        fontScale = 2f,
    ) {
        ReplaceSheet(
            state = replaceState(),
            onSearch = {},
            onFallbacks = {},
            onCommit = {},
            onDelete = {},
            onRetry = {},
            onDismiss = {},
        )
    }
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class TrackActionsScreenshotPhoneTest : TrackActionsScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h800dp-xhdpi")
class TrackActionsScreenshotTabletTest : TrackActionsScreenshotBase(width = 600)
