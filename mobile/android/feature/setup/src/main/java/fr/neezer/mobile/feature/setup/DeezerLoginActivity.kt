package fr.neezer.mobile.feature.setup

import android.annotation.SuppressLint
import android.content.Intent
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.net.http.SslError
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.SslErrorHandler
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon

/**
 * Captures the Deezer ARL by letting the person sign in on **Deezer's own login
 * page**, hosted in a throwaway WebView (lot MB4; ported as it is from
 * `android/app/.../DeezerLoginActivity.kt`, `docs/agents/android.md` *Linking the
 * Deezer account from the app*).
 *
 * Why it exists, in one paragraph: there is no email/password endpoint left that
 * a server could call — the legacy web one answers 403 since 2024, and the mobile
 * gateway needs keys extracted from Deezer's own binaries. What still works, and
 * is the only thing that should, is a real browser session: the person types
 * their credentials on deezer.com, Deezer sets its `arl` cookie, and this screen
 * reads that one cookie out of the jar.
 *
 * The rules that make it safe are the old app's, and none of them is optional:
 *
 * - **No JavaScript bridge is attached.** Nothing on deezer.com can reach this
 *   app (no `@JavascriptInterface`, no `addJavascriptInterface`).
 * - **The Deezer jar is wiped on entry** (the ARL is always the account just
 *   typed) **and again on success** (no second copy left on the device), scoped
 *   to `deezer.com` — never `removeAllCookies()`, which would sign the person out
 *   of their own server.
 * - **A bad certificate cancels**: deezer.com is a public host, and a wrong
 *   certificate there means somebody is between us and the password.
 * - **Cookie arrival is POLLED (~600 ms, only while the screen is up)**: Deezer's
 *   login is an XHR, so no navigation fires and there is no cookie-change
 *   callback to hook.
 * - **Google/Apple SSO is refused** inside a WebView: email + password is the
 *   supported route.
 * - **The password never reaches this app or the server**; only the ARL crosses
 *   back, in `RESULT_OK`'s `EXTRA_ARL`, and it goes straight to the configured
 *   server (`Screens.saveDeezerArl`).
 */
class DeezerLoginActivity : ComponentActivity() {

    companion object {
        const val EXTRA_ARL = "arl"

        private const val LOGIN_URL = "https://www.deezer.com/login"

        /** Every Deezer origin whose jar can hold (or hold a stale) `arl`. */
        private val DEEZER_URLS = listOf(
            "https://www.deezer.com/",
            "https://deezer.com/",
            "https://auth.deezer.com/",
            "https://account.deezer.com/",
        )

        /**
         * The shape the server accepts (`neezer/deezer/__init__.py`): the value
         * goes out as a cookie, so anything outside this alphabet is header
         * injection and is not an ARL anyway.
         */
        private val ARL_RE = Regex("[0-9A-Za-z._-]{32,255}")

        /**
         * Cookie polling interval while this screen is up. There is no
         * cookie-change callback in the framework and Deezer's login is an XHR,
         * so a short tick bounded to this one screen — stopped the instant the
         * ARL shows up — is the only reliable observer.
         */
        private const val POLL_MS = 600L
    }

    private lateinit var webView: WebView
    private var webViewDead = false
    private var done = false

    private val handler = Handler(Looper.getMainLooper())
    private val poll = object : Runnable {
        override fun run() {
            if (done || isFinishing) return
            val arl = harvestArl()
            if (arl != null) succeed(arl) else handler.postDelayed(this, POLL_MS)
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // The Sillon surface this screen wears: the app's own theme, so a person
        // never lands on a bare white WebView on a black app (rule 5).
        val colors = Sillon.theme(NzThemeKind.Encre)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(colors.bg.toArgb())
        }
        val header = TextView(this).apply {
            text = getString(R.string.deezer_login_title)
            setTextColor(colors.tx.toArgb())
            textSize = 20f
            setPadding(48, 48, 48, 32)
        }
        root.addView(header, LinearLayout.LayoutParams(-1, -2))
        val host = FrameLayout(this)
        root.addView(host, LinearLayout.LayoutParams(-1, 0, 1f))
        val cancel = Button(this).apply {
            text = "Annuler"
            setTextColor(colors.onbtn.toArgb())
            background = GradientDrawable().apply {
                cornerRadius = 999f
                setColor(colors.btn.toArgb())
            }
            setOnClickListener { cancel() }
        }
        root.addView(
            cancel,
            LinearLayout.LayoutParams(-1, -2).apply { setMargins(48, 24, 48, 48) },
        )
        setContentView(root)

        // A clean Deezer jar so the ARL that comes back is always the account
        // just typed, never a leftover session from a previous link.
        clearDeezerCookies()

        val cookies = CookieManager.getInstance()
        cookies.setAcceptCookie(true)

        webView = WebView(this)
        host.addView(webView, FrameLayout.LayoutParams(-1, -1))
        cookies.setAcceptThirdPartyCookies(webView, true)
        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            cacheMode = WebSettings.LOAD_DEFAULT
            // Drop the "; wv" marker that identifies an embedded WebView:
            // sign-in pages routinely degrade or refuse outright when they see
            // it, and this one has to render a real login form to be of use.
            userAgentString = userAgentString.replace("; wv", "")
        }

