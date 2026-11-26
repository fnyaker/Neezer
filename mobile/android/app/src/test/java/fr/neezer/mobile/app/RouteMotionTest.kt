package fr.neezer.mobile.app

import androidx.activity.ComponentActivity
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.Sillon
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The router's own motion (lot X1) against a minimal `NavHost`: the four
 * transitions by identity (the one piece of logic that can be wrong), and what
 * the semantics tree holds at a given frame — the only part of a transition a
 * Compose test can see.
 *
 * **The honest limit, as X0's `TransitionTest` states it.** Compose exposes no
 * API to read a RUNNING animation's spec, alpha or transform:
 * `getUnclippedBoundsInRoot` is a layout bound and a `graphicsLayer` translation
 * is invisible to it. So the CURVE is held by test 1 (exact object identity, the
 * strongest assertion available), and the WIRING by tests 2–4 (which page is
 * mounted when). Nothing here claims to "see" the 8 dp rise: the emulator
 * pictures at 50 % do that, and this worker has no `/dev/kvm` (lot rule 8 —
 * emulator proof comes from CI's `emulator` job), so those captures are owed.
 *
 * **One measured detail the plan did not spell out.** Even with
 * `EnterTransition.None`/`ExitTransition.None`, Navigation Compose keeps the
 * outgoing entry in `visibleEntries` for two frames after the target mounts
 * (measured: the target appears on frame 2, the outgoing entry is gone on
 * frame 4, at 16 ms per frame). That is NavHost's own bookkeeping, not a router
 * animation — so test 2 asserts the TARGET page is mounted exactly once and that
 * a tab swap is down to one page by frame 6, which a 200 ms enter transition
 * (the L1 regression) could not do; it does not assert a one-page frame 2 that
 * the framework itself cannot produce. Test 3 turns the same fact around: a pop's
 * 140 ms fade HOLDS both pages at frame 5, where `None` would have dropped the
 * leaving one at frame 4.
 *
 * No clock sleep: `mainClock.autoAdvance = false` where a frame is the subject,
 * and `advanceTimeBy(…)/advanceTimeByFrame()` are explicit (worker rule 6).
 */
@RunWith(RobolectricTestRunner::class)
@Config(
    sdk = [34],
    qualifiers = "w360dp-h800dp-xhdpi",
    // The real `NzApplication` loads `libnz_ffi.so` in `onCreate`; the JVM has
    // no `.so` and nothing here needs the core.
    application = android.app.Application::class,
)
class RouteMotionTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val full = NzMotion(NzMotionLevel.Full)
    private val reduced = NzMotion(NzMotionLevel.Reduced)
    private val off = NzMotion(NzMotionLevel.Off)

    private var nav: NavHostController? = null

    /** One frame at 16 ms, the Compose test's own step (`advanceTimeByFrame`). */
    private val frameMs = 16L

    /**
     * 1. The four, by identity: the only piece of logic that can be wrong, and
     *    identity is the strongest assertion Compose allows.
     *
     *    At reduced motion the fade STAYS (`!== None`, M14) and only the rise is
     *    gone — the plan's pseudo-code guarded on `!motion.moves`, which would
     *    also have silenced the opacity; M14 and this test decide (X0 did the
     *    same for `displaceOffset`).
     */
    @Test
    fun theFourTransitionsByIdentity() {
        assertSame(
            "un balayage d'onglet ne doit rien animer",
            EnterTransition.None,
            routeEnter(full, risePx = 8, tabSwap = true),
        )
        assertNotSame(
            "une poussée doit animer",
            EnterTransition.None,
            routeEnter(full, risePx = 8, tabSwap = false),
        )
        assertNotSame(
            "M14 : le fondu reste en mouvement réduit",
            EnterTransition.None,
            routeEnter(reduced, risePx = 0, tabSwap = false),
        )
        assertSame(
            "l'éco ne dessine rien",
            EnterTransition.None,
            routeEnter(off, risePx = 0, tabSwap = false),
        )
        assertSame(
            "un onglet ne s'anime pas même en mouvement réduit",
            EnterTransition.None,
            routeEnter(reduced, risePx = 0, tabSwap = true),
        )

        assertSame("M3 : aucune sortie sur une poussée", ExitTransition.None, routeExit())

        assertSame("l'éco ne dessine rien au retour", EnterTransition.None, routePopEnter(off))
        assertNotSame("un retour fondu à Full", EnterTransition.None, routePopEnter(full))
        assertNotSame(
            "M14 : le fondu du retour reste en mouvement réduit",
            EnterTransition.None,
            routePopEnter(reduced),
        )

        assertSame("l'éco ne dessine rien au retour", ExitTransition.None, routePopExit(off))
        assertNotSame("le retour sort en fondu à Full", ExitTransition.None, routePopExit(full))
        assertNotSame("le retour sort en fondu à Reduced", ExitTransition.None, routePopExit(reduced))
    }

    /**
     * 2. A tab-to-tab move at Full: the TARGET page is mounted once, and by
     *    frame 6 the shell is back to one page — `isTabSwap()` returned `None`,
     *    so the router did not run a 200 ms entrance over the pager.
     */
    @Test
    fun aTabSwapIsOnePageFromTheFrameTheTargetMountsAtFull() {
        assertTabSwapIsOnePageAt(NzMotionLevel.Full)
    }

    /** 2 (bis). The same at eco: nothing is drawn twice, at any frame. */
    @Test
    fun aTabSwapIsOnePageFromTheFrameTheTargetMountsAtOff() {
        assertTabSwapIsOnePageAt(NzMotionLevel.Off)
    }

    /**
     * 3. A pop at Full: the 140 ms fade holds both pages at frame 5 (a `None`
     *    exit would have dropped the leaving one at frame 4), then the initial
     *    one goes — the one place where a transition is visible in the tree, and
     *    therefore the proof the pop path is wired.
     */
    @Test
    fun aPopFadesBothPagesThenTheOldOneGoes() {
        host(NzMotionLevel.Full)
        rule.runOnIdle { nav!!.navigate(Routes.Album("1")) }
        rule.waitForIdle()
        // The push's own exit is None (M3): one page, at every instant.
        assertCount(1, "page-album")
        assertCount(0, "page-home")

        freeze()
        rule.runOnIdle { nav!!.popBackStack() }
        repeat(5) { advanceOneFrame() }
        assertEquals(
            "le retour doit garder les deux pages pendant son fondu de 140 ms",
            1,
            count("page-home"),
        )
        assertEquals("et la page qui s'en va est encore dessinée", 1, count("page-album"))

        rule.mainClock.advanceTimeBy(300)
        rule.waitForIdle()
        assertEquals("la page d'origine doit rester", 1, count("page-home"))
        assertEquals("la page quittée doit avoir disparu", 0, count("page-album"))
    }

    /** 4. The first composition: one marker, nothing added on the next frame. */
    @Test
    fun theFirstCompositionIsStatic() {
        host(NzMotionLevel.Full)
        assertCount(1, "page-home")
        freeze()
        advanceOneFrame()
        assertCount(1, "page-home")
        assertCount(0, "page-album")
    }

    /**
     * A tab swap: navigate Home → Search (two real pager pages) and read the
     * tree frame by frame. `isTabSwap()` is the production guard, not a copy.
     */
    private fun assertTabSwapIsOnePageAt(level: NzMotionLevel) {
        host(level)
        assertCount(1, "page-home")
        freeze()
        rule.runOnIdle { nav!!.navigate(Routes.Search()) { launchSingleTop = true } }
        var targetFrame = -1
        repeat(6) { index ->
            advanceOneFrame()
            val target = count("page-search")
            if (target > 0 && targetFrame < 0) {
                targetFrame = index + 1
                assertEquals(
                    "la page visée doit être montée UNE fois, pas deux ($level)",
                    1,
                    target,
                )
            }
        }
        assertEquals("la page visée doit être montée ($level)", 1, count("page-search"))
        assertTrue("la page visée n'a jamais été montée ($level)", targetFrame in 1..6)
        assertEquals(
            "un balayage d'onglet doit laisser UNE page après l'entrée ($level)",
            0,
            count("page-home"),
        )
    }

    /** The shell as `NavGraph` mounts it: the same four functions, the same guard. */
    private fun host(level: NzMotionLevel) {
        rule.setContent {
            NzTheme(motion = level) {
                val controller = rememberNavController()
                nav = controller
                val motion = Sillon.motion
                val risePx = with(LocalDensity.current) { motion.moveM.roundToPx() }
                NavHost(
                    navController = controller,
                    startDestination = Routes.Home,
                    enterTransition = { routeEnter(motion, risePx, tabSwap = isTabSwap()) },
                    exitTransition = { routeExit() },
                    popEnterTransition = { routePopEnter(motion) },
                    popExitTransition = { routePopExit(motion) },
                    sizeTransform = { null },
                ) {
                    composable<Routes.Home> { Marker("home") }
                    composable<Routes.Search> { Marker("search") }
                    composable<Routes.Album> { Marker("album") }
                }
            }
        }
        rule.waitForIdle()
    }

    /**
     * Stop the test clock. One frame is advanced first: under a frozen clock the
     * `NavHost`'s first back-stack collection has not applied yet, and a
     * navigation asked for before it is silently postponed (measured: the target
     * never mounted).
     */
    private fun freeze() {
        rule.mainClock.autoAdvance = false
        rule.mainClock.advanceTimeByFrame()
        rule.waitForIdle()
    }

    private fun advanceOneFrame() {
        rule.mainClock.advanceTimeBy(frameMs)
        rule.waitForIdle()
    }

    private fun count(tag: String): Int = rule.onAllNodesWithTag(tag).fetchSemanticsNodes().size

    private fun assertCount(expected: Int, tag: String) {
        rule.onAllNodesWithTag(tag).assertCountEquals(expected)
    }
}

/** A page the router can move: it fills the frame and says which one it is. */
@Composable
private fun Marker(name: String) {
    Box(Modifier.fillMaxSize().testTag("page-$name")) { Text(name) }
}
