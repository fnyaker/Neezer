package fr.neezer.mobile.feature.settings

import fr.neezer.mobile.designsystem.NzGlyph
import uniffi.nz_ffi.AccountData
import uniffi.nz_ffi.CountData
import uniffi.nz_ffi.SourceData
import java.time.Instant

/**
 * Réglages' two levels, in words (`routes/Settings.svelte`).
 *
 * A category is a page of its own, and the list is grouped exactly as the SPA
 * groups it — Écoute, Bibliothèque, Comptes & serveur — so a person who knows the
 * web app finds the same names in the same order. Adding a category here is what
 * a later lot does (LN14's « Appareils et amis », MB17's two admin pages): the
 * list is built FROM this table, so nothing else is rewritten.
 */
data class SettingCategory(
    /** The SPA's own id (`/settings/<id>`), kept so a link lands on the same page. */
    val id: String,
    val label: String,
    val glyph: NzGlyph,
    /** The group it is drawn under. */
    val group: SettingGroup,
    /** Only an administrator sees it (the server enforces it again). */
    val adminOnly: Boolean = false,
)

enum class SettingGroup(val label: String) {
    Listen("Écoute"),
    Library("Bibliothèque"),
    Accounts("Comptes & serveur"),
}

/** Every category, in the SPA's order. */
val SETTING_CATEGORIES: List<SettingCategory> = listOf(
    SettingCategory("playback", "Lecture & son", NzGlyph.Speaker, SettingGroup.Listen),
    SettingCategory("appearance", "Apparence", NzGlyph.Moon, SettingGroup.Listen),
    SettingCategory("viz", "Animations", NzGlyph.Sparkles, SettingGroup.Listen),
    SettingCategory("downloads", "Téléchargements", NzGlyph.Download, SettingGroup.Library),
    SettingCategory("discovery", "Découverte", NzGlyph.Compass, SettingGroup.Library),
    SettingCategory("lan", "Appareils à proximité", NzGlyph.People, SettingGroup.Accounts),
    SettingCategory("sources", "Sources", NzGlyph.Link, SettingGroup.Accounts),
    SettingCategory("account", "Compte", NzGlyph.User, SettingGroup.Accounts),
    SettingCategory("help", "Assistance", NzGlyph.Info, SettingGroup.Accounts),
)

/** The two pages MB17 adds, behind [adminsOnly]; drawn by `AdminSections()`. */
val ADMIN_CATEGORIES: List<SettingCategory> = listOf(
    SettingCategory("users", "Utilisateurs", NzGlyph.Users, SettingGroup.Accounts, adminOnly = true),
    SettingCategory("server", "Serveur", NzGlyph.Archive, SettingGroup.Accounts, adminOnly = true),
)

/** The groups, with the categories an administrator may see, in order. */
fun categoriesFor(isAdmin: Boolean): List<Pair<SettingGroup, List<SettingCategory>>> =
    SettingGroup.entries.mapNotNull { group ->
        val items = (SETTING_CATEGORIES + if (isAdmin) ADMIN_CATEGORIES else emptyList())
            .filter { it.group == group }
        items.takeIf { it.isNotEmpty() }?.let { group to it }
    }

/** `Settings.svelte#fold`: accent- and case-blind, which is what a thumb types. */
fun foldSearch(text: String): String = java.text.Normalizer
    .normalize(text, java.text.Normalizer.Form.NFD)
    .replace(Regex("\\p{Mn}+"), "")
    .lowercase()
    .trim()

/** The categories whose name matches [query] (the menu is filtered, not the page). */
fun filterCategories(
    categories: List<Pair<SettingGroup, List<SettingCategory>>>,
    query: String,
): List<Pair<SettingGroup, List<SettingCategory>>> {
    val needle = foldSearch(query)
    if (needle.isEmpty()) return categories
    return categories.mapNotNull { (group, items) ->
        items.filter { foldSearch(it.label).contains(needle) }.takeIf { it.isNotEmpty() }?.let { group to it }
    }
}

/** A category by its id, or `null` for a link to a page this build does not have. */
fun categoryOf(id: String?): SettingCategory? =
    (SETTING_CATEGORIES + ADMIN_CATEGORIES).firstOrNull { it.id == id }

// -- the sources ---------------------------------------------------------------

