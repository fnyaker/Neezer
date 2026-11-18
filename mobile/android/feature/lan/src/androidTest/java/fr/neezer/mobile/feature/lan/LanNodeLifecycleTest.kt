package fr.neezer.mobile.feature.lan

import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.net.DatagramSocket
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.CoreConfig
import uniffi.nz_ffi.LanSettingsFfi
import uniffi.nz_ffi.LanStateFfi
import uniffi.nz_ffi.NzCore

/**
 * The node's lifecycle on the emulator (lot LN14, and LN13's carry-over: `NzCore::shutdown`
 * does NOT join the LAN service, so the app must stop it first).
 *
 * Two facts, each an assertion rather than a claim:
 *
 * 1. **Cold start with the feature off adds nothing.** `LanInitProvider` only records the
 *    application and registers observers: no `nz-lan` thread, no socket on `45910` — a process that
 *    never logs in is exactly the "feature disabled" server, which `LanApi::start` answers
 *    `Unsupported` for without creating a directory, a socket or a thread (LN13's own test).
 * 2. **A real node is stopped and leaves nothing.** With a bench server, the phone enrols, binds
 *    its UDP port, and `LanApi::stop()` joins the `nz-lan` runtime and frees the port. This is the
 *    call the app's shutdown path must make before `NzCore::shutdown`.
 *
 * The second half needs `mobile/tools/perf-server.sh start --port 5610` (the bench library, whose
 * account is `bench` / `Bench1`); without it the test says so and skips, which is why the PR
 * carries its numbers from a run that had the server up.
 */
