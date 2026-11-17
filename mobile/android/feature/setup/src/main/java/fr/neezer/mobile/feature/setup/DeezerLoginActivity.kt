package fr.neezer.mobile.feature.setup

import android.app.Activity
import android.os.Bundle

/**
 * Deezer's own login page, in a throwaway WebView, to capture the `arl` cookie
 * (lot MB4; `DeezerLoginActivity` in the old app, ported as is).
 *
 * The rules that make it safe are the old app's, and MB4 must keep all of them:
 * no JS bridge is attached, Google/Apple SSO is refused inside a WebView, the
 * Deezer cookie jar is wiped on entry (the ARL is always the account just typed)
 * and again on success, and the cookie is read by polling (~600 ms, only while
 * the screen is up) because Deezer's login is an XHR and no navigation fires.
 *
 * MB0 creates the component and its manifest entry so the provider, the label
 * and the exported=false contract exist; it opens nothing.
 */
class DeezerLoginActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // MB4: the WebView, the cookie sweep and the delivery to Réglages.
        finish()
    }
}
