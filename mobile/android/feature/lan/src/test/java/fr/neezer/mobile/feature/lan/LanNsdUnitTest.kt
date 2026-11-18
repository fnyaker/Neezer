package fr.neezer.mobile.feature.lan

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import androidx.test.core.app.ApplicationProvider
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowNsdManager

/**
 * `LanNsd`'s NSD bookkeeping under the platform's real callback shape, on the JVM.
 *
 * Why this test exists: on the emulator (`LanNsdTest`, PR #365) the second-node
 * run died with a `java.util.ConcurrentModificationException` on a
 * `LinkedHashMap`. `NsdManager` calls its listeners from threads the app does not
 * own, and from API 34 `unregisterServiceInfoCallback` invokes
 * `onServiceInfoCallbackUnregistered()` **before it returns** — on the caller's
 * thread on this platform. `stop()` iterated `callbacks` with `forEach` while that
 * callback removed its own entry, so the map changed under its own iterator.
 *
 * Robolectric's `ShadowNsdManager` reproduces exactly that synchronous callback,
 * so both tests below are honest: [stopping_unregisters_every_live_callback…]
 * fails on the old `forEach` (its iterator is invalidated mid-loop), and
 * [a_callback_withdrawing_itself_while_stop_runs…] hammers a live callback from a
 * second thread while `stop()` snapshots. Nothing here needs a device or a sleep:
 * each step is an event, and the loops are bounded counts.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class LanNsdUnitTest {
    private val context: Context
        get() = ApplicationProvider.getApplicationContext()

    private val manager: NsdManager
        get() = context.getSystemService(Context.NSD_SERVICE) as NsdManager

    private val shadow: ShadowNsdManager
        get() = shadowOf(manager)

    private fun nsd(): LanNsd = LanNsd(context, onFound = { _, _ -> }, onLost = { _, _ -> })

    /** The browse the platform is driving (`ShadowNsdManager` annotates it nullable). */
    private fun browsing(): List<NsdManager.DiscoveryListener> =
        shadow.getDiscoveryListeners(LanNsd.SERVICE_TYPE).orEmpty()

    /** The live `ServiceInfoCallback`s the platform holds for `info`. */
    private fun liveCallbacks(info: NsdServiceInfo): List<NsdManager.ServiceInfoCallback> =
        shadow.getServiceInfoCallbacks(info).orEmpty()

    /** A discovered record, exactly as the platform hands one to `onServiceFound`. */
    private fun peer(name: String, port: Int): NsdServiceInfo = NsdServiceInfo().apply {
        serviceName = name
        serviceType = LanNsd.SERVICE_TYPE
        setPort(port)
        setAttribute("v", "1")
    }

    /** Starts the node and returns the browse the platform is now driving. */
    private fun startedNsd(): Pair<LanNsd, NsdManager.DiscoveryListener> {
        val nsd = nsd()
        nsd.start(PORT)
        assertTrue("the node announces and browses", nsd.isRunning)
        return nsd to browsing().single()
    }

    @Test
    fun stopping_unregisters_every_live_callback_without_reopening_the_map() {
        val (nsd, browse) = startedNsd()
        val peers = (0 until 3).map { index -> peer("nz-live$index", PORT + 1 + index) }
        // Three live callbacks: three entries in `callbacks`, each of which removes
        // itself when the platform withdraws it inside `stop()`.
        peers.forEach { browse.onServiceFound(it) }
        assertEquals(3, peers.sumOf { liveCallbacks(it).size })

        // On the old code this threw the emulator's ConcurrentModificationException.
        nsd.stop()

        assertFalse("the node is stopped", nsd.isRunning)
        assertTrue(
            "every callback was withdrawn from the platform",
            peers.all { liveCallbacks(it).isEmpty() },
        )
    }

    @Test
    fun a_callback_withdrawing_itself_while_stop_runs_never_breaks_the_map() {
        val (nsd, browse) = startedNsd()
        val failure = AtomicReference<Throwable>()
        val handoff = ArrayBlockingQueue<NsdManager.ServiceInfoCallback>(1)
        val running = AtomicBoolean(true)

        // The platform's thread: withdraws the callback `stop()` is about to
        // unregister, and reports a lost service, in a burst per handover.
        val platform = thread(name = "lan-nsd-platform") {
            while (running.get()) {
                val callback = handoff.poll(50, TimeUnit.MILLISECONDS) ?: continue
                try {
                    repeat(BURST) {
                        callback.onServiceInfoCallbackUnregistered()
                        callback.onServiceLost()
                    }
                } catch (t: Throwable) {
                    failure.compareAndSet(null, t)
                    return@thread
                }
            }
        }

        try {
            repeat(ROUNDS) { round ->
                val info = peer("nz-race$round", PORT + 1 + round)
                browse.onServiceFound(info)
                val handed = handoff.offer(
                    liveCallbacks(info).single(),
                    2,
                    TimeUnit.SECONDS,
                )
                assertTrue("round $round: the platform thread stopped taking callbacks", handed)
                // Runs while the platform thread is withdrawing that same callback.
                nsd.stop()
                assertFalse("round $round left the node running", nsd.isRunning)
            }
        } finally {
            running.set(false)
            platform.join(5_000)
        }

        assertNull(
            "a concurrent NSD callback must never break stop() or the tables: ${failure.get()}",
            failure.get(),
        )
    }

    @Test
    fun start_is_idempotent_and_stop_is_safe_twice() {
        val (nsd, _) = startedNsd()
        nsd.start(PORT + 1)
        nsd.stop()
        nsd.stop()
        assertFalse("stopping twice is not an error", nsd.isRunning)
    }

    companion object {
        /** This lot's own port, the same one `LanNsdTest` advertises. */
        private const val PORT: Int = 46050

        /** How many register/stop handovers the race test runs. */
        private const val ROUNDS: Int = 200

        /** Withdrawals the platform thread fires per handover. */
        private const val BURST: Int = 25
    }
}
