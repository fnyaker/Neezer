package fr.neezer.mobile.feature.nowplaying

import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.text.TextLayoutResult
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.nz_ffi.PlayState

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class PlayerHeaderOverflowTest {

    @get:Rule
    val rule = createComposeRule()

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    @Test
    fun statusLineDoesNotOverflowAt360() {
        val model = NowPlayingModel(
            player = FakePlayerPort(
                snapshot = FakePlayerPort.snapshot(
                    state = PlayState.PLAYING,
                ).copy(error = "service indisponible sur le serveur"),
            ),
            service = FakeService(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        val state = model.state.value

        var layout: TextLayoutResult? = null
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                PlayerHeader(
                    state = state,
                    model = model,
                    nav = navigator,
                    onClose = {},
                    wide = false,
                    onStatusLayout = { layout = it },
                )
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("service indisponible sur le serveur").assertExists()
        assertFalse("le statut déborde", layout?.hasVisualOverflow == true)
    }
}
EOF
cd /workspace/personal/repos/Neezer && cat > mobile/android/feature/nowplaying/src/test/java/fr/neezer/mobile/feature/nowplaying/PlayerHeaderOverflowTest.kt <<'EOF'
package fr.neezer.mobile.feature.nowplaying

import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.text.TextLayoutResult
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.nz_ffi.PlayState

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class PlayerHeaderOverflowTest {

    @get:Rule
    val rule = createComposeRule()

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    @Test
    fun statusLineDoesNotOverflowAt360() {
        val model = NowPlayingModel(
            player = FakePlayerPort(
                snapshot = FakePlayerPort.snapshot(
                    state = PlayState.PLAYING,
                ).copy(error = "service indisponible sur le serveur"),
            ),
            service = FakeService(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        val state = model.state.value

        var layout: TextLayoutResult? = null
        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Full) {
                PlayerHeader(
                    state = state,
                    model = model,
                    nav = navigator,
                    onClose = {},
                    wide = false,
                    onStatusLayout = { layout = it },
                )
            }
        }
        rule.waitForIdle()
        rule.onNodeWithText("service indisponible sur le serveur").assertExists()
        assertFalse("le statut déborde", layout?.hasVisualOverflow == true)
    }
}
EOF
echo done