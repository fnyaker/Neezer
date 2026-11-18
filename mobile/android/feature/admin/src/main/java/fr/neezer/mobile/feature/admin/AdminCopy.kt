package fr.neezer.mobile.feature.admin

import fr.neezer.mobile.bridge.NzErrors
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale
import uniffi.nz_ffi.AdminInviteRow
import uniffi.nz_ffi.AdminServerCard
import uniffi.nz_ffi.AdminUserRow
import uniffi.nz_ffi.NzException

/**
 * Les mots des pages d'administration (lot MB17) — `lib/adminusers.js`,
 * `Settings.svelte`, `AutomatchPanel.svelte`, `TranscribeSettings.svelte` et
 * `GenreLibrary.svelte`, portés.
 *
 * Pourquoi ici et pas en Rust : `nz-core` nomme des ids et des nombres
 * (`on_fav_album`, `no_candidate`, `listening`), et la phrase française qui les
 * accompagne est de la copie d'écran. MB16 a posé la même règle pour Réglages ;
 * une phrase ne traverse pas la frontière FFI.
 *
 * Rien ici n'invente : chaque table reprend les libellés de la SPA, y compris
 * les tournures (« Impossible : c'est le dernier administrateur actif. »).
 */
object AdminCopy {
    // --- le tableau des comptes -------------------------------------------

    /** `adminusers.js#PAGE_SIZE` — une page de 25. */
    const val PAGE_SIZE: Int = 25

    /** `adminusers.js#MAX_ROWS` — au-delà, on affine la recherche. */
    const val MAX_ROWS: Int = 250

    /** Combien d'invitations le tableau de bord charge (`AdminUsers.svelte`). */
    const val INVITES_LIMIT: Int = 200

    /** Les six filtres, dans l'ordre des pastilles de la SPA. */
    val FILTERS: List<AccountFilter> = listOf(
        AccountFilter.All,
        AccountFilter.Active,
        AccountFilter.Invited,
        AccountFilter.Admins,
        AccountFilter.Members,
        AccountFilter.Disabled,
    )

    /** Les quatre durées de l'invitation, et celle sur laquelle la feuille s'ouvre. */
    val INVITE_DURATIONS: List<Pair<UInt, String>> = listOf(
        3_600u to "1 heure",
        86_400u to "24 heures",
        604_800u to "7 jours",
        2_592_000u to "30 jours",
    )
    val INVITE_DEFAULT_TTL: UInt = 604_800u

    /** Les portées d'un archivage manuel (`Settings.svelte#startBackfill`). */
    val ARCHIVE_SCOPES: List<Pair<String, String>> = listOf(
        // Short on purpose: a pill is ONE line, and at a 200 % font scale the
        // long form ellipsized the primary action's label (measured).
        "all" to "Tout archiver",
        "favorites" to "Favoris",
        "playlists" to "Playlists",
        "podcasts" to "Podcasts",
    )

    /** `Settings.svelte#EVENT_LABELS` — ce qui déclenche un archivage. */
    val EVENT_LABELS: Map<String, String> = mapOf(
        "on_play_context" to "Lire un titre archive tout son album / sa playlist",
        "on_fav_track" to "Titre mis en favori",
        "on_fav_album" to "Album mis en favori",
        "on_fav_playlist" to "Playlist mise en favori",
        "on_fav_artist" to "Artiste mis en favori",
        "on_playlist_add" to "Titre ajouté à une playlist",
        "on_podcast" to "Nouveau podcast : archivage automatique activé",
    )

    /** `Settings.svelte#SCOPE_LABELS` — jusqu'où va un artiste mis en favori. */
    val SCOPE_LABELS: Map<String, String> = mapOf(
        "all" to "Toute la discographie",
        "releases" to "Les N sorties les plus récentes",
        "top" to "Les N titres les plus écoutés",
    )

    /** `Settings.svelte#ORDER_LABELS` — ce qui part en premier. */
    val ORDER_LABELS: Map<String, String> = mapOf(
        "oldest_play" to "Le plus anciennement écouté",
        "least_played" to "Le moins écouté",
        "largest" to "Le plus volumineux",
        "oldest" to "Le plus anciennement archivé",
    )

