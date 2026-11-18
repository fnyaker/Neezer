package fr.neezer.mobile.feature.admin

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.AdminServerCard

/**
 * La copie des pages d'administration, sans écran : les phrases que la SPA écrit
 * (`adminusers.js`, `Settings.svelte`), les compteurs, et la ligne de l'ARL.
 *
 * Le point qui compte ici est la règle 6 et la sécurité : un identifiant est en
 * ÉCRITURE SEULE. `AdminServerCard` n'a pas de champ qui pourrait porter l'ARL,
 * et la ligne que l'écran dessine ne montre que l'indice masqué du serveur et sa
 * provenance — jamais la valeur.
 */
class AdminCopyTest {
    @Test
    fun the_arl_line_shows_the_masked_hint_and_its_source_never_the_credential() {
        val masked = AdminServerCard(
            uploadQuotaGb = 0.0,
            deezerEnabled = true,
            arlSet = true,
            arlSource = "database",
            arlHint = "…a1b2",
            statusOk = true,
            statusReason = null,
            statusMessage = null,
        )
        assertEquals("ARL actif …a1b2 · source : saisi ici", AdminCopy.arlLine(masked))

        val fromConfig = masked.copy(arlSource = "config")
        assertEquals("ARL actif …a1b2 · source : configuration serveur", AdminCopy.arlLine(fromConfig))

        val none = masked.copy(arlSet = false, arlHint = null)
        assertEquals("Aucun ARL configuré", AdminCopy.arlLine(none))
        assertFalse("not even when unset does the line say a value", AdminCopy.arlLine(none).contains("a1b2"))

        // The record has NO field that could carry the credential: this is the
        // compile-time half of "write-only", and the list of its fields is the
        // whole surface the screen can draw.
        val fields = AdminServerCard::class.java.declaredFields.map { it.name }.toSet()
        assertFalse("no arl field on the card", fields.any { it.equals("arl", ignoreCase = true) })
        assertTrue(fields.contains("arlHint"))
    }

    @Test
    fun a_gauge_speaks_gigabytes_and_megabytes_not_bytes() {
        assertEquals("0 o", AdminCopy.bytes(0uL))
        assertEquals("1 Ko", AdminCopy.bytes(1024uL))
        assertEquals("12 Mo", AdminCopy.bytes(12uL * 1024u * 1024u))
        assertEquals("9,2 Go", AdminCopy.bytes(9_900_000_000uL))
        assertEquals("2,0 To", AdminCopy.bytes(2uL * 1024u * 1024u * 1024u * 1024u))
    }

    @Test
    fun the_count_line_counts_in_french() {
        assertEquals("1 compte", AdminCopy.countLabel(1, "compte"))
        assertEquals("3 comptes", AdminCopy.countLabel(3, "compte"))
        assertEquals("1 invitation", AdminCopy.countLabel(1, "invitation"))
        assertEquals("2 titres", AdminCopy.countLabel(2, "titre"))
    }

    @Test
    fun an_expiry_says_how_long_is_left_and_never_a_date_in_the_past() {
        val now = 1_800_000_000_000L
        val inThreeDays = "2027-02-03T09:00:00"
        assertTrue(AdminCopy.expiryLabel(inThreeDays, now).startsWith("expire dans"))
        assertEquals("expirée", AdminCopy.expiryLabel("2020-01-01T00:00:00", now))
        assertEquals("—", AdminCopy.expiryLabel(null, now))
    }

    @Test
    fun a_relative_time_says_ago_and_never_a_bare_number() {
        val now = 1_800_000_000_000L
        assertEquals("à l'instant", AdminCopy.relativeTime("2027-01-15T09:00:00", now))
        assertEquals("—", AdminCopy.relativeTime(null, now))
        assertTrue(AdminCopy.relativeTime("2027-01-10T09:00:00", now).startsWith("il y a"))
    }

    @Test
    fun the_filters_narrow_the_rows_already_loaded() {
        val rows = listOf(
            AccountRow.User(AdminFixtures.user(id = "u-1", name = "Nadia", admin = true)),
            AccountRow.User(AdminFixtures.user(id = "u-2", name = "Léo")),
            AccountRow.User(AdminFixtures.user(id = "u-3", name = "Sacha", disabled = true)),
            AccountRow.Invite(AdminFixtures.invite(id = "i-1", note = "Léa")),
        )
        assertEquals(4, filterRows(rows, AccountFilter.All, "").size)
        assertEquals("Léo and Nadia are active", 2, filterRows(rows, AccountFilter.Active, "").size)
        assertEquals(1, filterRows(rows, AccountFilter.Admins, "").size)
        assertEquals("Léo and Sacha are both members", 2, filterRows(rows, AccountFilter.Members, "").size)
        assertEquals(1, filterRows(rows, AccountFilter.Disabled, "").size)
        assertEquals("the invites only, and Léa matches", 1, filterRows(rows, AccountFilter.Invited, "lea").size)
        assertEquals(0, filterRows(rows, AccountFilter.Invited, "zoe").size)
        // The account search runs on the SERVER: the filter never hides a user row
        // because of the term (it is not a client-side scan of 25 rows).
        assertEquals(2, filterRows(rows, AccountFilter.Active, "lea").size)
    }

    @Test
    fun the_failure_detail_copied_is_the_screens_own_text() {
        val text = failuresText(AdminFixtures.failures(), "analyse")
        assertTrue(text.startsWith("Neezer — analyse : 2 en échec"))
        assertTrue(text.contains("1x ffmpeg a disparu"))
        assertTrue(text.contains("a.flac — ffmpeg a disparu"))
        assertTrue(text.contains("… et 1 autre(s)"))
    }

    @Test
    fun the_automatch_reasons_are_said_in_french() {
        assertEquals("aucun titre du même nom et du même artiste", AdminCopy.automatchReason("nothing with the same title and artist"))
        assertEquals("le plus proche : 97,5 %", AdminCopy.automatchReason("closest 97.5 % below the threshold"))
        assertEquals("une cause inconnue", AdminCopy.automatchReason("une cause inconnue"))
        assertEquals("99,5 %", AdminCopy.percent(0.995))
    }
}
