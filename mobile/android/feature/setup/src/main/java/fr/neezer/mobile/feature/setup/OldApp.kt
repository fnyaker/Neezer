package fr.neezer.mobile.feature.setup

import android.content.Context
import android.content.pm.PackageManager

/**
 * What the NATIVE app may know about the old WebView app (lot MB4, revised for
 * the coexistence release).
 *
 * The first MB4 read `SharedPreferences("settings")`, `filesDir/offline`,
 * `CookieManager` and a hidden WebView, which worked because the new APK
 * installed OVER `org.nsupysonic.app` and therefore ran in the same data
 * directory. With its own `applicationId` (`fr.neezer.mobile`) that is no
 * longer true: Android gives a different package no access to another app's
 * private files, preferences, cookies or WebView storage, by design.
 *
 * So the honest answer is the only one left: **detect** the old app, tell the
 * person what cannot be carried over, and let them sign in here. Nothing is
 * read, moved or deleted — the old app keeps every byte it had (rule 7), and
 * uninstalling either app never touches the other.
 */
internal object OldApp {
    /** The WebView app's package: the one this app is installed BESIDE. */
    const val PACKAGE = "org.nsupysonic.app"

    /**
     * Whether the old app is installed right now.
     *
     * `getPackageInfo` needs the `<package>` entry in our manifest's `<queries>`
     * from Android 11 on, or it answers `NameNotFoundException` even when the
     * app is there — the query would then be a silent false negative. Any other
     * failure (a broken PackageManager, a test double) is "not installed": a
     * notice must never crash the first screen.
     */
    fun installed(context: Context): Boolean =
        try {
            context.packageManager.getPackageInfo(PACKAGE, 0)
            true
        } catch (_: PackageManager.NameNotFoundException) {
            false
        } catch (_: Exception) {
            false
        }
}