    /** `GenreLibrary.svelte#STAGES` — les trois étapes de la mesure. */
    val STAGES: Map<String, String> = mapOf(
        "measure" to "Mesure",
        "embed" to "Empreintes",
        "decide" to "Classement",
    )

    /** `GenreLibrary.svelte#STAGE_HINTS` — ce que chaque étape regarde. */
    val STAGE_HINTS: Map<String, String> = mapOf(
        "measure" to "tempo, style",
        "embed" to "ce que le modèle écoute",
        "decide" to "genre retenu",
    )

    /** `GenreLibrary.svelte#WHY` — pourquoi la mesure s'efface. */
    val PAUSE_WHY: Map<String, String> = mapOf(
        "listening" to "quelqu'un écoute de la musique",
        "download" to "un téléchargement est en cours",
        "load" to "le serveur est occupé",
        "paused" to "la mesure est en pause",
    )

    /** `AutomatchPanel.svelte#STATUS` — l'issue d'un titre, par compte. */
    val AUTOMATCH_STATUS: Map<String, String> = mapOf(
        "match" to "remplacé",
        "no_reference" to "sans extrait Deezer",
        "no_candidate" to "sans candidat",
        "no_match" to "sous 99 %",
        "error" to "en erreur",
        "skipped" to "déjà réglé",
    )

    /** `AutomatchPanel.svelte#REASONS` — les causes que le serveur nomme en anglais. */
    val AUTOMATCH_REASONS: Map<String, String> = mapOf(
        "Deezer serves no preview for it" to "Deezer ne sert plus d'extrait de ce titre",
        "Deezer no longer knows the track" to "Deezer ne connaît plus ce titre",
        "nothing with the same title and artist" to "aucun titre du même nom et du même artiste",
        "below 99 %" to "le plus proche était sous 99 %",
        "the Deezer proxy is off" to "le proxy Deezer est désactivé",
        "no candidate could be heard" to "aucun candidat n'a pu être écouté",
    )

    /** `EnrichBackfill.svelte#METHODS` — comment un fait a été trouvé. */
    val ENRICH_METHODS: Map<String, String> = mapOf(
        "isrc" to "ISRC",
        "acoustid" to "empreinte",
        "search" to "recherche",
    )

    /** `lib/transcript.js#LANGUAGES` — l'ordre du sélecteur, `auto` d'abord. */
    val LANGUAGES: List<String> = listOf("auto", "fr", "en", "es", "de", "it", "pt", "nl")

    /** `Settings.svelte#MAX_MINUTES_MAX` et les raccourcis de durée. */
    const val MAX_MINUTES_MAX: Int = 1440
    val MINUTE_SHORTCUTS: List<Int> = listOf(30, 60, 120, 180, 240, 360)

    /** Les heures proposées pour la fenêtre nocturne (`GenreLibrary.svelte`). */
    val SCHEDULE_HOURS: List<Int> = listOf(1, 2, 3, 4, 6, 8, 10, 12)

    // --- les refus du serveur ---------------------------------------------

    /**
     * `adminusers.js#REFUSALS`, mot pour mot : le serveur répond un CODE, et
     * c'est ici qu'il devient une phrase.
     */
    val REFUSALS: Map<String, String> = mapOf(
        "last_admin" to "Impossible : c'est le dernier administrateur actif.",
        "self_delete" to "Vous ne pouvez pas supprimer votre propre compte.",
        "self_disable" to "Vous ne pouvez pas désactiver votre propre compte.",
        "server_account_holder" to
            "Ce compte porte le compte Deezer du serveur. Changez-en le titulaire avant de le supprimer.",
        "forbidden" to "Votre compte n'a plus les droits d'administration.",
        "not_found" to "Ce compte n'existe plus.",
        "invite_not_found" to "Cette invitation n'existe plus.",
        "invite_used" to "Cette invitation a déjà servi.",
        "invite_invalid" to "Cette invitation n'est plus valide.",
        "rate_limited" to "Trop de tentatives. Réessayez dans quelques minutes.",
        "invalid_note" to "La note doit faire 64 caractères au plus.",
        "invalid_expires_in" to "La durée doit être comprise entre 1 heure et 30 jours.",
        "too_large" to "La demande est trop volumineuse.",
        "server_error" to "Erreur du serveur. Réessayez.",
        "confirm_mismatch" to "Le nom saisi ne correspond pas : rien n'a été supprimé.",
    )

