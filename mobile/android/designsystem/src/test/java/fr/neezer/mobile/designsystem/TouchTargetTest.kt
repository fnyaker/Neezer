package fr.neezer.mobile.designsystem

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Column
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.assertWidthIsAtLeast
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Every interactive component exposes a **≥ 48 dp** target, measured on the
 * composed semantics node — not read off a constant (the MB1 acceptance asks for
 * a semantics test, and `webapp.md` §10 asks for 44 px on the web; a finger on a
 * moving bus needs 48).
 *
 * The sizes come from the layout, so a component that grows a `padding` or loses
 * a `sizeIn` fails here: `assertHeightIsAtLeast(48.dp)` fails on a node the
 * layout never laid out, and `hasClickAction` is what makes "interactive" mean
 * what a finger feels.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class TouchTargetTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val target = 48.dp

    @Test
    fun everyButtonKindIsBigEnough() {
        rule.setContent {
            NzTheme {
                Column {
                    NzButton(text = "Lire", onClick = {})
                    NzButton(text = "Mélanger", onClick = {}, kind = NzButtonKind.Secondary)
                    NzButton(text = "Télécharger", onClick = {}, kind = NzButtonKind.Ghost)
                }
            }
        }
        listOf("Lire", "Mélanger", "Télécharger").forEach { label ->
            rule.onNodeWithText(label).assertHeightIsAtLeast(target)
            rule.onNodeWithText(label).assertWidthIsAtLeast(target)
        }
    }

    @Test
    fun anIconButtonIs48Square() {
        rule.setContent { NzTheme { NzIconButton(NzGlyph.Search, "Rechercher", onClick = {}) } }
        rule.onNodeWithContentDescription("Rechercher").assertHeightIsAtLeast(target)
        rule.onNodeWithContentDescription("Rechercher").assertWidthIsAtLeast(target)
    }

    @Test
    fun aChipIsBigEnough() {
        rule.setContent { NzTheme { NzChip(text = "Hors-ligne", onClick = {}) } }
        rule.onNodeWithText("Hors-ligne").assertHeightIsAtLeast(target)
        rule.onNodeWithText("Hors-ligne").assertWidthIsAtLeast(target)
    }

    @Test
    fun everySegmentIsBigEnough() {
        rule.setContent {
            NzTheme { NzSegmented(options = listOf("Musique", "Podcasts"), selectedIndex = 0, onSelect = {}) }
        }
        rule.onNodeWithText("Musique").assertHeightIsAtLeast(target)
        rule.onNodeWithText("Podcasts").assertHeightIsAtLeast(target)
    }

    @Test
    fun aSwitchIsBigEnough() {
        rule.setContent { NzTheme { NzSwitch(checked = true, onCheckedChange = {}, label = "Animations") } }
        rule.onNodeWithText("Animations").assertHeightIsAtLeast(target)
        rule.onNodeWithText("Animations").assertWidthIsAtLeast(target)
    }

    @Test
    fun aFaderTakesAThumb() {
        rule.setContent { NzTheme { NzFader(value = 0.5f, onValueChange = {}, label = "Grave") } }
        rule.onNodeWithContentDescription("Grave").assertHeightIsAtLeast(target)
        rule.onNodeWithContentDescription("Grave").assertWidthIsAtLeast(target)
    }

    @Test
    fun aTrackRowAndItsMoreButtonAreBigEnough() {
        rule.setContent {
            NzTheme {
                TrackRowView(
                    row = nzDemoRow(1),
                    playing = false,
                    onClick = {},
                    onMore = {},
                    onSwipeQueue = {},
                )
            }
        }
        rule.onNodeWithText("Titre 1").assertHeightIsAtLeast(target)
        rule.onNodeWithContentDescription("Plus d'actions pour Titre 1").assertHeightIsAtLeast(target)
        rule.onNodeWithContentDescription("Plus d'actions pour Titre 1").assertWidthIsAtLeast(target)
    }

    @Test
    fun theRetryOfAFailedLoadIsBigEnough() {
        rule.setContent { NzTheme { LoadFailed(cause = "délai dépassé", onRetry = {}) } }
        rule.onNodeWithText("Réessayer").assertHeightIsAtLeast(target)
    }

    @Test
    fun aToastActionIsBigEnough() {
        NzToasts.clear()
        NzToasts.show("Échec de l'envoi", actionLabel = "Réessayer", onAction = {})
        rule.setContent { NzTheme { NzToastHost() } }
        rule.onNodeWithText("Réessayer").assertHeightIsAtLeast(target)
        NzToasts.clear()
    }
}
