package fr.neezer.mobile.feature.genres

import android.os.Looper
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.testkit.AtfSuppressions
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
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
import uniffi.nz_ffi.GenreRankedTag

/**
 * Le studio de genres, rendu et VERIFIED : les douze puces de genre
 * (`Chanson française`, `Hip-hop alternatif`, …) dans la feuille d'étiquetage et
 * l'arbre, à 360 et 412 dp et aux échelles 1.0 et 1.3 (la matrice du plan T3).
 *
 * T3 a remplacé les captures MB18 — qui écrivaient un PNG sans jamais le
 * comparer — par ces goldens. Les deux autres défauts rapportés du studio sont
 * des nombres que la puce doit tenir : chaque puce est une cible de 48 dp, les
 * puces d'une rangée partagent leur ligne, et la première commence à la marge de
 * contenu de la page.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi")
class GenresGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    private val scope = CoroutineScope(Dispatchers.Main)

    @Test
    fun treeAt360() = shotTree(360, 1.0f)

    @Test
    @Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi", fontScale = 1.3f)
    fun treeAt360At130Percent() = shotTree(360, 1.3f)

    @Test
    @Config(sdk = [35], qualifiers = "w412dp-h900dp-xhdpi")
    fun treeAt412() = shotTree(412, 1.0f)

    @Test
    @Config(sdk = [35], qualifiers = "w412dp-h900dp-xhdpi", fontScale = 1.3f)
    fun treeAt412At130Percent() = shotTree(412, 1.3f)

    private fun shotTree(width: Int, scale: Float) {
        render(tab = GenresCopy.TAB_TREE, fontScale = scale)
        rule.capture("genres-arbre-${width}dp-f$scale.png")
        scope.cancel()
    }

    private fun render(tab: String, fontScale: Float = 1f) {
        val model = GenresModel(service(), scope, io = Dispatchers.Main, toast = {})
        model.open()
        shadowOf(Looper.getMainLooper()).idle()
        model.selectTab(tab)
        shadowOf(Looper.getMainLooper()).idle()
        rule.setContent(fontScale) {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    GenresPage(nav = null, model = model)
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.composeRule.waitForIdle()
    }
}

/**
 * The tag sheet's twelve chips, and the geometry they must keep.
 *
 * The one suppression: [NzSheet]'s tap-anywhere veil is a full-window clickable
 * with no speakable text (ATF's `SpeakableTextPresentCheck`). It is a real
 * finding in the design system's file — the veil only takes a tap, and a
 * screen-reader user dismisses with the system back — but fixing it is a
 * production change T3 does not own (plan T3: production layout out of scope).
 * So the suppression is HERE, on this one screen's rule, and matches the exact
 * shape of that node: the check, `android.view.View`, clickable, and no text or
 * description. It is not global: every other golden class runs with none.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi")
class GenresChipsGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule(accessibilitySuppressions = AtfSuppressions.sheetVeil)

    private val scope = CoroutineScope(Dispatchers.Main)

    @Test
    fun twelveChipsAt360() = shotChips(360, 1.0f)

    @Test
    @Config(sdk = [35], qualifiers = "w360dp-h900dp-xhdpi", fontScale = 1.3f)
    fun twelveChipsAt360At130Percent() = shotChips(360, 1.3f)

    @Test
    @Config(sdk = [35], qualifiers = "w412dp-h900dp-xhdpi")
    fun twelveChipsAt412() = shotChips(412, 1.0f)

    @Test
    @Config(sdk = [35], qualifiers = "w412dp-h900dp-xhdpi", fontScale = 1.3f)
    fun twelveChipsAt412At130Percent() = shotChips(412, 1.3f)

    /**
     * T3's filter chips, as numbers: every chip of the genre row is a ≥ 48 dp
     * target, they all sit on one line (equal bottoms — a clipped chip is the
     * defect), and the first one starts on the page's 20 dp content padding.
     */
    @Test
    fun everyChipIsATargetOnOneLineAtTheContentPadding() {
        render(withSheet = false)
        val chips = GenresCopy.TABS.map { rule.composeRule.onNodeWithText(it).getUnclippedBoundsInRoot() }
        val bottoms = chips.map { it.bottom.value }
        val heights = chips.map { (it.bottom - it.top).value }
        println(
            "MEASURE t3 puces: heights=$heights lefts=${chips.map { it.left.value }} " +
                "bottoms=$bottoms",
        )
        heights.forEach { assertTrue("a chip is a 48 dp target ($it dp)", it >= 48f - 0.5f) }
        bottoms.zipWithNext { a, b -> assertEquals("the chips share one line", a, b, 1f) }
        assertEquals(
            "the first chip starts at the page's content padding",
            20f,
            chips.first().left.value,
            1f,
        )
        scope.cancel()
    }

    private fun shotChips(width: Int, scale: Float) {
        render(fontScale = scale)
        rule.capture("genres-etiquettes-${width}dp-f$scale.png")
        scope.cancel()
    }

    private fun render(withSheet: Boolean = true, fontScale: Float = 1f) {
        val model = GenresModel(service(), scope, io = Dispatchers.Main, toast = {})
        model.open()
        // The page's own reads land on the main looper: drain it BEFORE the screen
        // is driven, or the sheet opens on an empty list.
        shadowOf(Looper.getMainLooper()).idle()
        model.selectTab(GenresCopy.TAB_TAG)
        shadowOf(Looper.getMainLooper()).idle()
        if (withSheet) {
            model.openSheet("1001", "Premier titre")
            shadowOf(Looper.getMainLooper()).idle()
        }
        rule.setContent(fontScale) {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    GenresPage(nav = null, model = model)
                }
            }
        }
        shadowOf(Looper.getMainLooper()).idle()
        rule.composeRule.waitForIdle()
    }
}

private fun service(): FakeGenresService = prepared().apply {
    ranking["1001"] = TWELVE_CHIPS
}

/**
 * The matrix's twelve: real French genres, long enough that 1.3 has to either
 * fit or wrap — never clip.
 */
private val TWELVE_CHIPS = listOf(
    GenreRankedTag(101, "Chanson française", null, 24u, 0.42, 0.30, 0.50),
    GenreRankedTag(102, "Hip-hop alternatif", null, 19u, 0.40, 0.28, 0.48),
    GenreRankedTag(103, "Électro-pop", null, 17u, 0.38, 0.26, 0.46),
    GenreRankedTag(104, "Rock français", null, 15u, 0.36, 0.24, 0.44),
    GenreRankedTag(105, "Jazz manouche", null, 12u, 0.34, 0.22, 0.42),
    GenreRankedTag(106, "Musique de chambre", null, 11u, 0.32, 0.20, 0.40),
    GenreRankedTag(107, "Soul rétro", null, 10u, 0.30, null, null),
    GenreRankedTag(108, "Ambient planant", null, 9u, 0.28, null, null),
    GenreRankedTag(109, "Chanson engagée", null, 8u, 0.26, null, null),
    GenreRankedTag(110, "Afrobeat contemporain", null, 7u, 0.24, null, null),
    GenreRankedTag(111, "Punk mélodique", null, 6u, 0.22, null, null),
    GenreRankedTag(112, "Bossa nova", null, 5u, 0.20, null, null),
)

/** A fake service with the ranked vocabulary seeded: every render needs it. */
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