    /**
     * `adminusers.js#guardMessage`, porté : la phrase du code quand on peut le
     * lire, sinon celle du statut, sinon le serveur.
     *
     * Le code arrive dans `NzError::Server.detail` sous la forme `code: message`
     * (la traduction de `CoreError`), et un `409` — `nz-api`'s `Conflict` — ne
     * porte que la phrase du serveur, qui est déjà la bonne (« Impossible : c'est
     * le dernier administrateur actif. »).
     */
    fun guardMessage(e: Throwable): String {
        codeOf(e)?.let { code -> REFUSALS[code]?.let { return it } }
        val status = statusOf(e)
        return when {
            status == 0 -> "Le serveur ne répond pas (réseau)."
            status == 401 -> "Session expirée : reconnectez-vous."
            status == 403 -> REFUSALS.getValue("forbidden")
            status == 404 -> "Cet élément n'existe plus."
            status == 409 -> NzErrors.text(e)
            status == 429 -> REFUSALS.getValue("rate_limited")
            status != null && status >= 500 -> REFUSALS.getValue("server_error")
            else -> NzErrors.text(e)
        }
    }

    /** The server's machine code, when the failure still carries it. */
    private fun codeOf(e: Throwable): String? {
        val detail = (e as? NzException.Server)?.detail ?: return null
        val code = detail.substringBefore(':', "")
        return code.takeIf { it.isNotEmpty() && REFUSALS.containsKey(it) }
    }

    private fun statusOf(e: Throwable): Int? = (e as? NzException.Server)?.status?.toInt()

    // --- les dates ---------------------------------------------------------

