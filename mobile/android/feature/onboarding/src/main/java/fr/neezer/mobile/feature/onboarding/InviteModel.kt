package fr.neezer.mobile.feature.onboarding

import fr.neezer.mobile.bridge.NzErrors
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uniffi.nz_ffi.InviteField
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.OnboardingStep
import uniffi.nz_ffi.RedeemStatus
import java.time.LocalDateTime
import java.time.format.DateTimeFormatter
import java.util.Locale

/**
 * The invitation page's state machine (lot MB5b), `routes/Invite.svelte` ported.
 *
 * Plain Kotlin over [InviteService], so `InviteModelTest` proves each rule on the
 * JVM with a fake. The page's own order:
 *
 * 1. **The link's shape first**: a token that is not `<22>.<24>` is "plus
 *    valide" in **0 requests**.
 * 2. **One request to inspect**, and its three outcomes: valid (the form), a
 *    gone link (the page's invalid state, no retry), anything else (the cause
 *    and « Réessayer »).
 * 3. **The form validates before sending**: the name folds (accents and case
 *    dropped) as it is typed, the password is 12–128 with no composition rule,
 *    and the two must match. A refusal from the SERVER names its field and the
 *    sentence goes under it.
 * 4. **One request to create**, and the account lands where its onboarding says
 *    (`landingFor`).
 *
 * The draft (`nsInvite.draft`: the name and the request id, never the password)
 * lives in the core, so a connection that dies after the server created the
 * account replays the SAME attempt instead of making a second one.
 */
