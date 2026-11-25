package fr.neezer.mobile.feature.entity

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Artiste, album et crédits, rendered: Encre and Papier at 360 dp (the phone) and
 * 600 dp (the tablet's reading column), plus the 200 % font scale the acceptance
 * asks about, committed under `docs/agents/screenshots/mobile/MB7/`.
 *
 * The renderer is Robolectric's NATIVE graphics backend (Skia on the host): real
 * layout, real text with the three real fonts, real pixels, on the JVM — no
 * emulator, no window on the person's display (worker rule 7). The same recipe
 * MB1 measured and MB5 used.
 *
 * The screens drawn are the REAL ones over the REAL fixtures: what the images show
 * is what a person sees, not a mock.
 */
abstract class EntityScreenshotBase(private val width: Int) {
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
        content: @Composable () -> Unit,
    ) {
        render(theme, fontScale, content)
        saveTo(outputDir, name, theme)
    }

    protected fun render(
        theme: NzThemeKind,
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) {
        rule.setContent {
            NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) { content() }
                }
            }
        }
    }

    protected fun saveTo(dir: File, name: String, theme: NzThemeKind) {
        shadowOf(Looper.getMainLooper()).idle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        val file = File(dir, name)
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
        "mb7-%s-%ddp-%s%s.png".format(theme.name.lowercase(), width, screen, suffix(fontScale))

    private fun artistState() = ArtistModel.State(
        id = EntityFixtures.ARTIST_DEEZER,
        screen = ScreenState.Ready(EntityFixtures.artist()),
        tab = ArtistTab.Overview,
        discography = EntityFixtures.discography(),
        onDeezer = listOf(EntityFixtures.albumCard("777", "Seulement sur Deezer", "2019")),
        onDeezerFetched = true,
        tracks = ScreenState.Ready(FakeRows((0 until 40).map { EntityFixtures.row("Titre $it", it) })),
    )

    private fun artistTracksState() = artistState().copy(tab = ArtistTab.Tracks).let { state ->
        val tracks = state.tracks as ScreenState.Ready
        state.copy(tracks = tracks)
    }

    private fun artistCreditsState() = artistState().copy(
        tab = ArtistTab.Credits,
        credits = ScreenState.Ready(EntityFixtures.person()),
        creditsTracks = EntityFixtures.person().tracks,
    )

    private fun albumState(tracks: Int = 12, deezer: Boolean = true) = AlbumModel.State(
        id = EntityFixtures.ALBUM_DEEZER,
        screen = ScreenState.Ready(EntityFixtures.albumView(tracks = tracks)),
        onDeezer = if (deezer) listOf(EntityFixtures.row("Bonus", 12)) else emptyList(),
        onDeezerFetched = true,
    )

    private fun personState(rows: Int = 0) = PersonModel.State(
        name = "Nina Simone",
        screen = ScreenState.Ready(
            if (rows == 0) {
                EntityFixtures.person()
            } else {
                EntityFixtures.person(rows = (0 until rows).map { EntityFixtures.row("Œuvre $it", it) })
            },
        ),
    )

    private fun sheetState() = TrackCreditsModel.State(
        track = EntityFixtures.row("Sinnerman", 0),
        screen = ScreenState.Ready(TrackCreditsView.of(EntityFixtures.trackCredits())),
    )

    @Test
    fun artistOverviewEncre() = capture(name(NzThemeKind.Encre, "artiste"), NzThemeKind.Encre) {
        ArtistScreen(state = artistState(), actions = ArtistActions())
    }

    @Test
    fun artistOverviewPapier() = capture(name(NzThemeKind.Papier, "artiste"), NzThemeKind.Papier) {
        ArtistScreen(state = artistState(), actions = ArtistActions())
    }

    @Test
    fun artistTracksEncre() = capture(name(NzThemeKind.Encre, "artiste-titres"), NzThemeKind.Encre) {
        ArtistScreen(state = artistTracksState(), actions = ArtistActions())
    }

    @Test
    fun artistCreditsPapier() = capture(name(NzThemeKind.Papier, "artiste-credits"), NzThemeKind.Papier) {
        ArtistScreen(state = artistCreditsState(), actions = ArtistActions())
    }

    @Test
    fun artistAtTwoHundredPercent() = capture(
        name(NzThemeKind.Encre, "artiste", fontScale = 2f),
        NzThemeKind.Encre,
        fontScale = 2f,
    ) {
        ArtistScreen(state = artistState(), actions = ArtistActions())
    }

    @Test
    fun albumEncre() = capture(name(NzThemeKind.Encre, "album"), NzThemeKind.Encre) {
        AlbumScreen(state = albumState(), actions = AlbumActions())
    }

    @Test
    fun albumPapier() = capture(name(NzThemeKind.Papier, "album"), NzThemeKind.Papier) {
        AlbumScreen(state = albumState(), actions = AlbumActions())
    }

    @Test
    fun albumAtTwoHundredPercent() = capture(
        name(NzThemeKind.Papier, "album", fontScale = 2f),
        NzThemeKind.Papier,
        fontScale = 2f,
    ) {
        AlbumScreen(state = albumState(), actions = AlbumActions())
    }

    @Test
    fun creditsEncre() = capture(name(NzThemeKind.Encre, "credits"), NzThemeKind.Encre) {
        CreditsScreen(state = personState(), actions = CreditsActions())
    }

    @Test
    fun creditsPapier() = capture(name(NzThemeKind.Papier, "credits"), NzThemeKind.Papier) {
        CreditsScreen(state = personState(), actions = CreditsActions())
    }

    @Test
    fun creditsSheetEncre() = capture(name(NzThemeKind.Encre, "credits-titre"), NzThemeKind.Encre) {
        TrackCreditsSheet(
            state = sheetState(),
            track = EntityFixtures.row("Sinnerman", 0),
            visible = true,
            onDismiss = {},
            onRetry = {},
            onArtist = {},
            onPerson = {},
        )
    }

    @Test
    fun artistFailureEncre() = capture(name(NzThemeKind.Encre, "artiste-echec"), NzThemeKind.Encre) {
        ArtistScreen(
            state = ArtistModel.State(
                id = EntityFixtures.ARTIST_DEEZER,
                screen = ScreenState.Failed(cause = "Hors ligne", retry = {}),
            ),
            actions = ArtistActions(),
        )
    }

    /**
     * S2 : la bande annule la marge haute de la page (elle commence à 0), les
     * onglets sont à 20 dp sous elle, et la liste s'arrête 40 dp avant le bas.
     */
    @Test
    fun theArtistBandAndTheListFollowThePage() {
        render(NzThemeKind.Encre) {
            ArtistScreen(state = artistTracksState(), actions = ArtistActions())
        }
        rule.waitForIdle()
        val band = rule.onNodeWithTag(ARTIST_BAND_TAG).getUnclippedBoundsInRoot()
        val tab = rule.onNodeWithText("Aperçu").getUnclippedBoundsInRoot()
        println("MEASURE s2 artiste: width=${width}dp band.top=${band.top.value}dp tab.top=${tab.top.value}dp band.bottom=${band.bottom.value}dp")
        assertEquals("la bande commence au bord haut", 0f, band.top.value, 1f)
        assertEquals("les onglets sont à 20 dp sous la bande", 20f, tab.top.value - band.bottom.value, 1f)

        rule.onNodeWithTag(ARTIST_LIST_TAG).performScrollToNode(hasText("Titre 39"))
        rule.waitForIdle()
        val last = rule.onNode(hasText("Titre 39") and hasClickAction()).getUnclippedBoundsInRoot()
        val root = rule.onRoot().getUnclippedBoundsInRoot()
        println("MEASURE s2 artiste tail: width=${width}dp last.bottom=${last.bottom.value}dp root.bottom=${root.bottom.value}dp")
        assertEquals("la liste s'arrête 40 dp avant le bas", 40f, root.bottom.value - last.bottom.value, 1f)
        if (width == 360) saveTo(s2OutputDir, "s2-artiste-360dp.png", NzThemeKind.Encre)
    }

    /** S2 : la bande de l'album commence à 0 et ses 60 titres s'arrêtent à 40 dp. */
    @Test
    fun theAlbumBandAndTheListFollowThePage() {
        render(NzThemeKind.Encre) {
            AlbumScreen(state = albumState(tracks = 60, deezer = false), actions = AlbumActions())
        }
        rule.waitForIdle()
        val band = rule.onNodeWithTag(ALBUM_BAND_TAG).getUnclippedBoundsInRoot()
        println("MEASURE s2 album: width=${width}dp band.top=${band.top.value}dp")
        assertEquals("la bande commence au bord haut", 0f, band.top.value, 1f)

        rule.onNodeWithTag(ALBUM_LIST_TAG).performScrollToNode(hasText("Titre 60"))
        rule.waitForIdle()
        val last = rule.onNode(hasText("Titre 60") and hasClickAction()).getUnclippedBoundsInRoot()
        val root = rule.onRoot().getUnclippedBoundsInRoot()
        println("MEASURE s2 album tail: width=${width}dp last.bottom=${last.bottom.value}dp root.bottom=${root.bottom.value}dp")
        assertEquals("la liste s'arrête 40 dp avant le bas", 40f, root.bottom.value - last.bottom.value, 1f)
        if (width == 360) saveTo(s2OutputDir, "s2-album-360dp.png", NzThemeKind.Encre)
    }

    /** S2 : les crédits aussi commencent au bord haut, et 60 titres s'arrêtent à 40 dp. */
    @Test
    fun theCreditsBandAndTheListFollowThePage() {
        render(NzThemeKind.Encre) {
            CreditsScreen(state = personState(rows = 60), actions = CreditsActions())
        }
        rule.waitForIdle()
        val band = rule.onNodeWithTag(CREDITS_BAND_TAG).getUnclippedBoundsInRoot()
        println("MEASURE s2 credits: width=${width}dp band.top=${band.top.value}dp")
        assertEquals("la bande commence au bord haut", 0f, band.top.value, 1f)

        rule.onNodeWithTag(CREDITS_LIST_TAG).performScrollToNode(hasText("Œuvre 59"))
        rule.waitForIdle()
        val last = rule.onNode(hasText("Œuvre 59") and hasClickAction()).getUnclippedBoundsInRoot()
        val root = rule.onRoot().getUnclippedBoundsInRoot()
        println("MEASURE s2 credits tail: width=${width}dp last.bottom=${last.bottom.value}dp root.bottom=${root.bottom.value}dp")
        assertEquals("la liste s'arrête 40 dp avant le bas", 40f, root.bottom.value - last.bottom.value, 1f)
        if (width == 360) saveTo(s2OutputDir, "s2-credits-360dp.png", NzThemeKind.Encre)
    }

    private val s2OutputDir: File
        get() = File(
            File(
                System.getProperty("nz.repoRoot")
                    ?: error("nz.repoRoot is not set: the screenshot test needs the repository root"),
            ),
            "docs/agents/screenshots/mobile/S2",
        )
}

/** 360 dp: the phone the app is designed for first. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h2400dp-xhdpi")
class EntityScreenshotPhoneTest : EntityScreenshotBase(width = 360)

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w600dp-h2400dp-xhdpi")
class EntityScreenshotTabletTest : EntityScreenshotBase(width = 600)