    private val DATE: DateTimeFormatter =
        DateTimeFormatter.ofPattern("d MMM yyyy", Locale.FRENCH)
    private val DATETIME: DateTimeFormatter =
        DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM).withLocale(Locale.FRENCH)
    private val CLOCK: DateTimeFormatter = DateTimeFormatter.ofPattern("HH:mm", Locale.FRENCH)

    /**
     * Un horodatage du serveur (`2026-11-17T09:30:00`, **heure locale du
     * serveur**, sans fuseau) — lu comme la SPA le lit (`new Date(iso)`).
     */
    private fun at(iso: String?): LocalDateTime? {
        if (iso.isNullOrBlank()) return null
        return runCatching { LocalDateTime.parse(iso.trim().removeSuffix("Z")) }.getOrNull()
    }

    fun shortDate(iso: String?): String =
        at(iso)?.toLocalDate()?.format(DATE) ?: "—"

    fun absoluteTime(iso: String?): String {
        val when_ = at(iso) ?: return "—"
        // `DATETIME.format(...).replace(",", " à")` : la SPA écrit « 17 nov. 2026
        // à 09:30 », et `FormatStyle.MEDIUM` met une virgule.
        return DATETIME.format(when_).replace(",", " à")
    }

    fun clockTime(iso: String?): String = at(iso)?.format(CLOCK) ?: "—"

    /** The clock time of a `System.currentTimeMillis()` reading ("Actualisé à 14:05"). */
    fun clockTimeOf(millis: Long): String =
        java.time.Instant.ofEpochMilli(millis).atZone(ZoneId.systemDefault()).format(CLOCK)

    /** `adminusers.js#relativeTime` — « il y a 3 j », jamais un nombre nu. */
    fun relativeTime(iso: String?, now: Long = System.currentTimeMillis()): String {
        val when_ = at(iso) ?: return "—"
        val seconds = ((now - when_.atZone(ZoneId.systemDefault()).toInstant().toEpochMilli()) / 1000)
            .coerceAtLeast(0)
        val minutes = seconds / 60
        val hours = minutes / 60
        val days = hours / 24
        val months = days / 30
        val years = months / 12
        return when {
            seconds < 60 -> "à l'instant"
            minutes < 60 -> "il y a $minutes min"
            hours < 24 -> "il y a $hours h"
            days < 31 -> "il y a $days j"
            months < 12 -> "il y a $months mois"
            else -> "il y a $years an" + if (years > 1) "s" else ""
        }
    }

    /** `adminusers.js#expiryLabel` — « expire dans 6 j », « expirée ». */
    fun expiryLabel(iso: String?, now: Long = System.currentTimeMillis()): String {
        val when_ = at(iso) ?: return "—"
        val left = when_.atZone(ZoneId.systemDefault()).toInstant().toEpochMilli() - now
        if (left <= 0) return "expirée"
        val minutes = (left + 59_999) / 60_000
        val hours = (left + 3_599_999) / 3_600_000
        val days = (left + 86_399_999) / 86_400_000
        val months = (days + 29) / 30
        return when {
            minutes < 60 -> "expire dans $minutes min"
            hours < 48 -> "expire dans $hours h"
            days < 31 -> "expire dans $days j"
            else -> "expire dans $months mois"
        }
    }

    /**
     * The ARL card's line, and the WHOLE of what this app knows about the
     * credential: the server's masked hint and where it comes from, never the
     * ARL itself — `AdminServerCard` has no field that could carry it, so the
     * value cannot be drawn, logged or screenshotted by mistake.
     */
    fun arlLine(card: AdminServerCard?): String = when {
        card == null -> ""
        !card.arlSet -> "Aucun ARL configuré"
        else -> "ARL actif ${card.arlHint ?: ""} · source : " +
            (if (card.arlSource == "database") "saisi ici" else "configuration serveur")
    }

    /** `format.js#bytes` — what a gauge says, in Ko/Mo/Go. */
    fun bytes(count: ULong): String {
        val value = count.toDouble()
        return when {
            value >= 1024.0 * 1024 * 1024 * 1024 -> "%.1f To".format(Locale.FRENCH, value / (1024.0 * 1024 * 1024 * 1024))
            value >= 1024.0 * 1024 * 1024 -> "%.1f Go".format(Locale.FRENCH, value / (1024.0 * 1024 * 1024))
            value >= 1024.0 * 1024 -> "%.0f Mo".format(Locale.FRENCH, value / (1024.0 * 1024))
            value >= 1024.0 -> "%.0f Ko".format(Locale.FRENCH, value / 1024)
            else -> "${count.toLong()} o"
        }
    }

    /** `adminusers.js#countLabel` — « 3 comptes », « 1 compte ». */
    fun countLabel(count: Int, singular: String, plural: String = "${singular}s"): String =
        "$count " + if (count > 1) plural else singular

    /** `roleLabel` — le rôle d'un compte. */
    fun roleLabel(row: AdminUserRow): String = if (row.admin) "Administrateur" else "Membre"

    /** `stateLabel` — l'état d'un compte, en un mot. */
    fun stateLabel(row: AdminUserRow): String = if (row.disabled) "Désactivé" else "Actif"

    /** `stateLabel` for an invitation (`stateKey`'s five outcomes). */
    fun inviteStateLabel(invite: AdminInviteRow, now: Long = System.currentTimeMillis()): String =
        when (inviteKey(invite, now)) {
            "used" -> "Utilisée"
            "revoked" -> "Révoquée"
            "expired" -> "Expirée"
            else -> "Invité"
        }

    /** `adminusers.js#stateKey` for an invitation. */
    fun inviteKey(invite: AdminInviteRow, now: Long = System.currentTimeMillis()): String = when {
        invite.state == "used" -> "used"
        invite.state == "revoked" -> "revoked"
        invite.state == "expired" -> "expired"
        expiryLabel(invite.expires, now) == "expirée" -> "expired"
        else -> "invite"
    }

    /** `lastSeenLabel` — la dernière connexion, ou l'expiration d'une invitation. */
    fun lastSeenLabel(row: AdminUserRow, now: Long = System.currentTimeMillis()): String =
        row.lastLogin?.let { relativeTime(it, now) } ?: "—"

    fun lastSeenLabel(invite: AdminInviteRow, now: Long = System.currentTimeMillis()): String =
        invite.expires?.let { "Expire le ${shortDate(it)}" } ?: "—"

    /** `foldText` — la recherche du filtre est aveugle aux accents. */
    fun fold(text: String?): String = java.text.Normalizer.normalize(text ?: "", java.text.Normalizer.Form.NFD)
        .replace(Regex("\\p{Mn}+"), "")
        .lowercase(Locale.FRENCH)
        .trim()

    /** `inviteRow` : la note de l'admin, ou « Invitation sans note ». */
    fun inviteName(invite: AdminInviteRow): String =
        invite.note?.trim()?.takeIf { it.isNotEmpty() } ?: "Invitation sans note"

    /** `AutomatchPanel.svelte#reasonText` — la cause d'un titre non remplacé. */
    fun automatchReason(reason: String): String {
        if (reason.isBlank()) return ""
        val closest = Regex("^closest ([\\d.]+) %").find(reason)
        if (closest != null) {
            return "le plus proche : " + closest.groupValues[1].replace('.', ',') + " %"
        }
        return AUTOMATCH_REASONS[reason] ?: reason
    }

    /** `AutomatchPanel.svelte#pct` — « 99,5 % ». */
    fun percent(score: Double): String =
        String.format(Locale.FRENCH, "%.1f", score * 100) + " %"

    /** `TranscribeSettings.svelte#reasonText`, by the server's own word. */
    fun transcribeReason(reason: String?): String = when (reason) {
        null, "" -> ""
        "no_model" -> "Aucun modèle installé sur le serveur"
        "disabled" -> "La transcription est désactivée sur ce serveur"
        "unavailable" -> "Ce serveur ne peut pas transcrire"
        "no_engine" -> "Le moteur de transcription est absent"
        else -> reason
    }

    fun languageLabel(code: String): String = when (code) {
        "auto" -> "Détection automatique"
        "fr" -> "Français"
        "en" -> "Anglais"
        "es" -> "Espagnol"
        "de" -> "Allemand"
        "it" -> "Italien"
        "pt" -> "Portugais"
        "nl" -> "Néerlandais"
        else -> code
    }
}

