package fr.neezer.mobile.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import fr.neezer.mobile.bridge.NzBridge
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * La confiance TLS par le vérificateur du système, sur un appareil (v1.6.6).
 *
 * `rustls-platform-verifier`'s Android half is a Rust crate that reaches the JVM
 * class `org.rustls.platformverifier.CertificateVerifier` through JNI, by name:
 * R8 cannot see that edge, so a minified release without
 * `:bridge/consumer-rules.pro`'s keep rule strips the class and every
 * system-trust handshake fails — "Vérifier le certificat" then reported the
 * owner's server offline and the login refused a plain Let's Encrypt
 * certificate, while debug builds and the unminified smoke stayed green.
 *
 * This test runs in the ordinary instrumented suite (`connectedDebugAndroidTest`)
 * and does two things on a real device:
 *
 * 1. it loads the JNI-reached class by its ORIGINAL name — a build that lost it
 *    fails here with the name R8 dropped;
 * 2. it asks the Rust core for the leaf `https://letsencrypt.org` presents and
 *    requires `system_trusted == true`, which is the platform verifier answering
 *    over JNI. No certificate body, cookie or credential is logged.
 *
 * The MINIFIED artifact is guarded where it can be read reliably instead of
 * here: AGP minifies the androidTest APK too (it follows the tested build type),
 * and measuring that (run 45589469291) showed instrumentation dying before the
 * first test — `NoSuchMethodError: … Lkotlin/jvm/internal/k;`, the test APK's
 * R8 run renaming its own Kotlin runtime copy differently from the app's.
 * `mobile.yaml` therefore checks `mapping/release/mapping.txt` AND the universal
 * APK's dex for the class.
 *
 * No network means no answer at all, so the harness skips (and says why) instead
 * of failing a device that cannot reach the internet: the CI emulator has
 * egress, and a skip there is visible in the run.
 */
@RunWith(AndroidJUnit4::class)
class ReleaseTlsTrustTest {

    private fun hasNetwork(): Boolean {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            ?: return false
        val network = manager.activeNetwork ?: return false
        val capabilities = manager.getNetworkCapabilities(network) ?: return false
        return capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
    }

    @Test
    fun theShippedBuildTrustsALetsEncryptCertificate() {
        assumeTrue("aucun réseau sur cet émulateur : la sonde TLS ne peut pas répondre", hasNetwork())

        // The class the crate calls by name. `mobile.yaml` proves the R8 mapping
        // keeps it; this proves the same fact on the APK that is actually
        // installed, which is what a person runs.
        assertNotNull(
            "R8 a supprimé ou renommé org.rustls.platformverifier.CertificateVerifier : " +
                "le vérificateur TLS du système est injoignable par JNI " +
                "(voir :bridge/consumer-rules.pro)",
            Class.forName("org.rustls.platformverifier.CertificateVerifier"),
        )

        val core = NzBridge.core
        assertNotNull("le noyau Rust n'est pas disponible : ${NzBridge.failure}", core)

        val info = runBlocking { core!!.probeCertificate("https://letsencrypt.org") }
        assertTrue(
            "Let's Encrypt doit être digne de confiance pour le magasin d'Android : " +
                "system_trusted=${info.systemTrusted}, issuer=${info.issuer}",
            info.systemTrusted,
        )
    }
}
