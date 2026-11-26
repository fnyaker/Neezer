package fr.neezer.mobile.feature.nowplaying

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * **L'en-tête du lecteur est à 2 dp sous l'inset** (SB3, `spec-mobile-shell-bugs.md`
 * §SB3).
 *
 * Robolectric ne connaît pas d'inset (il lit 0), donc ce test ne prouve pas le
 * doublon qu'il y avait — c'est l'émulateur qui le lit (`ShellSmokeTest`,
 * `thePlayerSitsUnderTheStatusBarAndCoversTheBar`). Ce qu'il garde est le JETON :
 * 2 dp, la valeur de M3 pour une barre haute, et pas les 4 dp d'avant. Une
 * régression à `NzSpacing.xs` le fait échouer.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w360dp-h800dp-xhdpi")
class PlayerInsetsTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private val navigator = object : NzNavigator {
        override fun go(link: uniffi.nz_ffi.AppLink) = Unit
        override fun back() = Unit
        override fun openPlayer() = Unit
        override fun openQueue() = Unit
    }

    /**
     * Le bord haut de « Réduire le lecteur » : 2 dp avec un inset de 0.
     *
     * L'en-tête est la première rangée de `PlayerColumn` ; son chevron porte la
     * description que TalkBack lit, donc la cible est trouvée sans coordonnée.
     */
    @Test
    fun theCollapseButtonSitsTwoDpUnderTheStatusBarInset() {
        val port = FakePlayerPort()
        val model = NowPlayingModel(
            player = port,
            service = FakeService(),
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
        )
        model.refresh()
        val state = model.state.value

        rule.setContent {
            NzTheme(motion = NzMotionLevel.Full) {
                PlayerSurface(
                    state = state,
                    model = model,
                    nav = navigator,
                    lyrics = null,
                    about = null,
                    onClose = {},
                    liveReads = false,
                )
            }
        }
        rule.waitForIdle()

        val collapse = rule.onNodeWithContentDescription("Réduire le lecteur")
            .getUnclippedBoundsInRoot()
        println(
            "MEASURE sb3 header top: ${collapse.top.value} dp, " +
                "height ${(collapse.bottom - collapse.top).value} dp (attendu 2 dp, inset 0)",
        )
        assertEquals(
            "l'en-tête ne se colle pas à 2 dp sous la barre d'état",
            2.dp.value,
            collapse.top.value,
            0.5f,
        )
    }
}
