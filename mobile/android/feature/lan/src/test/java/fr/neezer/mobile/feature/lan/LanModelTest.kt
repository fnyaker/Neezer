package fr.neezer.mobile.feature.lan

import fr.neezer.mobile.bridge.ScreenState
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.LanFriendFfi
import uniffi.nz_ffi.LanStateFfi

/**
 * « Appareils à proximité »'s state machine, on the JVM (lot LN14).
 *
 * Every test names the rule it holds; where a number matters it is written next
 * to the assertion (10 s of `Searching`, the 0-request read). Nothing reads the
 * wall clock: the model runs on the test's own scheduler and each step is an
 * explicit `runCurrent()`.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class LanModelTest {
    /** A model over a fresh fake, started, with its first reads already landed. */
    private fun TestScope.model(
        port: FakeLanPort,
        playlists: LanPlaylistSource = FakePlaylists(),
        hintAfterMs: Long = NO_PEER_HINT_AFTER_MS,
        toast: (String) -> Unit = {},
    ): LanModel {
        val model = LanModel(
            port = port,
            scope = backgroundScope,
            playlists = playlists,
            io = UnconfinedTestDispatcher(testScheduler),
            toast = toast,
            hintAfterMs = hintAfterMs,
        )
        model.start()
        runCurrent()
        return model
    }

    @Test
    fun start_reads_the_node_once_and_asks_the_server_for_the_two_lists() = runTest {
        val port = FakeLanPort()
        val model = model(port)

        // The read is a clone of memory; the lists are the section's two requests.
        assertEquals(LanStateFfi.Searching, model.state.value.snapshot?.state)
        assertTrue(model.state.value.settings!!.enabled)
        assertEquals(listOf("subscribe", "devices", "friends", "start"), port.calls)

        model.start()
        runCurrent()
        assertEquals(1, port.calls.count { it == "devices" })
        assertEquals(1, port.calls.count { it == "subscribe" })
    }

    @Test
    fun the_switches_write_at_once_and_the_core_persists_them() = runTest {
        val port = FakeLanPort()
        val model = model(port)

        model.setEnabled(false)
        assertFalse(model.state.value.settings!!.enabled)
        assertFalse(port.settingsWrites.last().enabled)

        model.setServe(false)
        assertFalse(port.settingsWrites.last().serve)
    }

    @Test
    fun a_refused_write_puts_the_value_back_and_says_why() = runTest {
        val port = FakeLanPort()
        val toasts = mutableListOf<String>()
        val model = model(port, toast = { toasts += it })
        port.failSettings = IllegalStateException("Le réglage n'a pas pu être enregistré")

        model.setServe(false)

        assertTrue(model.state.value.settings!!.serve)
        assertEquals("Le réglage n'a pas pu être enregistré", model.state.value.notice)
        assertEquals(listOf("Le réglage n'a pas pu être enregistré"), toasts)
    }

    @Test
    fun the_last_tolerated_quality_cannot_be_removed() = runTest {
        val port = FakeLanPort(settingsValue = sampleSettings(tolerated = listOf("FLAC")))
        val model = model(port)

        model.toggleTolerated("FLAC")

        // The note: « at least one ». An empty list would mean "never take
        // anything from a peer", which is not what an untick asks for.
        assertEquals(listOf("FLAC"), model.state.value.settings!!.tolerated)
        assertEquals(listOf("FLAC"), port.settingsWrites.last().tolerated)
    }

    @Test
    fun a_tolerated_quality_is_added_and_kept_in_the_note_order() = runTest {
        val port = FakeLanPort(settingsValue = sampleSettings(tolerated = listOf("OPUS_320")))
        val model = model(port)

        model.toggleTolerated("OPUS_64")
        model.toggleTolerated("FLAC")

        assertEquals(listOf("FLAC", "OPUS_320", "OPUS_64"), model.state.value.settings!!.tolerated)
        assertEquals(listOf("FLAC", "OPUS_320", "OPUS_64"), port.settingsWrites.last().tolerated)
    }

    @Test
    fun renaming_a_device_moves_the_row_then_the_wire() = runTest {
        val port = FakeLanPort(devicesValue = listOf(sampleDevice(id = "d1", name = "Téléphone", current = false)))
        val model = model(port)

        model.renameDevice("d1", "  Salon  ")
        runCurrent()

        assertEquals("Salon", model.state.value.devicesSeen.single().name)
        assertTrue("rename:d1:Salon" in port.calls)
    }

    @Test
    fun a_refused_rename_puts_the_row_back_with_its_cause() = runTest {
        val port = FakeLanPort(devicesValue = listOf(sampleDevice(id = "d1", name = "Téléphone", current = false)))
        val model = model(port)
        port.failRename = IllegalStateException("HTTP 409 : nom déjà pris")

        model.renameDevice("d1", "Salon")
        runCurrent()

        assertEquals("Téléphone", model.state.value.devicesSeen.single().name)
        assertEquals("HTTP 409 : nom déjà pris", model.state.value.notice)
    }

    @Test
    fun revoking_removes_the_row_at_once_and_restores_it_on_refusal() = runTest {
        val port = FakeLanPort(
            devicesValue = listOf(
                sampleDevice(id = "self", current = true),
                sampleDevice(id = "d1", name = "Salon", current = false),
            ),
        )
        val model = model(port)

        model.revokeDevice("d1")
        assertEquals(listOf("self"), model.state.value.devicesSeen.map { it.id })
        runCurrent()
        assertTrue("revoke:d1" in port.calls)

        port.failRevoke = IllegalStateException("HTTP 500 : Erreur serveur.")
        model.revokeDevice("self")
        runCurrent()
        assertEquals(listOf("self"), model.state.value.devicesSeen.map { it.id })
        assertEquals("HTTP 500 : Erreur serveur.", model.state.value.notice)
    }

    @Test
    fun a_new_friend_refreshes_both_directions_of_trust() = runTest {
        val port = FakeLanPort()
        val model = model(port)
        val before = port.calls.count { it == "friends" }

        model.addFriend("  Alice  ")
        runCurrent()

        assertTrue("addFriend:Alice" in port.calls)
        assertEquals(before + 1, port.calls.count { it == "friends" })
    }

    @Test
    fun a_refused_friend_keeps_the_servers_own_sentence() = runTest {
        val port = FakeLanPort()
        val model = model(port)
        port.failAddFriend = IllegalStateException("Aucun compte de ce nom.")

        model.addFriend("Personne")
        runCurrent()

        assertEquals("Aucun compte de ce nom.", model.state.value.notice)
    }

    @Test
    fun a_friend_scope_writes_the_playlist_ids_and_unticks_all() = runTest {
        val port = FakeLanPort()
        val model = model(port)
        val alice = sampleFriend().userId

        model.setFriendAll(alice, true)
        runCurrent()
        assertTrue("setFriendScope:$alice:true:" in port.calls)
        assertTrue((model.state.value.friends as ScreenState.Ready).data.sharesTo.single().all)

        model.toggleFriendPlaylist(alice, "2")
        runCurrent()
        assertTrue("setFriendScope:$alice:false:2" in port.calls)
        val friend: LanFriendFfi = (model.state.value.friends as ScreenState.Ready).data.sharesTo.single()
        assertFalse(friend.all)
        assertEquals(listOf("2"), friend.playlists.map { it.id })
    }

    @Test
    fun a_refused_scope_puts_the_friend_back() = runTest {
        val port = FakeLanPort()
        val model = model(port)
        val alice = sampleFriend().userId
        port.failSetScope = IllegalStateException("HTTP 400 : playlist inconnue")

        model.setFriendAll(alice, true)
        runCurrent()

        val friend = (model.state.value.friends as ScreenState.Ready).data.sharesTo.single()
        assertFalse(friend.all)
        assertEquals("HTTP 400 : playlist inconnue", model.state.value.notice)
    }

    @Test
    fun a_friend_is_removed_from_both_lists_at_once() = runTest {
        val port = FakeLanPort()
        val model = model(port)
        val alice = sampleFriend().userId

        model.removeFriend(alice)
        runCurrent()

        val friends = (model.state.value.friends as ScreenState.Ready).data
        assertTrue(friends.sharesTo.none { it.userId == alice })
        assertEquals(listOf("trusted"), friends.trustedBy.map { it.userId })
    }

    @Test
    fun the_hotel_line_appears_after_ten_seconds_and_goes_when_a_peer_answers() = runTest {
        val port = FakeLanPort(
            snapshotValue = sampleSnapshot(state = LanStateFfi.Searching),
            devicesValue = listOf(sampleDevice(id = "self", current = true), sampleDevice(id = "pc", current = false)),
        )
        val model = model(port)
        assertFalse("nothing is shown in the first seconds", model.state.value.showNoPeerHint)

        // The clock is the test's: 9 999 ms is nothing, the 10th second is the line.
        advanceTimeBy(NO_PEER_HINT_AFTER_MS - 1)
        runCurrent()
        assertFalse(model.state.value.showNoPeerHint)
        advanceTimeBy(1)
        runCurrent()
        assertTrue(model.state.value.showNoPeerHint)

        // A peer answers: the hint goes away by itself.
        port.snapshotValue = sampleSnapshot(
            state = LanStateFfi.Connected(peers = 1u),
            peers = listOf(samplePeer()),
        )
        LanFeed.listener.changed(11uL)
        runCurrent()
        assertFalse(model.state.value.showNoPeerHint)
        assertEquals(1, model.state.value.snapshot?.peers?.size)
    }

    @Test
    fun the_hotel_line_stays_away_when_there_is_no_other_device() = runTest {
        val port = FakeLanPort(
            snapshotValue = sampleSnapshot(state = LanStateFfi.Searching),
            devicesValue = listOf(sampleDevice(id = "self", current = true)),
        )
        val model = model(port)

        advanceTimeBy(NO_PEER_HINT_AFTER_MS * 3)
        runCurrent()

        assertFalse(model.state.value.showNoPeerHint)
    }

    @Test
    fun a_dead_server_shows_the_cause_and_keeps_the_node_state() = runTest {
        val port = FakeLanPort()
        port.failDevices = IllegalStateException("Hors ligne")
        val model = model(port)

        val devices = model.state.value.devices
        assertTrue(devices is ScreenState.Failed)
        assertEquals("Hors ligne", (devices as ScreenState.Failed).cause)
        // The snapshot is the local node's and is still drawn: rule 3, never a
        // spinner past the first failure.
        assertEquals(LanStateFfi.Searching, model.state.value.snapshot?.state)
    }

    @Test
    fun an_unsupported_server_is_a_state_not_a_failure() = runTest {
        val port = FakeLanPort(snapshotValue = sampleSnapshot(state = LanStateFfi.Unsupported))
        port.failStart = IllegalStateException("le serveur n'a pas cette fonction")
        val model = model(port)

        assertTrue(model.state.value.unsupported)
        assertNull("no refusal is shown for a server that simply lacks the feature", model.state.value.notice)
    }

    @Test
    fun the_port_field_only_accepts_a_port_and_zero() = runTest {
        val port = FakeLanPort()
        val model = model(port)

        model.setPortDraft("12ab3")
        assertEquals("123", model.state.value.portDraft)
        model.commitPort()
        assertEquals(123, port.settingsWrites.last().port.toInt())

        model.setPortDraft("70000")
        model.commitPort()
        // Out of range: the field comes back to the committed port, nothing is written.
        assertEquals("123", model.state.value.portDraft)
        assertEquals(123, port.settingsWrites.last().port.toInt())
    }

    @Test
    fun a_picker_reads_the_cached_playlists_then_refreshes_them() = runTest {
        val playlists = FakePlaylists()
        val model = model(FakeLanPort(), playlists)

        model.openPicker(sampleFriend().userId)
        runCurrent()

        assertEquals(1, playlists.cachedReads)
        assertEquals(1, playlists.freshReads)
        assertEquals(listOf("Route", "Nuit"), model.state.value.playlistChoices.map { it.title })
    }
}
