package fr.neezer.mobile.feature.genres

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * Le studio rendu : ses quatre onglets en Encre et Papier à 360 dp (le téléphone)
 * et 600 dp (la colonne de lecture), plus l'échelle 200 % que l'acceptation
 * demande, sous `docs/agents/screenshots/mobile/MB18/`.
 *
 * Le rendu est le backend NATIF de Robolectric (Skia sur l'hôte) : vraie mise en
 * page, vrai texte, vrais pixels, sur la JVM — aucun émulateur, aucune fenêtre sur
 * l'écran de la personne (règle 7). La recette que MB1 a mesurée et que MB5, MB6,
 * MB7, MB16 et MB17 réutilisent.
 *
 * Ce qui est dessiné est la VRAIE page ([GenresPage]) sur [FakeGenresService] :
 * une capture d'une maquette n'aurait pas attrapé les défauts de mise en page.
 */
abstract class GenresScreenshotBase(protected val width: Int) {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    protected val outputDir: File
        get() = File(
            System.getProperty("nz.screenshots")
                ?: error("nz.screenshots is not set: the screenshot test writes the lot's images"),
        )

    protected fun capture(
        tab: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        service: FakeGenresService = prepared(),
        name: String = "mb18-${slug(tab)}-${theme.name.lowercase()}-${width}dp" +
            (if (fontScale > 1f) "-font${(fontScale * 100).toInt()}" else "") + ".png",
        open: ((GenresModel) -> Unit)? = null,
        before: ((GenresModel) -> Unit)? = null,
    ) {
        val scope = CoroutineScope(Dispatchers.Main)
        val model = GenresModel(service, scope, io = Dispatchers.Main, toast = {})
        model.open()
        // The page's own reads land on the main looper: drain it BEFORE a test
        // drives the screen, or it would open a sheet on an empty list.
        shadowOf(Looper.getMainLooper()).idle()
        before?.invoke(model)
        model.selectTab(tab)
        shadowOf(Looper.getMainLooper()).idle()
        open?.invoke(model)
        shadowOf(Looper.getMainLooper()).idle()

        rule.setContent {
            NzTheme(kind = theme, accent = null, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        GenresPage(nav = null, model = model)
                    }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()

        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        outputDir.mkdirs()
        val file = File(outputDir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
        scope.cancel()
    }

    /** A block rendered on its own (a page too tall for the window). */
    protected fun captureContent(
        theme: NzThemeKind,
        name: String,
        fontScale: Float = 1f,
        content: @androidx.compose.runtime.Composable () -> Unit,
    ) {
        rule.setContent {
            NzTheme(kind = theme, accent = null, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                            Box(
                                Modifier.padding(
                                    start = NzSpacing.pageX,
                                    end = NzSpacing.pageX,
                                    top = NzSpacing.l,
                                ),
                            ) { content() }
                        }
                    }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val full = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(full))
        val bitmap = cropToContent(full, theme)
        outputDir.mkdirs()
        val file = File(outputDir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

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

    // --- *Étiquetage* -------------------------------------------------------

    @Test
    fun taggingInEncre() = capture(GenresCopy.TAB_TAG, NzThemeKind.Encre)

    @Test
    fun taggingInPapier() = capture(GenresCopy.TAB_TAG, NzThemeKind.Papier)

    @Test
    fun taggingAtTwoHundredPercent() = capture(
        GenresCopy.TAB_TAG,
        NzThemeKind.Papier,
        fontScale = 2f,
    )

    private fun slug(tab: String): String = when (tab) {
        GenresCopy.TAB_LIBRARY -> "library"
        GenresCopy.TAB_TREE -> "tree"
        GenresCopy.TAB_MODEL -> "model"
        else -> "tagging"
    }
}

/**
 * 360 dp: the phone the app is designed for first, and the ONE class that also
 * captures the states a tab does not always show (a failure, an empty page, a
 * trained head, the two sheets, the 200 % font). The tablet class keeps the four
 * tabs: a lot's images are a review cost, and 30 PNGs were 5.5 MB.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h2400dp-xhdpi")
class GenresScreenshotPhoneTest : GenresScreenshotBase(width = 360) {
    /** A failed first read: the cause and « Réessayer », never a spinner. */
    @Test
    fun aFailedLoadNamesItsCause() = capture(
        tab = GenresCopy.TAB_TAG,
        theme = NzThemeKind.Encre,
        service = prepared().apply { failures["candidates"] = "Le serveur ne répond pas" },
        name = "mb18-failure-encre-${width}dp.png",
    )

    /** Nothing left to tag: the page says so instead of an empty card. */
    @Test
    fun nothingLeftToTag() = capture(
        tab = GenresCopy.TAB_TAG,
        theme = NzThemeKind.Papier,
        service = prepared().apply { candidates = emptyList() },
        name = "mb18-empty-papier-${width}dp.png",
    )

    // --- *Bibliothèque* -----------------------------------------------------

    @Test
    fun libraryInEncre() = capture(GenresCopy.TAB_LIBRARY, NzThemeKind.Encre)

    @Test
    fun libraryInPapier() = capture(GenresCopy.TAB_LIBRARY, NzThemeKind.Papier)

    @Test
    fun libraryAtTwoHundredPercent() = capture(
        GenresCopy.TAB_LIBRARY,
        NzThemeKind.Papier,
        fontScale = 2f,
    )

    // --- *Arbre* ------------------------------------------------------------

    @Test
    fun treeInEncre() = capture(GenresCopy.TAB_TREE, NzThemeKind.Encre)

    @Test
    fun treeInPapier() = capture(GenresCopy.TAB_TREE, NzThemeKind.Papier)

    // --- *Modèle* -----------------------------------------------------------

    @Test
    fun modelInEncre() = capture(GenresCopy.TAB_MODEL, NzThemeKind.Encre)

    @Test
    fun modelInPapier() = capture(GenresCopy.TAB_MODEL, NzThemeKind.Papier)

    /** A trained head: the verdict, the per-class table, the confusions. */
    @Test
    fun aTrainedHead() = capture(
        tab = GenresCopy.TAB_MODEL,
        theme = NzThemeKind.Encre,
        name = "mb18-head-encre-${width}dp.png",
        before = { model ->
            model.train()
            shadowOf(Looper.getMainLooper()).idle()
        },
        open = { _ -> Unit },
    )

    // --- the sheets ---------------------------------------------------------

    /** The bulk sheet: a disagreement arrives unticked. */
    @Test
    fun theBulkSheet() = capture(
        tab = GenresCopy.TAB_MODEL,
        theme = NzThemeKind.Papier,
        name = "mb18-bulk-papier-${width}dp.png",
        before = { model ->
            model.openBulk(tag = "Frenchcore", scopeLabel = "Album « Al »", album = "42")
        },
    )

    /** The tag sheet of one track. */
    @Test
    fun theTagSheet() = capture(
        tab = GenresCopy.TAB_TAG,
        theme = NzThemeKind.Encre,
        name = "mb18-tag-sheet-encre-${width}dp.png",
        before = { model ->
            model.openSheet("1001", "Premier titre")
        },
    )

}

/** 600 dp: the tablet shell's reading column. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w600dp-h1600dp-xhdpi")
class GenresScreenshotTabletTest : GenresScreenshotBase(width = 600)

/** A fake service with the ranked vocabulary seeded: every capture needs it. */
internal fun prepared(): FakeGenresService = FakeGenresService().apply {
    seedRanking()
    modelTab = modelTab.copy(
        model = uniffi.nz_ffi.GenreModelRow(
            usable = true,
            version = 3,
            created = "2026-11-18T10:00:00",
            labels = listOf("Hardtek", "Frenchcore"),
            kind = "linear",
            dim = 2560u,
            health = "watch",
            balanced = 0.82,
        ),
    )
    trainingState = FakeGenresService.idleTraining().copy(
        running = false,
        stage = "terminé",
        fraction = 1.0,
        head = FakeGenresService.head(),
    )
}
