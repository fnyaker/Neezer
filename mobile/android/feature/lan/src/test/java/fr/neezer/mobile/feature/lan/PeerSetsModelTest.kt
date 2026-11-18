package fr.neezer.mobile.feature.lan

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** « Sur vos appareils »'s state machine, on the JVM (lot LN14). */
@OptIn(ExperimentalCoroutinesApi::class)
class PeerSetsModelTest {
    private fun model(port: FakeLanPort, scope: kotlinx.coroutines.CoroutineScope) = PeerSetsModel(
        port = port,
        scope = scope,
        io = UnconfinedTestDispatcher(),
    )

    @Test
    fun the_replica_is_a_memory_read_and_pays_no_request() = runTest {
        val port = FakeLanPort()
        val model = model(port, backgroundScope)

        model.reload()

        // `Lan::sets()` is a short lock over the replicated pages: no `found`, no
        // `devices`, nothing on the wire.
        assertTrue("no request was made", port.calls.isEmpty())
        assertEquals(1, model.state.value.sets.size)
        assertEquals(1, model.state.value.peers)
    }

    @Test
    fun a_tap_ticks_one_set_and_the_key_is_the_ffi_spelling() = runTest {
        val port = FakeLanPort()
        val model = model(port, backgroundScope)
        model.reload()
        val key = PeerSetsModel.keyOf(sampleSet())

        assertEquals("${"aa".repeat(32)}:playlist:42", key)
        model.toggle(key)
        assertEquals(setOf(key), model.state.value.selected)
        assertTrue(model.state.value.canDownload)
    }

    @Test
    fun a_peer_group_selects_and_unselects_all_its_sets() = runTest {
        val port = FakeLanPort(
            peerSetsValue = listOf(
                sampleSet(ident = "1"),
                sampleSet(ident = "2"),
                sampleSet(peer = "bb".repeat(32), peerName = "Salon", ident = "3"),
            ),
        )
        val model = model(port, backgroundScope)
        model.reload()

        model.togglePeer("aa".repeat(32))
        assertEquals(2, model.state.value.selected.size)
        model.togglePeer("aa".repeat(32))
        assertTrue(model.state.value.selected.isEmpty())
    }

    @Test
    fun download_queues_exactly_the_ticked_keys_and_clears_the_selection() = runTest {
        val port = FakeLanPort()
        val model = model(port, backgroundScope)
        model.reload()
        val key = PeerSetsModel.keyOf(sampleSet())
        model.toggle(key)

        model.download()
        runCurrent()

        assertEquals(listOf(listOf(key)), port.downloaded)
        assertEquals("1 liste ajoutée aux téléchargements", model.state.value.note)
        assertTrue(model.state.value.selected.isEmpty())
    }

    @Test
    fun a_refusal_is_the_cores_own_cause_and_keeps_the_selection() = runTest {
        val port = FakeLanPort()
        val model = model(port, backgroundScope)
        model.reload()
        model.toggle(PeerSetsModel.keyOf(sampleSet()))
        port.failDownload = IllegalStateException("l'appareil « Bureau » ne répond plus")

        model.download()
        runCurrent()

        assertEquals("l'appareil « Bureau » ne répond plus", model.state.value.notice)
        assertEquals(1, model.state.value.selected.size)
        assertFalse(model.state.value.busy)
    }

    @Test
    fun a_set_that_left_the_replica_leaves_the_selection_with_it() = runTest {
        val port = FakeLanPort()
        val model = model(port, backgroundScope)
        model.reload()
        model.toggle(PeerSetsModel.keyOf(sampleSet()))

        port.peerSetsValue = emptyList()
        model.reload()

        assertTrue(model.state.value.selected.isEmpty())
        assertTrue(model.state.value.sets.isEmpty())
    }
}
