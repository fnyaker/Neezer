package fr.neezer.mobile.feature.offline

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers

/**
 * MB15's state machine, on the JVM: what a click says BEFORE the core answers,
 * what a failure says after, and the counts a button prints.
 *
 * No emulator, no `.so`, no network: the port is a fake of the CONTRACT, so a
 * green test here is a statement about the code a phone runs. The numbers are
 * the acceptance's own — a warm open costs no extra `state()` call, and a
 * refused download prints its cause and offers « Réessayer ».
 *
 * JUnit 4's message argument comes FIRST (`assertEquals(message, expected,
 * actual)`); every assertion below is written that way.
 */
class DownloadsModelTest {
    private val port = FakeOfflinePort()

    private fun model(): Pair<DownloadsModel, MutableList<Triple<String, String?, (() -> Unit)?>>> {
        val toasts = mutableListOf<Triple<String, String?, (() -> Unit)?>>()
        val model = DownloadsModel(
            port = port,
            scope = CoroutineScope(Dispatchers.Unconfined),
            io = Dispatchers.Unconfined,
            freeSpace = { 8L * 1024 * 1024 * 1024 },
            toast = { text, label, run -> toasts += Triple(text, label, run) },
            causeText = { error -> error.message ?: error::class.java.simpleName },
        )
        return model to toasts
    }

    /** The subscription's first value IS the screen: one `state()`, then events. */
    @Test
    fun a_warm_open_paints_the_subscribed_state_and_reads_the_store_once() {
        port.stateFlow.value = offlineState()
        val (model, _) = model()
        model.start()
        model.start() // a second composable mounting must not subscribe twice

        assertEquals("one subscription per process", 1, port.calls.count { it == "subscribe" })
        assertEquals(3uL, model.state.value.version)
        assertEquals("playlist:42", model.state.value.sets.single().context)
        assertEquals(194, model.state.value.sets.single().done.toInt())
        assertTrue("the device holds titles", model.state.value.hasDownloads)
        assertEquals(194u, model.state.value.storage?.tracks)
        // The warm open reads the state ONCE (the subscribe's own push) and the
        // play cache's size: no request, no poll.
        assertEquals("the state is read once", 1, port.calls.count { it == "state" })
    }

    /** A queued set shows in the same frame, then the worker's row replaces it. */
    @Test
    fun queuing_a_set_answers_first_and_then_follows_the_worker() {
        val (model, toasts) = model()
        model.start()
        // The fake holds the answer, so the OPTIMISTIC row is observable: the
        // whole point is what the screen shows before the core answers.
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        port.addGate = gate
        model.add("playlist:42", "OPUS_128", "Nuit", "")

        assertEquals(listOf("playlist:42"), model.state.value.pending.toList())
        gate.complete(Unit)
        assertEquals("add:playlist:42:OPUS_128", port.calls.last { it.startsWith("add:") })
        // The fake answers at once: the optimistic row is replaced by the core's.
        assertTrue(model.state.value.pending.isEmpty())
        val set = requireNotNull(model.state.value.set("playlist:42"))
        assertEquals(200, set.total.toInt())
        assertTrue(set.running)
        assertTrue(
            "the person is told the set was queued: ${toasts.single()}",
            toasts.single().first.contains("« Nuit » ajouté"),
        )
    }

    /** A refusal prints the CAUSE and offers « Réessayer », which runs it again. */
    @Test
    fun a_refused_download_names_its_cause_and_its_retry_runs_the_call_again() {
        val (model, toasts) = model()
        model.start()
        port.failAdd = IllegalStateException("Hors ligne")

        model.add("album:7", "", "Album", "")
        assertTrue("the optimistic row is rolled back", model.state.value.pending.isEmpty())
        val toast = toasts.single()
        assertTrue(
            "the cause is the person's words: ${toast.first}",
            toast.first.startsWith("Téléchargement refusé : Hors ligne"),
        )
        assertEquals("Réessayer", toast.second)

        port.failAdd = null
        toast.third?.invoke()
        assertEquals(
            "Réessayer runs the same call",
            2,
            port.calls.count { it.startsWith("add:album:7") },
        )
        assertNotNull(model.state.value.set("album:7"))
    }

