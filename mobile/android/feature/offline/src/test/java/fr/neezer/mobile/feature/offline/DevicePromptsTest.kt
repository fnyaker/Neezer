package fr.neezer.mobile.feature.offline

import android.Manifest
import android.app.Application
import androidx.test.core.app.ApplicationProvider
import fr.neezer.mobile.bridge.DevicePrompts
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * `DevicePrompts` (M13) : la demande de `POST_NOTIFICATIONS` a une mémoire, et
 * elle ne s'invite que là où les avis manquent.
 *
 * Trois faits, trois tests : avant Android 13 la question n'existe pas, la
 * première fois demande puis la réponse est retenue, et une autorisation déjà
 * accordée ne redemande rien. Ce que la JVM ne peut pas tenir est le DIALOGUE
 * système lui-même : il se prouve sur l'émulateur (API 35), pas ici.
 *
 * Le test vit dans `:feature:offline` et non dans `:bridge` : ce module-ci a déjà
 * Robolectric et `androidx.test:core`, `:bridge` n'a que JUnit — et la classe
 * testée y reste visible par `implementation(project(":bridge"))`. C'est la
 * décision écrite dans la section « Tests » du lot.
 */
@RunWith(RobolectricTestRunner::class)
class DevicePromptsTest {

    private val context: Application get() = ApplicationProvider.getApplicationContext()

    /** Sous Android 13, `POST_NOTIFICATIONS` n'existe pas : jamais de demande. */
    @Test
    @Config(sdk = [32])
    fun aDeviceBefore33NeverAsks() {
        assertFalse(DevicePrompts.shouldAskNotifications(context))
        // Et le noter ne change rien : il n'y a rien à retenir.
        DevicePrompts.noteNotificationsAnswered(context)
        assertFalse(DevicePrompts.shouldAskNotifications(context))
    }

    /** Android 13+ : une demande, puis plus jamais (même après un refus). */
    @Test
    @Config(sdk = [34])
    fun theFirstTimeAsksThenNeverAgain() {
        assertTrue(DevicePrompts.shouldAskNotifications(context))
        DevicePrompts.noteNotificationsAnswered(context)
        assertFalse(DevicePrompts.shouldAskNotifications(context))
    }

    /** Déjà accordée : aucune question, même si rien n'a été noté. */
    @Test
    @Config(sdk = [34])
    fun aGrantedPermissionIsNotAsked() {
        shadowOf(context).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        assertFalse(DevicePrompts.shouldAskNotifications(context))
    }
}
