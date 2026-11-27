package fr.neezer.mobile.app

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotDisplayed
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeWithVelocity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.height
import androidx.compose.ui.unit.width
import androidx.navigation.NavDestination
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import fr.neezer.mobile.bridge.LoadWhenShown
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTabBar
import fr.neezer.mobile.designsystem.NzTheme
import java.io.File
import kotlinx.coroutines.launch
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
            shell.contains("Tab.Accueil -> HomeEntry(nav, visible = visible)"),
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
        rule.onNodeWithContentDescription("Explorer").assertIsSelected()
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

    /**
     * **Le pager suit le doigt à CHAQUE image, et le lancer atterrit sur l'onglet
     * suivant** (T5, `plan-mobile-tests.md` §T5, bug 6).
     *
     * Le cas (b) ci-dessus ne lit que l'état final ; un pager qui saute d'un coup
     * à la fin — ou qui revient en arrière sur l'image du relâchement — le passe
     * encore. Ici l'horloge est tenue (`autoAdvance = false`) : six images de
     * 10 dp, 16 ms d'écart, la position de `page0` lue après chacune. 10 dp par
     * 16 ms ≈ 1 250 px/s, la vitesse que le plan nomme (1 200 px/s).
     *
     * Le slop de la plateforme est payé une fois avant les images mesurées (le
     * pager ne reçoit que le déplacement au-delà, comme les cas ci-dessus le
     * paient) ; chaque image doit ensuite suivre à 1:1 ±2 dp et jamais moins de
     * 60 % du dx injecté.
     */
    @Test
    fun thePagerFollowsTheFingerFrameByFrameAndTheFlingLandsOnTheNextTab() {
        shell()
        rule.mainClock.autoAdvance = false
        val stepDp = 10f
        rule.onNodeWithTag("page0").performTouchInput {
            down(center)
            advanceEventTime(16)
            moveBy(Offset(-viewConfiguration.touchSlop - 2f, 0f))
        }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        var previous = pageLeft()
        for (frame in 1..6) {
            rule.onNodeWithTag("page0").performTouchInput {
                advanceEventTime(16)
                moveBy(Offset(-stepDp.dp.toPx(), 0f))
            }
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            val left = pageLeft()
            val moved = previous - left
            println(
                "MEASURE l1 frame $frame: dx=${"%.1f".format(stepDp)}dp, " +
                    "page0.left=${"%.1f".format(left)}dp, moved=${"%.1f".format(moved)}dp",
            )
            assertTrue(
                "image $frame : le pager doit suivre le doigt " +
                    "(${"%.1f".format(moved)}dp sur ${"%.1f".format(stepDp)}dp)",
                moved >= stepDp * 0.6f,
            )
            assertEquals("image $frame : 1:1 avec le doigt (±2 dp)", stepDp, moved, 2f)
            previous = left
        }

        // Le doigt repart sans décider : le pager revient sur la page 0 (un
        // glissement annulé n'ouvre rien), puis le lancer part d'une geste neuve —
        // le `up` d'un lancer doit être dans le MÊME bloc que le dernier
        // mouvement, sinon l'horloge d'événements repart à zéro et le tracker lit
        // la pause (mesuré sur le lecteur, cas 3).
        rule.onNodeWithTag("page0").performTouchInput { cancel() }
        var returned = 0
        while (pager!!.currentPageOffsetFraction > 0.001f && returned < 1_000) {
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            returned += 16
        }
        assertEquals("un glissement annulé revient à la page 0", 0f, pager!!.currentPageOffsetFraction, 0.01f)

        // 1 200 px/s sur la moitié de la largeur : la vitesse que le plan nomme.
        rule.onNodeWithTag("page0").performTouchInput {
            swipeWithVelocity(
                start = center,
                end = Offset(centerX - width * 0.5f, centerY),
                endVelocity = 1_200f,
                durationMillis = 100,
            )
        }
        // La position absolue du pager : `currentPage + currentPageOffsetFraction`,
        // parce que la fraction change de base quand la page courante bascule en
        // cours de lancer (mesuré : 0,5 puis −0,46, un saut qui n'en est pas un).
        val fractions = mutableListOf<Float>()
        repeat(3) {
            rule.mainClock.advanceTimeByFrame()
            rule.waitForIdle()
            fractions += pager!!.currentPage + pager!!.currentPageOffsetFraction
        }
        println("MEASURE l1 release frames: absolutePage=$fractions")
        assertTrue(
            "le pager doit continuer après le relâchement ($fractions)",
            fractions.zipWithNext().all { (a, b) -> b >= a },
        )
        assertTrue(
            "et il doit avoir avancé, pas être resté sur place ($fractions)",
            fractions.last() > fractions.first(),
        )

        rule.mainClock.autoAdvance = true
        settle()
        assertEquals("le lancer doit poser la page 1", 1, pager!!.currentPage)
        assertEquals("l'onglet doit être ouvert exactement une fois", listOf(Tab.Rechercher), opened)
        rule.onNodeWithContentDescription("Explorer").assertIsSelected()
    }

    /**
     * **Un glissement lent revient sur l'onglet courant sans sauter sur l'image du
     * relâchement** (T5).
     *
     * 12 % de la largeur à ≈80 px/s : sous le seuil de distance comme sous le
     * seuil de vitesse du pager, donc seul le retour est possible. L'image qui
     * suit le relâchement doit encore être au doigt (un saut à 0 — l'ancien
     * défaut — s'y lirait 0), et le pager est revenu sur la page 0 ensuite.
     */
    @Test
    fun aSlowDragReturnsWithoutJumpingOnTheReleaseFrame() {
        shell()
        rule.mainClock.autoAdvance = false
        rule.onNodeWithTag("page0").performTouchInput {
            down(center)
            advanceEventTime(16)
            moveBy(Offset(-viewConfiguration.touchSlop - 4f, 0f))
            moveBy(Offset(-width * 0.12f, 0f))
            advanceEventTime(300)
            up()
        }
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val releasedAt = pageLeft()
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
        val afterRelease = pageLeft()
        println(
            "MEASURE l1 low release: released at ${"%.1f".format(releasedAt)}dp, " +
                "first frame ${"%.1f".format(afterRelease)}dp",
        )
        assertTrue(
            "l'image du relâchement ne doit pas sauter au début " +
                "(relâché à ${"%.1f".format(releasedAt)}dp, première image " +
                "${"%.1f".format(afterRelease)}dp)",
            kotlin.math.abs(afterRelease) > kotlin.math.abs(releasedAt) * 0.75f,
        )

        rule.mainClock.autoAdvance = true
        settle()
        assertEquals("12 % sans vitesse doit revenir à la page 0", 0, pager!!.currentPage)
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

    /**
     * **Un onglet revenu montre son dernier état** (lot SB2, spec
     * `spec-mobile-shell-bugs.md` §C) : le banc monte l'ARRANGEMENT RÉEL — les
     * quatre pages dans une seule couche sous le `NavHost`, les routes d'onglet
     * vides, la vraie `TabPagerSync` et la vraie porte `LoadWhenShown`.
     *
     * Ce que le cas prouve, dans l'ordre :
     *
     *  1. au lancement, les quatre pages sont composées (`builds` = 1 chacune) et
     *     UNE seule charge (`loads` = page 0) : une installation neuve ne lance
     *     pas quatre lectures avant d'avoir peint l'accueil (règle 1) ;
     *  2. glisser vers la page 1 ne reconstruit RIEN (`builds` inchangé) et charge
     *     la page 1 une fois ;
     *  3. revenir sur la page 0 ne recharge pas (`loads[0]` reste 1) ;
     *  4. une fiche poussée au-dessus cache la couche sans la démonter, et le
     *     retour la remontre sans une reconstruction ni une requête.
     */
    @Test
    fun theTabThatComesBackIsTheSameOne() {
        arrangedShell()

        assertEquals(
            "les quatre pages ne sont pas composées une fois chacune au lancement",
            mapOf(0 to 1, 1 to 1, 2 to 1, 3 to 1),
            builds.toMap(),
        )
        assertEquals(
            "au lancement, une seule page doit CHARGER (l'accueil)",
            mapOf(0 to 1),
            loads.toMap(),
        )

        rule.runOnIdle { scope!!.launch { pager!!.animateScrollToPage(1) } }
        settle()
        assertEquals("le glissement n'a pas posé la page 1", 1, pager!!.currentPage)
        assertEquals("une page quittée puis... non, ici: la page 1 n'a pas chargé", mapOf(0 to 1, 1 to 1), loads.toMap())
        assertEquals(
            "changer d'onglet a reconstruit une page (builds=$builds)",
            mapOf(0 to 1, 1 to 1, 2 to 1, 3 to 1),
            builds.toMap(),
        )

        rule.runOnIdle { scope!!.launch { pager!!.animateScrollToPage(0) } }
        settle()
        assertEquals("la page 0 a été reconstruite au retour (builds=$builds)", 1, builds[0])
        assertEquals("la page 0 a rechargé au retour (loads=$loads)", 1, loads[0])

        // Une fiche poussée : « fiche » est dessinée, la couche d'onglets reste
        // composée mais n'est plus placée (KeepComposed).
        rule.runOnIdle { navController!!.navigate(Routes.Genres) }
        settle()
        rule.onNodeWithText("fiche").assertIsDisplayed()
        rule.onNodeWithTag("page0").assertIsNotDisplayed()
        assertEquals("pousser une fiche a reconstruit une page", mapOf(0 to 1, 1 to 1, 2 to 1, 3 to 1), builds.toMap())

        rule.runOnIdle { navController!!.popBackStack() }
        settle()
        rule.onNodeWithTag("page0").assertIsDisplayed()
        assertEquals("revenir de la fiche a reconstruit la page 0", 1, builds[0])
        assertEquals("revenir de la fiche a rechargé la page 0", 1, loads[0])
    }

    /**
     * **Un onglet défilé garde son défilement** (garde du lot SB2, pas sa preuve :
     * le `SaveableStateHolder` d'avant le tenait déjà). Un aller-retour d'onglet
     * par la BARRE — la vraie porte — laisse la ligne 80 en place.
     */
    @Test
    fun aScrolledTabKeepsItsOffset() {
        arrangedShell()
        rule.onNodeWithTag("page0").performScrollToNode(hasText("ligne 80"))
        // Les quatre pages sont composées (SB2) : la ligne 80 existe dans chacune,
        // donc la lecture vise celle de la page 0 par son ancêtre.
        val row80 = hasText("ligne 80") and hasAnyAncestor(hasTestTag("page0"))
        rule.onNode(row80).assertIsDisplayed()

        rule.onNodeWithContentDescription("Rechercher").performClick()
        settle()
        assertEquals(1, pager!!.currentPage)
        rule.onNodeWithContentDescription("Accueil").performClick()
        settle()

        assertEquals("l'aller-retour n'a pas ramené l'accueil", 0, pager!!.currentPage)
        rule.onNode(row80).assertIsDisplayed()
    }

    /** The plan's own settle: frames advanced, then a bounded wait — never a sleep. */
    private fun settle() {
        rule.waitForIdle()
        rule.mainClock.advanceTimeBy(600)
        rule.waitForIdle()
    }

    /** Where `page0` is drawn, in dp: the pager's own placement, not a state read. */
    private fun pageLeft(): Float =
        rule.onNodeWithTag("page0").getUnclippedBoundsInRoot().left.value

    /** The destination the REAL router reported at the last composition. */
    private var lastDestination: NavDestination? = null

    /** Une page s'est-elle composée, et combien de fois (SB2 : doit rester 1). */
    private val builds = mutableMapOf<Int, Int>()

    /** Une page a-t-elle CHARGÉ, et combien de fois (SB2 : 1, seulement visible). */
    private val loads = mutableMapOf<Int, Int>()

    /** Le vrai routeur de l'arrangement, pour pousser une fiche et revenir. */
    private var navController: androidx.navigation.NavHostController? = null

    /** La portée du banc, pour animer le pager comme la barre le fait. */
    private var scope: kotlinx.coroutines.CoroutineScope? = null

    /**
     * L'arrangement que `NzApp` monte depuis SB2 : la couche d'onglets UNE fois,
     * Sous le `NavHost`, dont les quatre routes d'onglet ne dessinent rien. Le
     * slot `page` compte les compositions et passe la visibilité à la vraie
     * [LoadWhenShown].
     */
    private fun arrangedShell() {
        builds.clear()
        loads.clear()
        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                val nav = rememberNavController()
                navController = nav
                val entry by nav.currentBackStackEntryAsState()
                val destination = entry?.destination
                lastDestination = destination
                val state = rememberPagerState(initialPage = 0) { Tab.entries.size }
                pager = state
                val coroutineScope = rememberCoroutineScope()
                scope = coroutineScope
                fun openTab(tab: Tab) {
                    opened += tab
                    nav.navigate(routeOf(tab)) {
                        popUpTo(nav.graph.findStartDestination().id) { saveState = true }
                        launchSingleTop = true
                        restoreState = true
                    }
                }
                TabPagerSync(destination, state) { tab -> openTab(tab) }
                Column(Modifier.fillMaxSize()) {
                    Box(Modifier.weight(1f).fillMaxWidth()) {
                        val onTab = selectedIndexOf(destination) in Tab.entries.indices
                        TabLayer(
                            shown = onTab,
                            nav = FakeNav,
                            pager = state,
                            searchQuery = null,
                            libraryTab = null,
                        ) { index ->
                            // Une « construction » = une entrée en composition (lint
                            // refuse `remember` qui rend `Unit`) ; une page gardée
                            // composée n'y entre qu'une fois.
                            DisposableEffect(index) {
                                builds[index] = (builds[index] ?: 0) + 1
                                onDispose { }
                            }
                            LoadWhenShown(onTab && state.currentPage == index) {
                                loads[index] = (loads[index] ?: 0) + 1
                            }
                            ScrolledPage(index)
                        }
                        NavHost(navController = nav, startDestination = Routes.Home) {
                            composable<Routes.Home> { Unit }
                            composable<Routes.Search> { Unit }
                            composable<Routes.Library> { Unit }
                            composable<Routes.Podcasts> { Unit }
                            composable<Routes.Genres> { Text("fiche") }
                        }
                    }
                    TabBarHost(
                        destination = destination,
                        pager = state,
                        scope = coroutineScope,
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

/**
 * Une page d'onglet assez longue pour défiler, 100 lignes : le défilement doit
 * survivre à un changement d'onglet sans reconstruction.
 */
@Composable
private fun ScrolledPage(index: Int) {
    Column(
        Modifier
            .fillMaxSize()
            .testTag("page$index")
            .verticalScroll(rememberScrollState()),
    ) {
        repeat(100) { row ->
            Box(Modifier.fillMaxWidth().height(48.dp)) { Text("ligne $row") }
        }
    }
}
