package fr.neezer.mobile.feature.podcasts

import uniffi.nz_ffi.EpisodeData
import uniffi.nz_ffi.EpisodePositionData
import uniffi.nz_ffi.TranscriptInfoData
import uniffi.nz_ffi.nzPodcastBatchOptions
import uniffi.nz_ffi.nzPodcastLanguageLabel
import uniffi.nz_ffi.nzPodcastPageInfo
import uniffi.nz_ffi.nzPodcastReasonText
import uniffi.nz_ffi.nzPodcastTranscriptionCause

/**
 * The French words the podcast screens write, in ONE file (rule 5).
 *
 * Every sentence the SPA writes is here, translated from `Show.svelte`,
 * `TranscriptPanel.svelte`, `Podcasts.svelte` and `lib/transcript.js`, so a
 * screen never builds a phrase inline — and the arithmetic that a phrase needs
 * (a duration, a percentage, a page count, a batch size) is asked of the CORE,
 * not re-done here: `nzPodcast*` are Rust's own ports of `format.js` and
 * `transcript.js`.
 */
object PodcastsCopy {
    /** « 12 épisodes » / « 1 épisode » (`Podcasts.svelte`'s count). */
    fun episodeCount(count: Int): String =
        if (count > 1) "$count épisodes" else "$count épisode"

    /**
     * « Reprendre · il reste 1:02:05 » — the resume line under an episode
     * (`Show.svelte`'s `resume`).
     */
    fun resumeLine(position: EpisodePositionData): String =
        if (position.finished) "Terminé"
        else "Reprendre · il reste ${duration(position.remainingS.toInt())}"

    /** « 42 % » — a transcript's progress, as the chip writes it. */
    fun percent(info: TranscriptInfoData): String = "${info.percent} %"

    /** The transcript chip's words for a state (`Show.svelte`'s `tr-chip`). */
    fun transcriptChip(info: TranscriptInfoData): String = when {
        info.ready -> "Transcription"
        info.status == "queued" -> "En attente"
        info.active -> "${info.percent} %"
        info.status == "stopped" -> "Arrêtée"
        info.status == "error" -> "Échec"
        else -> ""
    }

    /** `langueLabel` for the transcription card. */
    fun language(code: String): String = nzPodcastLanguageLabel(code)

    /** Why the server cannot transcribe, in French (`reasonText`). */
    fun reason(code: String?): String =
        code?.takeIf { it.isNotBlank() }?.let { nzPodcastReasonText(it) }
            ?: "La transcription n'est pas configurée sur ce serveur."

    /** What a refused start says (`transcribeError`). */
    fun transcribeCause(code: String?): String =
        nzPodcastTranscriptionCause(code.orEmpty())

    /** The batch sizes the transcription card offers for `eligible` episodes. */
    fun batchChoices(eligible: Int): List<Int> = nzPodcastBatchOptions(eligible.c).map { it.toInt() }

    /** « 12 épisodes en file d'archivage. » */
    fun archiveQueued(count: Int): String =
        if (count > 1) "$count épisodes en file d'archivage" else "$count épisode en file d'archivage"

    /** « 12 épisodes en file de transcription. » */
    fun transcribeQueued(count: Int): String =
        if (count > 1) "$count épisodes en file de transcription"
        else "$count épisode en file de transcription"

    /** « 1 ignoré : L'épisode dépasse la durée maximale de transcription. » */
    fun transcribeSkipped(count: Int, cause: String?): String {
        val head = if (count > 1) "$count ignorés" else "$count ignoré"
        return if (cause.isNullOrBlank()) head else "$head : $cause"
    }

    /** « 3 épisodes · 2,5 h d'audio, environ 1,2 h de calcul. » */
    fun estimate(eligible: Int, audioSeconds: Long): String {
        val hours = (audioSeconds / 3600.0 * 10).round1()
        val cpu = (hours / 2 * 10).round1()
        return "${episodeCount(eligible)} · ${hours.hours()} d'audio, environ ${cpu.hours()} de calcul."
    }

    /** « page 2 sur 5 », the transcript's footer (`pageInfo`). */
    fun page(offset: Int, limit: Int, total: Int): String {
        val info = nzPodcastPageInfo(offset.c, limit.c, total.c)
        return "page ${info.page} sur ${info.pages}"
    }

    /** « Aucun passage pour « x » dans cette émission. » */
    fun noHit(query: String): String = "Aucun passage pour « $query » dans cette émission."