/** The SPA's `placeLabel`: what a source's place is called. */
fun placeLabel(source: SourceData): String = when (source.place) {
    "main" -> "Principale"
    "fallback" -> "Secours ${source.rank ?: 1}"
    else -> if (source.configured) "Coupée" else "Indisponible"
}

/** `SourceOrder.svelte#statusLine`: where the source sits, or why it rests. */
fun sourceStatus(source: SourceData, fallbacks: List<SourceData>): String {
    if (!source.configured) return source.setup.ifBlank { "Pas encore configurée sur ce serveur." }
    if (source.restingS > 0u) {
        val minutes = ((source.restingS + 59u) / 60u).coerceAtLeast(1u)
        val why = source.restingWhy.ifBlank { "n'a pas répondu" }
        return "A $why : laissée tranquille encore $minutes min."
    }
    return when (source.place) {
        "main" -> "Recherche, pages et musique. Toujours interrogée en premier."
        "fallback" -> {
            val before = fallbacks.filter { (it.rank ?: 0) < (source.rank ?: 0) }.map { it.name }
            when (before.size) {
                0 -> "Interrogée quand aucune autre source n'a un titre."
                1 -> "Interrogée quand ${before[0]} n'a pas un titre."
                else -> "Interrogée quand ${before.dropLast(1).joinToString(", ")} et ${before.last()} n'ont pas un titre."
            }
        }
        else -> "Coupée : ce qui en est déjà archivé reste lisible."
    }
}

/** `sources.js#iconOf`: a source's own glyph, else the generic one. */
fun sourceGlyph(id: String): NzGlyph = when (id) {
    "youtube" -> NzGlyph.Youtube
    "local" -> NzGlyph.CloudOff
    else -> NzGlyph.Music
}

// -- my accounts ---------------------------------------------------------------

/** `MyAccounts.svelte#KIND_LABEL`. */
val KIND_LABELS: Map<String, String> = mapOf(
    "artists" to "Artistes suivis",
    "tracks" to "Titres aimés",
    "albums" to "Albums",
    "playlists" to "Playlists",
)

private val COUNT_LABELS = mapOf(
    "artists" to "artiste",
    "tracks" to "titre",
    "albums" to "album",
    "playlists" to "playlist",
)

/** "12 titres, 1 playlist ajoutés" — the SPA's `counts()`. */
fun countsText(counts: List<CountData>): String {
    val parts = counts.map { "${it.count} ${COUNT_LABELS[it.kind] ?: it.kind}${if (it.count > 1u) "s" else ""}" }
    return if (parts.isEmpty()) "rien de nouveau" else parts.joinToString(", ") + " ajoutés"
}

/** `MyAccounts.svelte#ago`, from an ISO instant and a reference clock (testable). */
fun ago(iso: String?, nowMillis: Long = System.currentTimeMillis()): String {
    if (iso.isNullOrBlank()) return ""
    val then = runCatching { Instant.parse(iso).toEpochMilli() }.getOrNull() ?: return ""
    val seconds = ((nowMillis - then) / 1000.0).coerceAtLeast(0.0)
    return when {
        seconds < 90 -> "à l'instant"
        seconds < 3600 -> "il y a ${Math.round(seconds / 60)} min"
        seconds < 86_400 * 1.5 -> "il y a ${Math.round(seconds / 3600)} h"
        else -> "il y a ${Math.round(seconds / 86_400)} j"
    }
}

/** `MyAccounts.svelte#progress`: what the import is doing, and how far. */
data class ImportProgress(val text: String, val share: Double?)

fun importProgress(account: AccountData): ImportProgress? {
    val phase = account.runningPhase ?: return null
    val what = account.runningKind?.let { KIND_LABELS[it] } ?: ""
    return when {
        phase == "import" && account.runningTotal > 0u -> ImportProgress(
            "$what : ${account.runningDone} sur ${account.runningTotal}",
            account.runningDone.toDouble() / account.runningTotal.toDouble(),
        )
        phase == "list" -> ImportProgress("$what : lecture de la liste du compte…", null)
        phase == "import" -> ImportProgress("$what : rien de nouveau", 1.0)
        else -> ImportProgress("Connexion au compte…", null)
    }
}

