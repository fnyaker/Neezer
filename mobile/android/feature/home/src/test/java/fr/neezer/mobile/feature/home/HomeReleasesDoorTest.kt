package fr.neezer.mobile.feature.home

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * « Tout afficher » de la carte Nouveautés — the SPA's door on the home's shelf
 * header (`webapp/src/routes/Home.svelte:252`, `#/releases`).
 *
 * The screen is the REAL [HomeScreen] over [HomeFixtures], the same records the
 * MB5 screenshots draw, so the assertion is on what a person taps: a clickable
 * node whose text is « Tout afficher », and the one callback it calls.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class HomeReleasesDoorTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun theShelfHeaderOpensReleases() {
        var opened = 0
        rule.setContent {
            NzTheme {
                Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                    HomeScreen(
                        state = HomeModel.State(
                            screen = ScreenState.Ready(HomeFixtures.home()),
                            styles = HomeFixtures.styles(),
                            chosen = listOf("Techno"),
                            flow = HomeFixtures.flow(),
                        ),
                        actions = HomeActions(openReleases = { opened++ }),
                    )
                }
            }
        }

        rule.onNode(hasText("Tout afficher") and hasClickAction())
            .performScrollTo()
            .performClick()

        assertEquals("« Tout afficher » ouvre la page Nouveautés", 1, opened)
    }
}