    /** Removing a set answers first and comes BACK on a refusal (rule 4). */
    @Test
    fun a_refused_removal_puts_the_set_back() {
        port.stateFlow.value = offlineState()
        val (model, toasts) = model()
        model.start()
        port.failRemove = IllegalStateException("disque occupé")

        model.removeSet("playlist:42")
        assertEquals(
            "the set is back on screen",
            "playlist:42",
            model.state.value.set("playlist:42")?.context,
        )
        assertTrue(toasts.single().first.startsWith("Retrait impossible : disque occupé"))
        assertEquals("Réessayer", toasts.single().second)
    }

    /** The quality answers at once and rolls back with the cause. */
    @Test
    fun the_quality_answers_first_and_rolls_back_when_the_core_refuses_it() {
        port.stateFlow.value = offlineState()
        val (model, toasts) = model()
        model.start()
        port.failQuality = IllegalStateException("qualité inconnue : « X »")

        model.setQuality("OPUS_64")
        assertEquals(
            "the rollback restores what the core last said",
            "OPUS_320",
            model.state.value.settings?.quality,
        )
        assertTrue(toasts.single().first.startsWith("Qualité non enregistrée"))

        port.failQuality = null
        model.setQuality("OPUS_64")
        assertEquals("OPUS_64", model.state.value.settings?.quality)
        assertEquals("setQuality:OPUS_64", port.calls.last())
    }

    /** A failure the worker reports is a toast with its cause, in French. */
    @Test
    fun a_failed_title_reaches_the_person_with_its_cause() {
        val (model, toasts) = model()
        model.start()
        port.listener?.onFailure("3135556", "le serveur prépare ce titre (réessai dans 30 s)")
        assertEquals(
            "Téléchargement impossible : le serveur prépare ce titre (réessai dans 30 s)",
            toasts.single().first,
        )
    }

    /** The play cache clears optimistically, and a refusal puts its size back. */
    @Test
    fun clearing_the_play_cache_answers_first_and_rolls_back() {
        port.playCache = 12L * 1024 * 1024
        val (model, _) = model()
        model.start()
        assertEquals(12L * 1024 * 1024, model.state.value.playCacheBytes)
        port.failClear = IllegalStateException("cache verrouillé")
        model.clearPlayCache()
        assertEquals(12L * 1024 * 1024, model.state.value.playCacheBytes)
    }

    /** The French words the SPA prints: `1,5 Go`, `812 ko`, `Opus 320`. */
    @Test
    fun the_bytes_and_the_rungs_are_read_the_french_way() {
        assertEquals("0 o", DownloadsModel.bytesLabel(0))
        assertEquals("512 o", DownloadsModel.bytesLabel(512))
        assertEquals("1,5 Go", DownloadsModel.bytesLabel(1536L * 1024 * 1024))
        assertEquals("1,4 Go", DownloadsModel.bytesLabel((1480L * 1024 * 1024)))
        assertEquals("812 Ko", DownloadsModel.bytesLabel(812L * 1024))
        assertEquals("FLAC", DownloadsModel.qualityLabel("FLAC"))
        assertEquals("Opus 320", DownloadsModel.qualityLabel("OPUS_320"))
        assertEquals("", DownloadsModel.qualityLabel(""))
    }

    /** The button's own reading: a running set is not "Téléchargé". */
    @Test
    fun a_running_set_is_not_a_finished_one() {
        port.stateFlow.value = offlineState(done = 12u, failed = 0u, running = true)
        val (model, _) = model()
        model.start()
        val set = requireNotNull(model.state.value.set("playlist:42"))
        assertTrue("still fetching", set.running)
        assertEquals(0u, set.failed)
        assertFalse("an unfinished set is not complete", set.done >= set.total)

        port.publish(offlineState(done = 200u, failed = 0u, running = false))
        assertFalse(model.state.value.set("playlist:42")!!.running)
        assertNull("a context nobody queued has no row", model.state.value.set("album:7"))
    }

    /** `kind:id` splits on the FIRST colon; a context without one says so. */
    @Test
    fun a_context_is_split_once_and_a_malformed_one_says_so() {
        assertEquals("playlist" to "42", splitContext("playlist:42"))
        assertEquals("track" to "a:b", splitContext("track:a:b"))
        val error = runCatching { splitContext("42") }.exceptionOrNull()
        assertTrue(error is IllegalArgumentException)
        assertTrue(error!!.message!!.contains("contexte de téléchargement invalide"))
    }
}
