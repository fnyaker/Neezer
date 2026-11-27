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
import androidx.compose.animation.AnimatedContentTransitionScope
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavDestination
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzEase
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzIconButton
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
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
 * A push: the SPA's `pageEnter` — a fade plus 8 dp of rise over 200 ms.
 * NOTHING for a sweep between the four pager pages (the pager is already moving,
 * L1's reason kept as a guard) nor for an eco device (M13).
 *
 * **Reduced motion keeps the fade and drops only the rise** (M14): the plan's
 * pseudo-code guarded on `!motion.moves`, which is true at reduced too and would
 * have silenced the opacity the rule keeps; the acceptance test
 * (`routeEnter(reduced) !== None`) and M14 decide, as X0 already did for
 * `displaceOffset`. The 0 dp rise is the `moveM` token itself.
 *
 * The slide's spec is an `IntOffset` tween, not `motion.base()` (a `Float`
 * spec does not compile into `slideInVertically`); `displaceOffset` is the token
 * that carries the enter curve and is 0 at reduced.
 */
fun routeEnter(motion: NzMotion, risePx: Int, tabSwap: Boolean): EnterTransition = when {
    tabSwap || motion.level == NzMotionLevel.Off -> EnterTransition.None
    !motion.moves -> fadeIn(motion.base())
    else -> fadeIn(motion.base()) +
        slideInVertically(motion.displaceOffset(NzDur.Base, NzEase.Enter)) { risePx }
}

/** M3: on a push the old page leaves in the same frame — no exit to draw. */
fun routeExit(): ExitTransition = ExitTransition.None

/** A pop: the SPA's traversal — a 140 ms fade, nothing else (opacity kept at reduced, M14). */
fun routePopEnter(motion: NzMotion): EnterTransition =
    if (motion.level == NzMotionLevel.Off) EnterTransition.None else fadeIn(motion.fast())

fun routePopExit(motion: NzMotion): ExitTransition =
    if (motion.level == NzMotionLevel.Off) ExitTransition.None else fadeOut(motion.fast())

/**
 * The router's own test: a move between two of the four pager pages is the
 * PAGER's motion (L1), so the router draws nothing; Réglages and every detail
 * route are a push. Public so `RouteMotionTest` exercises the production guard
 * instead of a copy of it.
 */
fun AnimatedContentTransitionScope<NavBackStackEntry>.isTabSwap(): Boolean =
    selectedIndexOf(initialState.destination) in Tab.entries.indices &&
        selectedIndexOf(targetState.destination) in Tab.entries.indices

/**
 * What a page of the pager draws: the tab's own entry, in place of the four
 * separate `composable<…>` bodies the shell used to hold. The `when` is over the
 * enum, so a fifth tab is a compile error here rather than a page nobody draws.
 */
@Composable
fun TabPage(
    page: Int,
    nav: NzNavigator,
    visible: Boolean,
    searchQuery: String? = null,
    libraryTab: String? = null,
) {
    when (tabOfPage(page)) {
        Tab.Accueil -> HomeEntry(nav, visible = visible)
        Tab.Rechercher -> SearchEntry(searchQuery, nav, visible = visible)
        Tab.Bibliotheque -> LibraryEntry(libraryTab, nav, visible = visible)
        Tab.Podcasts -> PodcastsEntry(nav, visible = visible)
        null -> Unit
    }
}

