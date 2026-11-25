package fr.neezer.mobile.app

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerDefaults
import androidx.compose.foundation.pager.PagerSnapDistance
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.navigation.NavDestination
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzTabBar
import fr.neezer.mobile.designsystem.NzTabItem
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.feature.home.HomeEntry
import fr.neezer.mobile.feature.library.LibraryEntry
import fr.neezer.mobile.feature.podcasts.PodcastsEntry
import fr.neezer.mobile.feature.search.SearchEntry
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * The phone shell's four pageable destinations (docs/agents/mobile.md,
 * *Navigation, screens and the UI*; the plan's L1).
 *
 * [page] is the item's place in the pager — one page per tab, in the bar's order,
 * so « a page settled » and « the route the person is on » are the same fact from
 * two sides.
 */
enum class Tab(val label: String, val glyph: NzGlyph, val page: Int) {
    Accueil("Accueil", NzGlyph.Home, 0),
    Rechercher("Rechercher", NzGlyph.Compass, 1),
    Bibliotheque("Biblio", NzGlyph.Library, 2),
    Podcasts("Podcasts", NzGlyph.Mic, 3),
}

/** « Réglages » is the bar's fifth item and is NOT a page: it is a pushed route. */
const val SETTINGS_ITEM = 4

/**
 * The five items of the bar: the four pages, then the Réglages door. `Biblio` is
 * the SPA's own word (`MobileNav.svelte:63`), not « Bibliothèque » — five items on
 * a 360 dp bar are 72 dp each, and the long word does not fit.
 */
val TAB_ITEMS: List<NzTabItem> =
    Tab.entries.map { NzTabItem(it.glyph, it.label) } + NzTabItem(NzGlyph.Settings, "Réglages")

/** The tab a pager page is, or `null` for a page outside the four. */
fun tabOfPage(page: Int): Tab? = Tab.entries.getOrNull(page)

/** The route a tab stands for, as a `NavDestination.route` spells it. */
private fun Tab.routeName(): String? = when (this) {
    Tab.Accueil -> Routes.Home::class.qualifiedName
    Tab.Rechercher -> Routes.Search::class.qualifiedName
    Tab.Bibliotheque -> Routes.Library::class.qualifiedName
    Tab.Podcasts -> Routes.Podcasts::class.qualifiedName
}

/**
 * Whether the shown destination IS [tab] (so the bar highlights it). A tab's
 * routes carry arguments (`Search?q=…`), so the test is a prefix, never equality.
 */
fun NavDestination?.isOn(tab: Tab): Boolean {
    val name = this?.route ?: return false
    val route = tab.routeName() ?: return false
    return name.startsWith(route)
}

private fun NavDestination?.isSettings(): Boolean {
    val name = this?.route ?: return false
    return name.startsWith(Routes.Settings::class.qualifiedName ?: return false)
}

/**
 * The bar's selection: the page of the tab the person is on, [SETTINGS_ITEM] for
 * Réglages, and `-1` for any other route (a detail screen, the setup, the login) —
 * where no item is selected, exactly as the SPA's `MobileNav` highlights nothing
 * on a detail page.
 */
fun selectedIndexOf(destination: NavDestination?): Int =
    Tab.entries.firstOrNull { destination.isOn(it) }?.page
        ?: if (destination.isSettings()) SETTINGS_ITEM else -1

/**
 * What a page of the pager draws: the tab's own entry, in place of the four
 * separate `composable<…>` bodies the shell used to hold. The `when` is over the
 * enum, so a fifth tab is a compile error here rather than a page nobody draws.
 */
@Composable
fun TabPage(
    page: Int,
    nav: NzNavigator,
    searchQuery: String? = null,
    libraryTab: String? = null,
) {
    when (tabOfPage(page)) {
        Tab.Accueil -> HomeEntry(nav)
        Tab.Rechercher -> SearchEntry(searchQuery, nav)
        Tab.Bibliotheque -> LibraryEntry(libraryTab, nav)
        Tab.Podcasts -> PodcastsEntry(nav)
        null -> Unit
    }
}

/**
 * The four tab roots in ONE pager, so a horizontal drag moves the screens with the
 * finger, and a tap on the bar animates the same page instead of replacing the
 * whole content (two animations fighting each other was the "it resets after the
 * gesture" feeling on Android).
 *
 * The settle is `PagerDefaults`' own fling behaviour with the plan's numbers (§3):
 * `snapPositionalThreshold = 0.28f` — released past 28 % of the width the next
 * page wins even with no velocity — and `spring(1f, StiffnessMediumLow)` from the
 * release velocity. Under eco or reduced motion the settle lands at once
 * (`transitions.md` M14): the finger still moves the page, nothing animates after
 * it.
 *
 * `beyondViewportPageCount` is 1 only WHILE the finger is down: the neighbouring
 * page is composed from the first pixel of the drag (nothing appears half-way
 * through), and back to 0 on release so a settled shell holds one page. That is
 * the one extra compose this lot adds, and it costs a recomposition of the
 * neighbour — no request (each tab's read is its own, on the `User` lane, once;
 * `NzCore::stats()` counts them).
 *
 * @param page the page's content. The shell passes nothing and [TabPage] draws
 *   the real entry; a JVM test passes a text node, because those entries need the
 *   Rust core.
 */
