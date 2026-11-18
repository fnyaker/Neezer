package fr.neezer.mobile.feature.remote

import kotlinx.coroutines.flow.MutableStateFlow
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.RemoteCommand
import uniffi.nz_ffi.RemoteController
import uniffi.nz_ffi.RemoteDuration
import uniffi.nz_ffi.RemoteGrant
import uniffi.nz_ffi.RemoteLevel
import uniffi.nz_ffi.RemoteLink
import uniffi.nz_ffi.RemoteListener
import uniffi.nz_ffi.RemoteLocalSettings
import uniffi.nz_ffi.RemoteNow
import uniffi.nz_ffi.RemotePhase
import uniffi.nz_ffi.RemoteStatus
import uniffi.nz_ffi.TrackRow

/**
 * The remote service as a JVM test sees it: plain Kotlin, no JNA, no `.so`.
 *
 * Every call is counted ([calls]) so a test can hold the sheet's request budget,
 * and every answer is the FFI's own record — the screen's conversions are what
 * runs, not a copy of them.
 */
class FakeRemoteService(
    var links: MutableList<RemoteLink> = mutableListOf(),
    var failLinks: String? = null,
    var failCreate: String? = null,
    var failClaim: String? = null,
    var failSend: String? = null,
) : RemoteService {
    val calls = mutableListOf<String>()
    var hosting = false
    var started = 0
    var claimedWith: String? = null
    var left = 0
    var sent: MutableList<RemoteCommand> = mutableListOf()

    var status: RemoteStatus = statusOf()
    var now: RemoteNow = nowOf()
    var grant: RemoteGrant? = null
    private var listener: RemoteListener? = null

    val cached = mutableListOf<RemoteLink>()

    private fun record(call: String) {
        calls += call
    }

    override fun status(): RemoteStatus = status

    override fun now(): RemoteNow = now

    override fun position(): Double = now.positionS

    override fun queue(): ListHandle = error("un ListeHandle ne se fabrique pas sur la JVM")

    override fun cachedLinks(): List<RemoteLink> {
        record("cachedLinks")
        return cached.toList()
    }

    override suspend fun links(): List<RemoteLink> {
        record("links")
        failLinks?.let { throw IllegalStateException(it) }
        cached.clear()
        cached.addAll(links)
        return links.toList()
    }

    override suspend fun createLink(
        level: String,
        duration: String,
        label: String?,
    ): RemoteLink {
        record("createLink:$level:$duration:${label ?: ""}")
        failCreate?.let { throw IllegalStateException(it) }
        val made = link(
            id = "link-${links.size + 1}",
            level = level,
            label = label ?: "",
            expiry = if (duration == "none") "Sans limite" else "Expire dans 1 jour",
        )
        links.add(0, made)
        return made
    }

    override suspend fun revokeLink(id: String) {
        record("revokeLink:$id")
        links.removeAll { it.id == id }
    }

    override suspend fun revokeAll() {
        record("revokeAll")
        links.clear()
        hosting = false
    }

    override suspend fun claim(token: String): RemoteGrant {
        record("claim:$token")
        claimedWith = token
        failClaim?.let { throw IllegalStateException(it) }
        val held = grant ?: error("aucun prêt")
        status = status.copy(role = "controller", grant = held, phase = RemotePhase.LIVE, online = true)
        return held
    }

    override suspend fun startLoan() {
        record("startLoan")
        started += 1
    }

    override suspend fun leave(): String? {
        record("leave")
        left += 1
        return "friend"
    }

    override suspend fun send(command: RemoteCommand) {
        record("send:${command::class.simpleName}")
        failSend?.let { throw IllegalStateException(it) }
        sent += command
        now = now.copy(playing = command is RemoteCommand.Pause && false || command is RemoteCommand.Play)
    }

    override suspend fun startHost() {
        record("startHost")
        hosting = true
        status = status.copy(role = "controlled", hostActive = true, who = "Contrôlé par Léa")
    }

    override suspend fun resumeHost(): Boolean {
        record("resumeHost")
        return hosting
    }

    override fun stopHost(): Boolean {
        hosting = false
        return true
    }

    override fun hostedBefore(): Boolean = hosting

    override fun levels(): List<RemoteLevel> = listOf(
        RemoteLevel("queue", "File d'attente", "Lecture, pause, titre suivant, position, volume.", "File"),
        RemoteLevel("read", "Lecture seule", "Toute l'app sans rien modifier.", "Lecture seule"),
        RemoteLevel("full", "Complet", "Tout sauf l'administration.", "Complet"),
    )

    override fun durations(): List<RemoteDuration> = listOf(
        RemoteDuration("1h", "1 heure"),
        RemoteDuration("1d", "1 jour"),
        RemoteDuration("7d", "7 jours"),
        RemoteDuration("none", "Sans limite"),
    )

    override fun localSettings(): RemoteLocalSettings =
        RemoteLocalSettings("encre", "bars", false, "FLAC")

    override fun subscribe(listener: RemoteListener) {
        this.listener = listener
    }

    /** Make this service a LOAN, as `/api/me` would report one. */
    fun asLoan(grant: RemoteGrant) {
        this.grant = grant
        status = statusOf(role = "controller", grant = grant, phase = RemotePhase.LIVE, online = true)
    }

    /** What Rust pushes: a controller arrived, a command was acked, a loan ended. */
    fun push(status: RemoteStatus) {
        this.status = status
        listener?.onStatus(status)
    }

    fun endLoan(restored: String?) {
        listener?.onEnded(restored)
    }

    companion object {
        fun link(
            id: String = "link-1",
            level: String = "queue",
            label: String = "Léa",
            expiry: String = "Expire dans 1 jour",
            controllers: Int = 0,
        ) = RemoteLink(
            id = id,
            url = "https://neezer.test/rc/$id",
            level = level,
            levelLabel = when (level) {
                "read" -> "Lecture seule"
                "full" -> "Complet"
                "admin" -> "Administrateur"
                else -> "File d'attente"
            },
            label = label,
            title = if (label.isBlank()) "File d'attente" else "File d'attente · pour $label",
            expiry = expiry,
            controllers = controllers.toUInt(),
        )

        fun statusOf(
            role: String = "none",
            who: String = "",
            controllers: List<RemoteController> = emptyList(),
            grant: RemoteGrant? = null,
            phase: RemotePhase = RemotePhase.CONNECTING,
            online: Boolean = false,
            restored: String? = null,
        ) = RemoteStatus(
            role = role,
            hostActive = role == "controlled",
            who = who,
            controllers = controllers,
            hostLevel = null,
            applied = 0u,
            hostError = null,
            phase = phase,
            online = online,
            grant = grant,
            line = if (grant == null) "Connexion…" else "En direct · ${grant.levelLabel}",
            error = null,
            restored = restored,
            version = 1uL,
        )

        fun grant(level: String = "full", owner: String = "owner") = RemoteGrant(
            level = level,
            levelLabel = when (level) {
                "queue" -> "File d'attente"
                "read" -> "Lecture seule"
                "admin" -> "Administrateur"
                else -> "Complet"
            },
            owner = owner,
            device = "Linux · Neezer",
            label = "Salon",
            canBrowse = level != "queue",
            canKeep = level == "full" || level == "admin",
            canAdmin = level == "admin",
        )

        fun nowOf(
            playing: Boolean = false,
            index: Int = 0,
            position: Double = 42.0,
            duration: Double = 200.0,
            queueVersion: Long = 1,
            title: String = "Alpha",
        ) = RemoteNow(
            track = TrackRow(
                key = "abc",
                id = "abc",
                title = title,
                artists = "Artiste",
                album = "Album",
                albumId = null,
                durationS = 200u,
                cover = null,
                explicit = false,
                liked = false,
                disliked = false,
                unavailable = false,
                downloaded = false,
                source = "deezer",
                position = 0u,
            ),
            playing = playing,
            index = index.toLong(),
            len = 3u,
            positionS = position,
            durationS = duration,
            volume = 0.8,
            muted = false,
            shuffle = false,
            repeat = "off",
            status = if (playing) "playing" else "paused",
            online = true,
            queueVersion = queueVersion.toULong(),
        )
    }
}