class InviteModel(
    private val service: InviteService,
    private val scope: CoroutineScope,
    private val code: String,
    private val online: () -> Boolean = { true },
    private val describe: (Throwable) -> String = NzErrors::text,
    private val onCreated: (OnboardingStep, String) -> Unit = { _, _ -> },
    private val onOpenApp: () -> Unit = {},
) {
    enum class Phase { Checking, Invalid, Error, Ready, Created }

    enum class Field { Username, Password, Confirm }

    data class State(
        val phase: Phase = Phase.Checking,
        val label: String = "",
        /** ISO-8601, as the server says it. */
        val expires: String? = null,
        val failureTitle: String = "",
        val failure: String? = null,
        /** The person already signed in: the page says so and offers to log out. */
        val accountName: String? = null,
        val username: String = "",
        val password: String = "",
        val confirm: String = "",
        val showPassword: Boolean = false,
        val submitted: Boolean = false,
        val touched: Set<Field> = emptySet(),
        val fieldErrors: Map<Field, String> = emptyMap(),
        val formError: String = "",
        /** The form's own « Réessayer » (the link stays valid). */
        val pendingRetry: Boolean = false,
        val busy: Boolean = false,
        val leaving: Boolean = false,
        val leaveError: String = "",
    ) {
        /** The name's message, once the person has touched the field. */
        val usernameMessage: String
            get() = fieldErrors[Field.Username]
                ?: if (submitted || touched.contains(Field.Username)) "" else ""

        val firstProblem: Field?
            get() = when {
                fieldErrors[Field.Username] != null -> Field.Username
                fieldErrors[Field.Password] != null -> Field.Password
                fieldErrors[Field.Confirm] != null -> Field.Confirm
                else -> null
            }

        /** The form needs the person's own input before it can be sent. */
        val formReady: Boolean
            get() = username.isNotBlank() &&
                password.isNotEmpty() &&
                confirm.isNotEmpty() &&
                !busy
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var job: Job? = null
    private var inspected = false

    /** The page's own messages, computed by the core's rules. */
    fun usernameError(): String = service.usernameMessage(_state.value.username)

    fun passwordError(): String =
        service.passwordMessage(_state.value.password, service.foldUsername(_state.value.username))

    fun confirmError(): String =
        if (_state.value.password.isNotEmpty() &&
            _state.value.confirm.isNotEmpty() &&
            _state.value.password != _state.value.confirm
        ) {
            "Les deux mots de passe ne correspondent pas."
        } else {
            ""
        }

    fun strength(): uniffi.nz_ffi.PasswordStrength =
        service.strength(_state.value.password, service.foldUsername(_state.value.username))

    fun score(): Int = service.score(_state.value.password, service.foldUsername(_state.value.username))

    fun minPassword(): Int = service.rules.minPassword.toInt()

    /** The characters still missing before the floor (`missing`). */
    fun missing(): Int = (minPassword() - _state.value.password.length).coerceAtLeast(0)

    fun expiryLabel(): String = expiryLabel(_state.value.expires)

    // -- the visit ----------------------------------------------------------

    fun start() {
        if (inspected) return
        inspected = true
        _state.update { it.copy(accountName = service.signedIn()?.name) }
        inspect()
    }

    /** `inspect()`: the shape first (0 requests), then ONE request. */
    fun inspect() {
        if (!service.tokenOk(code)) {
            _state.update { it.copy(phase = Phase.Invalid, failureTitle = "", failure = null) }
            return
        }
        _state.update { it.copy(phase = Phase.Checking, failureTitle = "", failure = null) }
        job?.cancel()
        job = scope.launch {
            try {
                val info = service.inspect(code)
                if (!info.valid) {
                    _state.update { it.copy(phase = Phase.Invalid) }
                    return@launch
                }
                _state.update {
                    it.copy(
                        phase = Phase.Ready,
                        label = info.label,
                        expires = info.expires,
                        username = it.username.ifEmpty { info.draftName.orEmpty() },
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val status = statusOf(e)
                if (status == 404) {
                    _state.update { it.copy(phase = Phase.Invalid) }
                    return@launch
                }
                val title = if (status == 429) "Trop de tentatives" else "Impossible de vérifier l'invitation"
                val text = when {
                    status == 429 -> "Trop de tentatives. Réessaie dans quelques minutes."
                    status == null -> if (online()) {
                        "Le serveur ne répond pas. Vérifie ta connexion, puis réessaie."
                    } else {
                        "Tu es hors ligne. La vérification reprendra au retour du réseau."
                    }
                    else -> "Le serveur ne répond pas. Ton lien n'est pas en cause. Réessaie dans un instant."
                }
                _state.update { it.copy(phase = Phase.Error, failureTitle = title, failure = text) }
            }
        }
    }

    // -- the form -----------------------------------------------------------

    /** `onName`: folds WITHOUT touching the edges, so a trailing space survives. */
    fun onUsername(raw: String) {
        val folded = service.foldName(raw)
        _state.update {
            it.copy(
                username = folded,
                fieldErrors = it.fieldErrors - Field.Username,
                formError = "",
                pendingRetry = false,
            )
        }
    }

    fun onPassword(raw: String) {
        _state.update {
            it.copy(
                password = raw,
                fieldErrors = it.fieldErrors - Field.Password - Field.Confirm,
                formError = "",
                pendingRetry = false,
            )
        }
    }

    fun onConfirm(raw: String) {
        _state.update {
            it.copy(
                confirm = raw,
                fieldErrors = it.fieldErrors - Field.Confirm,
                formError = "",
                pendingRetry = false,
            )
        }
    }

    fun toggleShowPassword() {
        _state.update { it.copy(showPassword = !it.showPassword) }
    }

    /** `onBlur`: the value that is sent is folded and trimmed. */
    fun blur(field: Field) {
        _state.update {
            it.copy(
                username = if (field == Field.Username) service.foldUsername(it.username) else it.username,
                touched = it.touched + field,
            )
        }
    }

    /** The submit: the form's rules locally, then ONE request. */
    fun submit() {
        val current = _state.value
        if (current.busy || current.phase != Phase.Ready || current.accountName != null) return
        _state.update { it.copy(submitted = true, formError = "", pendingRetry = false) }
        val username = service.usernameMessage(current.username)
        val password = service.passwordMessage(current.password, service.foldUsername(current.username))
        val confirm = confirmError()
        val first = when {
            username.isNotEmpty() -> Field.Username
            password.isNotEmpty() -> Field.Password
            confirm.isNotEmpty() -> Field.Confirm
            else -> null
        }
        if (first != null) {
            _state.update {
                it.copy(
                    fieldErrors = mapOf(
                        Field.Username to username,
                        Field.Password to password,
                        Field.Confirm to confirm,
                    ).filterValues { message -> message.isNotEmpty() },
                )
            }
            return
        }
        _state.update { it.copy(busy = true, fieldErrors = emptyMap()) }
        job?.cancel()
        job = scope.launch {
            try {
                val answer = service.redeem(
                    code,
                    service.foldUsername(current.username),
                    current.password,
                )
                when (answer.status) {
                    RedeemStatus.CREATED -> {
                        val name = answer.me?.name.orEmpty()
                        _state.update { it.copy(busy = false, phase = Phase.Created) }
                        onCreated(answer.onboarding, name)
                    }
                    RedeemStatus.FIELD -> {
                        val field = when (answer.field) {
                            InviteField.USERNAME -> Field.Username
                            InviteField.PASSWORD -> Field.Password
                            InviteField.CONFIRM -> Field.Confirm
                            null -> Field.Username
                        }
                        _state.update {
                            it.copy(
                                busy = false,
                                fieldErrors = it.fieldErrors + (field to answer.message.ifBlank {
                                    "Cette valeur n'est pas acceptée."
                                }),
                            )
                        }
                    }
                    RedeemStatus.INVALID -> {
                        _state.update { it.copy(busy = false, phase = Phase.Invalid) }
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                val status = statusOf(e)
                if (status == 404) {
                    _state.update { it.copy(busy = false, phase = Phase.Invalid) }
                    return@launch
                }
                val pending = status == null || status >= 500
                val message = when {
                    status == 429 -> "Trop de tentatives. Réessaie dans quelques minutes."
                    status == 413 -> "Requête trop volumineuse."
                    status == null -> "La connexion s'est interrompue. Réessaie : " +
                        "ton lien reste valable tant que le compte n'est pas créé."
                    else -> "Le serveur n'a pas pu créer ton compte. Réessaie dans un instant."
                }
                _state.update {
                    it.copy(busy = false, pendingRetry = pending, formError = message)
                }
            }
        }
    }

    /** « Utiliser cette invitation »: the current session is closed first. */
    fun useInvitation() {
        if (_state.value.leaving) return
        _state.update { it.copy(leaving = true, leaveError = "") }
        job?.cancel()
        job = scope.launch {
            try {
                service.logout()
                _state.update { it.copy(accountName = null, leaving = false) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                _state.update {
                    it.copy(
                        leaving = false,
                        leaveError = "La déconnexion a échoué : vérifiez la connexion, puis réessayez. " +
                            "Votre session actuelle reste ouverte.",
                    )
                }
            }
        }
    }

    /** A signed-in person who came for nothing: « Retour à l'application ». */
    fun openApp() {
        onOpenApp()
    }

    /** Leaving the screen cancels the read in flight (the FFI future is dropped). */
    fun leave() {
        job?.cancel()
    }

    /** The HTTP status behind a failure, when it has one (generated `NzError`). */
    private fun statusOf(e: Throwable): Int? = when (e) {
        is NzException.Server -> e.status.toInt()
        is NzException.NotReady -> 429
        is NzException.Offline -> null
        else -> null
    }

    companion object {
        private val FORMAT = DateTimeFormatter.ofPattern("d MMMM yyyy 'à' HH:mm", Locale.FRENCH)

        /** `expiryLabel`: « Expire le 21 novembre 2026 à 18:00. », never a lie. */
        fun expiryLabel(iso: String?): String {
            if (iso.isNullOrBlank()) return ""
            val at = runCatching { LocalDateTime.parse(iso) }.getOrNull() ?: return ""
            return "Expire le ${FORMAT.format(at)}."
        }
    }
}
