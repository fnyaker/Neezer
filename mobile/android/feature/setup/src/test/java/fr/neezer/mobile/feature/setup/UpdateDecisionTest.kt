package fr.neezer.mobile.feature.setup

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The updater's programme, on the JVM (lot MB4's acceptance: "tests JVM du
 * programme de mise à jour").
 *
 * Every branch the old app's `Updater.verify`/`sameSigner` had, with the
 * `PackageManager`'s answers passed in as values: the same package, a higher
 * `versionCode`, and a signing lineage that contains the installed key. The
 * point of each row is a SENTENCE the screen prints instead of an installer
 * error — `signature` is the only one that keeps no "Réessayer".
 */
class UpdateDecisionTest {

    private val ours = setOf("aa11")

    private fun decide(
        packageName: String? = "fr.neezer.mobile",
        versionCode: Long = 1_235,
        installedVersionCode: Long = 1_234,
        signers: Set<String>? = ours,
        lineage: Set<String>? = ours,
        installedSigners: Set<String> = ours,
        archiveMultiple: Boolean = false,
        installedMultiple: Boolean = false,
    ): UpdateDecision = UpdateDecisionRule.decide(
        archivePackage = packageName,
        archiveVersionCode = versionCode,
        installedPackage = "fr.neezer.mobile",
        installedVersionCode = installedVersionCode,
        archiveSigners = signers,
        archiveLineage = lineage,
        installedSigners = installedSigners,
        archiveMultipleSigners = archiveMultiple,
        installedMultipleSigners = installedMultiple,
    )

    @Test
    fun aNewerBuildOfThisAppSignedWithOurKeyIsInstalled() {
        assertEquals(UpdateDecision.Install, decide())
        assertNull(UpdateDecision.Install.reason())
        assertTrue(UpdateDecision.Install.retryable())
    }

    @Test
    fun theSameOrAnOlderVersionIsNotAnError() {
        assertEquals(UpdateDecision.UpToDate, decide(versionCode = 1_234))
        assertEquals(UpdateDecision.UpToDate, decide(versionCode = 1))
        assertEquals("uptodate", UpdateDecision.UpToDate.reason())
    }

    @Test
    fun anotherPackageIsRefusedBeforeAndroidSeesIt() {
        assertEquals(UpdateDecision.NotThisApp, decide(packageName = "com.example.other"))
        assertEquals("a blank package is unreadable", UpdateDecision.Invalid, decide(packageName = "  "))
        assertEquals(UpdateDecision.Invalid, decide(packageName = null))
        assertEquals("package", UpdateDecision.NotThisApp.reason())
        assertEquals("invalid", UpdateDecision.Invalid.reason())
    }

    @Test
    fun anotherKeyIsASignatureAndItIsTheOneCauseWithNoRetry() {
        val other = decide(signers = setOf("bb22"), lineage = setOf("bb22"))
        assertEquals(UpdateDecision.DifferentSignature, other)
        assertEquals("signature", other.reason())
        assertFalse("a different key is not retried", other.retryable())
        // A CI build without the signing secrets is exactly this case.
        assertEquals(
            UpdateDecision.DifferentSignature,
            decide(signers = setOf("throwaway"), lineage = setOf("throwaway")),
        )
    }

    @Test
    fun aRotatedKeyWhoseLineageCarriesTheOursIsAccepted() {
        // Android's own rule: the update's history includes the installed key.
        assertEquals(
            UpdateDecision.Install,
            decide(signers = setOf("new"), lineage = setOf("new", "aa11")),
        )
        assertEquals(
            UpdateDecision.DifferentSignature,
            decide(signers = setOf("new"), lineage = setOf("new")),
        )
    }

    @Test
    fun severalSignersMustBeTheSameSet() {
        val two = setOf("aa11", "cc33")
        assertEquals(
            UpdateDecision.Install,
            decide(signers = two, lineage = two, installedSigners = two, archiveMultiple = true, installedMultiple = true),
        )
        assertEquals(
            UpdateDecision.DifferentSignature,
            decide(
                signers = setOf("aa11", "dd44"),
                lineage = setOf("aa11", "dd44"),
                installedSigners = two,
                archiveMultiple = true,
                installedMultiple = true,
            ),
        )
        // One side multiple, the other not: the sets differ, so it is a refusal.
        assertEquals(
            UpdateDecision.DifferentSignature,
            decide(signers = setOf("aa11", "cc33"), lineage = setOf("aa11", "cc33"), archiveMultiple = true),
        )
    }

    @Test
    fun unreadableSignersLeaveTheVerdictToTheInstaller() {
        // Some platform versions do not fill an ARCHIVE's signers: the old app
        // proceeded and let Android judge, rather than refusing a good update.
        assertEquals(UpdateDecision.Install, decide(signers = null, lineage = null))
        assertEquals(UpdateDecision.Install, decide(signers = emptySet(), lineage = emptySet()))
    }

    @Test
    fun theFloorAndTheArchiveAreCheckedBeforeThePackageFacts() {
        // `Updater.verify` refuses a short file and a non-zip with `invalid`
        // before it reads any packaging fact (ApkArchiveTest holds those two).
        assertEquals("invalid", UpdateDecision.Invalid.reason())
        assertTrue("a truncated file keeps Réessayer", UpdateDecision.Invalid.retryable())
    }
}
