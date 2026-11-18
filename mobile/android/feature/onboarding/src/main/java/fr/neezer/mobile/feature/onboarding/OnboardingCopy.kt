package fr.neezer.mobile.feature.onboarding

import uniffi.nz_ffi.AccountOptions
import uniffi.nz_ffi.AccountRun
import uniffi.nz_ffi.FeedCause
import uniffi.nz_ffi.FinishBlock
import uniffi.nz_ffi.ImportJob
import uniffi.nz_ffi.ImportState
import uniffi.nz_ffi.LinkedAccount
import uniffi.nz_ffi.PasswordStrength
import uniffi.nz_ffi.ProviderOptions

/**
 * The onboarding's French copy, ported from `lib/onboarding.js` and
 * `OnboardingAccount.svelte` so the words and the numbers cannot drift.
 *
 * Pure functions, no Compose: `OnboardingCopyTest` holds every sentence the page
 * holds, and the screens only place them.
 */
object OnboardingCopy {
    private val NUMBER = java.text.NumberFormat.getIntegerInstance(java.util.Locale.FRANCE)

    fun number(value: Int): String = NUMBER.format(value.coerceAtLeast(0))

    fun plural(count: Int, one: String, many: String): String = if (count > 1) many else one

    /** `artistCountLabel`: « 3 artistes ». */
    fun artistCount(count: Int): String = "${number(count)} ${plural(count, "artiste", "artistes")}"

    /** `progressHint`. */
    fun progressHint(count: Int): String = when {
        count < 1 -> "Choisissez-en au moins un."
        count < 3 -> "Encore ${3 - count} pour un bon départ."
        count < 5 -> "C'est un bon départ."
        else -> "Vous pouvez continuer ou terminer."
    }

    /** `finishMessage`: why the step cannot end yet. */
    fun finishMessage(blocked: FinishBlock?): String = when (blocked) {
        FinishBlock.MIN -> "Choisissez au moins un artiste pour terminer."
        FinishBlock.OFFLINE -> "Hors ligne : reconnectez-vous pour terminer."
        FinishBlock.PENDING -> "Enregistrement de vos choix…"
        null -> ""
    }

    /** `feedCause`: the provider's cause, never a raw code. */
    fun feedCause(cause: FeedCause): String = when (cause) {
        FeedCause.DEEZER_UNREACHABLE -> "Deezer ne répond pas."
        FeedCause.DISABLED -> "Le catalogue Deezer est désactivé sur ce serveur."
        FeedCause.HELD -> "Le catalogue est en pause après plusieurs échecs."
        FeedCause.ABSENT -> "Aucun catalogue n'est configuré sur ce serveur."
        FeedCause.BUSY -> "Le serveur prépare déjà la sélection."
        FeedCause.ERROR -> "Le serveur n'a pas pu préparer la sélection."
        FeedCause.UNKNOWN -> "La sélection n'a pas pu être préparée."
    }

    /** « Trop court », « Faible »… — the meter's word. */
    fun strengthLabel(strength: PasswordStrength): String = when (strength) {
        PasswordStrength.TOO_SHORT -> "Trop court"
        PasswordStrength.WEAK -> "Faible"
        PasswordStrength.FAIR -> "Correct"
        PasswordStrength.GOOD -> "Bon"
        PasswordStrength.VERY_GOOD -> "Très bon"
    }

    /** What an import's poll says, as the card draws it. */
    data class ImportProgress(
        val label: String,
        val percent: Int?,
        val busy: Boolean,
        val failed: Boolean,
        val done: Boolean = false,
        val saved: Boolean = false,
    )

