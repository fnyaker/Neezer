package fr.neezer.mobile.app

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import androidx.compose.ui.unit.width
import androidx.navigation.NavDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTabBar
import fr.neezer.mobile.designsystem.NzTheme
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.nz_ffi.AppLink

/**
 * La coquille de L1, sur un vrai mesurage et un vrai geste — Robolectric sur
 * l'hôte, aucun émulateur (worker rule 7). Le pager est le VRAI (`TabPager`, le
 * vrai `PagerDefaults`), le routeur est le VRAI `NavHost` (donc la route que le
 * pager compare est celle que Navigation enregistre, préfixe compris), et seules
 * les QUATRE ENTRÉES sont remplacées par un texte : elles demandent le noyau
 * Rust, qui n'existe pas sur la JVM.
 *
 * Les six cas du plan (§4 L1, « Tests to add »), et ce que chacun prouve :
 *
 *  a. quatre pages, la page 0 est l'accueil ;
 *  b. un glissement de 60 % de la largeur se pose sur la page 1 et n'ouvre
 *     l'onglet qu'UNE fois ;
 *  c. un glissement de 12 % sans vitesse revient à la page 0 et n'ouvre rien ;
 *  d. « Réglages » pousse la route et ne sélectionne aucun onglet ;
 *  e. à `fontScale = 2` aucun libellé n'est dessiné mais les cinq
 *     `contentDescription` restent (TalkBack ne perd pas le mot) ;
 *  f. la rangée tactile fait 64 dp et 72 dp de large sur 360 dp — la barre de
 *     cinq items du §3, mesurée et non affirmée.
 */
