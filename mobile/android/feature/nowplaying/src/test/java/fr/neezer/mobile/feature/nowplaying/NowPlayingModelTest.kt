package fr.neezer.mobile.feature.nowplaying

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.Quality
import uniffi.nz_ffi.Repeat
import uniffi.nz_ffi.SleepTimer

/**
 * The full-screen player's rules, proved on the JVM: no JNA, no `.so`, no device.
 *
 * Every assertion is a COUNT or a COMMAND (`output_stats`-free): "one tap sends
 * one command", "a drag that lands where it started sends nothing", "the rail
 * reads the position 0 times when nothing draws it". Times never appear — the
 * plan's rule is counts, not clocks.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class NowPlayingModelTest {

    private fun model(
        player: FakePlayerPort = FakePlayerPort(),
        service: FakeService = FakeService(),
        scope: TestScope,
        toasts: MutableList<String> = mutableListOf(),
    ): Pair<NowPlayingModel, MutableList<String>> {
        val model = NowPlayingModel(
            player = player,
            service = service,
            scope = scope,
            io = StandardTestDispatcher(scope.testScheduler),
            toast = { toasts += it },
        )
        return model to toasts
    }

    /** A tap answers in the same frame: the glyph flips BEFORE the command. */
    @Test
    fun a_tap_flips_the_glyph_and_sends_one_command() = runTest {
        val player = FakePlayerPort(snapshot = FakePlayerPort.snapshot(state = PlayState.PAUSED))
        val (model, _) = model(player, scope = this)
        model.refresh()
        model.toggle()
        assertEquals(PlayState.PLAYING, model.state.value.snapshot?.state)
        assertEquals(listOf(PlayerCommand.Play), player.commands)

        model.toggle()
        assertEquals(PlayState.PAUSED, model.state.value.snapshot?.state)
        assertEquals(listOf(PlayerCommand.Play, PlayerCommand.Pause), player.commands)
    }

    /** The repeat cycle is the SPA's: off → all → one → off, one command each. */
    @Test
    fun the_repeat_cycle_is_the_spas() = runTest {
        val player = FakePlayerPort()
        val (model, _) = model(player, scope = this)
        model.refresh()
        assertEquals(Repeat.OFF, model.state.value.repeat)
        model.cycleRepeat()
        assertEquals(Repeat.ALL, model.state.value.repeat)
        model.cycleRepeat()
        assertEquals(Repeat.ONE, model.state.value.repeat)
        model.cycleRepeat()
        assertEquals(Repeat.OFF, model.state.value.repeat)
        assertEquals(3, player.commands.count { it == PlayerCommand.CycleRepeat })
    }

    /** Shuffle moves NOW, and sends the flag it drew. */
    @Test
    fun shuffle_is_optimistic() = runTest {
        val player = FakePlayerPort()
        val (model, _) = model(player, scope = this)
        model.refresh()
        model.setShuffle(true)
        assertTrue(model.state.value.shuffle)
        assertEquals(listOf(PlayerCommand.SetShuffle(true)), player.commands)
    }

    /** A seek moves the rail at once and sends the seconds it shows. */
    @Test
    fun a_seek_moves_the_rail_then_the_engine() = runTest {
        val player = FakePlayerPort()
        val (model, _) = model(player, scope = this)
        model.refresh()
        model.seek(-4.0)
        assertEquals(0.0, model.state.value.position.positionS, 0.0)
        model.seek(42.5)
        assertEquals(42.5, model.state.value.position.positionS, 0.0)
        assertEquals(PlayerCommand.Seek(0.0), player.commands[0])
        assertEquals(PlayerCommand.Seek(42.5), player.commands[1])
    }

    /**
     * **Only what is AFTER the playhead can move or go.** The gesture's own rule
     * (`QueueList.svelte`): a drop on the playing row, or on a played one, sends
     * NOTHING — and a drag that lands where it started is not an edit either.
     */
    @Test
    fun the_queue_refuses_what_is_not_upcoming() = runTest {
        val player = FakePlayerPort()
        val (model, _) = model(player, scope = this)
        model.refresh() // index 1 of 4

        model.move(1, 3)
        model.move(0, 2)
        model.move(2, 2)
        assertTrue("nothing was sent: ${player.commands}", player.commands.isEmpty())

        model.move(2, 3)
        model.remove(0, "9001")
        model.remove(1, "9001")
        assertEquals(listOf(PlayerCommand.Move(2u, 3u)), player.commands)
    }

    /** « Vider la file » says what it did (rule 2: a silent button is a bug). */
    @Test
    fun clearing_the_queue_says_so() = runTest {
        val player = FakePlayerPort()
        val (model, toasts) = model(player, scope = this)
        model.refresh()
        model.clearUpcoming()
        assertEquals(listOf(PlayerCommand.ClearUpcoming), player.commands)
        assertEquals(listOf("File vidée"), toasts)

        // Nothing left to clear: no command, no toast.
        player.snapshot = FakePlayerPort.snapshot(index = 3u, len = 4u)
        model.refresh()
        model.clearUpcoming()
        assertEquals(1, player.commands.size)
        assertEquals(1, toasts.size)
    }

    /**
     * **The quality applies at the NEXT track, and says so** (desktop D14a's
     * rule): the setting is saved once, nothing is re-opened, and the toast names
     * the wait. Choosing the rung that is already set is not an edit.
     */
    @Test
    fun the_quality_applies_at_the_next_track() = runTest {
        val player = FakePlayerPort()
        val (model, toasts) = model(player, scope = this)
        model.refresh()
        model.setQuality(Quality.FLAC)
        assertEquals(Quality.FLAC, player.currentAudio.quality)
        assertEquals(1, player.settings.size)
        assertEquals(listOf("Qualité : FLAC · au prochain titre"), toasts)
        assertTrue(player.commands.isEmpty())

        model.setQuality(Quality.FLAC)
        assertEquals(1, player.settings.size)
    }

    /** The sleep timer is the PLAYER's deadline; the menu only sends it. */
    @Test
    fun the_sleep_timer_is_the_players() = runTest {
        val player = FakePlayerPort()
        val (model, toasts) = model(player, scope = this)
        model.refresh()
        model.setSleep(SleepTimer.Minutes(30u))
        model.setSleep(SleepTimer.EndOfTrack)
        model.setSleep(null)
        assertEquals(
            listOf(SleepTimer.Minutes(30u), SleepTimer.EndOfTrack, null),
            player.sleeps,
        )
        assertEquals("Le lecteur s'arrêtera dans 30 minutes", toasts[0])
        assertEquals("Le lecteur s'arrêtera à la fin de ce titre", toasts[1])
        assertEquals("Minuteur désactivé", toasts[2])
    }

    /**
     * A podcast's rate is remembered for its SHOW and applied through the player;
     * music never keeps one (the snapshot's episode flag is the rule).
     */
    @Test
    fun a_podcast_rate_belongs_to_its_show() = runTest {
        val player = FakePlayerPort(
            snapshot = FakePlayerPort.snapshot(
                current = FakePlayerPort.track(key = "ep1", episode = true, albumId = "show-7"),
            ),
        )
        val service = FakeService()
        val (model, _) = model(player, service = service, scope = this)
        model.refresh()
        assertEquals(1f, model.speedRate())

        model.chooseSpeed(1.5f)
        assertEquals(1.5f, service.speed("show-7"))
        assertEquals(1.5f, player.currentAudio.speed)
        assertEquals(1.5f, model.speedRate())
    }

    /** Music has no rate chip and cannot be sped up. */
    @Test
    fun music_never_keeps_a_rate() = runTest {
        val player = FakePlayerPort()
        val service = FakeService()
        val (model, _) = model(player, service = service, scope = this)
        model.refresh()
        assertNull(model.speedRate())
        model.chooseSpeed(2f)
        assertEquals(1f, player.currentAudio.speed)
        assertTrue(service.speeds.isEmpty())
    }

    /**
     * **The acceptance's "0 FFI calls with the screen off"**: the rail's read is
     * counted, and a frame that draws nothing touches no binding.
     */
    @Test
    fun a_frame_that_draws_nothing_reads_no_position() = runTest {
        val player = FakePlayerPort()
        val (model, _) = model(player, scope = this)
        model.refresh()

        repeat(10) { model.framePosition(drawn = false) }
        assertEquals(0, player.positionReads)
        assertEquals(0, model.ticker.reads)

        model.framePosition(drawn = true)
        assertEquals(1, player.positionReads)
        assertEquals(1, model.ticker.reads)
    }

    /** The words are asked for when the AUDIO is under way, never before it. */
    @Test
    fun the_words_wait_for_the_audio() = runTest {
        val player = FakePlayerPort(snapshot = FakePlayerPort.snapshot(state = PlayState.BUFFERING))
        val (model, _) = model(player, scope = this)
        model.refresh()
        assertNull(model.lyricsKeyWhenPlayable())

        player.snapshot = FakePlayerPort.snapshot(state = PlayState.PLAYING)
        model.onSnapshot(player.snapshot)
        assertEquals("9001", model.lyricsKeyWhenPlayable())
    }

    /** A queue change asks the player for a FRESH handle, and bumps the version. */
    @Test
    fun a_queue_change_rewindows_the_list() = runTest {
        val player = FakePlayerPort()
        val (model, _) = model(player, scope = this)
        model.refresh()
        assertEquals(0L, model.state.value.queueVersion)
        model.onQueueChanged(7L)
        assertEquals(7L, model.state.value.queueVersion)
    }

    /** A jump names the row it plays (the key is the queue's own identity). */
    @Test
    fun a_jump_names_its_row() = runTest {
        val player = FakePlayerPort()
        val (model, _) = model(player, scope = this)
        model.refresh()
        model.jump(3, "9004")
        model.jump(-1, "x")
        assertEquals(listOf(PlayerCommand.Jump(3u, "9004")), player.commands)
    }

    /** The counters the acceptance quotes come from the ports, not from a clock. */
    @Test
    fun the_state_draws_the_queue_counts() = runTest {
        val player = FakePlayerPort(snapshot = FakePlayerPort.snapshot(index = 1u, len = 4000u))
        val (model, _) = model(player, scope = this)
        model.refresh()
        assertEquals(3998, model.state.value.upcoming)
        assertEquals(215.0, model.state.value.durationS, 0.0)
    }
}
