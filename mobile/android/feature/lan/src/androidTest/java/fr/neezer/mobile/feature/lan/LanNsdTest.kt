package fr.neezer.mobile.feature.lan

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Android's discovery, measured on the headless emulator (lot LN14's acceptance).
 *
 * Two things only a device can prove:
 *
 * 1. **The app registers `_neezer-lan._udp` with the port it was given and a
 *    random `nz-…` name** — read back by a SECOND `NsdManager` browse in this
 *    process, which is the resolver the note describes.
 * 2. **The multicast lock is held only while browsing** — asserted in the
 *    process (`WifiManager.MulticastLock.isHeld`) AND read in `dumpsys wifi`,
 *    which is the acceptance's own command.
 *
 * Everything here waits on an event (a `StateFlow` first, a `CountDownLatch`)
 * with a timeout: no `sleep` decides anything.
 */
@RunWith(AndroidJUnit4::class)
class LanNsdTest {
    private val context: Context = InstrumentationRegistry.getInstrumentation().targetContext
    private val found = mutableListOf<Pair<String, Int>>()
    private val nsd = LanNsd(
        context,
        onFound = { host, port -> synchronized(found) { found += host to port } },
        onLost = { _, _ -> },
    )

    @After
    fun stop() {
        nsd.stop()
    }

    @Test
    fun registers_the_service_with_the_advertised_port_and_a_random_name() = runBlocking {
        val port = TEST_PORT
        val startedAt = System.nanoTime()
        nsd.start(port)
        withTimeout(15_000) { nsd.state.first { it.advertising } }
        val registeredMs = (System.nanoTime() - startedAt) / 1_000_000
        println("REGISTER ms=$registeredMs")

        val state = nsd.state.value
        assertTrue("the instance name is random and says nothing", Regex("^nz-[0-9a-f]{10}\$").matches(state.instanceName))
        assertEquals(port, state.advertisedPort)
        assertEquals(LanNsd.SERVICE_TYPE, "_neezer-lan._udp")

        // A second browser in the same process sees what the platform published.
        val manager = context.getSystemService(NsdManager::class.java)
        assertNotNull(manager)
        val seen = AtomicReference<NsdServiceInfo>()
        val latch = CountDownLatch(1)
        val listener = object : NsdManager.DiscoveryListener {
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit
            override fun onServiceLost(serviceInfo: NsdServiceInfo) = Unit
            override fun onServiceFound(serviceInfo: NsdServiceInfo) {
                if (serviceInfo.serviceName != state.instanceName) return
                if (serviceInfo.port > 0) {
                    seen.set(serviceInfo)
                    latch.countDown()
                    return
                }
                // A discovered record carries no address or port until it is
                // resolved — the step the app itself performs.
                manager.resolveService(
                    serviceInfo,
                    object : NsdManager.ResolveListener {
                        override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) = Unit
                        override fun onServiceResolved(info: NsdServiceInfo) {
                            seen.set(info)
                            latch.countDown()
                        }
                    },
                )
            }
        }
        manager.discoverServices(LanNsd.SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
        try {
            assertTrue("the announcement was not browsable in 20 s", latch.await(20, TimeUnit.SECONDS))
            val browsedMs = (System.nanoTime() - startedAt) / 1_000_000
            println("BROWSE ms=$browsedMs")
            val info = seen.get()
            assertEquals("the port registered is the one the node bound", port, info.port)
            println("BROWSED type=${info.serviceType} name=${info.serviceName} port=${info.port}")
            assertEquals("the TXT record carries the version and nothing else", "1", info.attributes["v"]?.toString(Charsets.UTF_8))
            assertEquals("exactly one attribute", 1, info.attributes.size)
        } finally {
            runCatching { manager.stopServiceDiscovery(listener) }
        }

        // The lock is the browse's, not the announcement's, and it is held now.
        assertTrue("browsing", nsd.state.value.browsing)
        assertTrue("the multicast lock is held while browsing", nsd.lockIsHeld)
        val during = multicastLocksHeld()
        println("DUMPSYS-DURING locks=$during own=${during.any { it.contains(LanNsd.LOCK_TAG) }}")
        assertTrue("dumpsys wifi lists our lock: $during", during.any { it.contains(LanNsd.LOCK_TAG) })

        nsd.stop()

        assertFalse("browsing stopped", nsd.state.value.browsing)
        assertFalse("the multicast lock is released", nsd.lockIsHeld)
        assertFalse("nothing is announced any more", nsd.state.value.advertising)
        val after = multicastLocksHeld()
        println("DUMPSYS-AFTER locks=$after own=${after.any { it.contains(LanNsd.LOCK_TAG) }}")
        assertFalse("no lock of ours survives the stop: $after", after.any { it.contains(LanNsd.LOCK_TAG) })
    }

