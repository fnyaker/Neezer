package fr.neezer.mobile.feature.onboarding

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.AccountRun
import uniffi.nz_ffi.FeedCause
import uniffi.nz_ffi.FinishBlock
import uniffi.nz_ffi.ImportState

/**
 * The onboarding's French copy, held to the SPA's own sentences
 * (`onboarding.test.mjs`, `OnboardingAccount.svelte`). Pure functions: no core,
 * no Compose, no device.
 */
class OnboardingCopyTest {
    @Test
    fun the_counts_and_the_hints_are_the_pages() {
        assertEquals("0 artiste", OnboardingCopy.artistCount(0))
        assertEquals("1 artiste", OnboardingCopy.artistCount(1))
        assertEquals("3 artistes", OnboardingCopy.artistCount(3))
        assertEquals("1\u202f200 artistes", OnboardingCopy.artistCount(1200))
        assertEquals("Choisissez-en au moins un.", OnboardingCopy.progressHint(0))
        assertEquals("Encore 2 pour un bon départ.", OnboardingCopy.progressHint(1))
        assertEquals("C'est un bon départ.", OnboardingCopy.progressHint(3))
        assertEquals("Vous pouvez continuer ou terminer.", OnboardingCopy.progressHint(5))
    }

    @Test
    fun the_finish_messages_say_why() {
        assertEquals(
            "Choisissez au moins un artiste pour terminer.",
            OnboardingCopy.finishMessage(FinishBlock.MIN),
        )
        assertEquals(
            "Hors ligne : reconnectez-vous pour terminer.",
            OnboardingCopy.finishMessage(FinishBlock.OFFLINE),
        )
        assertEquals(
            "Enregistrement de vos choix…",
            OnboardingCopy.finishMessage(FinishBlock.PENDING),
        )
        assertEquals("", OnboardingCopy.finishMessage(null))
    }

    @Test
    fun every_feed_cause_is_named() {
        assertEquals("Deezer ne répond pas.", OnboardingCopy.feedCause(FeedCause.DEEZER_UNREACHABLE))
        assertEquals(
            "Le catalogue Deezer est désactivé sur ce serveur.",
            OnboardingCopy.feedCause(FeedCause.DISABLED),
        )
        assertEquals(
            "Le catalogue est en pause après plusieurs échecs.",
            OnboardingCopy.feedCause(FeedCause.HELD),
        )
        assertEquals(
            "Aucun catalogue n'est configuré sur ce serveur.",
            OnboardingCopy.feedCause(FeedCause.ABSENT),
        )
        assertEquals(
            "La sélection n'a pas pu être préparée.",
            OnboardingCopy.feedCause(FeedCause.UNKNOWN),
        )
    }

    @Test
    fun the_import_phases_say_what_the_job_does() {
        assertEquals("", OnboardingCopy.importProgress(null).label)
        assertEquals(
            "Lecture de la playlist…",
            OnboardingCopy.importProgress(OnboardingFixtures.job(state = ImportState.READING)).label,
        )
        val matching = OnboardingCopy.importProgress(
            OnboardingFixtures.job(state = ImportState.MATCHING, done = 42, total = 118),
        )
        assertEquals("Recherche des titres : 42 sur 118", matching.label)
        assertEquals(35, matching.percent)
        assertEquals(
            "Ajout à votre bibliothèque…",
            OnboardingCopy.importProgress(OnboardingFixtures.job(state = ImportState.SAVING)).label,
        )
        val done = OnboardingCopy.importProgress(
            OnboardingFixtures.job(state = ImportState.DONE, matched = 118, total = 120),
        )
        assertEquals("118 titres trouvés sur 120.", done.label)
        assertEquals(100, done.percent)
        assertEquals(true, done.done)
        val saved = OnboardingCopy.importProgress(
            OnboardingFixtures.job(state = ImportState.SAVED, matched = 1),
        )
        assertEquals("Ajouté : 1 titre.", saved.label)
        assertTrue(saved.saved)
        val failed = OnboardingCopy.importProgress(
            OnboardingFixtures.job(state = ImportState.ERROR, error = "Cette playlist est vide"),
        )
        assertEquals("L'import a échoué : Cette playlist est vide.", failed.label)
        assertTrue(failed.failed)
    }

