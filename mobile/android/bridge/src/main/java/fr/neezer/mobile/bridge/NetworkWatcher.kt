package fr.neezer.mobile.bridge

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.util.Log
import uniffi.nz_ffi.NetworkState
import uniffi.nz_ffi.NzCore

/**
 * `ConnectivityManager` → `NzCore.set_network`, once per process.
 *
 * The rule this serves (`docs/agents/mobile.md`, *Network errors*): with no link
 * at all, the NEXT request answers "Hors ligne" **in zero sockets** instead of
 * waiting four seconds to learn what the phone already knew. [NetworkState.None]
 * also flips `nz-api`'s own offline watch, so a request already in flight stops
 * retrying.
 *
 * `metered` is carried as well: the Auto quality ladder's ceiling and the
 * downloads' policy read it (Réglages → Téléchargements), and it is a property of
 * the network, not of a screen.
 */
internal object NetworkWatcher {
    private const val TAG = "NeezerNetwork"

    private var manager: ConnectivityManager? = null
    private var callback: ConnectivityManager.NetworkCallback? = null
    private var answered = false

    fun start(context: Context, core: NzCore) {
        if (callback != null) return
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: run {
            Log.w(TAG, "no ConnectivityManager: assuming a network is there")
            return
        }
        this.manager = manager
        val watcher = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                read(core, network)
            }

            override fun onCapabilitiesChanged(
                network: Network,
                capabilities: NetworkCapabilities,
            ) {
                core.setNetwork(stateOf(capabilities))
                answered = true
            }

            override fun onLost(network: Network) {
                core.setNetwork(NetworkState.None)
                answered = true
            }

            override fun onUnavailable() {
                core.setNetwork(NetworkState.None)
                answered = true
            }
        }
        callback = watcher
        runCatching { manager.registerDefaultNetworkCallback(watcher) }
            .onFailure {
                Log.w(TAG, "the default-network callback was refused", it)
                callback = null
            }
    }

    fun stop() {
        val watcher = callback ?: return
        runCatching { manager?.unregisterNetworkCallback(watcher) }
        callback = null
        manager = null
        answered = false
    }

    /** Whether the callback has said anything yet (Aide's diagnostic line). */
    val started: Boolean
        get() = callback != null

    /** Whether a state has been reported (otherwise the core assumes online). */
    val settled: Boolean
        get() = answered

    private fun read(core: NzCore, network: Network) {
        val capabilities = manager?.getNetworkCapabilities(network)
        core.setNetwork(if (capabilities == null) NetworkState.Other(metered = true) else stateOf(capabilities))
        answered = true
    }

    private fun stateOf(capabilities: NetworkCapabilities): NetworkState {
        val metered = !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        return when {
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> NetworkState.Wifi(metered = metered)
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) ->
                NetworkState.Cellular(metered = metered)
            else -> NetworkState.Other(metered = metered)
        }
    }
}
