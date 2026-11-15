package org.nsupysonic.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/**
 * What the updater accepts before asking Android to install it: the file the
 * person reported ("Le fichier reçu n'est pas une application valide") was
 * whatever the server had left, so every shape of bad file must be refused.
 */
class ApkArchiveTest {

    @get:Rule
    val folder = TemporaryFolder()

    private var seq = 0

    private fun apk(manifest: ByteArray, padding: Int = 64 * 1024): File {
        val file = folder.newFile("neezer-${seq++}.apk")
        val body = ByteArray(padding).also { java.util.Random(7).nextBytes(it) }
        ZipOutputStream(file.outputStream()).use { zip ->
            zip.putNextEntry(ZipEntry("AndroidManifest.xml"))
            zip.write(manifest)
            zip.closeEntry()
            zip.putNextEntry(ZipEntry("classes.dex"))
            zip.write(body)
            zip.closeEntry()
        }
        return file
    }

    private val realManifest = byteArrayOf(0x03, 0x00, 0x08, 0x00) + ByteArray(60)

    @Test
    fun aRealApkIsAccepted() {
        assertTrue(ApkArchive.looksLikeApk(apk(realManifest)))
    }

    @Test
    fun aTruncatedDownloadIsRefused() {
        // A cut mid-file and a file shorter than any build: the two shapes a
        // dropped connection leaves behind.
        val full = apk(realManifest).readBytes()
        val cut = folder.newFile("cut.apk")
        cut.writeBytes(full.copyOf(full.size / 2))
        assertFalse(ApkArchive.looksLikeApk(cut))
        val tiny = folder.newFile("tiny.apk")
        tiny.writeBytes(full.copyOf(4 * 1024))
        assertFalse(ApkArchive.looksLikeApk(tiny))
    }

    @Test
    fun aPlainZipOrErrorPageIsRefused() {
        // The server's 202/502 JSON (once downloaded as if it were the APK) and
        // a zip that is not an Android package.
        val json = folder.newFile("fetching.apk")
        json.writeText("""{"state": "fetching"}""")
        assertFalse(ApkArchive.looksLikeApk(json))
        assertFalse(ApkArchive.looksLikeApk(apk("x".repeat(128).toByteArray())))
        val noManifest = folder.newFile("empty.apk")
        ZipOutputStream(noManifest.outputStream()).use { zip ->
            zip.putNextEntry(ZipEntry("classes.dex"))
            zip.write(ByteArray(64 * 1024))
            zip.closeEntry()
        }
        assertFalse(ApkArchive.looksLikeApk(noManifest))
    }

    @Test
    fun theSizeFloorIsNotTheTest() {
        // A file above the floor is still judged on its content, and one just
        // above it with a real manifest is accepted.
        val good = apk(realManifest, padding = ApkArchive.MIN_BYTES.toInt())
        assertTrue(ApkArchive.looksLikeApk(good))
        val bad = apk(ByteArray(0), padding = ApkArchive.MIN_BYTES.toInt())
        assertFalse(ApkArchive.looksLikeApk(bad))
    }
}