/** Les six pastilles du tableau des comptes (`FILTERS`). */
enum class AccountFilter(val id: String, val label: String) {
    All("all", "Tous"),
    Active("active", "Actifs"),
    Invited("invited", "Invités"),
    Admins("admins", "Administrateurs"),
    Members("members", "Membres"),
    Disabled("disabled", "Désactivés"),
    ;

    companion object {
        fun of(id: String?): AccountFilter = entries.firstOrNull { it.id == id } ?: All
    }
}

/** The one row type the account table draws: an account or an invitation. */
sealed interface AccountRow {
    val key: String

    data class User(val row: AdminUserRow) : AccountRow {
        override val key: String get() = "user:${row.id}"
    }

    data class Invite(val row: AdminInviteRow) : AccountRow {
        override val key: String get() = "invite:${row.id}"
    }
}

/**
 * `adminusers.js#filterRows`, porté : le filtre porte sur les lignes DÉJÀ
 * chargées, et la recherche ne s'applique qu'aux invitations (les comptes sont
 * filtrés par le serveur, ce qui est le seul moyen de chercher dans 4 000
 * comptes).
 */
fun filterRows(rows: List<AccountRow>, filter: AccountFilter, term: String): List<AccountRow> {
    val needle = AdminCopy.fold(term)
    return rows.filter { row ->
        when (row) {
            is AccountRow.User -> when (filter) {
                AccountFilter.Invited -> false
                AccountFilter.Active -> !row.row.disabled
                AccountFilter.Disabled -> row.row.disabled
                AccountFilter.Admins -> row.row.admin
                AccountFilter.Members -> !row.row.admin
                AccountFilter.All -> true
            }
            is AccountRow.Invite -> when (filter) {
                AccountFilter.All, AccountFilter.Invited -> needle.isEmpty() ||
                    AdminCopy.fold("${row.row.note ?: ""} ${AdminCopy.inviteName(row.row)}").contains(needle)
                else -> false
            }
        }
    }
}

/** The users alone, in the table's order, from the merged rows. */
fun List<AccountRow>.users(): List<AdminUserRow> = filterIsInstance<AccountRow.User>().map { it.row }

/** The invitations alone. */
fun List<AccountRow>.invites(): List<AdminInviteRow> = filterIsInstance<AccountRow.Invite>().map { it.row }
