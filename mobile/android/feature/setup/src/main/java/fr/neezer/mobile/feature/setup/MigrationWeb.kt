package fr.neezer.mobile.feature.setup

import android.annotation.SuppressLint
import android.content.Context
import android.util.Log
import android.webkit.ConsoleMessage
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import java.io.ByteArrayInputStream
import java.util.concurrent.TimeoutException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.json.JSONObject
import uniffi.nz_ffi.OldSettingRecord

/**
 * The hidden WebView that reads the OLD SPA's own storage (lot MB4, step 4).
 *
 * Why a WebView at all: `localStorage` and IndexedDB live in `app_webview/`,
 * partitioned by ORIGIN, and no platform API reads another origin's storage from
 * Kotlin. A WebView loading `<origin>/__nsmigrate` — same applicationId, same
 * data directory, same origin — sees exactly what the old app's page left
 * behind, and [`MigrationPage`] reports it.
 *
 * Four rules, each the reason a line exists here:
 *
 * - **Nothing reaches the network.** `shouldInterceptRequest` answers the migrate
 *   URL with the local page and EVERY other URL with an empty 404: a document
 *   that changed on the server can never be fetched, and a service worker left
 *   by the old SPA cannot make this app talk to anything.
 * - **No JavaScript bridge.** The page reports through `console.log`, which
 *   `WebChromeClient` hands over as an event — there is no
 *   `@JavascriptInterface` object for a page (or a compromised SPA) to call into
 *   this app.
 * - **It is bounded and it always cleans up.** The read waits for that event
 *   with a deadline, so a page that never answers becomes a CAUSE in the report
 *   instead of a migration that hangs (rule 3); the WebView is created, used and
 *   DESTROYED on the main thread, and never attached to any window: a WebView
 *   needs the UI thread, and this app has no screen open at that moment.
 * - **It is a throwaway.** No cookie is read from it, nothing is written to it,
 *   and no cache is kept.
 */
@SuppressLint("SetJavaScriptEnabled")
internal object MigrationWeb {
    /** What the page found: the settings, and the accounts of the downloads. */
    data class Found(
        val settings: List<OldSettingRecord>,
        val owners: Map<String, String>,
        val albums: Int,
    )

    /** How long the page has to answer before the migration names the cause. */
    const val TIMEOUT_MS = 20_000L

    /** The journal's tag, shared with the runner: a failed read says why. */
    private const val TAG = "NzMigration"

    /** The path the page is served at, under the configured origin. */
    const val PATH = "/__nsmigrate"

    /**
     * Reads `<origin>/__nsmigrate`'s storage. [origin] must be the configured
     * server's origin: another one is another partition, and its empty answer
     * would pass for a migrated one.
     */
    suspend fun read(context: Context, origin: String): Found {
        val url = "${origin.trimEnd('/')}$PATH"
        val answer = CompletableDeferred<String>()
        // A WebView is created, used and destroyed on the MAIN thread (that is
        // the configuration the platform supports); the caller's coroutine waits
        // on the page's own report, so nothing blocks the UI while it does.
        var view: WebView? = null
        var opening: Throwable? = null
        withContext(Dispatchers.Main) {
            view = runCatching { open(context, url, answer) }
                .onFailure { error ->
                    opening = error
                    Log.w(TAG, "the migration page could not open: ${error.message}", error)
                }
                .getOrNull()
        }
        if (view == null) {
            // A WebView that refuses to exist is a CAUSE the report prints, not
            // a silent `settings = 0` (rule 2).
            throw IllegalStateException(
                "la page de migration n'a pas pu s'ouvrir" +
                    (opening?.message?.let { " ($it)" } ?: ""),
            )
        }
        try {
            val payload = withTimeout(TIMEOUT_MS) { answer.await() }
            return parse(payload)
        } catch (error: Exception) {
            Log.w(TAG, "the migration page did not answer: ${error.message}")
            throw IllegalStateException("la page de migration n'a pas répondu")
        } finally {
            val dead = view
            withContext(Dispatchers.Main) {
                dead?.let { web ->
                    runCatching { web.stopLoading() }
                    runCatching { (web.parent as? android.view.ViewGroup)?.removeView(web) }
                    runCatching { web.destroy() }
                }
            }
        }
    }

    /** Creates the WebView, serves it the local page and arms the report. */
    private fun open(
        context: Context,
        url: String,
        answer: CompletableDeferred<String>,
    ): WebView {
        val web = WebView(context)
        web.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            cacheMode = WebSettings.LOAD_NO_CACHE
            blockNetworkImage = true
        }
        web.webViewClient = object : WebViewClient() {
            override fun shouldInterceptRequest(
                asked: WebView?,
                request: WebResourceRequest?,
            ): WebResourceResponse? {
                val target = request?.url?.toString() ?: return blocked()
                return if (target == url) page() else blocked()
            }

            override fun onReceivedError(
                view: WebView?,
                request: WebResourceRequest?,
                error: android.webkit.WebResourceError?,
            ) {
                Log.w(TAG, "page error: ${error?.errorCode} ${request?.url}")
            }
        }
        web.webChromeClient = object : WebChromeClient() {
            override fun onConsoleMessage(message: ConsoleMessage?): Boolean {
                val text = message?.message() ?: return false
                val at = text.indexOf(MigrationPage.PREFIX)
                if (at < 0) return false
                if (!answer.isCompleted) answer.complete(text.substring(at + MigrationPage.PREFIX.length))
                return true
            }
        }
        web.loadUrl(url)
        return web
    }

    private fun page(): WebResourceResponse {
        val headers = mapOf(
            "Content-Type" to "text/html; charset=utf-8",
            "Cache-Control" to "no-store",
        )
        return WebResourceResponse(
            "text/html",
            "utf-8",
            200,
            "OK",
            headers,
            ByteArrayInputStream(MigrationPage.HTML.toByteArray(Charsets.UTF_8)),
        )
    }

    private fun blocked(): WebResourceResponse = WebResourceResponse(
        "text/plain",
        "utf-8",
        404,
        "Not Found",
        mapOf("Cache-Control" to "no-store"),
        ByteArrayInputStream(ByteArray(0)),
    )

    /** The page's JSON, read tolerantly: a missing store is an empty answer. */
    private fun parse(payload: String): Found {
        val root = runCatching { JSONObject(payload) }.getOrNull()
            ?: return Found(emptyList(), emptyMap(), 0)
        val settings = ArrayList<OldSettingRecord>()
        root.optJSONObject("settings")?.let { settings.addAll(settingsFrom(it)) }
        val owners = HashMap<String, String>()
        root.optJSONArray("downloads")?.let { rows ->
            for (index in 0 until rows.length()) {
                val row = rows.optJSONObject(index) ?: continue
                val id = row.optString("id", "")
                if (id.isEmpty()) continue
                owners[id] = row.optString("owner", "")
            }
        }
        return Found(settings, owners, root.optInt("albums", 0))
    }

    /** `Found`, or `null` when the page could not answer — never an exception. */
    suspend fun readOrNull(context: Context, origin: String): Found? = try {
        read(context, origin)
    } catch (_: Exception) {
        null
    }
}
