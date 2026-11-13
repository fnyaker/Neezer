package org.nsupysonic.app

import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

/**
 * The native side's GETs (cover art for the notification, files for the share
 * sheet), with the rules the WebView's own requests follow:
 *
 *  - CLEARTEXT only to the configured server. The manifest has to allow http
 *    because a home server may well be http://192.168.1.10 — and the manifest
 *    cannot know which host that is — so the narrowing happens here.
 *  - The session COOKIE only to the configured server. Redirects are followed
 *    by hand: HttpURLConnection re-sends a header set with setRequestProperty
 *    to wherever a redirect points, which handed the session to any host the
 *    server (or a man in the middle) redirected to.
 *  - The pinned certificate only for the configured server (Ssl.apply): any
 *    other host gets the system's checks, whatever "verify SSL" says.
 *  - An optional [allow] predicate is asked about EVERY hop before a socket is
 *    opened: a caller that fetches a URL it did not build (a cover, a share
 *    link) can hold the whole redirect chain to an allow-list. The default
 *    (null) keeps the old behaviour for the callers that already own the URL.
 */
object Net {
    private const val MAX_REDIRECTS = 5

    /**
     * Open [url] and return the connection once it answered 2xx (the caller
     * reads and disconnects it). [cookie] goes only to the configured server;
     * [headers] (a Range, say) go with every hop. [allow] is evaluated on the
     * initial URL and on each redirect's Location before anything is opened:
     * false throws `IOException("host refused")`.
     */
    fun get(
        prefs: Prefs, url: String, cookie: String?, connectTimeout: Int, readTimeout: Int,
        headers: Map<String, String> = emptyMap(),
        allow: ((String) -> Boolean)? = null,
    ): HttpURLConnection {
        var current = url
        for (hop in 0..MAX_REDIRECTS) {
            // Before the parse: a refused host never reaches DNS, a parser or
            // a socket.
            if (allow != null && !allow(current)) throw IOException("host refused")
            // Parse ONCE. The origin decision, the cleartext rule, the pin and
            // the connection below all use this one URL object: no second
            // parser can name another host.
            val parsed = try {
                URL(current)
            } catch (_: Exception) {
                throw IOException("not an http(s) URL")
            }
            val scheme = parsed.protocol.lowercase()
            if (scheme != "http" && scheme != "https") {
                throw IOException("not an http(s) URL")
            }
            if (parsed.host.isNullOrEmpty()) throw IOException("not an http(s) URL")
            val ours = prefs.isServerOrigin(parsed.protocol, parsed.host, parsed.port)
            if (scheme == "http" && !ours) throw IOException("cleartext refused off the server")
            val conn = parsed.openConnection() as HttpURLConnection
            Ssl.apply(conn, prefs.verifySsl, if (ours) prefs.pinFor(prefs.serverOrigin()) else null)
            conn.connectTimeout = connectTimeout
            conn.readTimeout = readTimeout
            conn.instanceFollowRedirects = false
            for ((k, v) in headers) conn.setRequestProperty(k, v)
            if (ours && !cookie.isNullOrEmpty()) conn.setRequestProperty("Cookie", cookie)
            val code = conn.responseCode
            if (code in 300..399) {
                val location = conn.getHeaderField("Location")
                conn.disconnect()
                if (location.isNullOrEmpty()) throw IOException("redirect without Location")
                current = URL(parsed, location).toString()
                continue
            }
            if (code !in 200..299) {
                conn.disconnect()
                throw IOException("HTTP $code")
            }
            return conn
        }
        throw IOException("too many redirects")
    }
}
