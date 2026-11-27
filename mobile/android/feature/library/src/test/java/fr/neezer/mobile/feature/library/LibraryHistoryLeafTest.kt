package fr.neezer.mobile.feature.library

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.assertIsDisplayed
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.designsystem.NzTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.time.LocalDate

/**
 * The « Historique » leaf of the library opens without crashing the app.
 *
 * `HistoryScreen` is a `LazyColumn` of its own. Drawn as an `item` of the
 * library's `LazyColumn` (the leaf's body), it was measured with an unbounded
 * height and Compose threw « Vertically scrollable component was measured with
 * an infinity maximum height constraints » — the app died the moment the leaf
 * opened. The leaf must be a sibling the screen lays out, not a nested list.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class LibraryHistoryLeafTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun historyState(): HistoryModel.State {
        val page = LibraryFixtures.page()
        return HistoryModel.State(
            screen = ScreenState.Ready(page),
            events = page.events,
            summary = page.summary,
            next = page.next,
        )
    }

    @Test
    fun theHistoryLeafOpensAndDrawsItsDaysInsteadOfCrashing() {
        val state = LibraryModel.State(
            leaf = LibraryLeaf.History,
            foldersSupported = true,
        )
        val history = historyState()
        rule.setContent {
            NzTheme {
                Box(Modifier.fillMaxSize()) {
                    LibraryScreen(
                        state = state,
                        actions = LibraryActions(
                            historyItems = {
                                rememberHistoryItems(
                                    state = history,
                                    actions = HistoryActions(),
                                    showHead = false,
                                    today = LocalDate.of(2026, 11, 18),
                                )
                            },
                        ),
                    )
                }
            }
        }
        rule.waitForIdle()
        rule.onNodeWithTag(LIBRARY_LIST_TAG).assertIsDisplayed()
        rule.onNodeWithText("Hier").assertIsDisplayed()
    }
}
