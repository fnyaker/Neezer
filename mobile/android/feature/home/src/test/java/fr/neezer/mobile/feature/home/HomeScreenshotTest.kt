package fr.neezer.mobile.feature.home

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.testkit.ScreenGoldenRule
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The home, rendered and VERIFIED: the hero and two shelves with long French
 * titles, at 360, 412 and 600 dp and at the 1.0 and 1.3 font scales — the
 * plan's exact golden matrix.
 *
 * T3 replaced MB5's write-only captures with these goldens: the image is the
 * contract, so a hero that loses its margin or a shelf whose long title clips
 * fails the suite instead of being reviewed by hope. Robolectric's NATIVE
 * graphics backend draws it with the real fonts on the JVM (no emulator, no
 * window — worker rule 7). S2's own margin/rhythm assertions stay below: T3 adds
 * the picture, it does not duplicate the geometry S2 already proves.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class HomeGoldenTest {

    @get:Rule
    val rule = ScreenGoldenRule()

    @Test
    fun heroAndShelvesAt360() = shot(360, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi", fontScale = 1.3f)
    fun heroAndShelvesAt360At130Percent() = shot(360, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h900dp-xhdpi")
    fun heroAndShelvesAt412() = shot(412, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w412dp-h900dp-xhdpi", fontScale = 1.3f)
    fun heroAndShelvesAt412At130Percent() = shot(412, 1.3f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi")
    fun heroAndShelvesAt600() = shot(600, 1.0f)

    @Test
    @Config(sdk = [34], qualifiers = "w600dp-h900dp-xhdpi", fontScale = 1.3f)
    fun heroAndShelvesAt600At130Percent() = shot(600, 1.3f)

    private fun shot(width: Int, scale: Float) {
        render(defaultState(), scale)
        // The 1.3 proof (plan T3): the hero's own text height is printed at both
        // scales, so the PR can show 1.3 is 1.3× the 1.0 line — not a re-crop.
        val titlePx = rule.composeRule
            .onNodeWithText("BONJOUR", substring = true)
            .fetchSemanticsNode()
            .boundsInRoot
            .height
        println(
            "MEASURE t3 echelle: width=${width}dp fontScale=$scale titreHeightPx=$titlePx",
        )
        rule.capture("home-accueil-heros-${width}dp-f$scale.png")
    }

    private fun render(state: HomeModel.State = defaultState(), fontScale: Float = 1f) {
        rule.setContent(fontScale) {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                        HomeScreen(state = state, actions = HomeActions())
                    }
                }
            }
        }
    }

    private fun defaultState(): HomeModel.State = HomeModel.State(
        screen = ScreenState.Ready(HomeFixtures.home()),
        styles = HomeFixtures.styles(),
        chosen = listOf("Techno"),
        flow = HomeFixtures.flow(),
    )

    /**
     * S2 : la marge de page (20 dp) et le rythme des sections (32 dp), lus dans
     * l'arbre de sémantique — la marge sur le héros, l'écart entre le héros et la
     * première section qui le suit.
     */
    @Test
    fun thePageMarginsAndSectionRhythm() {
        render()
        rule.composeRule.waitForIdle()
        val hero = rule.composeRule.onNodeWithTag(HOME_HERO_TAG).getUnclippedBoundsInRoot()
        val resume = rule.composeRule.onNodeWithTag(HOME_RESUME_TAG).getUnclippedBoundsInRoot()
        println(
            "MEASURE s2 accueil: hero.top=${hero.top.value}dp " +
                "hero.bottom=${hero.bottom.value}dp resume.top=${resume.top.value}dp",
        )
        assertEquals("la marge haute de la page est celle du SPA (20 dp)", 20f, hero.top.value, 1f)
        assertEquals("deux sections sont à 32 dp", 32f, resume.top.value - hero.bottom.value, 1f)
    }
}
