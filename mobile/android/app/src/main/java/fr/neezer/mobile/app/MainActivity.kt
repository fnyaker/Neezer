package fr.neezer.mobile.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import fr.neezer.mobile.feature.settings.AppSettings
import fr.neezer.mobile.feature.settings.NzAppearance
import fr.neezer.mobile.feature.settings.NzAppearanceTheme

/**
 * The app's ONE activity (docs/agents/mobile.md, "Navigation, screens and the
 * UI"): a Compose shell, no fragments, no WebView, nothing to rebuild when a
 * renderer dies.
 *
 * `configChanges` in the manifest keeps a theme, font-size or language change
 * from recreating it; Compose re-reads the configuration itself.
 *
 * Incoming intents:
 *
 * - `ACTION_SEND` of `text/plain`: a link somebody shared — it becomes a route
 *   through [AppGraph.openLinkLater] (the import, a party, a playlist…);
 * - `ACTION_VIEW` of `neezer://…` or a `/app/#/…` URL: the same door.
 */
class MainActivity : ComponentActivity() {

    private lateinit var graph: AppGraph

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // The phone's player surface: the session, the notification, the lock
        // screen, the headset keys, the car and a remote device all reach the
        // engine through ONE service, and nothing else starts it (measured: a tap
        // loaded the queue and opened no output — MB3c's service was only ever
        // created by its own tests). Started with the activity whose shell draws
        // every play control; the output itself opens on the first title that
        // plays, not here (see `NzMediaService.follow`).
        fr.neezer.mobile.playback.NzMediaService.start(this)
        graph = AppGraph(application)

        // BEFORE the first frame, and never inside one: the device's theme, its
        // accent and the eco switch, read from the `kv` store the core opened in
        // `Application.onCreate` (one indexed SELECT per key: 197.6 µs for all
        // 17, MB16's measured number). A person who chose Papier must not see
        // Encre first.
        NzAppearance.load(AppSettings.service())

        handleIntent(intent)

        setContent {
            // The theme, the accent and the eco switch are Réglages' (MB16):
            // `NzAppearance` read them ONCE in `onCreate` (above), before the
            // first frame, so a launch in Papier never flashes Encre, and every
            // change since then repaints through the same state — no reload.
            NzAppearanceTheme {
                NzApp(graph)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    /** A share or a deep link, whichever the intent carries. */
    private fun handleIntent(intent: Intent?) {
        if (intent == null) return
        val url = when (intent.action) {
            Intent.ACTION_SEND -> intent.getStringExtra(Intent.EXTRA_TEXT)
            Intent.ACTION_VIEW -> intent.dataString
            else -> null
        }
        url?.takeIf { it.isNotBlank() }?.let(graph::openLinkLater)
    }
}