    @Test
    fun a_second_node_on_the_same_device_is_resolved_and_handed_to_the_core() = runBlocking {
        // The phone has no session in this test, so this process plays the peer:
        // a plain registration of the plan's service type on its own port.
        val manager = context.getSystemService(NsdManager::class.java) ?: error("no NsdManager")
        val peerPort = TEST_PORT + 1
        val peer = NsdServiceInfo().apply {
            serviceName = "nz-testpeer"
            serviceType = LanNsd.SERVICE_TYPE
            setPort(peerPort)
            setAttribute("v", "1")
        }
        val registered = CountDownLatch(1)
        val registration = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) = registered.countDown()
            override fun onRegistrationFailed(info: NsdServiceInfo, errorCode: Int) = Unit
            override fun onServiceUnregistered(info: NsdServiceInfo) = Unit
            override fun onUnregistrationFailed(info: NsdServiceInfo, errorCode: Int) = Unit
        }
        manager.registerService(peer, NsdManager.PROTOCOL_DNS_SD, registration)
        try {
            assertTrue("the fake peer registered", registered.await(10, TimeUnit.SECONDS))

            val startedAt = System.nanoTime()
            nsd.start(TEST_PORT + 2)
            // The handoff is what the core sees: `found(host, port)`.
            withTimeout(25_000) {
                nsd.state.first { state -> state.resolved.any { it.endsWith(":$peerPort") } }
            }
            println("RESOLVE ms=" + (System.nanoTime() - startedAt) / 1_000_000)
            val handed = synchronized(found) { found.toList() }
            assertTrue("the resolved pair reached the node: $handed", handed.any { it.second == peerPort })
            println("RESOLVED ${nsd.state.value.resolved}")
        } finally {
            runCatching { manager.unregisterService(registration) }
        }
    }

    @Test
    fun a_port_of_zero_announces_nothing() {
        nsd.start(0)
        assertFalse(nsd.isRunning)
        assertFalse(nsd.state.value.advertising)
    }

    /**
     * The `Multicast Locks held:` block of `dumpsys wifi` (the acceptance's own
     * command), and only that block: the dump also carries a log tail in which
     * our own tag appears, so a whole-output grep proves nothing.
     */
    private fun multicastLocksHeld(): List<String> {
        val lines = dumpsysWifi()
        val start = lines.indexOfFirst { it.trim().startsWith("Multicast Locks held") }
        if (start < 0) return emptyList()
        return lines.drop(start + 1)
            .takeWhile { it.isNotBlank() && it.startsWith(" ") }
            .map { it.trim() }
    }

    /** `dumpsys wifi`, read through the shell (the acceptance's own command). */
    private fun dumpsysWifi(): List<String> {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val pfd = instrumentation.uiAutomation.executeShellCommand("dumpsys wifi")
        return pfd.fileDescriptor.let { descriptor ->
            java.io.FileInputStream(descriptor).bufferedReader().useLines { lines -> lines.toList() }
        }
    }

    companion object {
        /** This lot's own port (`45910 + 10 × 14`), plus one and two for a peer. */
        const val TEST_PORT: Int = 46050
    }
}
