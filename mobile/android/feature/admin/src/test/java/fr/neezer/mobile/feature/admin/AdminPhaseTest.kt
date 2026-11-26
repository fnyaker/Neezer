package fr.neezer.mobile.feature.admin

import androidx.activity.ComponentActivity
import androidx.compose.animation.core.TweenSpec
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import fr.neezer.mobile.designsystem.NzDur
import fr.neezer.mobile.designsystem.NzMotion
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.designsystem.inPlaceSpecs
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * La page *Utilisateurs* change de phase en fondu (lot X6) : « Chargement des
 * comptes… » laisse la place au tableau **dans la même image** en éco, et la
 * cause d'une lecture échouée est une phase à part (elle n'a ni tête ni filtres).
 *
 * **La ligne de « job » du plan n'a pas de seam ici.** `AdminSections.kt` ne
 * dessine aucun `LazyColumn` (le parent Réglages défile déjà ; les tableaux de
 * l'admin sont hors portée) : un `animateItem` n'y a pas de `LazyItemScope`. Ce
 * fichier tient donc la phase, qui EST le changement d'état de la section.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w360dp-h900dp-xhdpi")
class AdminPhaseTest {

    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    private fun model(fake: FakeAdminService): AdminModel = AdminModel(
        service = fake,
        scope = CoroutineScope(Dispatchers.Unconfined),
        io = Dispatchers.Unconfined,
        toast = {},
    )

    @Test
    fun theUsersPhaseReplacesInTheSameFrameAtEco() {
        val fake = FakeAdminService()
        // No invitation: the loading hint is the phase only while no ROW at all is
        // there (the screen's own rule), and an invite would be one.
        fake.invites.clear()
        fake.usersGate = CompletableDeferred()
        val model = model(fake)
        model.open(AdminModel.SECTION_USERS)
        assertEquals("the page starts on its loading phase", ADMIN_USERS_LOADING, adminUsersPhase(model.state))
        assertEquals("admin is read from the session", true, model.state.admin)

        rule.setContent {
            NzTheme(kind = NzThemeKind.Encre, motion = NzMotionLevel.Off) {
                Box(Modifier.fillMaxSize().background(Sillon.colors.bg)) {
                    AdminPage(section = AdminModel.SECTION_USERS, model = model)
                }
            }
        }
        rule.mainClock.advanceTimeBy(250)
        rule.waitForIdle()
        // Two nodes say it: the count line and the phase's hint (the screen's own
        // wording, unchanged by this lot).
        rule.onAllNodesWithText("Chargement des comptes…").assertCountEquals(2)
        rule.onNodeWithText("Léo").assertDoesNotExist()

        fake.usersGate?.complete(Unit)
        rule.waitForIdle()
        rule.onNodeWithText("Léo").assertExists()
        rule.onNodeWithText("Chargement des comptes…").assertDoesNotExist()

        val full = inPlaceSpecs(NzMotion(NzMotionLevel.Full))
        assertEquals(NzDur.Fast, (full.fade as TweenSpec<*>).durationMillis)
        assertEquals(NzDur.Fast, (full.move as TweenSpec<*>).durationMillis)
        assertNull(full.exit)
        assertEquals(0, (inPlaceSpecs(NzMotion(NzMotionLevel.Off)).fade as TweenSpec<*>).durationMillis)
    }

    @Test
    fun thePhaseFunctionReadsTheFourStates() {
        assertEquals(
            ADMIN_USERS_FAILED,
            adminUsersPhase(AdminModel.State(usersFailure = "Le serveur n'a pas répondu.")),
        )
        assertEquals(
            ADMIN_USERS_LOADING,
            adminUsersPhase(AdminModel.State(usersLoading = true)),
        )
        assertEquals(ADMIN_USERS_EMPTY, adminUsersPhase(AdminModel.State(rows = emptyList())))
        assertEquals(
            ADMIN_USERS_LIST,
            adminUsersPhase(AdminModel.State(rows = listOf(AccountRow.User(AdminFixtures.user())))),
        )
    }
}