/** `MyAccounts.svelte#statusLine`: what the last import did, or why it cannot. */
fun accountStatus(account: AccountData, nowMillis: Long = System.currentTimeMillis()): String {
    if (account.runningPhase != null) return ""
    when (account.status) {
        "auth" -> return "Le compte refuse la connexion : reliez-le à nouveau."
        "broken" -> return "Compte illisible sur le serveur (${account.error ?: "cause inconnue"}, détail dans le journal) : déliez-le puis reliez-le."
    }
    if (account.lastAt == null) return "Pas encore importé"
    val when_ = ago(account.lastAt, nowMillis)
    account.lastReason?.let { reason ->
        if (reason == "network") return "Service injoignable $when_ : nouvel essai dans l'heure"
    }
    account.lastError?.let { return "Dernier import $when_ : échec ($it)" }
    val kept = account.lastKept.sumOf { it.count.toLong() }
    val added = countsText(account.lastAdded)
    return if (kept > 0) {
        "Dernier import $when_ : $added · $kept déjà traités ici, laissés tels quels"
    } else {
        "Dernier import $when_ : $added"
    }
}

/** `MyAccounts.svelte#useLine`: what a personal Deezer account is also used for. */
fun accountUse(account: AccountData): String {
    if (account.provider != "deezer" || account.server) return ""
    if (account.status == "auth" || account.status == "broken") return ""
    return if (account.lossless == false) {
        "Vos recherches et vos pages Deezer passent par ce compte. Sans FLAC (offre gratuite), la musique est récupérée par un autre compte."
    } else {
        "Vos recherches, vos pages et votre musique Deezer passent par ce compte."
    }
}

// -- the local settings --------------------------------------------------------

/**
 * The quality card's five rungs, id and label, in the sheet's order.
 *
 * Five, not six: `AUTO` is the player's own default (the card used to show
 * `FLAC` while the player played `Auto`), and `OPUS_256`/`OPUS_192` have no rung
 * in `nz_audio::Quality` — offering a rate the engine cannot load is the bug,
 * not the simplification (D14a already removed them from the player's menu).
 */
val QUALITY_CHOICES: List<Pair<String, String>> = listOf(
    "AUTO" to "Auto",
    "FLAC" to "FLAC",
    "OPUS_320" to "Opus 320",
    "OPUS_128" to "Opus 128",
    "OPUS_64" to "Opus 64",
)

/**
 * The card's id → the engine's rung. `AUTO` included: the player starts on it,
 * and a card that could not name it would draw nothing selected on a fresh
 * device.
 */
val QUALITY_RUNGS: Map<String, uniffi.nz_ffi.Quality> = mapOf(
    "AUTO" to uniffi.nz_ffi.Quality.AUTO,
    "FLAC" to uniffi.nz_ffi.Quality.FLAC,
    "OPUS_320" to uniffi.nz_ffi.Quality.OPUS320,
    "OPUS_128" to uniffi.nz_ffi.Quality.OPUS128,
    "OPUS_64" to uniffi.nz_ffi.Quality.OPUS64,
)

/** `format.js#bytes`: 1,2 Go / 340 Mo / 12 ko, in French. */
fun formatBytes(bytes: Long): String {
    val units = listOf("o", "ko", "Mo", "Go", "To")
    var value = bytes.toDouble()
    var unit = 0
    while (value >= 1024 && unit < units.lastIndex) {
        value /= 1024
        unit++
    }
    val rounded = if (unit == 0) value.toLong().toString() else String.format(java.util.Locale.FRANCE, "%.1f", value).removeSuffix(",0")
    return "$rounded ${units[unit]}"
}

/** A quality id as the badge says it ("OPUS_320" → "Opus 320"). */
fun qualityLabel(id: String): String =
    QUALITY_CHOICES.firstOrNull { it.first == id }?.second ?: id

/** `lib/theme.js#THEME_INFO`, the three cards of *Apparence*. */
val THEME_CHOICES: List<Pair<String, String>> = listOf(
    "encre" to "Encre",
    "papier" to "Papier",
    "pochette" to "Pochette",
)

/** The discovery steps, `lib/discovery.js#STEPS`. */
data class DiscoveryStep(val name: String, val from: Double)

val DISCOVERY_STEPS: List<DiscoveryStep> = listOf(
    DiscoveryStep("Proche", 0.0),
    DiscoveryStep("Voisin", 0.25),
    DiscoveryStep("Exploration", 0.5),
    DiscoveryStep("Aventure", 0.75),
)

/** `discovery.js#stepOf`. */
fun discoveryStepOf(level: Double): DiscoveryStep =
    DISCOVERY_STEPS.lastOrNull { level >= it.from } ?: DISCOVERY_STEPS.first()
