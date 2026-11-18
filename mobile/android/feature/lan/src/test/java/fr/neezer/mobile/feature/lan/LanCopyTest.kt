package fr.neezer.mobile.feature.lan

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant
import java.time.ZoneId
import uniffi.nz_ffi.LanStateFfi

/** The French words of the two screens, and the pure rules behind them (LN14). */
class LanCopyTest {
    @Test
    fun the_last_tolerated_quality_survives_an_untick() {
        assertEquals(listOf("FLAC"), toggleTolerated(listOf("FLAC"), "FLAC"))
    }

    @Test
    fun tolerated_choices_keep_the_notes_order() {
        assertEquals(
            listOf("FLAC", "OPUS_320", "OPUS_128"),
            toggleTolerated(listOf("OPUS_128", "FLAC"), "OPUS_320"),
        )
        assertEquals(listOf("OPUS_320"), toggleTolerated(listOf("FLAC", "OPUS_320"), "FLAC"))
    }

    @Test
    fun a_discovered_port_is_bounded_before_it_reaches_the_ffi() {
        assertTrue(validDiscoveredPort(1))
        assertTrue(validDiscoveredPort(45910))
        assertTrue(validDiscoveredPort(65535))
        assertFalse(validDiscoveredPort(0))
        assertFalse(validDiscoveredPort(-1))
        assertFalse(validDiscoveredPort(65536))
        assertFalse(validDiscoveredPort(Int.MAX_VALUE))
    }

    @Test
    fun a_port_is_zero_or_a_real_port() {
        assertEquals(0, parsePort("0"))
        assertEquals(45910, parsePort(" 45910 "))
        assertNull(parsePort("65536"))
        assertNull(parsePort("abc"))
        assertNull(parsePort(""))
    }

    @Test
    fun bytes_follow_the_spas_rule() {
        assertEquals("0 o", bytesText(0))
        assertEquals("812 ko", bytesText(812 * 1024L))
        assertEquals("9,5 Mo", bytesText((9.5 * 1024 * 1024).toLong()))
        assertEquals("350 Mo", bytesText(350L * 1024 * 1024))
        assertEquals("1,2 Go", bytesText((1.2 * 1024 * 1024 * 1024).toLong()))
    }

    @Test
    fun the_key_line_names_the_day_or_the_failure() {
        val zone = ZoneId.of("Europe/Paris")
        // 2027-12-18T00:00:00Z is the 18th in Paris too (01:00).
        val text = keyText(1_797_552_000L, null, zone)
        assertEquals("Clé valable jusqu'au 18 décembre", text)
        assertEquals(
            "Impossible de renouveler la clé LAN : serveur injoignable",
            keyText(null, "serveur injoignable", zone),
        )
        assertTrue(keyText(null, null, zone).startsWith("Aucune clé LAN"))
    }

    @Test
    fun each_node_state_has_one_sentence() {
        assertEquals("Échange arrêté", stateText(LanStateFfi.Off))
        assertEquals("En attente de la clé LAN", stateText(LanStateFfi.NotEnrolled))
        assertEquals("Recherche d'appareils sur ce Wi-Fi…", stateText(LanStateFfi.Searching))
        assertEquals("Connecté à 1 appareil", stateText(LanStateFfi.Connected(peers = 1u)))
        assertEquals("Connecté à 3 appareils", stateText(LanStateFfi.Connected(peers = 3u)))
        assertTrue(stateText(LanStateFfi.Unsupported).startsWith("Ce serveur"))
    }

    @Test
    fun a_peer_is_named_by_what_it_says_and_never_by_a_model() {
        assertEquals("Android", platformText("android"))
        assertEquals("Ordinateur", platformText("desktop"))
        assertEquals("Mon appareil", relationText(true))
        assertEquals("Ami", relationText(false))
        assertEquals("connecté · 7 ms", signalText(samplePeer(rttMs = 7)))
        assertEquals("connecté", signalText(samplePeer(rttMs = 0)))
        assertEquals("1240 titres · 3 listes", peerDetailText(samplePeer()))
        assertEquals("rien de partagé pour l'instant", peerDetailText(samplePeer(tracks = 0, sets = 0)))
    }

    @Test
    fun a_transfer_says_which_way_and_why_it_is_urgent() {
        val text = transferText(sampleTransfer(bytes = 12_000_000, total = 40_000_000))
        assertTrue(text, text.startsWith("Réception depuis Bureau"))
        assertTrue(text, text.contains("11 Mo / 38 Mo"))
        assertTrue(text, text.endsWith("· lecture — Insomnia"))
        assertTrue(transferText(sampleTransfer(send = true)).startsWith("Envoi vers Bureau"))
    }

    @Test
    fun a_set_says_what_it_is_and_where_it_is_complete() {
        assertEquals("Playlist", setKindText("playlist"))
        assertEquals("Podcast", setKindText("show"))
        assertEquals(
            "50 titres · 12 sur cet appareil · complet sur Bureau",
            setDetailText(tracks = 50, local = 12, complete = true, peer = "Bureau"),
        )
        assertEquals("1 titre", setDetailText(tracks = 1, local = 0, complete = false, peer = "Bureau"))
    }

    @Test
    fun the_advanced_line_names_the_fallback_port() {
        assertEquals("Le port sera choisi quand l'échange démarrera.", portText(0, false))
        assertEquals("Port 45910/udp", portText(45910, false))
        assertTrue(portText(52111, true).contains("ouvrez 52111/udp"))
    }

    @Test
    fun the_empty_state_says_why_when_nothing_is_connected() {
        assertTrue(peerSetsEmptyText(0).startsWith("Aucun appareil connecté"))
        assertTrue(peerSetsEmptyText(2).startsWith("Vos appareils n'ont rien partagé"))
    }

    @Test
    fun a_device_is_seen_in_words_and_never_as_an_iso_instant() {
        val now = Instant.parse("2026-11-18T09:00:00Z").toEpochMilli()
        assertEquals("vu à l'instant", seenText("2026-11-18T08:59:30Z", now))
        assertEquals("vu il y a 5 min", seenText("2026-11-18T08:55:00Z", now))
        assertEquals("vu il y a 3 h", seenText("2026-11-18T06:00:00Z", now))
        assertEquals("vu il y a 2 j", seenText("2026-11-16T09:00:00Z", now))
        assertTrue(seenText("2026-10-01T09:00:00Z", now).startsWith("vu le "))
        assertEquals("", seenText(null, now))
        assertEquals("", seenText("pas une date", now))
    }

    @Test
    fun a_random_instance_name_carries_nothing_about_the_phone() {
        val names = (1..50).map { LanNsd.randomInstanceName() }
        assertEquals(50, names.distinct().size)
        names.forEach { name ->
            assertTrue(name, Regex("^nz-[0-9a-f]{10}$").matches(name))
        }
    }
}
