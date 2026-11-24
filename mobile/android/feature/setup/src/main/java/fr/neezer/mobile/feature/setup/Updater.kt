package fr.neezer.mobile.feature.setup

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import androidx.core.net.toUri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The app's own update, fetched from the configured server and installed in
 * place (lot MB4; ported from `android/app/.../Updater.kt`, `docs/agents/android.md`
 * *Updating itself from the server*).
 *
 * What changed from the old app, and why:
 *
 * - **The download is Rust's** (`Screens.fetchUpdate`, the `Bulk` lane): the old
 *   Kotlin `HttpURLConnection` carried the session cookie, followed no redirect
 *   by hand and re-implemented `Retry-After`; `nz-api` owns all three now, and
 *   nothing else in this app may open its own socket.
 * - **The checks stayed here**, because they are Android's own facts: the zip's
 *   binary manifest (`ApkArchive`), the package, the `versionCode` and the
 *   signing lineage (`UpdateDecisionRule`) — both held by JVM tests.
 * - **The `202` is no longer mistaken for a file**: Rust refuses a body that is
 *   not an application and answers `NotReady`, which this class reports as
 *   `preparing` — the cause the old page printed.
 *
 * The screen shows progress and the outcome through [onEvent]; every cause but
 * the signature keeps "Réessayer" (`updateCauseRetryable`).
 */
class Updater(
    private val activity: Activity,
    private val fetch: suspend (destination: String) -> UpdateFile,
    private val onEvent: (UpdateEvent) -> Unit,
) {
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val running = AtomicBoolean(false)

    /** A verified APK waiting for the "install unknown apps" permission. */
    @Volatile
    private var waiting: File? = null

    /** One tap = one attempt; nothing retries by itself (rule 3). */
    fun start() {
        if (!running.compareAndSet(false, true)) return
        scope.launch {
            try {
                val directory = File(activity.cacheDir, "update").apply { mkdirs() }
                onEvent(UpdateEvent.Download(0.0))
                val file = runCatching { fetch(directory.absolutePath) }
                    .getOrElse { error ->
                        onEvent(UpdateEvent.Error(causeOf(error)))
                        return@launch
                    }
                onEvent(UpdateEvent.Download(1.0))
                val apk = File(file.path)
                when (val decision = verify(apk)) {
                    UpdateDecision.Install -> withContext(Dispatchers.Main) { install(apk) }
                    UpdateDecision.UpToDate -> onEvent(UpdateEvent.UpToDate)
                    else -> {
                        apk.delete()
                        onEvent(UpdateEvent.Error(decision.reason() ?: "invalid"))
                    }
                }
            } finally {
                running.set(false)
            }
        }
    }

    /** Called from `onResume`: back from the permission screen, carry on. */
    fun resume() {
        val apk = waiting ?: return
        waiting = null
        if (canInstall()) install(apk) else onEvent(UpdateEvent.Error("permission"))
    }

    fun shutdown() {
        scope.cancel()
    }

    /** `Install` when [apk] may be handed to Android, else why not. */
    private fun verify(apk: File): UpdateDecision {
        if (apk.length() < ApkArchive.MIN_BYTES) return UpdateDecision.Invalid
        if (!ApkArchive.looksLikeApk(apk)) return UpdateDecision.Invalid
        val manager = activity.packageManager
        val flags = if (Build.VERSION.SDK_INT >= 28) {
            PackageManager.GET_SIGNING_CERTIFICATES
        } else {
            @Suppress("DEPRECATION")
            PackageManager.GET_SIGNATURES
        }
        val archive = try {
            manager.getPackageArchiveInfo(apk.path, flags)
        } catch (_: Exception) {
            null
        } ?: return UpdateDecision.Invalid
        val installed = try {
            manager.getPackageInfo(activity.packageName, flags)
        } catch (_: Exception) {
            return UpdateDecision.Invalid
        }
        val theirs = signingFacts(archive)
        val ours = signingFacts(installed)
        return UpdateDecisionRule.decide(
            archivePackage = archive.packageName,
            archiveVersionCode = archive.versionCode(),
            installedPackage = activity.packageName,
            installedVersionCode = installed.versionCode(),
            archiveSigners = theirs?.signers,
            archiveLineage = theirs?.lineage,
            installedSigners = ours?.signers ?: emptySet(),
            archiveMultipleSigners = theirs?.multiple ?: false,
            installedMultipleSigners = ours?.multiple ?: false,
        )
    }

    /** What a package's signatures say, read on the one API level that has them. */
    private class SigningFacts(
        val signers: Set<String>,
        val lineage: Set<String>,
        val multiple: Boolean,
    )

    /**
     * `SigningInfo` exists from API 28; below it an archive's signers are simply
     * unreadable, which the rule reads as "the installer is the judge" (the old
     * app's `sameSigner` said the same).
     */
    private fun signingFacts(info: PackageInfo): SigningFacts? =
        if (Build.VERSION.SDK_INT >= 28) signingFacts28(info) else null

    @androidx.annotation.RequiresApi(28)
    private fun signingFacts28(info: PackageInfo): SigningFacts? {
        val signing = info.signingInfo ?: return null
        return SigningFacts(
            signers = digests(signing.apkContentsSigners),
            lineage = digests(signing.signingCertificateHistory),
            multiple = signing.hasMultipleSigners(),
        )
    }

    @Suppress("DEPRECATION")
    private fun PackageInfo.versionCode(): Long =
        if (Build.VERSION.SDK_INT >= 28) longVersionCode else versionCode.toLong()

    private fun digests(sigs: Array<android.content.pm.Signature>?): Set<String> {
        val md = MessageDigest.getInstance("SHA-256")
        return (sigs ?: emptyArray()).map { sig ->
            md.digest(sig.toByteArray()).joinToString("") { byte -> "%02x".format(byte) }
        }.toSet()
    }

    /** `REQUEST_INSTALL_PACKAGES` is a grant since API 26, which is this app's floor. */
    private fun canInstall(): Boolean =
        activity.packageManager.canRequestPackageInstalls()

    private fun install(apk: File) {
        if (!canInstall()) {
            waiting = apk
            onEvent(UpdateEvent.Permission)
            try {
                activity.startActivity(
                    Intent(
                        Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                        "package:${activity.packageName}".toUri(),
                    ),
                )
            } catch (_: Exception) {
                waiting = null
                onEvent(UpdateEvent.Error("permission"))
            }
            return
        }
        try {
            val uri = FileProvider.getUriForFile(
                activity,
                "${activity.packageName}.fileprovider",
                apk,
            )
            val view = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, "application/vnd.android.package-archive")
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            onEvent(UpdateEvent.Install)
            activity.startActivity(view)
        } catch (_: Exception) {
            onEvent(UpdateEvent.Error("invalid"))
        }
    }

    /**
     * The Rust side's cause, in the old page's vocabulary. The sentences are
     * `nz_error_text`'s ("le serveur prépare la mise à jour", "téléchargement
     * incomplet", "hors ligne"): this only maps them to the reason the screen
     * already knows how to show.
     */
    private fun causeOf(error: Throwable): String {
        val text = error.message.orEmpty()
        return when {
            // The server has no native channel (it predates this release): a
            // cause of its own, never the WebView APK and never a bare 404.
            text.contains("indisponible", ignoreCase = true) -> "unavailable"
            text.contains("prépar", ignoreCase = true) -> "preparing"
            text.contains("incomplet", ignoreCase = true) -> "truncated"
            text.contains("application valide", ignoreCase = true) -> "archive"
            text.contains("Hors ligne", ignoreCase = true) -> "network"
            text.contains("session", ignoreCase = true) -> "network"
            else -> "network"
        }
    }
}

