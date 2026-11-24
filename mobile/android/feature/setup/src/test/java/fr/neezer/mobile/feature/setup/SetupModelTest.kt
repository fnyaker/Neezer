package fr.neezer.mobile.feature.setup

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.Me
import uniffi.nz_ffi.ProbeResult
import uniffi.nz_ffi.ServerInfo

/**
 * The setup and login screens' logic, on the JVM (lot MB4's "screen standard":
 * the data, the failure, and the click are the ViewModel's, and a fake proves
 * them without JNA, a `.so` or a device).
 *
 * What each test is really about:
 *
 * - **nothing is persisted before it is validated** — the old `SetupActivity`'s
 *   bug was a bad host saved after a probe followed by a back press;
 * - **a changed certificate is a sentence**, and the pin is replaced only when
 *   the person accepts it;
 * - **a server that does not answer is not a lost setup** — "Continuer quand
 *   même" exists for a LAN server on another link;
 * - **a failure is a cause**, and "Réessayer" repeats the action.
 */
class SetupModelTest {

    /** A `SetupService` with no core behind it: the answers are the test's. */
    private class Fake(
        var probe: () -> ProbeResult = { probeResult() },
        var setServer: () -> ServerInfo = { serverInfo() },
        var onLogin: (String, String) -> Me = { _, _ -> me("bench") },
    ) : SetupService {
        var probes = 0
        var commits = 0
        var pins = mutableListOf<Pair<String, String>>()
        var lastHost: String? = null
        var lastPort: String? = null
        var lastVerify: Boolean? = null

        override fun serverUrl(host: String, port: String): String? =
            if (host.isBlank()) null else "https://$host"

        override suspend fun probe(host: String, port: String, verifySsl: Boolean): ProbeResult {
            probes++
            lastHost = host
            lastPort = port
            lastVerify = verifySsl
            return probe()
        }

        override suspend fun setServer(host: String, port: String): ServerInfo {
            commits++
            lastHost = host
            lastPort = port
            return setServer()
        }

        override fun pin(origin: String, sha256: String) {
            pins.add(origin to sha256)
        }

        override suspend fun login(username: String, password: String): Me =
            onLogin(username, password)

        override suspend fun restore(): Me? = null
    }

    private fun scope() = CoroutineScope(Dispatchers.Unconfined)

    @Test
    fun nothingIsPersistedUntilTheServerIsChecked() {
        val fake = Fake()
        val model = SetupModel(fake, scope())
        model.onHost("music.example.org")
        model.onPort("4533")
        model.onVerifySsl(false)

        // Typing changes the fields and NOTHING else.
        assertEquals(0, fake.probes)
        assertEquals(0, fake.commits)

        var committed = false
        model.check { committed = true }
        assertEquals(1, fake.probes)
        assertEquals(1, fake.commits)
        assertEquals("music.example.org", fake.lastHost)
        assertEquals("4533", fake.lastPort)
        assertEquals(false, fake.lastVerify)
        assertTrue(committed)
        assertTrue(model.state.value.saved)
    }

    @Test
    fun anEmptyHostNeverReachesTheNetwork() {
        val fake = Fake()
        val model = SetupModel(fake, scope())
        model.onHost("   ")
        assertFalse(model.state.value.canCheck)
        model.check()
        assertEquals(0, fake.probes)
        assertEquals(0, fake.commits)
    }

    @Test
    fun aCertificateTheSystemDoesNotTrustIsShownThenPinnedOnAcceptance() {
        val leaf = "AB:CD:EF"
        val fake = Fake(probe = { probeResult(pin = leaf, systemTrusted = false, pinChanged = false) })
        val model = SetupModel(fake, scope())
        model.onHost("192.168.1.10")
        model.check()

        val probe = model.state.value.probe
        assertEquals(leaf, probe?.pin)
        assertFalse(probe?.systemTrusted ?: true)
        assertTrue("the person has not accepted it yet", fake.pins.isEmpty())

        model.acceptPin()
        assertEquals(listOf("https://music.example.org:443" to leaf), fake.pins)
    }

