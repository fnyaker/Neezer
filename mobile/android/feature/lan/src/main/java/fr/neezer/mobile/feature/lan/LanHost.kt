package fr.neezer.mobile.feature.lan

import android.app.Application
import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.util.Log
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.feature.offline.OfflineHost
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerListener
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.LanStateFfi

/**
 * When the LAN discovery runs, and nothing else (lot LN14 — the battery rule).
 *
 * The note (`docs/agents/lan-nodes.md`, *Discovery*) and `mobile.md`'s *Mobile
 * constraints* say it once: browse and announce while the app is in the
 * foreground, **or** playing, **or** while the downloads worker runs, and stop
 * 60 s after the last of those. With the screen off and nothing playing the node
 * must send **zero** packets — a counter, not a hope (`Lan::wire`, LN10).
 *
 * This class is the whole policy and nothing else: no Android, no clock. The
 * test drives it with [runTest]'s virtual time and the two callbacks, so « 60 s
 * later » is an event the test causes, never a second it waits.
 */
class LanActivityScheduler(
    private val scope: CoroutineScope,
    private val graceMs: Long,
    private val onActive: () -> Unit,
    private val onIdle: () -> Unit,
) {
    private var pending: Job? = null

    @Volatile
    var active: Boolean = false
        private set

    /** The three counted flags, on every change (never a timer). */
    fun update(foreground: Boolean, playing: Boolean, downloading: Boolean) {
        val wanted = foreground || playing || downloading
        if (wanted) {
            pending?.cancel()
            pending = null
            if (!active) {
                active = true
                onActive()
            }
            return
        }
        if (!active || pending != null) return
        // The grace is what makes a screen-off pause not cost a re-announce, and
        // a person who comes back within a minute not wait for one.
        pending = scope.launch {
            delay(graceMs)
            active = false
            pending = null
            onIdle()
        }
    }

    /** Stops the pending idle job and reports idle at once (process death, a test). */
    fun shutdown() {
        pending?.cancel()
        pending = null
        if (active) {
            active = false
            onIdle()
        }
    }

    companion object {
        /** The note's 60 s, in milliseconds. */
        const val SLEEP_GRACE_MS: Long = 60_000L
    }
}

/**
 * The LAN node's process-wide wiring (lot LN14): the node, Android's NSD, the
 * three activity flags and the network callback.
 *
 * It is installed by [LanInitProvider] at process start — a library module may
 * not edit `:app`'s `NzApplication`, and a `ContentProvider` in this module's
 * manifest is the standard way a library joins the app's start-up without one
 * line outside its own files. Nothing is created before the person has a
 * session: `LanApi::start` answers `Unsupported` on a server without the feature
 * (LN13 proved no directory, no socket and no thread), and without a port there
 * is nothing to announce, so [LanNsd] never starts.
 */
object LanHost {
    private const val TAG = "NeezerLan"

    private var app: Application? = null
    private var scope: CoroutineScope? = null
    private var port: LanPort? = null
    private var nsd: LanNsd? = null
    private var scheduler: LanActivityScheduler? = null
    private var playerListener: PlayerListener? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null

    /**
     * The foreground flag, from `ProcessLifecycleOwner` — the SAME signal
     * `:bridge`'s `ForegroundWatcher` gives the core, which is what makes a
     * screen-off stop the browse (`docs/agents/mobile.md`, *Mobile constraints*).
     */
    private var lifecycleObserver: DefaultLifecycleObserver? = null

    private var foreground = false
    private var playing = false
    private var downloading = false

    private val _active = MutableStateFlow(false)

    /** Whether discovery is meant to run right now (the acceptance reads it). */
    val active: StateFlow<Boolean> = _active.asStateFlow()

    /** The discovery, for the device tests and Aide. */
    val discovery: LanNsd?
        get() = nsd

    /** The node, as this module talks to it. */
    val node: LanPort?
        get() = port

    private var screenModel: LanModel? = null

