package fr.neezer.mobile.feature.party

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.PartyPhase

/**
 * The host's sheet, as plain Kotlin (lot MB20). Every assertion is one rule of
 * `PartySheet.svelte`: the request a tap sends, the arm-before-end, the link
 * handed out, the failure WITH its cause, and the door hidden on an older server.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class PartyModelTest {
    private class FakeShare(var canCopy: Boolean = true, var canShare: Boolean = true) : PartyShare {
        var copied: String? = null
        var shared: String? = null

        override fun copy(text: String): Boolean {
            copied = text
            return canCopy
        }

        override fun share(title: String, text: String, url: String): Boolean {
            shared = url
            return canShare
        }
    }

    @Test
    fun `the door finds out once whether the server has parties`() = runTest {
        val service = FakePartyService(supported = true)
        val model = PartyModel(service, this, FakeShare(), errorText = ::testErrorText)
        model.probe()
        advanceUntilIdle()
        assertEquals("one OPTIONS, not one per frame", 1, service.supportProbes)
        assertFalse(model.state.value.unsupported)
    }

    /// Rule 7: only a server that really said "no" hides the door, and a doubt
    /// (an offline probe) must never take the feature away.
    @Test
    fun `an older server hides the door and a doubt does not`() = runTest {
        val older = FakePartyService(supported = false)
        val hidden = PartyModel(older, this, FakeShare(), errorText = ::testErrorText)
        hidden.probe()
        advanceUntilIdle()
        assertTrue(hidden.state.value.unsupported)

        val doubtful = FakePartyService(supported = null)
        val shown = PartyModel(doubtful, this, FakeShare(), errorText = ::testErrorText)
        shown.probe()
        advanceUntilIdle()
        assertFalse(shown.state.value.unsupported)
    }

    /// A tap on « Démarrer » is ONE request, and the sheet shows the link it
    /// answered — the SPA's `startParty()`.
    @Test
    fun `starting costs one request and shows the link`() = runTest {
        val service = FakePartyService()
        val model = PartyModel(service, this, FakeShare(), errorText = ::testErrorText)
        model.open()
        advanceUntilIdle()
        model.start()
        advanceUntilIdle()
        assertEquals(1, service.hosts)
        assertTrue(model.state.value.hosting)
        assertEquals("https://neezer.lan/party/p1", model.state.value.data?.link)
        assertFalse(model.state.value.busy)
    }

    /// A start that fails says WHY, in the person's words (rule 3) — never a
    /// spinner, never a code.
    @Test
    fun `a failed start is a sentence with its cause`() = runTest {
        val service = FakePartyService(hostError = NzException.Offline())
        val model = PartyModel(service, this, FakeShare(), errorText = ::testErrorText)
        model.start()
        advanceUntilIdle()
        assertFalse(model.state.value.busy)
        val message = model.state.value.message.orEmpty()
        assertTrue("the sentence opens with the action: $message", message.startsWith(PartyCopy.START_FAILED))
        assertTrue("the cause, not a code: $message", message.contains("Hors ligne"))
    }

    /// « Terminer » arms first: a phone in a pocket must not end a party. The
    /// second press sends the ONE delete.
    @Test
    fun `ending asks once and then ends`() = runTest {
        val service = FakePartyService()
        val model = PartyModel(service, this, FakeShare(), errorText = ::testErrorText)
        model.open()
        advanceUntilIdle()
        model.start()
        advanceUntilIdle()

        model.end()
        assertTrue("the first press only arms", model.state.value.confirmEnd)
        assertEquals("nothing was sent yet", 0, service.leaves)

        model.end()
        advanceUntilIdle()
        assertEquals(1, service.leaves)
        assertFalse(model.state.value.hosting)
        assertFalse(model.state.value.open)
        assertFalse(model.state.value.confirmEnd)
        assertEquals(PartyCopy.ENDED, model.state.value.message)
    }

    /// Handing the link out: the share sheet carries the link, and a platform
    /// without one falls back to the copy (the SPA's `share`).
    @Test
    fun `the share sheet falls back to the copy`() = runTest {
        val service = FakePartyService()
        val share = FakeShare(canShare = false)
        val model = PartyModel(service, this, share, errorText = ::testErrorText)
        model.open()
        advanceUntilIdle()
        model.start()
        advanceUntilIdle()

        model.shareLink()
        assertEquals("https://neezer.lan/party/p1", share.shared)
        assertEquals("the fallback copied it", "https://neezer.lan/party/p1", share.copied)
        assertEquals(PartyCopy.COPIED, model.state.value.message)

        share.canCopy = false
        model.copyLink()
        assertEquals(PartyCopy.COPY_FAILED, model.state.value.message)
    }

    /// The *Décalage* chip: a run of taps is ONE write, and it lands after the
    /// last one (`Party.svelte#nudge`). Measured as a count, not a duration.
    @Test
    fun `a run of trim taps is one write`() = runTest {
        val service = FakePartyService()
        val model = PartyModel(service, this, FakeShare(), trimDebounceMs = 0, errorText = ::testErrorText)
        model.open()
        advanceUntilIdle()
        model.start()
        advanceUntilIdle()

        repeat(3) { model.nudge(50) }
        advanceUntilIdle()
        assertEquals("150 ms of draft", 150.0, service.prefsNow.trimMs, 0.001)
        assertEquals(150, model.trimShown())
    }

    /// The chip never leaves what a device can be told: the core clamps to
    /// −500…+1000 ms, and so does the draft.
    @Test
    fun `the trim stays inside what a device can be told`() = runTest {
        val service = FakePartyService()
        val model = PartyModel(service, this, FakeShare(), trimDebounceMs = 0, errorText = ::testErrorText)
        model.open()
        advanceUntilIdle()
        repeat(40) { model.nudge(100) }
        advanceUntilIdle()
        assertEquals(1000.0, service.prefsNow.trimMs, 0.001)
        repeat(40) { model.nudge(-100) }
        advanceUntilIdle()
        assertEquals(-500.0, service.prefsNow.trimMs, 0.001)
    }

    /// A push from Rust paints at once, and the party it names is the one the
    /// sheet shows — including a party started from another screen.
    @Test
    fun `a push from rust paints the sheet`() = runTest {
        val service = FakePartyService()
        val model = PartyModel(service, this, FakeShare(), errorText = ::testErrorText)
        model.open()
        advanceUntilIdle()
        service.push(hostState(link = "https://neezer.lan/party/zz", audience = "2 personnes à l'écoute"))
        assertTrue(model.state.value.hosting)
        assertEquals("https://neezer.lan/party/zz", model.state.value.data?.link)
        assertEquals("2 personnes à l'écoute", model.state.value.data?.audience)
    }

    /// A resume at launch re-opens the party this device was hosting, and costs
    /// nothing for somebody who never hosted (the request is only made when a
    /// party exists or a recent one is remembered — `Host::should_ask`).
    @Test
    fun `opening the sheet re-opens a party this device was hosting`() = runTest {
        val never = FakePartyService(resumeLink = null, hostLink = "")
        val quiet = PartyModel(never, this, FakeShare(), errorText = ::testErrorText)
        quiet.open()
        advanceUntilIdle()
        assertEquals(1, never.resumes)
        assertFalse(quiet.state.value.hosting)

        val again = FakePartyService(resumeLink = "https://neezer.lan/party/p1")
        again.stateNow = hostState()
        val resumed = PartyModel(again, this, FakeShare(), errorText = ::testErrorText)
        resumed.open()
        advanceUntilIdle()
        assertTrue(resumed.state.value.hosting)
    }

    /// Closing the sheet ends nothing: the party is Rust's and the server's.
    @Test
    fun `closing the sheet does not end the party`() = runTest {
        val service = FakePartyService()
        val model = PartyModel(service, this, FakeShare(), errorText = ::testErrorText)
        model.open()
        advanceUntilIdle()
        model.start()
        advanceUntilIdle()
        model.close()
        assertEquals(0, service.leaves)
        assertFalse(model.state.value.open)
        assertNull(model.state.value.message)
    }

    /// The movie the guest sees as `PartyPhase`: an ended party is not "hosting"
    /// any more, so the sheet goes back to its three arguments.
    @Test
    fun `an ended party stops being a hosted one`() = runTest {
        val service = FakePartyService()
        val model = PartyModel(service, this, FakeShare(), errorText = ::testErrorText)
        model.open()
        advanceUntilIdle()
        model.start()
        advanceUntilIdle()
        service.push(partyState(phase = PartyPhase.ENDED))
        assertFalse(model.state.value.hosting)
    }
}