/**
 * Les quatre racines d'onglet dans UN pager, pour qu'un glissement horizontal
 * emmène les écrans avec le doigt et qu'un tap sur la barre anime la même page au
 * lieu de remplacer tout le contenu (deux animations qui se battent : « ça se
 * remet à zéro après le geste » sur Android).
 *
 * Le règlement est le `PagerDefaults` du plan (§3) : `snapPositionalThreshold =
 * 0.28f` — relâché au-delà de 28 % de la largeur la page suivante gagne, même
 * sans vitesse — et `spring(1f, StiffnessMediumLow)` depuis la vitesse du
 * relâchement. Sous éco ou mouvement réduit le règlement se pose d'un coup
 * (`transitions.md` M14) : le doigt déplace toujours la page, rien n'anime après.
 *
 * **Les quatre pages restent composées** (`beyondViewportPageCount = 3`, lot
 * SB2) : la personne ne veut pas voir un onglet se reconstruire, donc un onglet
 * revenu retrouve son modèle, son défilement et ses données sans une requête. Ce
 * que ça coûte : trois compositions de plus au lancement, mesurées dans
 * l'acceptation ; une page qu'on n'a pas encore visitée ne CHARGE rien
 * (`LoadWhenShown`, la porte des quatre entrées).
 *
 * @param shown `false` quand la personne n'est pas sur une racine d'onglet (une
 *   fiche, Réglages, la mise en route) : les pages restent composées mais aucune
 *   n'est visible, donc aucune ne charge ni ne mesure sa mise en page
 *   ([KeepComposed]).
 * @param page the page's content. The shell passes nothing and [TabPage] draws
 *   the real entry; a JVM test passes a text node, because those entries need the
 *   Rust core.
 */
@Composable
fun TabPager(
    nav: NzNavigator,
    pager: PagerState,
    shown: Boolean = true,
    searchQuery: String? = null,
    libraryTab: String? = null,
    page: (@Composable (Int) -> Unit)? = null,
) {
    val motion = Sillon.motion
    HorizontalPager(
        state = pager,
        modifier = Modifier.fillMaxSize(),
        beyondViewportPageCount = Tab.entries.size - 1,
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
        if (page != null) {
            page(index)
        } else {
            TabPage(index, nav, shown && pager.currentPage == index, searchQuery, libraryTab)
        }
    }
}

/**
 * Garde [content] composé, mais ne le mesure ni ne le place quand [shown] est
 * faux : rien n'est dessiné, aucun toucher ne l'atteint, et sa composition
 * (modèles, `remember`, défilement) survit. C'est ce qui permet au pager des
 * onglets de vivre sous le `NavHost` sans se montrer derrière une fiche ou
 * Réglages (lot SB2).
 *
 * **Cachée, la couche sort aussi de l'arbre d'accessibilité** : `clearAndSetSemantics`
 * retire tout son sous-arbre de la sémantique, donc TalkBack n'annonce pas un
 * écran qu'on ne voit pas, et un test qui cherche « la première zone qui
 * défile » ne tombe pas sur le pager des onglets caché derrière Réglages
 * (mesuré : `ShellSmokeTest.settingsShowsTheRustVersion` et `CrashCardTest`
 * échouaient sans ça, sur l'émulateur).
 */
@Composable
fun KeepComposed(shown: Boolean, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val semantics = if (shown) Modifier else Modifier.clearAndSetSemantics { }
    Layout(content = content, modifier = modifier.fillMaxSize().then(semantics)) { measurables, constraints ->
        val placeables = if (shown) measurables.map { it.measure(constraints) } else emptyList()
        layout(constraints.maxWidth, constraints.maxHeight) {
            placeables.forEach { it.place(0, 0) }
        }
    }
}

/**
 * Les quatre racines d'onglet, montées UNE fois pour toute l'app, sous le
 * `NavHost` (lot SB2). Les routes d'onglet du graphe ne dessinent rien : elles
 * disent seulement où l'on est. Quand la destination n'est pas un onglet
 * (fiche, Réglages, Setup), la couche est cachée mais reste composée.
 */
@Composable
fun TabLayer(
    shown: Boolean,
    nav: NzNavigator,
    pager: PagerState,
    searchQuery: String?,
    libraryTab: String?,
    page: (@Composable (Int) -> Unit)? = null,
) {
    KeepComposed(shown) {
        TabPager(nav, pager, shown, searchQuery, libraryTab, page)
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
