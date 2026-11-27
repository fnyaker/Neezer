package fr.neezer.mobile.app

import android.content.Context
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.ImageBitmap
import fr.neezer.mobile.bridge.CoilCovers
import fr.neezer.mobile.designsystem.LocalNzCoverLoader
import fr.neezer.mobile.designsystem.NzCoverLoader
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
                // Le seam du design system, fourni UNE fois : sans ce
                // `CompositionLocalProvider` il reste `null` et chaque pochette
                // est sa rainure (mesuré par l'audit : le seul fournisseur de
                // l'arbre était un test).
                CompositionLocalProvider(
                    LocalNzCoverLoader provides CoilNzCoverLoader(applicationContext),
                ) {
                    NzApp(graph)
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    /** DIAGNOSTIC (temporary): does the window receive the gesture at all? */
    override fun dispatchTouchEvent(ev: android.view.MotionEvent): Boolean {
        if (ev.actionMasked == android.view.MotionEvent.ACTION_DOWN ||
            ev.actionMasked == android.view.MotionEvent.ACTION_UP ||
            ev.actionMasked == android.view.MotionEvent.ACTION_CANCEL
        ) {
            android.util.Log.i(
                "NZTOUCH",
                "act=${ev.actionMasked} x=${ev.x.toInt()} y=${ev.y.toInt()} pointers=${ev.pointerCount}",
            )
        }
        return super.dispatchTouchEvent(ev)
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

/**
 * Le seam des pochettes du design system, branché sur Coil (audit, bloqueur 3).
 *
 * Le design system déclare `LocalNzCoverLoader` avec `null` par défaut et peint la rainure
 * déterministe dessous : sans fournisseur, chaque pochette de l'app reste sa rainure et AUCUN test
 * n'échoue. Fourni UNE fois, ici, autour de `NzApp`.
 *
 * Le pont est dans `:app` et non dans `:bridge` parce que `:designsystem` dépend de `:bridge`
 * (`api(project(":bridge"))`, ses lignes dessinent des `TrackRow` UniFFI) : une interface de
 * `:designsystem` implémentée depuis `:bridge` serait un cycle de modules. `CoilCovers` reste dans
 * `:bridge` (le fetcher, le keyer et Coil y vivent déjà).
 */
private class CoilNzCoverLoader(private val context: Context) : NzCoverLoader {
    @Composable
    override fun Image(url: String, sizePx: Int) {
        CoilCovers.Image(url = url, sizePx = sizePx)
    }

    /**
     * Le fond du lecteur (JF) demande les PIXELS, pas une image composée : c'est
     * lui qui floute, en Kotlin, sur tous les niveaux d'API. Seul `:app` voit
     * `:designsystem` et `:bridge`, donc c'est ici que le pont se fait.
     */
    override suspend fun bitmap(url: String, sizePx: Int): ImageBitmap? =
        CoilCovers.bitmap(context, url, sizePx)
}
