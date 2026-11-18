package fr.neezer.mobile.feature.party

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PartyPhase

/**
 * The guest's screen, as plain Kotlin (lot MB20): `routes/Party.svelte`'s five
 * faces and its two numbers (the copy first, the network second).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PartyGuestModelTest {
    private fun model(service: FakePartyService, scope: kotlinx.coroutines.CoroutineScope) =
        PartyGuestModel(service, scope, id = "p1", trimDebounceMs = 0, errorText = ::testErrorText)

    /// A cold open: ONE request, and the card is the landing card.
    @Test
    fun `a cold card is one request`() = runTest {
        val service = FakePartyService()
        val guest = model(service, this)
        guest.load()
        advanceUntilIdle()
        assertEquals(1, service.peeks)
        assertEquals("the copy is read too, but it is empty", 1, service.cachedPeeks)
        assertEquals(PartyPhase.LANDING, guest.state.value.phase)
        assertFalse(guest.state.value.stale)
        assertEquals("Camille", guest.state.value.data?.host)
    }

    /// A warm open paints the device's copy at once and then refreshes it: the
    /// screen is never blank while the network answers.
    @Test
    fun `a warm open paints the copy before the answer`() = runTest {
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        val service = FakePartyService(
            cached = partyState(host = "Camille (mémorisé)"),
            onPeek = { gate.await() },
        )
        val guest = model(service, this)
        guest.load()
        // The network read is HELD, so what is on screen is the copy and nothing
        // else — the event the order is asserted on, not a delay.
        runCurrent()
        assertTrue("the copy is painted", guest.state.value.stale)
        assertEquals("Camille (mémorisé)", guest.state.value.data?.host)
        assertEquals(1, service.peeks)

        gate.complete(Unit)
        advanceUntilIdle()
        assertEquals(1, service.cachedPeeks)
        assertFalse("the answer replaces it", guest.state.value.stale)
        assertEquals("Camille", guest.state.value.data?.host)
    }

    /// A failed refresh with a copy KEEPS the copy and says it is one; with no
    /// copy it is a failure with its cause and a retry that asks again.
    @Test
    fun `a failure keeps the copy or names its cause`() = runTest {
        val offline = FakePartyService(peekError = NzException.Offline())
        val without = model(offline, this)
        without.load()
        advanceUntilIdle()
        assertEquals(PartyPhase.FAILED, without.state.value.phase)
        assertEquals("Hors ligne", without.state.value.failure)

        without.retry()
        advanceUntilIdle()
        assertEquals("the retry asks once more", 2, offline.peeks)

        val service = FakePartyService(cached = partyState(), peekError = NzException.Offline())
        val with = model(service, this)
        with.load()
        advanceUntilIdle()
        assertEquals(PartyPhase.LANDING, with.state.value.phase)
        assertTrue("the copy is shown, and it says so", with.state.value.stale)
        assertNull(with.state.value.failure)
    }

    /// A link that never was one is MISSING, not a failure: the page's own page.
    @Test
    fun `a dead link is a page, not a failure`() = runTest {
        val service = FakePartyService(card = null)
        val guest = model(service, this)
        guest.load()
        advanceUntilIdle()
        assertEquals(PartyPhase.MISSING, guest.state.value.phase)
        assertNull(guest.state.value.failure)
    }

    /// The tap: ONE seat, the name trimmed and bounded to 32 characters, and the
    /// live view straight away.
    @Test
    fun `joining takes one seat with a bounded name`() = runTest {
        val service = FakePartyService()
        val guest = model(service, this)
        guest.load()
        advanceUntilIdle()
        val typed = "   Camille de la pièce du fond, vraiment très longue   "
        guest.setName(typed)
        guest.join()
        advanceUntilIdle()
        assertEquals(1, service.joins)
        // The field keeps 32 characters as they are typed; the SEAT carries the
        // trimmed name (`Party.svelte`'s `name.trim().slice(0, 32)`).
        assertEquals(typed.take(32).trim(), service.lastJoinedName)
        assertEquals(PartyPhase.LIVE, guest.state.value.phase)
        assertFalse(guest.state.value.joining)
    }

    /// A join the server refuses comes back as a sentence, and the card stays so
    /// the person can try again (the SPA's toast, not a dead screen).
    @Test
    fun `a refused seat keeps the card and says why`() = runTest {
        val service = FakePartyService(
            joinError = NzException.Server(status = 429u, detail = "party full"),
        )
        val guest = model(service, this)
        guest.load()
        advanceUntilIdle()
        guest.join()
        advanceUntilIdle()
        assertEquals(PartyPhase.LANDING, guest.state.value.phase)
        assertFalse(guest.state.value.joining)
        assertNotNull(guest.state.value.message)
    }

    /// Leaving is a request and a page; « Rejoindre à nouveau » reads the card
    /// again instead of re-joining blindly.
    @Test
    fun `leaving then rejoining reads the card again`() = runTest {
        val service = FakePartyService()
        val guest = model(service, this)
        guest.load()
        advanceUntilIdle()
        guest.join()
        advanceUntilIdle()
        guest.leave()
        advanceUntilIdle()
        assertEquals(1, service.leaves)
        assertEquals(PartyPhase.LEFT, guest.state.value.phase)

        guest.rejoin()
        advanceUntilIdle()
        assertEquals("the card is read again", 2, service.peeks)
        assertEquals(PartyPhase.LANDING, guest.state.value.phase)
    }

    /// The volume and the *Décalage* are the LISTENER's: the volume is applied on
    /// the spot (a fader must answer at once), the trim once per run of taps.
    @Test
    fun `the volume answers at once and the trim once per run`() = runTest {
        val service = FakePartyService()
        val guest = model(service, this)
        guest.load()
        advanceUntilIdle()
        guest.setVolume(0.42)
        assertEquals(0.42, service.prefsNow.volume, 0.001)
        assertEquals(0.42, guest.state.value.volume, 0.001)

        guest.nudge(150)
        advanceUntilIdle()
        assertEquals(150.0, service.prefsNow.trimMs, 0.001)
        assertEquals(150, guest.state.value.trimMs)
        guest.nudge(-1000)
        advanceUntilIdle()
        assertEquals("clamped to what a device can be told", -500.0, service.prefsNow.trimMs, 0.001)
    }

    /// A push from Rust moves the room: the track, the status line, and the path
    /// a party takes when its host ends it.
    @Test
    fun `a push from rust repaints the live view`() = runTest {
        val service = FakePartyService()
        val guest = model(service, this)
        guest.load()
        advanceUntilIdle()
        guest.join()
        advanceUntilIdle()

        service.push(
            partyState(
                track = partyTrack(title = "Suivant", id = "3000002"),
                status = "En pause chez Camille",
                tone = "wait",
                playing = false,
            ),
        )
        assertEquals("Suivant", guest.state.value.data?.track?.title)
        assertEquals("En pause chez Camille", guest.state.value.data?.status)

        service.push(partyState(phase = PartyPhase.ENDED))
        assertEquals(PartyPhase.ENDED, guest.state.value.phase)
    }

    /// The playhead is the CORE's figure, read hot: the model asks Rust and adds
    /// nothing of its own (a wall-clock estimate here would drift against the
    /// room).
    @Test
    fun `the playhead is the cores own number`() = runTest {
        val service = FakePartyService()
        service.position = 41.25
        val guest = model(service, this)
        assertEquals(41.25, guest.positionNow(), 0.0)
    }
}
