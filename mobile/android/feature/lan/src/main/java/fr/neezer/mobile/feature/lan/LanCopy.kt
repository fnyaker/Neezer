package fr.neezer.mobile.feature.lan

import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import uniffi.nz_ffi.LanDirectionFfi
import uniffi.nz_ffi.LanPeerFfi
import uniffi.nz_ffi.LanSnapshotFfi
import uniffi.nz_ffi.LanStateFfi
import uniffi.nz_ffi.LanTransferFfi
import uniffi.nz_ffi.LanUrgencyFfi

/**
 * The words of the two LAN screens, as pure functions.
 *
 * They live here — not inside the composables — because each one is a RULE the
 * JVM suite holds: « connecté », the tolerated qualities, the French cause of a
 * refusal, the 10 s hint. A sentence that can be asserted is a sentence that
 * cannot drift from the note (`docs/agents/lan-nodes.md`, *Screens and
 * Réglages*).
 */

/** The four tolerated qualities, in the note's own order (`lan.tolerated`). */
val TOLERATED_CHOICES: List<Pair<String, String>> = listOf(
    "FLAC" to "FLAC",
    "OPUS_320" to "Opus 320",
    "OPUS_128" to "Opus 128",
    "OPUS_64" to "Opus 64",
)

/** The note's explanation, verbatim: what the list is FOR (rule 6: say it). */
const val TOLERATED_HINT =
    "Utilisées quand un appareil proche a le titre et que le serveur est joignable. " +
        "Sans serveur, la qualité de l'appareil est toujours acceptée."

/** The hotel line the person must read when nothing answers (rule 3). */
const val NO_PEER_HINT =
    "Aucun appareil trouvé. Sur un Wi-Fi d'hôtel, les appareils sont souvent isolés : " +
        "partagez la connexion du téléphone."

/** How many seconds without a peer before the hotel line appears. */
const val NO_PEER_HINT_AFTER_MS: Long = 10_000

/** A quality id as the person reads it ("OPUS_320" → "Opus 320"). */
fun qualityText(id: String): String =
    TOLERATED_CHOICES.firstOrNull { it.first == id }?.second ?: id

/** « Échanger avec mes appareils » on, « Partager mes sons » off: one sentence. */
fun stateText(state: LanStateFfi): String = when (state) {
    LanStateFfi.Off -> "Échange arrêté"
    LanStateFfi.Unsupported -> "Ce serveur ne propose pas l'échange entre appareils proches"
    LanStateFfi.NotEnrolled -> "En attente de la clé LAN"
    LanStateFfi.Searching -> "Recherche d'appareils sur ce Wi-Fi…"
    is LanStateFfi.Connected -> when (val peers = state.peers.toInt()) {
        1 -> "Connecté à 1 appareil"
        else -> "Connecté à $peers appareils"
    }
}

/** The signal line of a peer: « connecté » plus what it means for a thumb. */
fun signalText(peer: LanPeerFfi): String {
    val latency = peer.rttMs.toInt()
    return if (latency <= 0) "connecté" else "connecté · $latency ms"
}

/** « Mon appareil » / « Ami » — the two relations the node knows. */
fun relationText(own: Boolean): String = if (own) "Mon appareil" else "Ami"

/** The platform, in a word (never a model name: the node sends a platform). */
fun platformText(platform: String): String = when (platform.lowercase(Locale.ROOT)) {
    "android" -> "Android"
    "desktop" -> "Ordinateur"
    "ios" -> "iPhone"
    "" -> "Appareil"
    else -> platform.replaceFirstChar { it.uppercase(Locale.FRANCE) }
}

/** What a peer holds, in one line: its titles and its complete sets. */
fun peerDetailText(peer: LanPeerFfi): String {
    val tracks = peer.tracks.toInt()
    val sets = peer.sets.toInt()
    if (tracks == 0 && sets == 0) return "rien de partagé pour l'instant"
    val tracksPart = if (tracks == 1) "1 titre" else "$tracks titres"
    val setsPart = if (sets == 1) "1 liste" else "$sets listes"
    return "$tracksPart · $setsPart"
}

/** A quality rung, from the server's own spelling ("OPUS_128"). */
fun wireQualityText(quality: String): String = qualityText(quality)

/**
 * The DBus-quality rule of the SPA's `format.js#bytes`, ported: one decimal
 * BELOW ten units, a rounded integer above. `feature:settings` owns the same
 * function for its own module; a feature module may not import another's private
 * helpers, and duplicating six lines is cheaper than a shared module the plan
 * does not have.
 */
fun bytesText(bytes: Long): String {
    val units = listOf("o", "ko", "Mo", "Go", "To")
    var value = bytes.coerceAtLeast(0).toDouble()
    var unit = 0
    while (value >= 1024 && unit < units.lastIndex) {
        value /= 1024
        unit++
    }
    val rounded = if (unit == 0 || value >= 10) {
        value.toLong().toString()
    } else {
        String.format(Locale.FRANCE, "%.1f", value).removeSuffix(",0")
    }
    return "$rounded ${units[unit]}"
}

