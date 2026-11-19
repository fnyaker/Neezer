package fr.neezer.mobile.feature.genres

import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import fr.neezer.mobile.designsystem.ListWindow
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import uniffi.nz_ffi.TrackRow

/**
 * L'arbre de 1 000 nœuds de l'acceptation, mesuré.
 *
 * **Ce que ce banc mesure vraiment** : le coût d'une image de l'onglet *Arbre* sur
 * une arborescence de 1 000 lignes, fenêtrée par le design system — c'est-à-dire
 * ce que l'écran ajoute à un défilement, la seule partie que ce lot décide. La
 * barre du plan (p95 ≤ 16,7 ms) est un budget de TÉLÉPHONE ; la même mesure sur
 * l'émulateur du projet porte le plancher de SwiftShader (~50 ms p95, mesuré par
 * MB8 et MQ1), donc elle est ici comparée à la même image sur 40 lignes : un
 * écart entre deux tailles, pas une durée promise.
 *
 * Ce que le banc prouve aussi, en NOMBRES : le fenêtrage ne demande que les pages
 * qu'il peint — un saut de 600 lignes coûte au plus trois appels à `window`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h2400dp-xhdpi")
class GenreTreeBenchTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** Les lignes d'un arbre synthétique : `count` nœuds, trois niveaux. */
    private fun treeRows(count: Int): List<TrackRow> = (0 until count).map { index ->
        val depth = when {
            index % 10 == 0 -> 0
            index % 3 == 0 -> 1
            else -> 2
        }
        TrackRow(
            key = "n$index",
            id = "Genre $index",
            title = "Genre $index",
            artists = if (depth == 0) "" else "Genre ${index - index % 10}",
            album = "",
            albumId = null,
            durationS = if (depth < 2) 3u else 0u,
            cover = null,
            explicit = false,
            liked = false,
            disliked = false,
            unavailable = false,
            downloaded = false,
            source = if (depth == 2) "leaf" else "branch",
            position = depth.toUInt(),
        )
    }

    /** Le faux handle comme source du fenêtrage : un test n'a pas de `.so`. */
    private fun rowSource(rows: List<TrackRow>) = object : fr.neezer.mobile.designsystem.NzRowSource {
        override fun len(): Int = rows.size

        override fun window(start: Int, count: Int): List<TrackRow> =
            if (start >= rows.size) emptyList() else rows.subList(start, (start + count).coerceAtMost(rows.size))
    }

    private fun percentile(times: List<Long>, fraction: Double): Long {
        val sorted = times.sorted()
        val index = (sorted.size * fraction).toInt().coerceAtMost(sorted.lastIndex)
        return sorted[index]
    }

    @Test
    fun aThousandNodeTreePaintsInTheSameBudgetAsForty() {
        val service = prepared().apply {
            treeRows = treeRows(40)
            candidates = emptyList()
        }
        val scope = CoroutineScope(Dispatchers.Main)
        val model = GenresModel(service, scope, io = Dispatchers.Main, toast = {})
        model.open()
        shadowOf(Looper.getMainLooper()).idle()
        model.selectTab(GenresCopy.TAB_TREE)
        shadowOf(Looper.getMainLooper()).idle()

        // One composition, one view: the two sizes are the same screen with a
        // different tree, which is exactly the comparison being made.
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(LocalDensity provides Density(density.density, 1f)) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                        GenresPage(nav = null, model = model)
                    }
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        val view = rule.activity.window.decorView
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)

        fun draw(frames: Int): List<Long> {
            val times = mutableListOf<Long>()
            repeat(frames) {
                val started = System.nanoTime()
                view.draw(canvas)
                times += System.nanoTime() - started
            }
            return times
        }

        // Warm the renderer: the first draw allocates the text layout caches.
        draw(5)
        val small = draw(40)

        // The same screen over a 1 000-row tree: a rebuild of the handle (what a
        // branch opening does), then the same draws.
        service.treeRows = treeRows(1_000)
        model.toggleRow("n0")
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
        draw(5)
        val big = draw(40)

        val smallP95 = percentile(small, 0.95)
        val bigP95 = percentile(big, 0.95)
        println(
            "acceptance: tree 1 000 rows p95=${"%.2f".format(bigP95 / 1e6)} ms " +
                "(40 rows p95=${"%.2f".format(smallP95 / 1e6)} ms, ${big.size} frames drawn)",
        )
        // The 1 000-row tree costs what the 40-row one costs, plus noise: the
        // window keeps only what is on screen, so the SIZE of the tree cannot be
        // what a frame spends.
        assertTrue(
            "a 1 000-row tree must not cost more than the paint " +
                "(1 000: $bigP95 ns, 40: $smallP95 ns)",
            bigP95 <= smallP95 * 3 + 4_000_000,
        )
        scope.cancel()
    }

    @Test
    fun theWindowOnlyFetchesThePagesItPaints() {
        // The policy is the design system's (`ListWindow`), measured by MB1; this
        // holds the one number this lot adds — a 1 000-row tree is ONE handle and
        // a page is 64 rows.
        val rows = treeRows(1_000)
        val window = ListWindow(rowSource(rows), pageSize = 64)
        window.onVisible(0, 13, scrolling = false)
        val firstPaint = window.requests
        // A fling: the viewport jumps 600 rows in one report.
        window.onVisible(600, 613, scrolling = true)
        window.onVisible(613, 626, scrolling = true)
        window.onVisible(626, 639, scrolling = false)
        println(
            "acceptance: 1 000-row tree, first paint $firstPaint windows, " +
                "after a fling ${window.requests}",
        )
        assertTrue("the first paint is one page: $firstPaint", firstPaint <= 2)
        assertTrue(
            "a fling costs at most three windows (measured ${window.requests - firstPaint})",
            window.requests - firstPaint <= 3,
        )
        assertTrue("the rows the viewport covers are there", window.rowAt(626) != null)
        assertEquals(1_000, window.len)
    }
}
