package fr.neezer.mobile.car

import android.content.Context
import android.content.Intent
import fr.neezer.mobile.bridge.NzBridge

/**
 * The few questions the car asks about "the app" now that there is no WebView
 * and no `Prefs`: the configured server's origin, and the door back into the
 * app.
 *
 * The car mode used to read `Prefs.baseUrl()` and `Prefs.configured` (the old
 * app's own SharedPreferences) and to name `MainActivity::class.java` directly.
 * Neither exists here: the server is whatever `NzCore` was configured with, and
 * `:car` cannot see `:app` (the dependency goes the other way). So:
 *
 * - the **origin** comes from `NzCore.server()`, the one definition of "our
 *   server" the whole app shares (session, pin, allow-lists). The car resolves
 *   the server-relative cover paths it is handed against it and nothing else;
 * - the **app is opened by its launcher intent**, not by a class: `:app` owns
 *   `MainActivity`, this app's one launcher entry, and the package manager
 *   resolves it. That is also what keeps the car compiling when the shell moves.
 */
internal object CarServer {
    /** `scheme://host:port`, or "" when no server is configured (or no core). */
    fun origin(): String = try {
        NzBridge.core?.server()?.origin?.trimEnd('/').orEmpty()
    } catch (_: Throwable) {
        // A core that cannot answer is "no server": the car says so instead of
        // crashing a service that runs while the person drives.
        ""
    }

    /** Whether a server exists at all — the old `Prefs.configured`. */
    fun configured(): Boolean = origin().isNotEmpty()

    /**
     * "Is this URL our server, path ignored?" — the same-origin predicate the
     * cover pre-filter takes. It compares scheme, host and port exactly (never a
     * prefix of a string: `https://music.example.evil.com` is not our server).
     */
    fun sameOrigin(url: String): Boolean {
        val origin = origin()
        if (origin.isEmpty()) return false
        val target = try {
            java.net.URI(url.trim())
        } catch (_: Exception) {
            return false
        }
        val base = try {
            java.net.URI(origin)
        } catch (_: Exception) {
            return false
        }
        return target.scheme.equals(base.scheme, ignoreCase = true) &&
            target.host.equals(base.host, ignoreCase = true) &&
            effectivePort(target) == effectivePort(base)
    }

    private fun effectivePort(uri: java.net.URI): Int = when {
        uri.port > 0 -> uri.port
        uri.scheme.equals("https", ignoreCase = true) -> 443
        else -> 80
    }

    /**
     * The intent that brings the app to the person: the launcher entry the
     * package manager knows (`MainActivity`), from a service — so it
     * carries `NEW_TASK` and clears the top, which is what the car's
     * notification and its "Ouvre Neezer…" toasts need.
     */
    fun openApp(context: Context): Intent? = try {
        context.packageManager.getLaunchIntentForPackage(context.packageName)
            ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    } catch (_: Exception) {
        null
    }
}