        webView.webViewClient = object : WebViewClient() {
            override fun onReceivedSslError(
                view: WebView?,
                sslHandler: SslErrorHandler?,
                error: SslError?,
            ) {
                // Never relaxed: "Vérifier le certificat" exists for the person's
                // own home server, not for a public host carrying a password.
                sslHandler?.cancel()
                Toast.makeText(
                    this@DeezerLoginActivity,
                    "Connexion refusée : le certificat de Deezer n'est pas valide",
                    Toast.LENGTH_LONG,
                ).show()
                cancel()
            }

            override fun shouldOverrideUrlLoading(
                view: WebView?,
                request: WebResourceRequest?,
            ): Boolean {
                val scheme = request?.url?.scheme?.lowercase()
                // Keep the whole sign-in journey inside this WebView, but swallow
                // app links — `intent://`/`market://`/`deezer://` would bounce
                // the person into the Deezer app or the Play Store mid-login.
                return scheme != "http" && scheme != "https"
            }

            override fun onPageFinished(view: WebView?, url: String?) {
                // The post-login navigation usually lands before the next tick.
                harvestArl()?.let { succeed(it) }
            }
        }

        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (!webViewDead && webView.canGoBack()) webView.goBack() else cancel()
            }
        })

        webView.loadUrl(LOGIN_URL)
    }

    override fun onResume() {
        super.onResume()
        if (!done) handler.post(poll)
    }

    override fun onPause() {
        handler.removeCallbacks(poll)
        super.onPause()
    }

    /** The `arl` cookie from any Deezer origin, or null while there isn't one. */
    private fun harvestArl(): String? {
        val cookies = CookieManager.getInstance()
        for (url in DEEZER_URLS) {
            val raw = try {
                cookies.getCookie(url)
            } catch (_: Exception) {
                null
            } ?: continue
            for (part in raw.split(';')) {
                val pair = part.trim()
                if (!pair.startsWith("arl=")) continue
                val value = pair.substring(4).trim()
                if (ARL_RE.matches(value)) return value
            }
        }
        return null
    }

    /**
     * Expire every cookie on the Deezer origins. There is no "remove cookies for
     * this domain" API, so each one is overwritten with an already-expired copy —
     * on the host itself and on the `.deezer.com` parent, since a cookie set for
     * the parent domain is not cleared by expiring a host-only one.
     */
    private fun clearDeezerCookies() {
        val cookies = CookieManager.getInstance()
        for (url in DEEZER_URLS) {
            val raw = try {
                cookies.getCookie(url)
            } catch (_: Exception) {
                null
            } ?: continue
            val host = Uri.parse(url).host
            for (part in raw.split(';')) {
                val name = part.substringBefore('=').trim()
                if (name.isEmpty()) continue
                cookies.setCookie(url, "$name=; Max-Age=0; Path=/")
                cookies.setCookie(url, "$name=; Max-Age=0; Path=/; Domain=.deezer.com")
                if (host != null) cookies.setCookie(url, "$name=; Max-Age=0; Path=/; Domain=$host")
            }
        }
        try {
            cookies.flush()
        } catch (_: Exception) {
            /* nothing to flush */
        }
    }

    private fun succeed(arl: String) {
        if (done) return
        done = true
        handler.removeCallbacks(poll)
        // The ARL is a full-account credential: now that it is on its way to the
        // server the person chose, no second copy stays in this app's jar.
        // Expiring it locally does not end the Deezer session.
        clearDeezerCookies()
        setResult(RESULT_OK, Intent().putExtra(EXTRA_ARL, arl))
        finish()
    }

    private fun cancel() {
        if (done) return
        done = true
        handler.removeCallbacks(poll)
        setResult(RESULT_CANCELED)
        finish()
    }

    override fun onDestroy() {
        handler.removeCallbacks(poll)
        // Detach before destroy, and never touch the WebView afterwards: a
        // destroyed WebView left in the view tree is drawn from freed memory.
        if (!webViewDead && ::webView.isInitialized) {
            webViewDead = true
            try {
                (webView.parent as? ViewGroup)?.removeView(webView)
                webView.destroy()
            } catch (_: Exception) {
                /* already gone */
            }
        }
        super.onDestroy()
    }
}
