package fr.neezer.mobile.feature.setup

import fr.neezer.mobile.bridge.NzErrors
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uniffi.nz_ffi.Me
import uniffi.nz_ffi.ProbeResult
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.ServerInfo

/**
 * What the setup and login screens ask of the core (lot MB4).
 *
 * An interface, not the generated `Screens` class, so the two screens' logic is
 * plain Kotlin that `SetupModelTest` runs on the JVM with a fake — no JNA, no
 * `.so`, no device. `NzScreens` is the only implementation, and it is a
 * one-to-one delegation.
 */
interface SetupService {
    /** The server root `host`/`port` mean, normalised by the core. */
    fun serverUrl(host: String, port: String): String?

    /** Learns the certificate and reads `GET /api/version`, saving nothing. */
    suspend fun probe(host: String, port: String, verifySsl: Boolean): ProbeResult

    /** Commits the server (its origin is remembered even if the probe fails). */
    suspend fun setServer(host: String, port: String): ServerInfo

    /** Pins one leaf for one origin (trust on first use). */
    fun pin(origin: String, sha256: String)

    /** `POST /api/login`; the jar keeps the cookie. */
    suspend fun login(username: String, password: String): Me

    /** `GET /api/me` with the stored cookie, `null` when nobody is there. */
    suspend fun restore(): Me?
}

/**
 * The one real implementation: `nz-ffi`'s `Screens` for this lot's calls, and
 * `NzCore` for the session ones (MB2 put `login`/`restore` on the core object —
 * they are the shell's, not a screen's).
 */
class NzScreens(
    private val screens: Screens,
    private val core: uniffi.nz_ffi.NzCore,
) : SetupService {
    override fun serverUrl(host: String, port: String): String? =
        screens.serverUrl(host, port)

    override suspend fun probe(host: String, port: String, verifySsl: Boolean): ProbeResult =
        screens.probeServer(host, port, verifySsl)

    override suspend fun setServer(host: String, port: String): ServerInfo =
        screens.setServer(host, port)

    override fun pin(origin: String, sha256: String) {
        screens.pinServerCertificate(origin, sha256)
    }

    override suspend fun login(username: String, password: String): Me =
        core.login(username, password)

    override suspend fun restore(): Me? = core.restore()
}

/**
 * Whether the probe's leaf is one the person must accept on screen before the
 * server may be used.
 *
 * Two cases, and one non-case:
 * - the platform does not vouch for the chain (`systemTrusted == false`): the pin
 *   is the only trust there is, and it must be the person's tap;
 * - a pin is held and the server now presents another leaf (`pinChanged`): the
 *   stored pin refuses every request until it is replaced, so accepting is the
 *   only way forward (the old app re-pinned on save, with no question asked);
 * - the platform trusts the chain and the pin (if any) is the same leaf: nothing
 *   to accept, and `CertificateCard` draws no button for it either.
 */
internal fun ProbeResult.needsAcceptance(): Boolean = pin != null && (!systemTrusted || pinChanged)

/**
 * The setup screen's state machine (lot MB4).
 *
 * Every rule the old `SetupActivity` learned the hard way is here, in order:
 *
 * - **Nothing is persisted before it is validated.** The probe uses the typed
 *   values; only "Se connecter" commits them (`setServer`), so a person who hits
 *   Retry then backs out never leaves a bad host saved.
 * - **A certificate the system does not vouch for is SHOWN, then accepted.** Its
 *   SHA-256 is on screen, `pin` is the value to keep, and a leaf that differs
 *   from the one already pinned says "le certificat a changé" — the pin is
 *   replaced only when the person accepts it.
 * - **An unaccepted leaf BLOCKS the account screen.** When the probe needs an
 *   acceptance, `check` stops on the certificate card: it commits nothing and
 *   navigates nowhere, and `acceptPin` pins first, then commits the origin (so
 *   the core's client is rebuilt over that pin) and hands the screen on. v1.6.5
 *   did the opposite, and a phone met a refused certificate on the login.
 * - **A server that does not answer is not a lost setup.** The origin is
 *   remembered, the cause is on screen, and "Continuer quand même" exists for a
 *   LAN server on another link (the old app's escape hatch).
 */
