package fr.neezer.mobile.feature.setup

import android.annotation.SuppressLint
import android.content.Context
import android.os.Handler
import android.os.HandlerThread
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
 *   DESTROYED on its own `HandlerThread` (a WebView needs a Looper, and this app
 *   has no window open at that moment).
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
        val thread = HandlerThread("nz-migrate-web")
        thread.start()
        val handler = Handler(thread.looper)
        try {
            handler.post { load(context, url, answer) }
            val payload = withTimeout(TIMEOUT_MS) { answer.await() }
            return parse(payload)
        } catch (_: TimeoutException) {
            throw IllegalStateException("la page de migration n'a pas répondu")
        } finally {
            handler.post { thread.quitSafely() }
        }
    }

    /** Creates the WebView on the handler's thread, loads [url], destroys it. */
    private fun load(context: Context, url: String, answer: CompletableDeferred<String>) {
        var finished = false
        var web: WebView? = null
        fun finish(payload: String?) {
            if (finished) return
            finished = true
            if (payload != null && !answer.isCompleted) answer.complete(payload)
            web?.let { view ->
                runCatching { view.stopLoading() }
                runCatching { (view.parent as? android.view.ViewGroup)?.removeView(view) }
                runCatching { view.destroy() }
            }
            web = null
        }
        try {
            val view = WebView(context)
            web = view
            view.settings.apply {
                javaScriptEnabled = true
                domStorageEnabled = true
                databaseEnabled = true
                // The page is local and its storage is local: no image, no font,
                // no script is ever fetched (the interceptor would refuse them
                // anyway, this just never asks).
                cacheMode = WebSettings.LOAD_NO_CACHE
                blockNetworkImage = true
            }
            view.webViewClient = object : WebViewClient() {
                override fun shouldInterceptRequest(
                    asked: WebView?,
                    request: WebResourceRequest?,
                ): WebResourceResponse? {
                    val target = request?.url?.toString() ?: return blocked()
                    return if (target == url) page() else blocked()
                }
            }
            view.webChromeClient = object : WebChromeClient() {
                override fun onConsoleMessage(message: ConsoleMessage?): Boolean {
                    val text = message?.message() ?: return false
                    val at = text.indexOf(MigrationPage.PREFIX)
                    if (at < 0) return false
                    finish(text.substring(at + MigrationPage.PREFIX.length))
                    return true
                }
            }
            view.loadUrl(url)
            // A page that never reports (no storage, a Chromium that refused the
            // document) is still destroyed: the caller's deadline ends the wait,
            // and this timer ends the view a moment after it.
            Handler(android.os.Looper.myLooper() ?: return).postDelayed(
                { finish(null) },
                TIMEOUT_MS + 1_000,
            )
        } catch (error: Throwable) {
            finish("""{"error":"${error.javaClass.simpleName}"}""")
        }
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