    /** `importProgress`: the phase, the count, and the CAUSE when it failed. */
    fun importProgress(job: ImportJob?): ImportProgress {
        if (job == null) return ImportProgress("", null, false, false)
        val total = job.total.toInt().coerceAtLeast(0)
        val done = job.done.toInt().coerceAtLeast(0)
        val matched = job.matched.toInt().coerceAtLeast(0)
        val percent = if (total > 0) (100 * done / total).coerceIn(0, 100) else null
        return when (job.state) {
            ImportState.READING -> ImportProgress(
                "Lecture de la playlist…",
                null,
                busy = true,
                failed = false,
            )
            ImportState.MATCHING -> ImportProgress(
                if (total > 0) {
                    "Recherche des titres : ${number(done)} sur ${number(total)}"
                } else {
                    "Recherche des titres…"
                },
                percent,
                busy = true,
                failed = false,
            )
            ImportState.SAVING -> ImportProgress(
                "Ajout à votre bibliothèque…",
                null,
                busy = true,
                failed = false,
            )
            ImportState.SAVED -> ImportProgress(
                "Ajouté : ${number(matched)} ${plural(matched, "titre", "titres")}.",
                100,
                busy = false,
                failed = false,
                done = true,
                saved = true,
            )
            ImportState.ERROR -> ImportProgress(
                "L'import a échoué : ${job.error ?: "cause inconnue"}.",
                null,
                busy = false,
                failed = true,
            )
            ImportState.DONE -> ImportProgress(
                "${number(matched)} ${plural(matched, "titre trouvé", "titres trouvés")} sur ${number(total)}.",
                100,
                busy = false,
                failed = false,
                done = true,
            )
        }
    }

    /** `importNote`: the provider's note beside a finished import. */
    fun importNote(job: ImportJob?): String =
        if (job != null && job.state == ImportState.DONE) job.error.orEmpty() else ""

    /** A run's progress line (`accountSummary`). */
    data class Summary(val text: String, val tone: Tone)

    enum class Tone { Idle, Busy, Ok, Error }

    /** `accountSummary`: what a linked (or not) account's card says. */
    fun accountSummary(account: LinkedAccount?): Summary {
        if (account == null) return Summary("Non relié", Tone.Idle)
        val running: AccountRun? = account.running
        if (running != null) {
            val done = running.done.toInt()
            val total = (running.total ?: 0u).toInt()
            return when (running.phase) {
                "connect" -> Summary("Connexion au compte…", Tone.Busy)
                "list" -> Summary("Lecture de la liste…", Tone.Busy)
                "import" -> {
                    if (total > 0) {
                        Summary(
                            "Import en cours : ${number(done)} sur ${number(total)}",
                            Tone.Busy,
                        )
                    } else {
                        Summary("Import en cours…", Tone.Busy)
                    }
                }
                else -> Summary("Import en cours…", Tone.Busy)
            }
        }
        if (account.status == "auth") {
            return Summary("Le compte refuse la connexion : reliez-le à nouveau.", Tone.Error)
        }
        if (account.status == "network") {
            return Summary("Le fournisseur ne répond pas : l'import reprendra plus tard.", Tone.Error)
        }
        if (account.lastReason == "auth") {
            return Summary("Le dernier import a échoué : le compte refuse la connexion.", Tone.Error)
        }
        if (account.lastReason == "network") {
            return Summary("Le dernier import a échoué : le fournisseur ne répondait pas.", Tone.Error)
        }
        val artists = account.addedArtists.toInt().coerceAtLeast(0)
        val tracks = account.addedTracks.toInt().coerceAtLeast(0)
        val parts = buildList {
            if (artists > 0) add("${number(artists)} ${plural(artists, "artiste", "artistes")}")
            if (tracks > 0) add("${number(tracks)} ${plural(tracks, "titre", "titres")}")
        }
        return when {
            parts.isNotEmpty() -> Summary("Dernier import : ${parts.joinToString(", ")}.", Tone.Ok)
            else -> Summary("Compte relié", Tone.Ok)
        }
    }

    /** What a YouTube card says when the server cannot offer it. */
    fun youtubeState(provider: ProviderOptions): Summary? = when {
        !provider.available && provider.reason == "absent" ->
            Summary("YouTube Music n'est pas installé sur ce serveur.", Tone.Error)
        !provider.available ->
            Summary("YouTube Music est désactivé sur ce serveur.", Tone.Error)
        provider.denied -> Summary("Votre compte n'a pas accès à ce service.", Tone.Error)
        else -> accountSummary(provider.account)
    }

    /** What a Deezer card says when the server cannot offer it. */
    fun deezerState(provider: ProviderOptions): Summary? = when {
        !provider.available -> Summary("Indisponible sur ce serveur.", Tone.Error)
        else -> accountSummary(provider.account)
    }

    /** `unmatched`'s sentence for the disclosure, singular or plural. */
    fun unmatchedLabel(count: Int): String =
        "$count ${if (count > 1) "titres n'ont pas été trouvés" else "titre n'a pas été trouvé"}"

    /** Whether the account step offers anything at all. */
    fun playlistAvailable(options: AccountOptions): Boolean =
        options.imports.any { it.available }
}