    /**
     * The process's ONE screen model: the switches in Réglages and the route see
     * the same state, so flipping one somewhere is never a second node talking.
     */
    fun model(): LanModel {
        screenModel?.let { return it }
        val lanPort = port ?: FfiLanPort()
        val built = LanModel(
            port = lanPort,
            scope = scope ?: CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate),
            playlists = BridgePlaylists,
            toast = { message -> fr.neezer.mobile.designsystem.NzToasts.show(message) },
        )
        screenModel = built
        return built
    }

    /**
     * Installs the observers once. `port` and `graceMs` are the seams the device
     * test uses; the app passes neither.
     */
    fun install(
        application: Application,
        lanPort: LanPort = FfiLanPort(),
        graceMs: Long = LanActivityScheduler.SLEEP_GRACE_MS,
    ) {
        if (app != null) return
        app = application
        port = lanPort
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        this.scope = scope

        val discovery = LanNsd(
            application,
            onFound = { host, foundPort -> runCatching { lanPort.found(host, foundPort) } },
            onLost = { host, lostPort -> runCatching { lanPort.lost(host, lostPort) } },
        )
        nsd = discovery

        val activity = LanActivityScheduler(
            scope = scope,
            graceMs = graceMs,
            onActive = { activate() },
            onIdle = { deactivate() },
        )
        scheduler = activity
        schedulerActive = activity
        _active.value = false

        // The node's change feed: a new port (the server probe landed, a renewal
        // rotated the key) has to reach the announcement, and a peer arriving is
        // a change a screen must see. `subscribe` keeps a listener registered
        // before the node exists (LN13), so this works from process start.
        runCatching { lanPort.subscribe(LanFeed.listener) }
        scope.launch { LanFeed.version.collect { onNodeChanged() } }

        observeForeground()
        observePlayer()
        observeDownloads(scope)
        observeNetwork(application)
        // A build whose core is missing (a library's own test APK) has no node:
        // the host stays installed and dormant, it does not fail at start-up.
        val state = runCatching { lanPort.snapshot().state }.getOrNull()
        if (state == LanStateFfi.Unsupported) {
            Log.i(TAG, "the server has no LAN exchange: nothing will start")
        }
        update(foreground = false, playing = false, downloading = false)
    }

    private var schedulerActive: LanActivityScheduler? = null

    /** The three flags; the observers and the device test both come here. */
    fun update(foreground: Boolean, playing: Boolean, downloading: Boolean) {
        this.foreground = foreground
        this.playing = playing
        this.downloading = downloading
        // A provider runs BEFORE `Application.onCreate`, so the first install sees
        // no core: the player's observer is attached on the first update that can
        // reach one (an event, not a retry timer).
        if (playerListener == null) observePlayer()
        runCatching { port?.setActive(foreground, playing, downloading) }
        schedulerActive?.update(foreground, playing, downloading)
        _active.value = schedulerActive?.active == true
    }

    /** The app came to the foreground (or left it): `ProcessLifecycleOwner`. */
    fun setForeground(visible: Boolean) = update(visible, playing, downloading)

    /** Stops everything and forgets the observers: process death and a test. */
    fun shutdown() {
        scheduler?.shutdown()
        schedulerActive = null
        scheduler = null
        nsd?.stop()
        nsd = null
        runCatching { port?.stop() }
        playerListener = null
        lifecycleObserver?.let { observer ->
            runCatching { ProcessLifecycleOwner.get().lifecycle.removeObserver(observer) }
        }
        lifecycleObserver = null
        networkCallback?.let { callback ->
            runCatching {
                app?.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(callback)
            }
        }
        networkCallback = null
        scope = null
        port = null
        app = null
        screenModel = null
        _active.value = false
    }

    private fun activate() {
        _active.value = true
        val lanPort = port ?: return
        scope?.launch {
            runCatching { lanPort.start() }
                .onFailure { Log.i(TAG, "the node did not start: ${it.message}") }
            announce(lanPort)
        }
        announce(lanPort)
    }

    private fun deactivate() {
        _active.value = false
        nsd?.stop()
        runCatching { port?.setActive(false, false, false) }
    }

    /**
     * Announces only when there is a port to announce: `advertised_port()` is 0
     * before the node binds, and the node binds after the server probe (LN13).
     * A change that carries the port therefore announces — this is called from
     * both the activation and the node's change feed.
     */
    private fun announce(lanPort: LanPort) {
        val discovery = nsd ?: return
        // « Échanger avec mes appareils » off: the core has closed its socket, and
        // announcing a port nobody listens on would wake peers for nothing.
        val settings = runCatching { lanPort.settings() }.getOrNull()
        if (settings?.enabled == false) {
            if (discovery.isRunning) discovery.stop()
            return
        }
        if (discovery.isRunning) return
        val bound = runCatching { lanPort.advertisedPort() }.getOrDefault(0)
        if (bound <= 0) return
        discovery.start(bound)
    }

    private fun observeForeground() {
        if (lifecycleObserver != null) return
        val observer = object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                update(foreground = true, playing = playing, downloading = downloading)
            }

            override fun onStop(owner: LifecycleOwner) {
                update(foreground = false, playing = playing, downloading = downloading)
            }
        }
        lifecycleObserver = observer
        ProcessLifecycleOwner.get().lifecycle.addObserver(observer)
    }

    private fun observePlayer() {
        val player = runCatching { NzBridge.core?.player() }.getOrNull() ?: return
        val listener = object : PlayerListener {
            override fun onSnapshot(snapshot: PlayerSnapshot) {
                // A Rust thread: write the flag and return, never call back in.
                update(foreground, playing = snapshot.state == PlayState.PLAYING, downloading)
            }

            override fun onQueueChanged(version: ULong) = Unit

            override fun onFault(cause: String) = Unit
        }
        playerListener = listener
        runCatching { player.subscribe(listener) }
        playing = runCatching { player.snapshot().state == PlayState.PLAYING }.getOrDefault(false)
    }

    private fun observeDownloads(scope: CoroutineScope) {
        scope.launch {
            OfflineHost.model().state.collect { state ->
                val running = state.sets.any { it.running }
                if (running != downloading) update(foreground, playing, downloading = running)
            }
        }
    }

    private fun observeNetwork(application: Application) {
        val manager = application.getSystemService(ConnectivityManager::class.java) ?: return
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                runCatching { port?.networkChanged() }
                port?.let(::announce)
            }

            override fun onLost(network: Network) {
                runCatching { port?.networkChanged() }
            }
        }
        networkCallback = callback
        runCatching { manager.registerDefaultNetworkCallback(callback) }
            .onFailure { networkCallback = null }
    }

    /** The port the host would announce right now (0 when the node has none). */
    fun advertisedPort(): Int = runCatching { port?.advertisedPort() }.getOrNull() ?: 0

    /** The node's change feed: a new port, a new state, a peer arrived. */
    fun onNodeChanged() {
        val lanPort = port ?: return
        val settings = runCatching { lanPort.settings() }.getOrNull()
        if (settings?.enabled == false) {
            nsd?.stop()
            return
        }
        if (schedulerActive?.active == true) announce(lanPort)
    }

    /** Used by [LanInitProvider] only; the test's own context is not an app. */
    internal fun context(): Context? = app
}