class SetupModel(
    private val service: SetupService,
    private val scope: CoroutineScope,
) {
    data class State(
        val host: String = "",
        val port: String = "",
        val verifySsl: Boolean = true,
        val checking: Boolean = false,
        /** The server answer, once the probe succeeded. */
        val probe: ProbeResult? = null,
        /** The cause of the last failure, in the person's words. */
        val cause: String? = null,
        /** The typed values reached the core: the server is saved. */
        val saved: Boolean = false,
        /** The notice of the migration, when one ran. */
        val notice: String? = null,
    ) {
        /** Whether "Se connecter" may be pressed. */
        val canCheck: Boolean get() = host.isNotBlank() && !checking

        /**
         * The probe showed a leaf the person must accept before this server may
         * be used: it is exactly the condition `CertificateCard` draws its
         * "Accepter" button under, so the screen and the state machine cannot
         * disagree about who is waiting for whom.
         */
        val needsAcceptance: Boolean get() = probe?.needsAcceptance() ?: false
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    fun onHost(value: String) = _state.update { it.copy(host = value, cause = null) }

    fun onPort(value: String) = _state.update { it.copy(port = value, cause = null) }

    fun onVerifySsl(value: Boolean) = _state.update { it.copy(verifySsl = value) }

    /** Seeds the fields from what the core already knows (a configured person). */
    fun seed(host: String?, port: String?) {
        if (host.isNullOrBlank()) return
        _state.update { it.copy(host = host, port = port.orEmpty()) }
    }

    fun migrationNotice(notice: String?) = _state.update { it.copy(notice = notice) }

    /** "Vérifier le serveur": the probe, then the origin is committed. */
    fun check(onDone: () -> Unit = {}) {
        val state = _state.value
        if (!state.canCheck) return
        _state.update { it.copy(checking = true, cause = null) }
        scope.launch {
            try {
                val probe = service.probe(state.host, state.port, state.verifySsl)
                // A leaf the platform does not trust (or one that changed under a
                // held pin) is NOT a server the person may sign in to yet: the
                // screen stays on the certificate card and only "Accepter" —
                // `acceptPin`, which pins FIRST and commits the origin after —
                // takes them to the account. Measured on v1.6.5: `check`
                // committed the server and navigated with the leaf unpinned, so
                // the login request met a certificate the platform refused and
                // the person read "certificat non accepté" with no way forward.
                if (probe.needsAcceptance()) {
                    _state.update { it.copy(checking = false, probe = probe) }
                    return@launch
                }
                service.setServer(state.host, state.port)
                _state.update { it.copy(checking = false, probe = probe, saved = true) }
                onDone()
            } catch (error: Exception) {
                _state.update {
                    it.copy(checking = false, cause = NzErrors.text(error))
                }
            }
        }
    }

    /**
     * "Continuer quand même" (the old app's escape hatch): the origin is
     * committed without a successful probe — a LAN server that is only reachable
     * at home is still the server the person chose.
     */
    fun continueAnyway(onDone: () -> Unit = {}) {
        val state = _state.value
        if (state.host.isBlank()) return
        _state.update { it.copy(checking = true) }
        scope.launch {
            try {
                service.setServer(state.host, state.port)
                _state.update { it.copy(checking = false, saved = true, cause = null) }
                onDone()
            } catch (error: Exception) {
                // An offline link is the normal case here: the origin is stored
                // anyway, so this counts as a success with a note.
                _state.update {
                    it.copy(checking = false, saved = true, cause = NzErrors.text(error))
                }
                onDone()
            }
        }
    }

    /** Accepts the leaf the probe showed, which replaces a changed pin. */
    fun acceptPin(onDone: () -> Unit = {}) {
        val state = _state.value
        val probe = state.probe ?: return
        val pin = probe.pin ?: return
        runCatching { service.pin(probe.origin, pin) }
            .onSuccess {
                _state.update { it.copy(probe = it.probe?.copy(pinChanged = false)) }
                // ORDER MATTERS, and the core is what proves it: `set_server`
                // rebuilds the one HTTP client over the pin it finds
                // (`nz_core::core::server::build_api_with`), so the login and
                // every later request go through the leaf the person just
                // accepted. Committing the origin first would hand the session
                // the platform verifier alone — the v1.6.5 defect.
                scope.launch {
                    try {
                        service.setServer(state.host, state.port)
                        _state.update { it.copy(saved = true, cause = null) }
                        onDone()
                    } catch (error: Exception) {
                        _state.update { it.copy(cause = NzErrors.text(error)) }
                    }
                }
            }
            .onFailure { error ->
                _state.update { it.copy(cause = NzErrors.text(error)) }
            }
    }
}

/**
 * The login screen's state machine: one request, the cause on failure, and
 * nothing retried by itself (rule 3).
 */
class LoginModel(
    private val service: SetupService,
    private val scope: CoroutineScope,
) {
    data class State(
        val username: String = "",
        val password: String = "",
        val busy: Boolean = false,
        val cause: String? = null,
        val signedIn: Boolean = false,
    ) {
        val canSubmit: Boolean
            get() = username.isNotBlank() && password.isNotBlank() && !busy
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    fun onUsername(value: String) = _state.update { it.copy(username = value, cause = null) }

    fun onPassword(value: String) = _state.update { it.copy(password = value, cause = null) }

    fun submit(onDone: () -> Unit = {}) {
        val state = _state.value
        if (!state.canSubmit) return
        _state.update { it.copy(busy = true, cause = null) }
        scope.launch {
            try {
                service.login(state.username, state.password)
                _state.update { it.copy(busy = false, signedIn = true, password = "") }
                onDone()
            } catch (error: Exception) {
                _state.update { it.copy(busy = false, cause = NzErrors.text(error)) }
            }
        }
    }

    /** A launch with a cookie: the person is already signed in. */
    fun restore(onSignedIn: () -> Unit = {}) {
        scope.launch {
            val me = runCatching { service.restore() }.getOrNull()
            if (me != null) {
                _state.update { it.copy(signedIn = true, username = me.name) }
                onSignedIn()
            }
        }
    }
}