    @Test
    fun the_unmatched_line_names_its_count() {
        assertEquals("1 titre n'a pas été trouvé", OnboardingCopy.unmatchedLabel(1))
        assertEquals("2 titres n'ont pas été trouvés", OnboardingCopy.unmatchedLabel(2))
    }

    @Test
    fun an_account_says_the_truth_never_a_finished_import_by_default() {
        assertEquals("Non relié", OnboardingCopy.accountSummary(null).text)
        assertEquals("Compte relié", OnboardingCopy.accountSummary(OnboardingFixtures.linkedAccount()).text)
        assertEquals(
            "Import en cours : 4 sur 10",
            OnboardingCopy.accountSummary(
                OnboardingFixtures.linkedAccount(
                    running = AccountRun(phase = "import", done = 4u, total = 10u),
                ),
            ).text,
        )
        assertEquals(
            "Connexion au compte…",
            OnboardingCopy.accountSummary(
                OnboardingFixtures.linkedAccount(running = AccountRun(phase = "connect", done = 0u, total = null)),
            ).text,
        )
        assertEquals(
            "Le compte refuse la connexion : reliez-le à nouveau.",
            OnboardingCopy.accountSummary(OnboardingFixtures.linkedAccount(status = "auth")).text,
        )
        assertEquals(
            "Le fournisseur ne répond pas : l'import reprendra plus tard.",
            OnboardingCopy.accountSummary(OnboardingFixtures.linkedAccount(status = "network")).text,
        )
        assertEquals(
            "Dernier import : 42 artistes, 118 titres.",
            OnboardingCopy.accountSummary(
                OnboardingFixtures.linkedAccount(addedArtists = 42, addedTracks = 118),
            ).text,
        )
        assertEquals(
            "Dernier import : 1 artiste.",
            OnboardingCopy.accountSummary(OnboardingFixtures.linkedAccount(addedArtists = 1)).text,
        )
        assertEquals(
            OnboardingCopy.Tone.Error,
            OnboardingCopy.accountSummary(OnboardingFixtures.linkedAccount(lastReason = "auth")).tone,
        )
    }

    @Test
    fun a_provider_that_is_switched_off_still_says_why() {
        assertEquals(
            "YouTube Music n'est pas installé sur ce serveur.",
            OnboardingCopy.youtubeState(
                OnboardingFixtures.provider(available = false, canLink = false, reason = "absent"),
            )!!.text,
        )
        assertEquals(
            "YouTube Music est désactivé sur ce serveur.",
            OnboardingCopy.youtubeState(
                OnboardingFixtures.provider(available = false, canLink = false, reason = "disabled"),
            )!!.text,
        )
        assertEquals(
            "Votre compte n'a pas accès à ce service.",
            OnboardingCopy.youtubeState(OnboardingFixtures.provider(denied = true))!!.text,
        )
        assertEquals(
            "Indisponible sur ce serveur.",
            OnboardingCopy.deezerState(
                OnboardingFixtures.provider(available = false, canLink = false, reason = "deezer_disabled"),
            )!!.text,
        )
    }

    @Test
    fun a_finished_import_keeps_its_note() {
        assertTrue(OnboardingCopy.importNote(OnboardingFixtures.finishedJob()).contains("Deezer"))
        assertEquals(
            "",
            OnboardingCopy.importNote(OnboardingFixtures.job(state = ImportState.ERROR, error = "x")),
        )
        assertNull(OnboardingCopy.youtubeState(OnboardingFixtures.provider())?.tone?.let { null })
        assertFalse(OnboardingCopy.playlistAvailable(OnboardingFixtures.options(imports = emptyList())))
        assertTrue(OnboardingCopy.playlistAvailable(OnboardingFixtures.options()))
    }
}
