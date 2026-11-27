package fr.neezer.mobile.feature.party

import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.width
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class PartyDoorTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun settle() {
        rule.waitForIdle()
        shadowOf(Looper.getMainLooper()).idle()
        rule.waitForIdle()
    }

    /** A FakeShare local copy, so this test does not depend on PartyModelTest. */
    private class FakeShare(var canCopy: Boolean = true, var canShare: Boolean = true) : PartyShare {
        override fun copy(text: String): Boolean = canCopy
        override fun share(title: String, text: String, url: String): Boolean = canShare
    }

    @Test
    fun theSheetIsNotConfinedToTheDoorsSlot() {
        val model = PartyModel(
            FakePartyService(supported = null),
            CoroutineScope(Dispatchers.Main),
            FakeShare(),
            errorText = ::testErrorText,
        )
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize()) {
                    Box(Modifier.width(48.dp)) { PartyButton(model = model) }
                }
            }
        }
        settle()
        rule.onNodeWithContentDescription(PartyCopy.DOOR).performClick()
        settle()
        val bounds = rule.onNodeWithText(PartyCopy.START).getUnclippedBoundsInRoot()
        val width = bounds.right - bounds.left
        assertTrue("button width $width < 200.dp", width >= 200.dp)
    }

    @Test
    fun aServerWithoutTheRoutesHidesTheDoor() {
        val model = PartyModel(
            FakePartyService(supported = false),
            CoroutineScope(Dispatchers.Main),
            FakeShare(),
            errorText = ::testErrorText,
        )
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, accent = null, motion = NzMotionLevel.Full) {
                Box(Modifier.fillMaxSize()) {
                    Box(Modifier.width(48.dp)) { PartyButton(model = model) }
                }
            }
        }
        settle()
        rule.onNodeWithContentDescription(PartyCopy.DOOR).assertDoesNotExist()
    }
}