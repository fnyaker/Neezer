package fr.neezer.mobile.feature.lan

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.util.Log
import java.security.SecureRandom
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * Android's half of the discovery (lot LN14): `NsdManager` announces our node and
 * browses for the others, and a `MulticastLock` is held **only while browsing**.
 *
 * The rules this file exists to keep (`docs/agents/lan-nodes.md`, *Discovery*, and
 * AOSP's "the system only takes the multicast lock itself for a foreground
 * service of importance ≥ FOREGROUND_SERVICE, from T extensions 7"):
 *
 * - **The announcement carries nothing identifying.** The instance name is
 *   `nz-<10 hex>`, re-drawn by [randomInstanceName] on every start (a network
 *   change restarts us), and the only TXT attribute is `v=1`. Never the phone's
 *   name.
 * - **The lock is released in EVERY path.** It is acquired in [start] right
 *   before the browse and released in [stop], which is called from a `finally`
 *   in the host's stop path — a lock left held is a battery drain, and Android
 *   reports it in `dumpsys wifi`.
 * - **One resolution at a time before API 34.** `resolveService` refuses a second
 *   listener while one is active ("Cannot be in use for an active service
 *   resolution"), so [discover] queues them; from 34 the framework's
 *   `ServiceInfoCallback` keeps a discovered service's address up to date, which
 *   is what the note's *Android discovery* names.
 * - **A callback never owns the network.** `found`/`lost` are handed to the core
 *   immediately (a metrics-free FFI call); nothing here parses a name into a
 *   path, and a resolution's address list is capped at [MAX_ADDRESSES].
 *
 * The NSD bookkeeping is Android's, so this class is exercised on the device
 * (`LanNsdTest`, emulator) — the JVM suite covers [randomInstanceName] and the
 * queue's policy through [resolveQueueCapacity], which is all of it that is pure.
 */
class LanNsd(
    private val context: Context,
    private val onFound: (host: String, port: Int) -> Unit,
    private val onLost: (host: String, port: Int) -> Unit,
) {
    /** What the screen (and the acceptance) can read about the discovery. */
    data class State(
        /** Our own announcement: the name we registered and the port we gave it. */
        val advertising: Boolean = false,
        val instanceName: String = "",
        val advertisedPort: Int = 0,
        /** A browse is running, which is exactly when the lock is held. */
        val browsing: Boolean = false,
        /** The `MulticastLock` is held right now (never true while not browsing). */
        val lockHeld: Boolean = false,
        /** The `(host:port)` pairs handed to the core, deduplicated, in order. */
        val resolved: List<String> = emptyList(),
        val lastError: String? = null,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private val nsd: NsdManager? = context.getSystemService(Context.NSD_SERVICE) as? NsdManager
    private val wifi: WifiManager? =
        context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager

    private val callbackExecutor: ExecutorService by lazy {
        Executors.newSingleThreadExecutor { runnable ->
            Thread(runnable, "neezer-nsd").apply { isDaemon = true }
        }
    }

    private var registration: NsdManager.RegistrationListener? = null
    private var discovery: NsdManager.DiscoveryListener? = null
    private var lock: WifiManager.MulticastLock? = null

    /** One resolution at a time below API 34; the queue is capped by [MAX_QUEUED]. */
    private val queue = ArrayDeque<NsdServiceInfo>()
    private var resolving: NsdManager.ResolveListener? = null

    /** From API 34: one `ServiceInfoCallback` per discovered service. */
    private val callbacks = mutableMapOf<String, NsdManager.ServiceInfoCallback>()

    /** The last `(host, port)` per instance name, so a `lost` names what `found` did. */
    private val known = LinkedHashMap<String, Pair<String, Int>>()

    private var retries = 0

    /** Announces this node on `port` and starts browsing. Idempotent. */
    fun start(port: Int) {
        if (port <= 0 || port > 65535) {
            _state.update { it.copy(lastError = "port à annoncer invalide : $port") }
            return
        }
        if (registration != null) return
        val manager = nsd ?: run {
            _state.update { it.copy(lastError = "Ce téléphone n'a pas de découverte de services (NsdManager).") }
            return
        }
        // The lock comes FIRST: on API 26–32 (and an un-updated 33) the platform
        // only takes it for a foreground service, so an announcement made before
        // it would never leave the phone.
        acquireLock()
        register(manager, port)
        browse(manager)
    }

    /** Stops browsing and announcing and RELEASES the lock. Safe twice. */
    fun stop() {
        val manager = nsd
        try {
            discovery?.let { listener -> runCatching { manager?.stopServiceDiscovery(listener) } }
        } finally {
            discovery = null
            try {
                registration?.let { listener -> runCatching { manager?.unregisterService(listener) } }
            } finally {
                registration = null
                // A resolve in flight has no stop on every API: dropping the
                // listener is enough, its late answer finds no state to update.
                resolving = null
                queue.clear()
                if (Build.VERSION.SDK_INT >= 34 && manager != null) {
                    callbacks.forEach { (_, callback) ->
                        runCatching { manager.unregisterServiceInfoCallback(callback) }
                    }
                }
                callbacks.clear()
                known.clear()
                releaseLock()
                _state.update {
                    it.copy(advertising = false, browsing = false, lockHeld = false, advertisedPort = 0)
                }
            }
        }
    }

    /** The port really announced (the platform accepts it, `advertisedPort()` chose it). */
    val isRunning: Boolean
        get() = registration != null || discovery != null

    val lockIsHeld: Boolean
        get() = lock?.isHeld == true

    private fun register(manager: NsdManager, port: Int) {
        val service = NsdServiceInfo().apply {
            serviceName = randomInstanceName()
            serviceType = SERVICE_TYPE
            setPort(port)
            // The ONE attribute the note allows: a stranger learns "a Neezer node
            // is at this address" and nothing else.
            setAttribute("v", "1")
        }
        val listener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) {
                registration = this
                retries = 0
                _state.update {
                    it.copy(
                        advertising = true,
                        instanceName = info.serviceName,
                        advertisedPort = port,
                    )
                }
            }

            override fun onRegistrationFailed(info: NsdServiceInfo, errorCode: Int) {
                // A name conflict is the one failure worth one more draw
                // (RFC 6762 §8.1: probe, and on a clash, announce another name).
                if (retries < MAX_NAME_RETRIES) {
                    retries++
                    runCatching { manager.registerService(info.copyForRetry(), NsdManager.PROTOCOL_DNS_SD, this) }
                        .onSuccess { registration = this }
                        .onFailure { fail(it) }
                } else {
                    fail(IllegalStateException("l'annonce a échoué (code $errorCode)"))
                }
            }

            override fun onServiceUnregistered(info: NsdServiceInfo) = Unit

            override fun onUnregistrationFailed(info: NsdServiceInfo, errorCode: Int) = Unit
        }
        registration = listener
        runCatching { manager.registerService(service, NsdManager.PROTOCOL_DNS_SD, listener) }
            .onFailure { failure ->
                registration = null
                fail(failure)
            }
    }

    private fun browse(manager: NsdManager) {
        val listener = object : NsdManager.DiscoveryListener {
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                fail(IllegalStateException("la recherche a échoué (code $errorCode)"))
                stop()
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit

            override fun onDiscoveryStarted(serviceType: String) {
                _state.update { it.copy(browsing = true, lockHeld = lock?.isHeld == true) }
            }

            override fun onDiscoveryStopped(serviceType: String) {
                _state.update { it.copy(browsing = false) }
            }

            override fun onServiceFound(info: NsdServiceInfo) {
                // The platform answers `_neezer-lan._udp.` (a trailing dot is
                // legal in DNS-SD) or the `.local.` form depending on the API
                // level: comparing for equality dropped EVERY service, which is
                // the bug this test would have hidden behind "nothing found".
                if (!info.serviceType.startsWith(SERVICE_TYPE)) return
                discover(info)
            }

            override fun onServiceLost(info: NsdServiceInfo) {
                forget(info.serviceName)
            }
        }
        discovery = listener
        runCatching { manager.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener) }
            .onFailure { failure ->
                discovery = null
                fail(failure)
            }
    }

    /** Resolution, one at a time below API 34 (the platform refuses a second). */
    private fun discover(info: NsdServiceInfo) {
        val manager = nsd ?: return
        if (Build.VERSION.SDK_INT >= 34) {
            monitor(manager, info)
            return
        }
        enqueue(manager, info)
    }

    /** One resolution at a time; the rest wait (capped, never unbounded). */
    private fun enqueue(manager: NsdManager, info: NsdServiceInfo) {
        if (resolving != null) {
            if (queue.size >= MAX_QUEUED) return
            queue.addLast(info)
            return
        }
        resolveNext(manager, info)
    }

    private fun resolveNext(manager: NsdManager, info: NsdServiceInfo) {
        val listener = object : NsdManager.ResolveListener {
            override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                resolving = null
                // A failed resolution is not a peer: the core is not told, and the
                // queue goes on (one hint lost, never a stall).
                pump(manager)
            }

            override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                resolving = null
                deliver(serviceInfo)
                pump(manager)
            }
        }
        resolving = listener
        runCatching { resolveOne(manager, info, listener) }
            .onFailure {
                resolving = null
                pump(manager)
            }
    }

    /**
     * One resolution, on the executor overload where the platform has it
     * (`resolveService(info, executor, listener)` is T extensions 3, i.e. API 33;
     * the two-argument call is API 16 and works everywhere below).
     */
    private fun resolveOne(manager: NsdManager, info: NsdServiceInfo, listener: NsdManager.ResolveListener) {
        if (Build.VERSION.SDK_INT >= 33) {
            manager.resolveService(info, callbackExecutor, listener)
        } else {
            manager.resolveService(info, listener)
        }
    }

    private fun pump(manager: NsdManager) {
        if (resolving != null) return
        val next = queue.removeFirstOrNull() ?: return
        resolveNext(manager, next)
    }

    /**
     * API 34's live callback: a discovered service's addresses stay up to date,
     * which is what the note's *Android discovery* names. Guarded at the call
     * site (`discover`), and annotated so lint's `NewApi` sees the guard.
     */
    @androidx.annotation.RequiresApi(34)
    private fun monitor(manager: NsdManager, info: NsdServiceInfo) {
        if (callbacks.containsKey(info.serviceName)) return
        if (callbacks.size >= MAX_QUEUED) return
        val callback = object : NsdManager.ServiceInfoCallback {
            override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {
                callbacks.remove(info.serviceName)
                // A device whose extension refused the live callback still
                // resolves: the one-shot call is deprecated, not gone.
                enqueue(manager, info)
            }

            override fun onServiceUpdated(serviceInfo: NsdServiceInfo) {
                deliver(serviceInfo)
            }

            override fun onServiceLost() {
                forget(info.serviceName)
            }

            override fun onServiceInfoCallbackUnregistered() {
                callbacks.remove(info.serviceName)
            }
        }
        callbacks[info.serviceName] = callback
        runCatching { manager.registerServiceInfoCallback(info, callbackExecutor, callback) }
            .onFailure { callbacks.remove(info.serviceName) }
    }

    /**
     * Hands every address of a resolved service to the core, at most
     * [MAX_ADDRESSES] of them: an announcement can carry as many addresses as its
     * host likes, and each one is a dial.
     */
    private fun deliver(info: NsdServiceInfo) {
        val port = info.port
        if (port <= 0 || port > 65535) return
        val hosts = addressesOf(info).take(MAX_ADDRESSES)
        if (hosts.isEmpty()) return
        hosts.forEach { host -> onFound(host, port) }
        val first = hosts.first()
        known[info.serviceName] = first to port
        if (known.size > MAX_QUEUED) known.remove(known.keys.first())
        val key = "$first:$port"
        _state.update {
            if (key in it.resolved) it else it.copy(resolved = (it.resolved + key).take(MAX_QUEUED))
        }
    }

    private fun forget(name: String) {
        val last = known.remove(name) ?: return
        onLost(last.first, last.second)
    }

    private fun addressesOf(info: NsdServiceInfo): List<String> {
        if (Build.VERSION.SDK_INT >= 34) {
            val addresses = info.hostAddresses
            if (!addresses.isNullOrEmpty()) {
                return addresses.mapNotNull { address -> address.hostAddress }.filter { it.isNotBlank() }
            }
        }
        val host = info.host ?: return emptyList()
        return listOfNotNull(host.hostAddress?.takeIf { it.isNotBlank() })
    }

    private fun acquireLock() {
        if (lock != null) return
        val manager = wifi ?: return
        runCatching {
            manager.createMulticastLock(LOCK_TAG).apply {
                setReferenceCounted(false)
                acquire()
            }
        }.onSuccess { held ->
            lock = held
            _state.update { it.copy(lockHeld = held.isHeld) }
        }.onFailure {
            Log.w(TAG, "the multicast lock could not be taken", it)
        }
    }

    private fun releaseLock() {
        val held = lock ?: return
        lock = null
        runCatching { if (held.isHeld) held.release() }
        _state.update { it.copy(lockHeld = false) }
    }

    private fun fail(error: Throwable) {
        Log.w(TAG, "NSD: ${error.message}", error)
        _state.update { it.copy(lastError = error.message ?: error::class.java.simpleName) }
    }

    private fun NsdServiceInfo.copyForRetry(): NsdServiceInfo = NsdServiceInfo().apply {
        serviceName = randomInstanceName()
        serviceType = SERVICE_TYPE
        setPort(_state.value.advertisedPort)
        setAttribute("v", "1")
    }

    companion object {
        private const val TAG = "NeezerLan"

        /** The one service type of the plan, without the `.local.` the wire adds. */
        const val SERVICE_TYPE: String = "_neezer-lan._udp"

        /** The instance prefix the note pins (`INSTANCE_PREFIX` in `nz-lan`). */
        const val INSTANCE_PREFIX: String = "nz-"

        /**
         * The lock's name in `dumpsys wifi`: a person (and a test) can find it
         * there, and the acceptance reads it before and after a browse.
         */
        const val LOCK_TAG: String = "neezer-lan"

        /** Address resolutions return at most (LN3's 8-address cap, same reason). */
        const val MAX_ADDRESSES: Int = 8

        /** Instances remembered / resolves queued at once. */
        const val MAX_QUEUED: Int = 64

        /** A name clash re-draws the name twice, then it is a failure. */
        const val MAX_NAME_RETRIES: Int = 2

        private val random = SecureRandom()

        /** `nz-` + 10 hex from the OS CSPRNG: nothing about the phone. */
        fun randomInstanceName(): String {
            val bytes = ByteArray(5)
            random.nextBytes(bytes)
            return INSTANCE_PREFIX + bytes.joinToString("") { "%02x".format(it) }
        }

        /** The queue's capacity, for the JVM test of the resolve policy. */
        fun resolveQueueCapacity(): Int = MAX_QUEUED
    }
}