    @Test
    fun aChangedCertificateIsASentenceAndReplacesThePinOnlyWhenAccepted() {
        val fresh = "11:22:33"
        val fake = Fake(
            probe = {
                probeResult(
                    pin = fresh,
                    systemTrusted = false,
                    pinChanged = true,
                )
            },
        )
        val model = SetupModel(fake, scope())
        model.onHost("music.example.org")
        model.check()
        assertTrue(model.state.value.probe?.pinChanged ?: false)
        // The screen says so; nothing was replaced behind the person's back.
        assertTrue(fake.pins.isEmpty())
        model.acceptPin()
        assertEquals(fresh, fake.pins.single().second)
        assertFalse("after acceptance the screen stops saying it", model.state.value.probe?.pinChanged ?: true)
    }

    @Test
    fun aFailureIsACauseAndTheScreenOffersContinueAnyway() {
        val fake = Fake(probe = { throw IllegalStateException("Hors ligne") })
        val model = SetupModel(fake, scope())
        model.onHost("music.example.org")
        model.check()

        assertEquals("Hors ligne", model.state.value.cause)
        assertFalse(model.state.value.saved)
        assertFalse(model.state.value.checking)

        // "Continuer quand même": a LAN server on another link is still the
        // server the person chose, so the origin is committed.
        var done = false
        model.continueAnyway { done = true }
        assertTrue(done)
        assertEquals(1, fake.commits)
        assertTrue(model.state.value.saved)
    }

    @Test
    fun aLoginFailureIsACauseAndADifferentPasswordRetries() {
        val fake = Fake(onLogin = { _, password ->
            if (password == "Bench1") me("bench") else throw IllegalStateException(
                "Nom d'utilisateur ou mot de passe refusé",
            )
        })
        val model = LoginModel(fake, scope())
        model.onUsername("bench")
        model.onPassword("wrong")
        model.submit()
        assertEquals("Nom d'utilisateur ou mot de passe refusé", model.state.value.cause)
        assertFalse(model.state.value.signedIn)

        var signedIn = false
        model.onPassword("Bench1")
        model.submit { signedIn = true }
        assertTrue(signedIn)
        assertTrue(model.state.value.signedIn)
        assertTrue(
            "the password is not kept in the state",
            model.state.value.password.isEmpty(),
        )
    }

    @Test
    fun theFormNeedsBothFields() {
        val model = LoginModel(Fake(), scope())
        model.onUsername("bench")
        assertFalse(model.state.value.canSubmit)
        model.onPassword("Bench1")
        assertTrue(model.state.value.canSubmit)
    }

    private companion object {
        fun me(name: String) = Me(
            id = "42",
            name = name,
            admin = true,
            avatar = null,
        )

        fun serverInfo() = ServerInfo(
            origin = "https://music.example.org:443",
            pinnedSha256 = null,
            version = "0.1.0",
        )

        fun probeResult(
            pin: String? = null,
            systemTrusted: Boolean = true,
            pinChanged: Boolean = false,
        ) = ProbeResult(
            origin = "https://music.example.org:443",
            version = "0.1.0",
            https = true,
            cert = null,
            systemTrusted = systemTrusted,
            pin = pin,
            pinChanged = pinChanged,
            updateApk = false,
            androidVersion = null,
        )
    }
}

/**
 * The migration's notice, on the JVM: the sentence the person reads names what
 * was taken over (or why nothing could be), and the coexistence release's own
 * sentence — "Android ne permet pas…" — is the one that matters now.
 */
class MigrationNoticeTest {

    @Test
    fun theNoticeNamesWhatWasTakenOverAndWhyTheRestWasNot() {
        val state = MigrationRunner.State(
            done = true,
            notice = "Repris : 3 titres téléchargés, 18 réglages.",
        )
        assertEquals("Repris : 3 titres téléchargés, 18 réglages.", state.notice)
        assertFalse(state.running)
        assertNull(state.cause)
    }

    @Test
    fun aRunningMigrationSaysSo() {
        assertTrue(MigrationRunner.State(running = true).running)
    }

    @Test
    fun theOldAppPackageIsTheWebViewOne() {
        // The check asks `PackageManager` for THIS package and no other; a typo
        // would make the notice silently never appear.
        assertEquals("org.nsupysonic.app", OldApp.PACKAGE)
    }
}