    /** « 7 passages », the search's counter. */
    fun hitCount(total: Int): String = if (total > 1) "$total passages" else "$total passage"

    /** The search's warning when the server scanned only the recent episodes. */
    fun searchScope(episodes: Int): String =
        "Recherche dans les $episodes épisodes transcrits les plus récents."

    /** « 2 caractères minimum. » */
    const val SEARCH_MIN = "2 caractères minimum."

    /** « La recherche dans les transcriptions a besoin du serveur. » */
    const val SEARCH_OFFLINE = "La recherche dans les transcriptions a besoin du serveur."

    /** « Trop de recherches. Réessayez dans 12 s. » */
    fun searchRateLimited(seconds: Int): String = "Trop de recherches. Réessayez dans $seconds s."

    /** « Aucun abonnement pour l'instant. » — the empty grid. */
    const val NO_SUBSCRIPTION = "Aucun abonnement pour l'instant."

    /** The hint under the empty grid, for an administrator who can add one. */
    const val NO_SUBSCRIPTION_HINT = "Cherchez un podcast ci-dessus pour vous abonner."

    /** « Aucun épisode disponible. » */
    const val NO_EPISODE = "Aucun épisode disponible."

    /** « Aucun podcast pour « x ». » */
    fun noSearchResult(query: String): String = "Aucun podcast pour « $query »."

    /** « Ce podcast n'existe plus sur Deezer. … » — a local show's note. */
    const val LOCAL_NOTE =
        "Ce podcast n'existe plus sur Deezer. Tout ce qui a été archivé reste ici, jouable et " +
            "téléchargeable comme avant — c'est désormais un podcast local."

    /** The chip that says an episode's audio is on the server. */
    const val ARCHIVED = "Archivé"

    /** The chip that says the provider cannot serve an episode any more. */
    const val UNAVAILABLE = "non archivé"

    /** « Les chapitres seront lus à la fin de l'archivage. » */
    const val CHAPTERS_UNARCHIVED = "Les chapitres seront lus à la fin de l'archivage."

    /** « Aucun chapitre pour cet épisode. » */
    const val NO_CHAPTER = "Aucun chapitre pour cet épisode."

    /** « Chapitres indisponibles. » — a failure, with « Réessayer ». */
    const val CHAPTERS_FAILED = "Chapitres indisponibles."

    /** « Hors ligne : les chapitres n'ont pas encore été chargés. » */
    const val CHAPTERS_OFFLINE = "Hors ligne : les chapitres n'ont pas encore été chargés."

    /** « Hors ligne — dernière copie » — the note over a kept list. */
    const val OFFLINE_COPY = "Hors ligne — dernière copie"

    /** « Hors ligne · version mémorisée » — the screen standard's note. */
    const val STALE = "Hors ligne · version mémorisée"

    /** The transcription panel's description of what happens where. */
    const val TRANSCRIPT_LOCAL =
        "Le texte est produit sur ce serveur à partir du fichier archivé. Rien n'est envoyé à un tiers."

    /** « Un épisode d'une heure demande souvent plus d'une heure de calcul… » */
    const val TRANSCRIPT_SLOW =
        "Un épisode d'une heure demande souvent plus d'une heure de calcul ; le serveur continue " +
            "si vous fermez cet écran."

    /** `format.js#duration`: `m:ss`, or `h:mm:ss` past the hour. */
    fun duration(seconds: Int): String {
        val s = seconds.coerceAtLeast(0)
        val hours = s / 3600
        val minutes = (s % 3600) / 60
        val rest = s % 60
        return if (hours > 0) {
            "$hours:${minutes.toString().padStart(2, '0')}:${rest.toString().padStart(2, '0')}"
        } else {
            "$minutes:${rest.toString().padStart(2, '0')}"
        }
    }

    /** The line under an episode: its length, or its resume state. */
    fun episodeLine(episode: EpisodeData): String {
        val position = episode.position
        return when {
            position == null -> episode.durationLabel
            position.finished -> "Terminé"
            else -> resumeLine(position)
        }
    }

    private fun Double.round1(): Double = kotlin.math.round(this * 10) / 10

    /** « 2,5 h » / « 4,8 h » — French decimal comma, one decimal. */
    private fun Double.hours(): String {
        val text = toString().replace('.', ',')
        return "$text h"
    }
}
