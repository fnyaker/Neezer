package org.nsupysonic.app

import java.io.File
import java.io.InputStream
import java.util.zip.ZipFile

/**
 * What makes a file an APK: a zip whose `AndroidManifest.xml` is Android's
 * BINARY XML (the header `getPackageArchiveInfo` parses), above the size of
 * any real build. Pure JVM on purpose — `ApkArchiveTest` holds it against a
 * truncated file, a plain zip and a real one.
 */
object ApkArchive {
    const val MIN_BYTES = 64L * 1024
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