/** What the screen draws while (and after) an update. */
sealed interface UpdateEvent {
    data class Download(val progress: Double) : UpdateEvent

    /** The system's install dialog is up. */
    data object Install : UpdateEvent

    /** "Installer des applications inconnues" must be allowed for this app. */
    data object Permission : UpdateEvent

    data object UpToDate : UpdateEvent

    /**
     * A cause the person reads: `network`, `preparing`, `truncated`, `archive`,
     * `invalid`, `package`, `signature`, `permission`.
     */
    data class Error(val reason: String) : UpdateEvent
}

/** The `fetchUpdate` answer, as Kotlin sees it (UniFFI's record). */
typealias UpdateFile = uniffi.nz_ffi.UpdateFileRecord

/** The French cause an [UpdateEvent.Error] prints, never a code. */
fun updateCauseText(reason: String): String = when (reason) {
    "unavailable" ->
        "Mise à jour indisponible : ce serveur ne publie pas l'application Android native. " +
            "Mettez le serveur à jour, ou installez l'APK natif publié avec la version du serveur."
    "network" -> "Le serveur n'a pas pu envoyer la mise à jour"
    "preparing" -> "Le serveur prépare la mise à jour"
    "truncated" -> "Le fichier reçu est incomplet"
    "archive" -> "Le fichier reçu n'est pas une application valide"
    "invalid" -> "Le fichier reçu n'est pas une application valide"
    "package" -> "Cette mise à jour n'est pas Neezer"
    "signature" -> "Signature différente : installe l'APK publié par le serveur"
    "permission" -> "Autorise les applications inconnues pour installer la mise à jour"
    else -> "La mise à jour a échoué"
}

/** Whether that cause keeps a "Réessayer" (everything but the signature). */
fun updateCauseRetryable(reason: String): Boolean = reason != "signature"
