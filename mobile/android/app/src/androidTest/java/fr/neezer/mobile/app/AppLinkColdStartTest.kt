package fr.neezer.mobile.app

import android.content.Intent
import android.net.Uri
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import fr.neezer.mobile.feature.playlist.PLAYLIST_MARKER
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A shared playlist link opens the playlist page instead of closing the app.
 *
 * The crash this test was written for (measured 2026-11-18, MB9's headless
 * capture): `adb shell am start -a android.intent.action.VIEW -d
 * "neezer://playlist/<id>"` on a STOPPED app died with
 * `java.lang.IllegalStateException: You must call setGraph() before calling
 * getGraph()` — `MainActivity.onCreate` handed the intent to `AppGraph`, which
 * navigated the moment it had a controller, before the `NavHost` that composes
 * afterwards had set its graph. A person tapping a playlist link in a chat app
 * watched the app disappear.
 *
 * It is an INSTRUMENTED test because the bug lived in the Activity's first
 * composition: the Intent is the one that starts the activity, so the pending
 * link and the missing graph are the real ones. The device is the emulator
 * `mobile/tools/ci-emulator.sh` starts.
 *
 * Both directions are asserted:
 *
 * - the link that STARTED the activity (`ActivityScenario.launch(intent)`);
 * - the link that arrives while it runs (`onNewIntent`, the warm door).
 *
 * A screen that cannot load its data is fine here — what is asserted is that the
 * PLAYLIST route is the current one and that the app is still alive; the page's
 * own content needs a configured server, which this test does not assume.
 */
@RunWith(AndroidJUnit4::class)
class AppLinkColdStartTest {

    private val device: UiDevice
        get() = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    /**
     * A `neezer://playlist/<id>` view for [MainActivity].
     *
     * [cold] is the intent a chat app's tap builds for a stopped app; the warm
     * door is the same intent WITHOUT `CLEAR_TASK` (clearing the task would tear
     * the running activity down, which is not what "a link arrives while the app
     * runs" means — measured: the first version of this test asserted the
     * scenario was RESUMED and failed on exactly that).
     */
    private fun playlistIntent(id: String, cold: Boolean = true): Intent {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        return Intent(Intent.ACTION_VIEW, Uri.parse("neezer://playlist/$id")).apply {
            setClassName(context, MainActivity::class.java.name)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (cold) addFlags(Intent.FLAG_ACTIVITY_CLEAR_TASK)
        }
    }

    @Test
    fun a_link_that_starts_the_app_opens_the_playlist_page() {
        ActivityScenario.launch<MainActivity>(playlistIntent("pl-cold-start")).use { scenario ->
            // The SCREEN names its own route since L1 removed the shell's top bar:
            // `PLAYLIST_MARKER` is on the playlist's root in every state (skeleton,
            // failure, page), so a deep link is still proved by the screen it
            // opened and not by a title that only exists once the data arrived.
            val opened = device.wait(Until.hasObject(By.desc(PLAYLIST_MARKER)), TIMEOUT_MS)
            assertTrue("la page playlist ne s'est pas ouverte", opened)
            scenario.onActivity { activity ->
                assertFalse("l'activité s'est fermée après le lien", activity.isFinishing)
                assertNotNull("l'activité n'a pas de graphe", activity.window)
            }
            assertTrue(
                "l'application n'est plus au premier plan : elle a planté",
                scenario.state == Lifecycle.State.RESUMED,
            )
        }
    }

    @Test
    fun a_link_that_arrives_while_the_app_runs_opens_the_playlist_page_too() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        ActivityScenario.launch<MainActivity>(
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        ).use { scenario ->
            device.wait(Until.hasObject(By.desc(PLAYLIST_MARKER)), TIMEOUT_MS)
            // The warm door: the same link while the app runs.
            scenario.onActivity { activity ->
                activity.startActivity(playlistIntent("pl-warm", cold = false))
            }
            assertTrue(
                "le lien reçu à chaud n'a pas ouvert la playlist",
                device.wait(Until.hasObject(By.desc(PLAYLIST_MARKER)), TIMEOUT_MS),
            )
            assertTrue(
                "l'application a planté sur le lien à chaud",
                scenario.state != Lifecycle.State.DESTROYED,
            )
        }
    }

    /**
     * M10 : un lien de titre ne laisse plus l'app muette (audit, bloqueur 8).
     *
     * Le serveur n'a aucune lecture qui rend l'album d'un titre, et la table de
     * routes du SPA n'a pas `/track/:id` : la branche `AppLink.Track` ne peut
     * pas ouvrir d'écran. Elle le DIT (rule 3 : un échec dit pourquoi) et l'app
     * reste sur son écran. Le parseur qui produit `AppLink::Track` est celui que
     * `mobile/rust/crates/nz-core/src/links/mod.rs:401` teste déjà.
     */
    @Test
    fun aTrackLinkSaysItCannotOpen() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(Intent.ACTION_SEND, null).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_TEXT, "https://neezer.example/app/#/track/3000168")
            setClass(context, MainActivity::class.java)
        }
        ActivityScenario.launch<MainActivity>(intent).use { scenario ->
            assertTrue(scenario.state.isAtLeast(Lifecycle.State.RESUMED))
            // Le toast est un nœud de la coquille : il porte la phrase entière.
            assertTrue(
                "le lien de titre n'a rien dit",
                device.wait(Until.hasObject(By.textContains("ne s'ouvre pas encore")), TIMEOUT_MS),
            )
        }
    }

    private companion object {
        /** The emulator's first paint plus the core's own start: MQ1 measured 13–25 s. */
        const val TIMEOUT_MS = 15_000L
    }
}
