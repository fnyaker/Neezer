package fr.neezer.mobile.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import fr.neezer.mobile.designsystem.NzTheme

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
        graph = AppGraph(application)

        handleIntent(intent)

        setContent {
            // The theme is `encre` until Réglages › Apparence says otherwise
            // (MB16); reading it from the core would make the first frame wait
            // on disk, which rule 4 forbids.
            NzTheme {
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
