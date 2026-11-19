package fr.neezer.mobile.feature.party

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PartyMemberData
import uniffi.nz_ffi.PartyPhase
import uniffi.nz_ffi.PartyPrefs
import uniffi.nz_ffi.PartyRole
import uniffi.nz_ffi.PartyStateData
import uniffi.nz_ffi.PartyTrackData

/**
 * What the party's JVM tests drive: a fake [PartyService] that counts its calls,
 * and the state builders the SPA's own answers produce.
 *
 * The fake is what makes the MODELS testable without a `.so` (MB13's
 * `PodcastService` lesson): every rule this lot adds — a card painted from the
 * copy, a join that trims the name, a failure with its cause — is an assertion on
 * plain Kotlin, and the device test proves the same thing over JNI.
 */
@OptIn(ExperimentalCoroutinesApi::class)
fun partyScope(): TestScope = TestScope(UnconfinedTestDispatcher())

/**
 * The French cause a JVM test can read: `NzErrors.text` calls `nz_error_text`
 * over JNI, and there is no `.so` on the test JVM. The real register is Rust's
 * (`nz-ffi`'s own test), and the app uses it; this only lets a model test say WHY
 * a start failed without loading a native library.
 */
fun testErrorText(e: Throwable): String = when {
    e is NzException.Offline -> "Hors ligne"
    e is NzException.Timeout -> "Le serveur ne répond pas"
    else -> e.message?.takeIf { it.isNotBlank() } ?: e::class.java.simpleName
}

/** One track, as the public state publishes it. */
fun partyTrack(
    id: String = "3000001",
    title: String = "Balancing the Dark",
    artist: String = "Kaya Project",
    durationS: Double = 312.0,
    cover: String? = "/api/party/p1/cover/3000001",
): PartyTrackData = PartyTrackData(
    id = id,
    title = title,
    artist = artist,
    album = "Songs from the Inside",
    durationS = durationS,
    isEpisode = false,
    cover = cover,
)

/** One member, as the host's list names it. */
fun partyMember(
    name: String = "Salon",
    status: String = "En phase · ±0.4 ms",
    latencyMs: UInt = 0u,
    synced: Boolean = true,
    waiting: Boolean = false,
): PartyMemberData = PartyMemberData(
    name = name,
    role = "guest",
    latencyMs = latencyMs,
    status = status,
    initial = name.trim().take(1).uppercase().ifEmpty { "?" },
    synced = synced,
    waiting = waiting,
)

/**
 * The state a screen paints, every field spelled: the generated record has no
 * defaults, and a test that leaves one out by accident is a compile error here —
 * which is what makes a missing wire field loud.
 */
fun partyState(
    role: PartyRole = PartyRole.Guest(id = "p1", hostName = "Camille"),
    phase: PartyPhase = PartyPhase.LIVE,
    host: String = "Camille",
    track: PartyTrackData? = partyTrack(),
    next: PartyTrackData? = null,
    live: Boolean = true,
    playing: Boolean = true,
    loading: Boolean = false,
    positionS: Double = 42.5,
    durationS: Double = 312.0,
    status: String = "Calé sur Camille · ±0.4 ms",
    tone: String = "ok",
    clockSpreadMs: Double? = 0.4,
    clockRttMs: Double? = 8.0,
    offline: Boolean = false,
    members: List<PartyMemberData> = emptyList(),
    listeners: UInt = 1u,
    link: String = "",
    volume: Double = 1.0,
    trimMs: Double = 0.0,
    osLatencyMs: Long? = null,
    error: String? = null,
    inviteLine: String = "Camille vous invite à écouter",
    liveLine: String = "En direct avec Camille",
    nextLine: String? = null,
    titleLine: String = "Balancing the Dark",
    audience: String = "Personne à l'écoute",
    phaseTitle: String? = null,
    phaseBody: String? = null,
    progress: Double = 13.6,
    version: ULong = 7u,
): PartyStateData = PartyStateData(
    role = role,
    phase = phase,
    host = host,
    track = track,
    next = next,
    live = live,
    playing = playing,
    loading = loading,
    positionS = positionS,
    durationS = durationS,
    status = status,
    tone = tone,
    clockSpreadMs = clockSpreadMs,
    clockRttMs = clockRttMs,
    offline = offline,
    members = members,
    listeners = listeners,
    link = link,
    volume = volume,
    trimMs = trimMs,
    osLatencyMs = osLatencyMs,
    error = error,
    inviteLine = inviteLine,
    liveLine = liveLine,
    nextLine = nextLine,
    titleLine = titleLine,
    audience = audience,
    phaseTitle = phaseTitle,
    phaseBody = phaseBody,
    progress = progress,
    version = version,
)

