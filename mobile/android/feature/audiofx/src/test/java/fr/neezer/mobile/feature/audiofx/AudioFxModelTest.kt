package fr.neezer.mobile.feature.audiofx

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The effects card's rules, on the JVM over [FakeAudioFx]: every control writes
 * once and draws what the store answered, a refused write puts the old value back
 * and says why, the meter runs only while the card is open, and the warning's
 * button moves the player's volume — and can undo it.
 *
 * The numbers in the assertions are the ones the SPA's own rules produce
 * (`webapp/test/eqpresets.test.mjs`, `test/eq/run.mjs`).
 */
class AudioFxModelTest {
    @Test
    fun theCardReadsItsStateOnceAndShowsIt() {
        val backend = FakeAudioFx()
        val model = AudioFxModel(backend)
        assertNull("nothing is drawn before the first read", model.state)
        model.start()
        val state = model.state!!
        assertEquals(listOf(6f, 5f, 4f, 2f, 0f, 0f, 0f, 0f, 0f, 0f), state.bands)
        assertEquals(0.35f, state.bass, 1e-6f)
        assertEquals("the chain matches « Basses+ »", "bass", model.selectedId())
        assertTrue("opening the card writes nothing", backend.writes.isEmpty())
    }

    @Test
    fun everyControlWritesOnceAndTheStoreAnswers() {
        val backend = FakeAudioFx()
        val model = AudioFxModel(backend)
        model.start()

        model.setEqEnabled(false)
        assertFalse(model.state!!.eqEnabled)
        assertEquals("eq=false", backend.writes.last())
        assertEquals("change=eq", backend.writes.dropLast(1).last(), "a switch starts a new episode")

        model.setBand(4, 9f)
        assertEquals(9f, model.state!!.bands[4])
        assertEquals("band:4=9.0", backend.writes.last())
        assertEquals("change=band:4", backend.writes.dropLast(1).last())

        model.setBass(1.7f)
        assertEquals("the bass is a 0..1 lift", 1f, model.state!!.bass, 1e-6f)

        model.setBalance(2f)
        assertEquals(1f, model.state!!.balance)

        model.setMono(true)
        assertTrue(model.state!!.mono)

        model.setNormalization("high")
        assertEquals("high", model.state!!.normalization)

        model.setCrossfade(true, 4f)
        assertEquals(4f, model.state!!.crossfadeS)
        model.setSkipFade(false, 1.5f)
        assertFalse(model.state!!.crossfadeOnSkip)

        model.setTrim(false)
        assertFalse(model.state!!.trim)

        model.setOutput("direct")
        assertEquals("direct", model.state!!.output)
    }

    @Test
    fun aRefusedWritePutsTheOldValueBackAndKeepsTheCause() {
        val backend = FakeAudioFx()
        val model = AudioFxModel(backend)
        model.start()
        val before = model.state!!
        backend.failure = IllegalStateException("Le moteur audio n'a pas démarré")
        model.setEqEnabled(false)
        assertEquals("the value goes back to what it was", before, model.state)
        assertNotNull(model.failure)
        assertEquals("Le moteur audio n'a pas démarré", model.failure)
        assertFalse(model.busy)
    }

    @Test
    fun aPresetIsAppliedSavedUpdatedAndDeleted() {
        val backend = FakeAudioFx()
        val model = AudioFxModel(backend)
        model.start()

        val car = model.state!!.presets.first { it.id == "p-car" }
        model.usePreset(car)
        assertEquals("p-car", model.selected)
        assertEquals("p-car", model.state!!.activePreset)

        model.openNaming()
        assertEquals("", model.draftName)
        model.editName("Casque nommé bien trop longtemps pour quarante caractères")
        assertEquals("40 characters at most", 40, model.draftName!!.length)
        model.confirmSave()
        assertNull("the field closes on success", model.draftName)
        assertEquals("p-new", model.selected)
        assertTrue(model.state!!.presets.any { it.name.startsWith("Casque") })

        model.updateSelected()
        assertTrue(backend.writes.contains("update=p-new"))

        model.deleteSelected()
        assertNull(model.selected)
        assertTrue(model.state!!.presets.none { it.id == "p-new" })

        // An empty name saves nothing at all.
        model.openNaming()
        model.confirmSave()
        assertNotNull("the field stays open", model.draftName)
    }

    @Test
    fun theMeterRunsOnlyWhileTheCardIsOpen() {
        val backend = FakeAudioFx()
        val model = AudioFxModel(backend)
        model.start()
        assertFalse(backend.meterSubscribed)

        model.openMeter()
        assertTrue(backend.meterSubscribed)
        assertNull("a new visit waits for its own first reading", model.meter)
        backend.pushFrame(FxFixtures.measured())
        assertNotNull(model.meter)
        assertEquals(7.3, model.meter!!.peakDb, 1e-9)
        assertEquals("clip", model.meter!!.warning!!.tone)

        model.closeMeter()
        assertTrue(backend.meterStopped)
        assertFalse(backend.meterSubscribed)
        assertNull("a closed card keeps no reading", model.meter)
        // Opening twice does not double the subscription.
        model.openMeter()
        val subscribed = backend.meterSubscribed
        model.openMeter()
        assertTrue(subscribed && backend.meterSubscribed)
    }

    @Test
    fun theWarningButtonLowersTheVolumeAndCanUndoIt() {
        val backend = FakeAudioFx(volume = 0.8f)
        val model = AudioFxModel(backend)
        model.start()
        model.openMeter()
        backend.pushFrame(FxFixtures.measured())

        // 6,5 dB of cut is ×0.4732: 0.8 → 0.3786.
        model.reduceVolume(6.5)
        assertEquals(0.3786f, backend.volume, 1e-3f)
        assertEquals("change=volume", backend.writes.last(), "a new episode starts")
        // The toast carries the undo; pressing it restores the level exactly.
        val toast = fr.neezer.mobile.designsystem.NzToasts.toasts.last()
        assertEquals("Volume réduit de 6,5 dB", toast.text)
        toast.onAction?.invoke()
        assertEquals(0.8f, backend.volume, 1e-6f)
    }

    @Test
    fun theReadoutsAreFrenchAndNeverSayMinusZero() {
        assertEquals("\u221212,0 dB", FxFormat.db(-12.04))
        assertEquals("0,0 dB", FxFormat.db(0.0, plus = true))
        assertEquals("+6,0 dB", FxFormat.db(6.0, plus = true))
        assertEquals("0,0 dB", FxFormat.db(-0.02))
        assertEquals("+6", FxFormat.gain(6f))
        assertEquals("\u22123", FxFormat.gain(-3f))
        assertEquals("0", FxFormat.gain(0f))
        assertEquals("6,0 s", FxFormat.fade(6f))
        assertEquals("enchaîné direct", FxFormat.fade(0f))
        assertEquals("1,5 s", FxFormat.skipFade(1.5f))
        assertEquals("coupe douce", FxFormat.skipFade(0f))
        assertEquals("16k", FxFormat.frequency(16_000))
        assertEquals("62", FxFormat.frequency(62))
        assertEquals("35 %", FxFormat.percent(0.35f))
    }
}