@Composable
fun TabPager(
    nav: NzNavigator,
    pager: PagerState,
    searchQuery: String? = null,
    libraryTab: String? = null,
    page: (@Composable (Int) -> Unit)? = null,
) {
    val motion = Sillon.motion
    HorizontalPager(
        state = pager,
        modifier = Modifier.fillMaxSize(),
        beyondViewportPageCount = if (pager.isScrollInProgress) 1 else 0,
        flingBehavior = PagerDefaults.flingBehavior(
            state = pager,
            pagerSnapDistance = PagerSnapDistance.atMost(1),
            snapAnimationSpec = if (motion.moves) {
                spring(dampingRatio = Spring.DampingRatioNoBouncy, stiffness = Spring.StiffnessMediumLow)
            } else {
                snap()
            },
            snapPositionalThreshold = 0.28f,
        ),
    ) { index ->
        if (page != null) page(index) else TabPage(index, nav, searchQuery, libraryTab)
    }
}

/**
 * The one place a page and a route agree (the plan's L1):
 *
 *  * a route that changed from OUTSIDE the pager (a deep link, a back, a tap on
 *    the bar from a route that is not a page) scrolls the pager to its page,
 *    without animation — the person did not drag, so nothing should slide;
 *  * a page the finger settled on opens its tab, once.
 *
 * `pagerDriven` is what keeps the two from echoing: the route change a settle
 * CAUSED must not scroll the pager back, and the scroll a route caused must not
 * open the tab it came from. It is one flag because the two effects run in order,
 * in the same composition.
 *
 * A shell that is NOT on a page (Setup, Login, a detail route, Réglages) owns
 * nothing here: its settled page is whatever the person left behind, and acting
 * on it would open a tab nobody asked for — a fresh install would skip its own
 * setup screen.
 */
@Composable
fun TabPagerSync(
    destination: NavDestination?,
    pager: PagerState,
    onOpenTab: (Tab) -> Unit,
) {
    var pagerDriven by remember { mutableStateOf(false) }
    LaunchedEffect(destination) {
        if (pagerDriven) {
            pagerDriven = false
            return@LaunchedEffect
        }
        val page = selectedIndexOf(destination)
        if (page in Tab.entries.indices && page != pager.currentPage) pager.scrollToPage(page)
    }
    LaunchedEffect(pager.settledPage) {
        if (selectedIndexOf(destination) !in Tab.entries.indices) return@LaunchedEffect
        val tab = tabOfPage(pager.settledPage) ?: return@LaunchedEffect
        if (!destination.isOn(tab)) {
            pagerDriven = true
            onOpenTab(tab)
        }
    }
}

/**
 * The bar and what an index means.
 *
 *  * [SETTINGS_ITEM] pushes Réglages (single top): the SPA's fifth item, the door
 *    MB0's top bar held before this lot.
 *  * Any other index animates the pager — but ONLY while the shell is on a page.
 *    From Setup, Login, a detail screen or Réglages the pager is not what the
 *    person is looking at, so the tap is a route change ([onOpenTab]) and
 *    [TabPagerSync] scrolls the page afterwards.
 */
@Composable
fun TabBarHost(
    destination: NavDestination?,
    pager: PagerState,
    scope: CoroutineScope,
    onOpenTab: (Tab) -> Unit,
    onOpenSettings: () -> Unit,
) {
    val selected = selectedIndexOf(destination)
    NzTabBar(
        items = TAB_ITEMS,
        selected = selected,
        onPick = { index ->
            val tab = tabOfPage(index)
            when {
                tab == null -> onOpenSettings()
                selected !in Tab.entries.indices -> onOpenTab(tab)
                else -> scope.launch { pager.animateScrollToPage(tab.page) }
            }
        },
    )
}

/**
 * The back row of a screen that is not a tab: ONE 48 dp glyph on the start edge,
 * and nothing else — no title, no background, no Réglages action. It replaces the
 * top bar the shell used to draw, so a detail screen is still reachable from under
 * the status bar without a banner over the content.
 *
 * Its own inset is zero: the `Scaffold`'s content padding already carries the
 * status bar, and adding it twice is the 24 dp of air that made the old bar look
 * detached.
 */
@Composable
fun DetailBackRow(onBack: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth().height(56.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Start,
    ) {
        NzIconButton(
            glyph = NzGlyph.ChevronLeft,
            contentDescription = "Retour",
            onClick = onBack,
        )
    }
}
