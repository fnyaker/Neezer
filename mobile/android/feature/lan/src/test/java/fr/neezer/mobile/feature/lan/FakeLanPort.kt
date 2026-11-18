package fr.neezer.mobile.feature.lan

import uniffi.nz_ffi.LanDeviceFfi
import uniffi.nz_ffi.LanDirectionFfi
import uniffi.nz_ffi.LanFailureFfi
import uniffi.nz_ffi.LanFriendFfi
import uniffi.nz_ffi.LanFriendsFfi
import uniffi.nz_ffi.LanListener
import uniffi.nz_ffi.LanPeerFfi
import uniffi.nz_ffi.LanPlaylistFfi
import uniffi.nz_ffi.LanSettingsFfi
import uniffi.nz_ffi.LanSnapshotFfi
import uniffi.nz_ffi.LanStateFfi
import uniffi.nz_ffi.LanTransferFfi
import uniffi.nz_ffi.LanUrgencyFfi
import uniffi.nz_ffi.PeerSetFfi

/**
 * The JVM suite's node: what the real one answers, without a `.so` (lot LN14).
 *
 * Every call is recorded, so a test asserts the WIRE (`settingsWrites`, `calls`)
 * and not only what the screen ended up drawing — which is how the optimistic
 * write and its rollback are told apart from a screen that never sent anything.
 */
class FakeLanPort(
    var snapshotValue: LanSnapshotFfi = sampleSnapshot(),
    var settingsValue: LanSettingsFfi = sampleSettings(),
    var devicesValue: List<LanDeviceFfi> = listOf(sampleDevice()),
    var friendsValue: LanFriendsFfi = sampleFriends(),
    var peerSetsValue: List<PeerSetFfi> = listOf(sampleSet()),
    var advertised: Int = 46050,
) : LanPort {
    val calls = mutableListOf<String>()
    val settingsWrites = mutableListOf<LanSettingsFfi>()
    val downloaded = mutableListOf<List<String>>()
    var activeFlags: Triple<Boolean, Boolean, Boolean>? = null
    var memoryBytes: Long = 1_536

    /** Injected refusals, one per operation (`null` = the real answer). */
    var failDevices: Throwable? = null
    var failFriends: Throwable? = null
    var failRename: Throwable? = null
    var failRevoke: Throwable? = null
    var failAddFriend: Throwable? = null
    var failSetScope: Throwable? = null
    var failRemoveFriend: Throwable? = null
    var failSettings: Throwable? = null
    var failDownload: Throwable? = null
    var failStart: Throwable? = null

    override fun snapshot(): LanSnapshotFfi = snapshotValue

    override fun settings(): LanSettingsFfi = settingsValue

    override fun setSettings(settings: LanSettingsFfi) {
        calls += "setSettings"
        failSettings?.let { throw it }
        settingsValue = settings
        settingsWrites += settings
    }

    override fun subscribe(listener: LanListener) {
        calls += "subscribe"
    }

    override suspend fun start() {
        calls += "start"
        failStart?.let { throw it }
    }

    override fun stop() {
        calls += "stop"
    }

    override fun found(host: String, port: Int) {
        calls += "found:$host:$port"
    }

    override fun lost(host: String, port: Int) {
        calls += "lost:$host:$port"
    }

    override fun networkChanged() {
        calls += "networkChanged"
    }

    override fun setActive(foreground: Boolean, playing: Boolean, downloading: Boolean) {
        activeFlags = Triple(foreground, playing, downloading)
    }

    override fun advertisedPort(): Int = advertised

    override fun peerSets(): List<PeerSetFfi> = peerSetsValue

    override fun downloadSets(keys: List<String>) {
        calls += "downloadSets"
        failDownload?.let { throw it }
        downloaded += keys
    }

    override suspend fun devices(): List<LanDeviceFfi> {
        calls += "devices"
        failDevices?.let { throw it }
        return devicesValue
    }

    override suspend fun revoke(id: String) {
        calls += "revoke:$id"
        failRevoke?.let { throw it }
    }

    override suspend fun rename(id: String, name: String) {
        calls += "rename:$id:$name"
        failRename?.let { throw it }
    }

    override suspend fun friends(): LanFriendsFfi {
        calls += "friends"
        failFriends?.let { throw it }
        return friendsValue
    }

    override suspend fun addFriend(name: String) {
        calls += "addFriend:$name"
        failAddFriend?.let { throw it }
    }

    override suspend fun setFriendScope(userId: String, all: Boolean, playlists: List<String>) {
        calls += "setFriendScope:$userId:$all:${playlists.joinToString(",")}"
        failSetScope?.let { throw it }
    }

    override suspend fun removeFriend(userId: String) {
        calls += "removeFriend:$userId"
        failRemoveFriend?.let { throw it }
    }

    override fun memoryBytes(): Long = memoryBytes
}

