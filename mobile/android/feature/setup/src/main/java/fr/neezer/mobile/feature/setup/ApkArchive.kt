package fr.neezer.mobile.feature.setup

import java.io.File
import java.io.InputStream
import java.util.zip.ZipFile

/**
 * What makes a file an APK: a zip whose `AndroidManifest.xml` is Android's
 * BINARY XML (the header `getPackageArchiveInfo` parses), above the size of any
 * real build.
 *
 * Ported from `android/app/src/main/java/org/nsupysonic/app/ApkArchive.kt`
 * unchanged (lot MB4, `docs/agents/android.md` *Updating itself from the
 * server*): the reason it exists is the reported "Le fichier reçu n'est pas une
 * application valide", which was the server's `{"state":"fetching"}` JSON saved
 * as an APK. Pure JVM on purpose — `ApkArchiveTest` holds it against a truncated
 * file, a plain zip and a real one.
 */
object ApkArchive {
    /** A real build is a thousand times bigger than this. */
    const val MIN_BYTES = 64L * 1024

    /** `RES_XML_TYPE` and the 8-byte header of Android's binary XML. */
    private val MANIFEST_HEADER = byteArrayOf(0x03, 0x00, 0x08, 0x00)

    fun looksLikeApk(file: File): Boolean {
        if (file.length() < MIN_BYTES) return false
        return try {
            ZipFile(file).use { zip ->
                val entry = zip.getEntry("AndroidManifest.xml")
                if (entry == null) {
                    false
                } else {
                    zip.getInputStream(entry).use { input ->
                        val head = readFully(input, MANIFEST_HEADER.size)
                        head != null && head.contentEquals(MANIFEST_HEADER)
                    }
                }
            }
        } catch (_: Exception) {
            false
        }
    }

    private fun readFully(input: InputStream, size: Int): ByteArray? {
        val out = ByteArray(size)
        var at = 0
        while (at < size) {
            val n = input.read(out, at, size - at)
            if (n < 0) return null
            at += n
        }
        return out
    }
}

/**
 * Whether an archive may be handed to Android's installer — decided BEFORE the
 * installer is asked, so the screen can say "signature différente" instead of
 * showing an installer error.
 *
 * Ported from the old app's `Updater.verify`/`sameSigner`, with the
 * `PackageManager` facts passed in as plain values: the policy is a pure
 * function, and `UpdateDecisionTest` holds every branch on the JVM (the
 * `PackageManager` itself is a device object; what it *answers* is what this
 * decides on).
 */
sealed interface UpdateDecision {
    /** Hand it to the installer. */
    data object Install : UpdateDecision

    /** Same version or older: nothing to do, and not an error. */
    data object UpToDate : UpdateDecision

    /** Not this app at all. */
    data object NotThisApp : UpdateDecision

    /** Signed with another key: an installer would refuse it, so say why first. */
    data object DifferentSignature : UpdateDecision

    /** Unreadable as a package. */
    data object Invalid : UpdateDecision

    /** The cause the screen prints, or `null` for [Install]. */
    fun reason(): String? = when (this) {
        Install -> null
        UpToDate -> "uptodate"
        NotThisApp -> "package"
        DifferentSignature -> "signature"
        Invalid -> "invalid"
    }

    /** Whether "Réessayer" makes sense: everything but a different signature. */
    fun retryable(): Boolean = this != DifferentSignature
}

object UpdateDecisionRule {
    /**
     * Android's own rule, checked ahead of it: one signer, and the update's
     * lineage (its key and the keys it rotated from) includes the installed
     * app's key; several signers, the same set. When the archive's signers could
     * not be read (some platform versions do not fill them for an archive), the
     * installer stays the judge — `null` here means "unknown", and the caller
     * proceeds as the platform decides.
     */
    fun decide(
        archivePackage: String?,
        archiveVersionCode: Long,
        installedPackage: String,
        installedVersionCode: Long,
        archiveSigners: Set<String>?,
        archiveLineage: Set<String>?,
        installedSigners: Set<String>,
        archiveMultipleSigners: Boolean,
        installedMultipleSigners: Boolean,
    ): UpdateDecision {
        if (archivePackage.isNullOrBlank()) return UpdateDecision.Invalid
        if (archivePackage != installedPackage) return UpdateDecision.NotThisApp
        if (archiveVersionCode <= installedVersionCode) return UpdateDecision.UpToDate
        if (archiveSigners.isNullOrEmpty() && archiveLineage.isNullOrEmpty()) {
            // Unreadable signers: the installer is the judge (the old app's rule).
            return UpdateDecision.Install
        }
        if (installedSigners.isEmpty()) return UpdateDecision.Invalid
        if (archiveMultipleSigners || installedMultipleSigners) {
            // Several signers on both sides must be the SAME set.
            val same = (archiveSigners ?: emptySet()) == installedSigners
            return if (same) UpdateDecision.Install else UpdateDecision.DifferentSignature
        }
        val lineage = (archiveLineage ?: emptySet()) + (archiveSigners ?: emptySet())
        return if (lineage.containsAll(installedSigners)) {
            UpdateDecision.Install
        } else {
            UpdateDecision.DifferentSignature
        }
    }
}