/** The host's own state: a link, a party id, and the guests the sheet lists. */
fun hostState(
    link: String = "https://neezer.lan/party/p1",
    members: List<PartyMemberData> = emptyList(),
    audience: String = "Personne à l'écoute",
    status: String = "Synchronisation de l'horloge…",
    clockSpreadMs: Double? = null,
    trimMs: Double = 0.0,
): PartyStateData = partyState(
    role = PartyRole.Host(id = "p1", link = link, active = true, lost = false),
    phase = PartyPhase.LIVE,
    host = "Camille",
    live = true,
    members = members,
    audience = audience,
    status = status,
    tone = if (clockSpreadMs == null) "wait" else "ok",
    clockSpreadMs = clockSpreadMs,
    clockRttMs = clockSpreadMs?.let { 8.0 },
    link = link,
    trimMs = trimMs,
    liveLine = "",
    inviteLine = "",
)

/**
 * A fake service: it answers what a test tells it to, counts its calls, and lets
 * a test push a state as the Rust side would (`push`).
 */
class FakePartyService(
    var card: PartyStateData? = partyState(),
    var supported: Boolean? = true,
    var cached: PartyStateData? = null,
    var hostLink: String = "https://neezer.lan/party/p1",
    var resumeLink: String? = null,
    var peekError: Exception? = null,
    var joinError: Exception? = null,
    var hostError: Exception? = null,
    var stateNow: PartyStateData = partyState(),
    /** What the network read waits on, so a test can hold it and see the copy. */
    var onPeek: (suspend () -> Unit)? = null,
) : PartyService {
    var peeks = 0
    var cachedPeeks = 0
    var hosts = 0
    var resumes = 0
    var joins = 0
    var leaves = 0
    var supportProbes = 0
    var lastJoinedName: String? = null
    var prefsNow = PartyPrefs("", 1.0, 0.0)
    var position = 0.0
    private var listener: ((PartyStateData) -> Unit)? = null

    override suspend fun supported(): Boolean? {
        supportProbes++
        return supported
    }

    override suspend fun cachedPeek(id: String): PartyStateData? {
        cachedPeeks++
        return cached
    }

    override suspend fun peek(id: String): PartyStateData? {
        peeks++
        onPeek?.invoke()
        peekError?.let { throw it }
        return card
    }

    override suspend fun host(): String {
        hosts++
        hostError?.let { throw it }
        stateNow = hostState(link = hostLink)
        return hostLink
    }

    override suspend fun resume(): String? {
        resumes++
        return resumeLink
    }

    override suspend fun join(id: String, name: String): PartyStateData {
        joins++
        lastJoinedName = name
        joinError?.let { throw it }
        stateNow = partyState(phase = PartyPhase.LIVE)
        return stateNow
    }

    override suspend fun leave() {
        leaves++
        stateNow = partyState(phase = PartyPhase.LEFT)
    }

    override fun state(): PartyStateData = stateNow

    override fun positionNow(): Double = position

    override fun prefs(): PartyPrefs = prefsNow

    override fun setName(name: String) {
        prefsNow = prefsNow.copy(name = name)
    }

    override fun setVolume(volume: Double) {
        prefsNow = prefsNow.copy(volume = volume)
    }

    override fun setTrim(ms: Double) {
        prefsNow = prefsNow.copy(trimMs = ms)
    }

    override fun subscribe(onChange: (PartyStateData) -> Unit) {
        listener = onChange
    }

    /** One push, as the Rust side would send on a change. */
    fun push(data: PartyStateData) {
        stateNow = data
        listener?.invoke(data)
    }
}