/** The playlists a friend's picker offers, without `/api/me/playlists`. */
class FakePlaylists(
    var choices: List<LanPlaylistFfi> = listOf(
        LanPlaylistFfi(id = "1", title = "Route"),
        LanPlaylistFfi(id = "2", title = "Nuit"),
    ),
    var fail: Throwable? = null,
    var cachedReads: Int = 0,
    var freshReads: Int = 0,
) : LanPlaylistSource {
    override fun cached(): List<LanPlaylistFfi> {
        cachedReads++
        return choices
    }

    override suspend fun fresh(): List<LanPlaylistFfi> {
        freshReads++
        fail?.let { throw it }
        return choices
    }
}

// -- sample records -----------------------------------------------------------

fun sampleSettings(
    enabled: Boolean = true,
    serve: Boolean = true,
    tolerated: List<String> = listOf("FLAC", "OPUS_320", "OPUS_128"),
    port: Int = 45910,
): LanSettingsFfi = LanSettingsFfi(
    enabled = enabled,
    serve = serve,
    tolerated = tolerated,
    port = port.toUShort(),
)

fun sampleSnapshot(
    state: LanStateFfi = LanStateFfi.Searching,
    peers: List<LanPeerFfi> = emptyList(),
    transfers: List<LanTransferFfi> = emptyList(),
    failures: List<LanFailureFfi> = emptyList(),
    keyUntil: Long? = 1_800_000_000L,
    keyError: String? = null,
    port: Int = 45910,
    portFallback: Boolean = false,
): LanSnapshotFfi = LanSnapshotFfi(
    state = state,
    keyUntil = keyUntil,
    keyError = keyError,
    peers = peers,
    transfers = transfers,
    failures = failures,
    port = port.toUShort(),
    portFallback = portFallback,
)

fun samplePeer(
    pub: String = "aa".repeat(32),
    name: String = "Bureau",
    own: Boolean = true,
    platform: String = "desktop",
    rttMs: Int = 7,
    tracks: Int = 1_240,
    sets: Int = 3,
): LanPeerFfi = LanPeerFfi(
    devicePub = pub,
    name = name,
    own = own,
    platform = platform,
    rttMs = rttMs.toUInt(),
    tracks = tracks.toUInt(),
    sets = sets.toUInt(),
)

fun sampleDevice(
    id: String = "dev-1",
    name: String = "Téléphone",
    platform: String = "android",
    current: Boolean = true,
    revoked: String? = null,
): LanDeviceFfi = LanDeviceFfi(
    id = id,
    name = name,
    platform = platform,
    created = "2026-11-01T10:00:00Z",
    lastSeen = "2026-11-18T09:00:00Z",
    revoked = revoked,
    current = current,
)

fun sampleFriend(
    userId: String = "9b2a6f0c-0000-4000-8000-000000000001",
    name: String = "Alice",
    all: Boolean = false,
    playlists: List<LanPlaylistFfi> = emptyList(),
): LanFriendFfi = LanFriendFfi(userId = userId, name = name, all = all, playlists = playlists)

fun sampleFriends(
    sharesTo: List<LanFriendFfi> = listOf(sampleFriend()),
    trustedBy: List<LanFriendFfi> = listOf(sampleFriend(userId = "trusted", name = "Bastien", all = true)),
): LanFriendsFfi = LanFriendsFfi(sharesTo = sharesTo, trustedBy = trustedBy)

fun sampleSet(
    peer: String = "aa".repeat(32),
    peerName: String = "Bureau",
    own: Boolean = true,
    kind: String = "playlist",
    ident: String = "42",
    title: String = "Route",
    tracks: Int = 50,
    local: Int = 0,
    complete: Boolean = true,
    bytes: Long = 350_000_000,
): PeerSetFfi = PeerSetFfi(
    peer = peer,
    peerName = peerName,
    own = own,
    kind = kind,
    ident = ident,
    title = title,
    cover = null,
    tracks = tracks.toUInt(),
    local = local.toUInt(),
    complete = complete,
    bytes = bytes.toULong(),
)

fun sampleTransfer(
    track: String = "3135556",
    title: String = "Insomnia",
    peer: String = "Bureau",
    send: Boolean = false,
    bytes: Long = 12_000_000,
    total: Long = 40_000_000,
    urgency: LanUrgencyFfi = LanUrgencyFfi.PLAY,
): LanTransferFfi = LanTransferFfi(
    track = track,
    title = title,
    peer = peer,
    direction = if (send) LanDirectionFfi.SEND else LanDirectionFfi.RECV,
    bytes = bytes.toULong(),
    total = total.toULong(),
    urgency = urgency,
)

fun sampleFailure(what: String = "Télécharger Route", cause: String = "l'appareil « PC » ne répond plus"): LanFailureFfi =
    LanFailureFfi(what = what, cause = cause)
