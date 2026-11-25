package fr.neezer.mobile.feature.audiofx

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * **The normalization control reaches the player.** The card's rule, on the JVM
 * over [FakeAudioFx]: picking « Élevée » is ONE call to the backend carrying
 * `"high"`, and the card redraws the segment that was picked.
 *
 * This is the link the older tests could not see: they checked that the drawn
 * string came back (`AudioFxModelTest.theCardReadsItsStateOnceAndShowsIt`), never
 * that the tap reached `PlayerApi.fxSetNormalization`. The sample half of the
 * proof is `nz-core`'s `the_normalisation_gain_lowers_the_samples_by_the_expected_db`
 * and `nz-ffi`'s `fx_set_normalization_reaches_the_live_player`; this file holds
 * the screen's end of the same contract (`docs/agents/mobile.md`, T6).
 */
class NormalizationEffectTest {
    @Test
    fun pickingAHighLevelCallsThePlayerOnceAndRedrawsThatSegment() {
        val backend = FakeAudioFx()
        val model = AudioFxModel(backend)
        model.start()
        // The card opens on the store's own value: no write, no call.
        assertEquals("medium", model.state!!.normalization)
        assertEquals(emptyList<String>(), backend.writes)

        model.setNormalization("high")

        // The CALL, not the rendered text: the level the person picked, once,
        // then the episode the warning's hold restarts on.
        assertEquals(listOf("norm=high", "change=norm"), backend.writes)
        // The store answered and the card draws it: « Élevée » is the fourth
        // segment of `FxCopy.NORM_LEVELS`, and the selected index is what the
        // choice row paints.
        val state = model.state!!
        assertEquals("high", state.normalization)
        assertEquals(
            3,
            FxCopy.NORM_LEVELS.indexOfFirst { it.second == state.normalization },
        )
        assertEquals("Élevée", FxCopy.NORM_LEVELS[3].first)
        // A refused write (the core is gone) puts the old level back and says
        // why: the card never claims a setting the engine did not take.
        backend.failure = IllegalStateException("Le moteur audio n'a pas démarré")
        model.setNormalization("low")
        assertEquals("high", model.state!!.normalization)
        assertEquals("Le moteur audio n'a pas démarré", model.failure)
    }

    @Test
    fun everyLevelOfTheCardIsTheChainsOwnWord() {
        // The four segments are the engine's four levels, in its own spelling:
        // a typo here would be an "unknown value" and silently mean off.
        assertEquals(
            listOf("off", "low", "medium", "high"),
            FxCopy.NORM_LEVELS.map { it.second },
        )
        val backend = FakeAudioFx()
        val model = AudioFxModel(backend)
        model.start()
        for ((index, level) in FxCopy.NORM_LEVELS.withIndex()) {
            model.setNormalization(level.second)
            assertEquals(level.second, model.state!!.normalization)
            assertEquals(index, FxCopy.NORM_LEVELS.indexOfFirst { it.second == model.state!!.normalization })
            assertNull(model.failure)
        }
    }
}