@RunWith(RobolectricTestRunner::class)
@Config(
    sdk = [34],
    qualifiers = "w360dp-h800dp-xhdpi",
    // Le VRAI `Application` de l'app charge `libnz_ffi.so` dans `onCreate` : sur
    // la JVM il n'y a pas de `.so`, et rien ici n'a besoin du noyau.
    application = android.app.Application::class,
)
class ShellTabsTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    /** The tabs the shell opened, in order (both doors go through it). */
    private val opened = mutableListOf<Tab>()

    /** The Réglages door's own counter: it is not a tab and must not be one. */
    private var settings = 0

    /** The pager the composition built, read after `waitForIdle`. */
    private var pager: PagerState? = null

    private object FakeNav : NzNavigator {
        override fun go(link: AppLink) = Unit

        override fun back() = Unit

        override fun openPlayer() = Unit

        override fun openQueue() = Unit
    }

    /** The route the shell's `open(tab)` navigates to, as the router spells it. */
    private fun routeOf(tab: Tab) = when (tab) {
        Tab.Accueil -> Routes.Home
        Tab.Rechercher -> Routes.Search()
        Tab.Bibliotheque -> Routes.Library()
        Tab.Podcasts -> Routes.Podcasts
    }

    @Test
    fun thePagerHoldsTheFourTabRoots() {
        shell()
        assertEquals("le pager n'a pas les quatre racines d'onglet", 4, pager!!.pageCount)
        assertEquals("la coquille ne s'ouvre pas sur l'accueil", 0, pager!!.currentPage)
        assertEquals("la page 0 n'est pas l'accueil", Tab.Accueil, tabOfPage(0))
        // La barre a CINQ items : les quatre pages, puis la porte de Réglages.
        assertEquals(5, TAB_ITEMS.size)
        // (a) « page 0 = HomeEntry » : la liaison ne se compose pas sans le noyau
        // Rust (les quatre entrées dessinent alors leur phrase « le noyau n'est
        // pas disponible »), donc elle se lit dans la source — le `when` est de
        // toute façon exhaustif sur l'enum, un cinquième onglet ne compilerait pas.
        val shell = File(
            System.getProperty("nz.repoRoot") ?: error("nz.repoRoot is not set"),
            "mobile/android/app/src/main/java/fr/neezer/mobile/app/Shell.kt",
        ).readText()
        assertTrue(
            "la page de l'accueil ne dessine pas HomeEntry",
            shell.contains("Tab.Accueil -> HomeEntry(nav)"),
        )
        rule.onNodeWithTag("page0").assertExists()
    }

    @Test
    fun aLongDragSettlesOnTheSecondPageAndOpensItOnce() {
        shell()
        rule.onNodeWithTag("page0").performTouchInput {
            down(center)
            advanceEventTime(16)
            // The platform's slop first: a pager starts moving after it, like a
            // finger does (G1's measurement: 32 px at 2× on Robolectric).
            moveBy(Offset(-viewConfiguration.touchSlop - 8f, 0f))
            moveBy(Offset(-width * 0.60f, 0f))
            advanceEventTime(16)
            up()
        }
        settle()
        assertEquals("60 % de la largeur ne pose pas la page 1", 1, pager!!.currentPage)
        assertEquals("l'onglet n'a pas été ouvert exactement une fois", listOf(Tab.Rechercher), opened)
        rule.onNodeWithContentDescription("Rechercher").assertIsSelected()
    }

    @Test
    fun aShortDragWithoutVelocityReturnsAndOpensNothing() {
        shell()
        rule.onNodeWithTag("page0").performTouchInput {
            down(center)
            advanceEventTime(16)
            moveBy(Offset(-viewConfiguration.touchSlop - 4f, 0f))
            moveBy(Offset(-width * 0.12f, 0f))
            advanceEventTime(300)
            up()
        }
        settle()
        assertEquals("12 % sans vitesse ne revient pas à la page 0", 0, pager!!.currentPage)
        assertTrue("un glissement avorté a ouvert un onglet : $opened", opened.isEmpty())
        rule.onNodeWithContentDescription("Accueil").assertIsSelected()
    }

    @Test
    fun settingsIsTheFifthItemAndSelectsNoTab() {
        shell()
        rule.onNodeWithContentDescription("Réglages").performClick()
        settle()
        assertEquals("la porte de Réglages n'a pas été appelée", 1, settings)
        assertTrue("Réglages est passé pour un onglet : $opened", opened.isEmpty())
        // Sur la route de Réglages, aucun ONGLET n'est sélectionné — et l'item
        // « Réglages », lui, l'est : c'est sa raison d'être (le `aria-current` du
        // SPA sur /settings), sinon la personne ne saurait pas où elle est.
        rule.onNodeWithContentDescription("Accueil").assertIsNotSelected()
        rule.onNodeWithContentDescription("Biblio").assertIsNotSelected()
        rule.onNodeWithContentDescription("Réglages").assertIsSelected()
        rule.onNodeWithText("réglages").assertExists()
        assertEquals(4, selectedIndexOf(lastDestination))
    }

    @Test
    fun atFontScaleTwoTheWordsGoAndTheDescriptionsStay() {
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                val density = LocalDensity.current
                CompositionLocalProvider(
                    LocalDensity provides Density(density = density.density, fontScale = 2f),
                ) {
                    NzTabBar(items = TAB_ITEMS, selected = 0, onPick = {})
                }
            }
        }
        for (item in TAB_ITEMS) {
            rule.onAllNodesWithText(item.label).assertCountEquals(0)
            rule.onAllNodesWithContentDescription(item.label).assertCountEquals(1)
        }
    }

    @Test
    fun theTabRowsAreSixtyFourDpAndSeventyTwoDpWide() {
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                NzTabBar(items = TAB_ITEMS, selected = 0, onPick = {})
            }
        }
        val bounds = rule.onNodeWithContentDescription("Accueil").getUnclippedBoundsInRoot()
        assertEquals("la cible tactile de la barre ne fait pas 64 dp", 64.dp, bounds.height)
        // 360 dp / 5 items: the plan's risk 2, measured.
        assertEquals("cinq items ne partagent pas 360 dp en 72 dp", 72.dp, bounds.width)
    }

    /** The plan's own settle: frames advanced, then a bounded wait — never a sleep. */
    private fun settle() {
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(600)
        rule.waitForIdle()
    }

    /** The destination the REAL router reported at the last composition. */
    private var lastDestination: NavDestination? = null

    /**
     * The shell as `NzApp` mounts it: one `NavHost` for the four tab routes, the
     * pager above it, the bar below it. The two doors the shell offers (a bar tap
     * and a settle) call the same `openTab`, exactly as `nav.open(tab)` does.
     */
    private fun shell() {
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                val nav = rememberNavController()
                val entry by nav.currentBackStackEntryAsState()
                val destination = entry?.destination
                lastDestination = destination
                val state = rememberPagerState(initialPage = 0) { Tab.entries.size }
                pager = state
                val scope = rememberCoroutineScope()
                fun openTab(tab: Tab) {
                    opened += tab
                    nav.navigate(routeOf(tab)) { launchSingleTop = true }
                }
                TabPagerSync(destination, state) { tab -> openTab(tab) }
                Column(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f).fillMaxWidth()) {
                        NavHost(navController = nav, startDestination = Routes.Home) {
                            composable<Routes.Home> { TabPager(FakeNav, state, page = { FakePage(it) }) }
                            composable<Routes.Search> { TabPager(FakeNav, state, page = { FakePage(it) }) }
                            composable<Routes.Library> { TabPager(FakeNav, state, page = { FakePage(it) }) }
                            composable<Routes.Podcasts> { TabPager(FakeNav, state, page = { FakePage(it) }) }
                            composable<Routes.Settings> { Text("réglages") }
                        }
                    }
                    TabBarHost(
                        destination = destination,
                        pager = state,
                        scope = scope,
                        onOpenTab = { tab -> openTab(tab) },
                        onOpenSettings = {
                            settings++
                            nav.navigate(Routes.Settings()) { launchSingleTop = true }
                        },
                    )
                }
            }
        }
        settle()
    }
}

/** A page the pager can move: it fills the whole width and says which one it is. */
@Composable
private fun FakePage(index: Int) {
    Box(Modifier.fillMaxSize().testTag("page$index")) { Text("page $index") }
}