/** One transfer in flight: « Envoi vers PC · 12 Mo / 40 Mo · lecture ». */
fun transferText(transfer: LanTransferFfi): String {
    val way = when (transfer.direction) {
        LanDirectionFfi.SEND -> "Envoi vers"
        LanDirectionFfi.RECV -> "Réception depuis"
    }
    val what = transfer.title.ifBlank { transfer.track }
    val progress = if (transfer.total > 0uL) {
        "${bytesText(transfer.bytes.toLong())} / ${bytesText(transfer.total.toLong())}"
    } else {
        bytesText(transfer.bytes.toLong())
    }
    val urgency = when (transfer.urgency) {
        LanUrgencyFfi.PLAY -> "lecture"
        LanUrgencyFfi.PREFETCH -> "suite"
        LanUrgencyFfi.BULK -> "téléchargement"
    }
    return "$way ${transfer.peer} · $progress · $urgency — $what"
}

/** What a set is, in a word (`SetInfo.kind`). */
fun setKindText(kind: String): String = when (kind) {
    "playlist" -> "Playlist"
    "album" -> "Album"
    "show" -> "Podcast"
    "track" -> "Titre"
    else -> "Liste"
}

/** « 40 titres · 12 sur cet appareil », or « complet sur PC » when it is named. */
fun setDetailText(tracks: Int, local: Int, complete: Boolean, peer: String = ""): String {
    val total = if (tracks == 1) "1 titre" else "$tracks titres"
    val parts = mutableListOf(total)
    if (local > 0) parts += "$local sur cet appareil"
    // A row already sits under its peer's header: naming it again only costs the
    // line its last words (measured at 360 dp).
    if (complete && peer.isNotBlank()) parts += "complet sur $peer"
    return parts.joinToString(" · ")
}

/** « complet » / « partiel », the word the row's right side carries. */
fun completenessText(complete: Boolean): String = if (complete) "complet" else "partiel"

/**
 * When a device was last seen, in French and never as an ISO instant (the first
 * capture printed `2026-11-18T09:00:00Z`; rule 5: no raw machine spelling).
 */
fun seenText(iso: String?, nowMillis: Long = System.currentTimeMillis()): String {
    if (iso.isNullOrBlank()) return ""
    val then = runCatching { Instant.parse(iso).toEpochMilli() }.getOrNull() ?: return ""
    val minutes = ((nowMillis - then) / 60_000).coerceAtLeast(0)
    return when {
        minutes < 2 -> "vu à l'instant"
        minutes < 60 -> "vu il y a $minutes min"
        minutes < 60 * 24 -> "vu il y a ${minutes / 60} h"
        minutes < 60 * 24 * 7 -> "vu il y a ${minutes / (60 * 24)} j"
        else -> {
            val date = Instant.ofEpochMilli(then)
                .atZone(java.time.ZoneId.systemDefault())
                .format(DateTimeFormatter.ofPattern("d MMMM", Locale.FRANCE))
            "vu le $date"
        }
    }
}

/**
 * The credential's validity, in the note's sentence: « Clé valable jusqu'au 18
 * décembre », or the renewal failure with its cause (rule 2: never just a count).
 */
fun keyText(until: Long?, error: String?, zone: ZoneId = ZoneId.systemDefault()): String {
    if (!error.isNullOrBlank()) return "Impossible de renouveler la clé LAN : $error"
    if (until == null || until <= 0L) return "Aucune clé LAN pour l'instant : elle arrive à la prochaine connexion."
    val formatter = DateTimeFormatter.ofPattern("d MMMM", Locale.FRANCE)
    val day = Instant.ofEpochSecond(until).atZone(zone).format(formatter)
    return "Clé valable jusqu'au $day"
}

/** The advanced line: the port really bound, and whether the preferred one was taken. */
fun portText(port: Int, fallback: Boolean): String = when {
    port <= 0 -> "Le port sera choisi quand l'échange démarrera."
    fallback -> "Port $port (le $LAN_DEFAULT_PORT était pris : ouvrez $port/udp sur un ordinateur)."
    else -> "Port $port/udp"
}

/** The node's default UDP port (the note's `lan.port`). */
const val LAN_DEFAULT_PORT: Int = 45910

/**
 * A port a discovery announcement carries: `1..65535`. Nothing on the wire is
 * trusted (worker rule 10), and the FFI's `u16` is the last place a wrong value
 * could become one — discovery is only a hint, and a hostile announcement must
 * cost a bound check, never a panic.
 */
fun validDiscoveredPort(port: Int): Boolean = port in 1..65535

/** The port a person typed: `0` means any, and anything else must be a port. */
fun parsePort(text: String): Int? {
    val value = text.trim().toIntOrNull() ?: return null
    return if (value in 0..65535) value else null
}

/** The settings' tolerated list, as chips draw it: never empty (rule 6). */
fun toggleTolerated(current: List<String>, id: String): List<String> {
    val set = current.toMutableList()
    if (id in set) {
        // The note: « at least one ». Unticking the last one would mean "never
        // take anything from a peer", which the core reads as the default set.
        if (set.size == 1) return set
        set.remove(id)
    } else {
        set += id
    }
    return set.sortedBy { id2 -> TOLERATED_CHOICES.indexOfFirst { it.first == id2 } }
}

/** What the header of « Sur vos appareils » says when no peer exposes anything. */
fun peerSetsEmptyText(peers: Int): String = when (peers) {
    0 -> "Aucun appareil connecté pour l'instant. Ouvrez Neezer sur un autre appareil du même Wi-Fi."
    else -> "Vos appareils n'ont rien partagé pour l'instant."
}

/** The state line of the whole node, for « Appareils à proximité »'s header. */
fun snapshotSummary(snapshot: LanSnapshotFfi): String = stateText(snapshot.state)
