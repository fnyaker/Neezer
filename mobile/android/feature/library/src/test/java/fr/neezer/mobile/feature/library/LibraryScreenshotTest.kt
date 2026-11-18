package fr.neezer.mobile.feature.library

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import java.time.LocalDate
import java.time.ZoneId
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The library and the history, rendered: Encre and Papier at 360 dp (the phone)
 * and 600 dp (the tablet's reading column), the sheets, the failure card and the
 * 200 % font scale the acceptance asks about, committed under
 * `docs/agents/screenshots/mobile/MB8/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The same recipe
 * MB1 measured and MB5/MB7 used.
 *
 * What is drawn is the REAL screen over the REAL records: a state built by hand
 * (a render asks no network) with the same `LibraryModel.State` the model
 * produces.
 */
abstract class LibraryScreenshotBase(private val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    private fun capture(
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        /** A sheet is aligned to the BOTTOM of its frame: a phone-sized one reads
         * as the device does, where the test window is 2 400 dp tall. */
        frameHeight: androidx.compose.ui.unit.Dp? = null,
        content: @Composable () -> Unit,
    ) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    if (frameHeight != null) {
                        Box(
                            Modifier
                                .fillMaxWidth()
                                .height(frameHeight)
                                .background(Sillon.colors.bg),
                        ) {
                            content()
                        }
                    } else {
                        Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
                    }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        // A framed capture (a sheet) is cut at its own frame: the test window is
        // 2 400 dp tall and the ground below the frame is not the page's.
        val bitmap = if (frameHeight != null) {
            val density = view.resources.displayMetrics.density
            val pixels = (frameHeight.value * density).toInt().coerceIn(1, full.height)
            Bitmap.createBitmap(full, 0, 0, full.width, pixels)
        } else {
            cropToContent(full, theme)
        }
        val file = File(outputDir, name)
        file.parentFile?.mkdirs()
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
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
        "mb8-%s-%ddp-%s%s.png".format(theme.name.lowercase(), width, screen, suffix(fontScale))

    // -- the states ------------------------------------------------------------

    private fun favoritesState(count: Int = 12, leaf: LibraryLeaf = LibraryLeaf.Favorites) =
        LibraryModel.State(
            leaf = leaf,
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

    private fun historyState() = HistoryModel.State(
        screen = ScreenState.Ready(LibraryFixtures.page()),
        events = LibraryFixtures.page().events,
        summary = LibraryFixtures.page().summary,
        next = LibraryFixtures.page().next,
    )

    private val today = LocalDate.of(2026, 11, 17)
    private val zone: ZoneId = ZoneId.of("Europe/Paris")

    // -- the captures ----------------------------------------------------------

    @Test
    fun favoritesEncre() = capture(name(NzThemeKind.Encre, "favoris"), NzThemeKind.Encre) {
        LibraryScreen(state = favoritesState(), actions = LibraryActions())
    }

    @Test
    fun favoritesPapier() = capture(name(NzThemeKind.Papier, "favoris"), NzThemeKind.Papier) {
        LibraryScreen(state = favoritesState(), actions = LibraryActions())
    }

    @Test
    fun favoritesAtTwoHundredPercent() = capture(
        name(NzThemeKind.Encre, "favoris", fontScale = 2f),
        NzThemeKind.Encre,
        fontScale = 2f,
    ) {
        LibraryScreen(state = favoritesState(), actions = LibraryActions())
    }

    @Test
    fun artistsEncre() = capture(name(NzThemeKind.Encre, "artistes"), NzThemeKind.Encre) {
        LibraryScreen(state = favoritesState(leaf = LibraryLeaf.Artists), actions = LibraryActions())
    }

    @Test
    fun albumsPapier() = capture(name(NzThemeKind.Papier, "albums"), NzThemeKind.Papier) {
        LibraryScreen(state = favoritesState(leaf = LibraryLeaf.Albums), actions = LibraryActions())
    }

    @Test
    fun playlistsEncre() = capture(name(NzThemeKind.Encre, "playlists"), NzThemeKind.Encre) {
        LibraryScreen(state = favoritesState(leaf = LibraryLeaf.Playlists), actions = LibraryActions())
    }

    @Test
    fun playlistsPapier() = capture(name(NzThemeKind.Papier, "playlists"), NzThemeKind.Papier) {
        LibraryScreen(state = favoritesState(leaf = LibraryLeaf.Playlists), actions = LibraryActions())
    }

    @Test
    fun organizerEncre() = capture(name(NzThemeKind.Encre, "organiser"), NzThemeKind.Encre) {
        LibraryScreen(
            state = favoritesState(leaf = LibraryLeaf.Playlists).copy(
                organizing = true,
                organizeStatus = "Déplacé en position 2 sur 2",
            ),
            actions = LibraryActions(),
        )
    }

    @Test
    fun folderSheetPapier() = capture(
        name(NzThemeKind.Papier, "dossier"),
        NzThemeKind.Papier,
        frameHeight = 820.dp,
    ) {
        LibraryScreen(
            state = favoritesState(leaf = LibraryLeaf.Playlists).copy(
                folderSheet = FolderSheet(title = "Route de nuit"),
            ),
            actions = LibraryActions(),
        )
    }

    @Test
    fun moveSheetEncre() = capture(
        name(NzThemeKind.Encre, "deplacer"),
        NzThemeKind.Encre,
        frameHeight = 820.dp,
    ) {
        LibraryScreen(
            state = favoritesState(leaf = LibraryLeaf.Playlists).copy(
                moveTarget = MoveTarget(
                    playlistId = LibraryFixtures.MATIN,
                    title = "Matin",
                    currentFolder = null,
                ),
            ),
            actions = LibraryActions(),
        )
    }

    @Test
    fun localEncre() = capture(name(NzThemeKind.Encre, "mes-fichiers"), NzThemeKind.Encre) {
        LibraryScreen(state = favoritesState(leaf = LibraryLeaf.Local), actions = LibraryActions())
    }

    @Test
    fun unavailablePapier() = capture(name(NzThemeKind.Papier, "indisponibles"), NzThemeKind.Papier) {
        LibraryScreen(state = favoritesState(leaf = LibraryLeaf.Unavailable), actions = LibraryActions())
    }

    @Test
    fun dislikesEncre() = capture(name(NzThemeKind.Encre, "je-n-aime-pas"), NzThemeKind.Encre) {
        LibraryScreen(state = favoritesState(leaf = LibraryLeaf.Dislikes), actions = LibraryActions())
    }

    @Test
    fun failureEncre() = capture(name(NzThemeKind.Encre, "echec"), NzThemeKind.Encre) {
        LibraryScreen(
            state = LibraryModel.State(
                favorites = ScreenState.Failed(cause = "Hors ligne", retry = {}),
                foldersSupported = true,
            ),
            actions = LibraryActions(),
        )
    }

    @Test
    fun historyEncre() = capture(name(NzThemeKind.Encre, "historique"), NzThemeKind.Encre) {
        HistoryScreen(state = historyState(), actions = HistoryActions(), today = today, zone = zone)
    }

    @Test
    fun historyPapier() = capture(name(NzThemeKind.Papier, "historique"), NzThemeKind.Papier) {
        HistoryScreen(state = historyState(), actions = HistoryActions(), today = today, zone = zone)
    }

    @Test
    fun historyAtTwoHundredPercent() = capture(
        name(NzThemeKind.Papier, "historique", fontScale = 2f),
        NzThemeKind.Papier,
        fontScale = 2f,
    ) {
        HistoryScreen(state = historyState(), actions = HistoryActions(), today = today, zone = zone)
    }

    @Test
    fun historyEmptyEncre() = capture(name(NzThemeKind.Encre, "historique-vide"), NzThemeKind.Encre) {
        HistoryScreen(
            state = HistoryModel.State(
                screen = ScreenState.Ready(LibraryFixtures.emptyPage()),
                summary = LibraryFixtures.emptyPage().summary,
            ),
            actions = HistoryActions(),
            today = today,
            zone = zone,
        )
    }
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h2400dp-xhdpi")
class LibraryScreenshotPhoneTest : LibraryScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h2400dp-xhdpi")
class LibraryScreenshotTabletTest : LibraryScreenshotBase(width = 600)