@RunWith(AndroidJUnit4::class)
class LanNodeLifecycleTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private var core: NzCore? = null

    @After
    fun stop() {
        runCatching { core?.lan()?.stop() }
        runCatching { core?.shutdown() }
        core = null
    }

    @Test
    fun a_process_that_never_logged_in_has_no_lan_thread_and_no_socket() {
        // The provider has run at process start (it is in this module's manifest),
        // and no session exists: this is a phone whose server has the feature off.
        assertEquals("no node, so no nz-lan thread", emptyList<String>(), threadsNamed("nz-lan"))
        assertTrue("the discovery is not browsing", LanHost.discovery?.isRunning != true)

        // Nothing holds the plan's default UDP port.
        val free = bindUdp(DEFAULT_PORT)
        assertTrue("$DEFAULT_PORT is free, so nothing bound it: $free", free.isSuccess)
        println("COLD-START threads=${threadsNamed("nz-lan").size} port=$DEFAULT_PORT free=${free.isSuccess}")
    }

    @Test
    fun a_real_node_binds_its_port_and_stop_joins_it_and_frees_the_port() = runBlocking {
        val dir = File(context.filesDir, "lan-lifecycle-${System.nanoTime()}/neezer").apply { mkdirs() }
        val cache = File(context.cacheDir, "lan-lifecycle-${System.nanoTime()}/neezer").apply { mkdirs() }
        val built = NzCore(
            CoreConfig(
                dataDir = dir.absolutePath,
                cacheDir = cache.absolutePath,
                appVersion = "test",
                versionCode = 1uL,
                sdkInt = Build.VERSION.SDK_INT.toUInt(),
                deviceName = "emulator LN14",
            ),
        )
        core = built
        // The probe goes through the CORE (`nz-api`), the same client the node
        // uses: it is the only HTTP stack here allowed to speak cleartext to an
        // origin the person configured (`Net.kt`'s rule), and it proves the
        // server, not Java's network-security policy.
        val reached = runCatching { built.setServer(SERVER) }
        println("BENCH-SERVER $SERVER -> ${reached.getOrNull() ?: reached.exceptionOrNull()}")
        assumeTrue("no bench server on $SERVER; start mobile/tools/perf-server.sh", reached.isSuccess)
        built.login("bench", "Bench1")
        val lan = built.lan()

        // « Échanger » off: the node answers Off and binds nothing.
        lan.setSettings(
            LanSettingsFfi(
                enabled = false,
                serve = true,
                tolerated = listOf("FLAC", "OPUS_320", "OPUS_128"),
                port = 0.toUShort(),
            ),
        )
        lan.start()
        // With « Échanger » off the node does NOT enrol (LN10's `start_or_restore`
        // returns before the server): its state is Off and it binds nothing.
        settled(5_000) { lan.snapshot().state == LanStateFfi.Off }
        val disabledPort = lan.advertisedPort().toInt()
        assertEquals("a disabled node binds no port", 0, disabledPort)
        println("DISABLED state=${lan.snapshot().state} port=$disabledPort threads=${threadsNamed("nz-lan").size}")

        // The person turns it on: the port is bound and the runtime thread is there.
        lan.setSettings(
            LanSettingsFfi(
                enabled = true,
                serve = true,
                tolerated = listOf("FLAC", "OPUS_320", "OPUS_128"),
                port = 0.toUShort(),
            ),
        )
        settled(20_000) { lan.advertisedPort() > 0.toUShort() }
        val bound = lan.advertisedPort().toInt()
        assertTrue("the node bound a UDP port: $bound", bound > 0)
        assertTrue("the nz-lan runtime thread exists: ${threadsNamed("nz-lan")}", threadsNamed("nz-lan").isNotEmpty())
        val inUse = bindUdp(bound)
        assertTrue("the port is really taken while the node runs: ${inUse.exceptionOrNull()}", inUse.isFailure)
        println("STARTED port=$bound threads=${threadsNamed("nz-lan")} taken=${inUse.isFailure}")

        // The app's shutdown path: stop the LAN before the core (LN13's carry-over).
        lan.stop()

        settled(10_000) { threadsNamed("nz-lan").isEmpty() }
        assertEquals("no nz-lan thread survives stop()", emptyList<String>(), threadsNamed("nz-lan"))
        val again = bindUdp(bound)
        assertTrue("the port is free again: $again", again.isSuccess)
        assertEquals("the port freed is the one bound", bound, again.getOrNull())
        println("STOPPED port=$bound free=${again.isSuccess} threads=${threadsNamed("nz-lan").size}")
    }

    // -- helpers ------------------------------------------------------------

    /**
     * Binds `port/udp` and answers the port, or fails with `BindException` when
     * somebody already holds it — the acceptance's "the port is free again".
     */
    private fun bindUdp(port: Int): Result<Int> = runCatching {
        // `DatagramSocket()` is ALREADY bound (to an ephemeral port); the null
        // SocketAddress constructor is the one that leaves it unbound, so `bind`
        // can answer "already in use" for the port we care about.
        val socket = DatagramSocket(null as java.net.SocketAddress?)
        try {
            socket.bind(java.net.InetSocketAddress(port))
            socket.localPort
        } finally {
            socket.close()
        }
    }

    /** Threads of THIS process whose name contains `needle` (`/proc/self/task`). */
    private fun threadsNamed(needle: String): List<String> {
        val tasks = File("/proc/self/task").listFiles() ?: return emptyList()
        return tasks.mapNotNull { task ->
            runCatching { File(task, "comm").readText().trim() }.getOrNull()?.takeIf { it.contains(needle) }
        }
    }

    /**
     * Waits for a condition with a BOUND. `LanApi` publishes no "port bound" event, so this polls a
     * memory read every 50 ms; the timeout is what keeps a broken node from hanging the run.
     */
    private fun settled(timeoutMs: Long, condition: () -> Boolean) {
        val deadline = System.nanoTime() + timeoutMs * 1_000_000
        while (System.nanoTime() < deadline) {
            if (condition()) return
            Thread.sleep(50)
        }
        assertTrue("condition not reached in $timeoutMs ms", condition())
    }

    companion object {
        /** The bench server, as the emulator sees this machine (`perf-server.sh --port 5610`). */
        const val SERVER: String = "http://10.0.2.2:5610"

        /** The node's default UDP port (the note's `lan.port`). */
        const val DEFAULT_PORT: Int = 45910
    }
}
