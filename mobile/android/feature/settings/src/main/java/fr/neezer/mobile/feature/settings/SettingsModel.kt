package fr.neezer.mobile.feature.settings

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.AccessData
import uniffi.nz_ffi.AccountData
import uniffi.nz_ffi.AccountsData
import uniffi.nz_ffi.DeezerStatusData
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.ServerSettingsData
import uniffi.nz_ffi.ServicesData
import uniffi.nz_ffi.SettingsData
import uniffi.nz_ffi.SourceData
import uniffi.nz_ffi.SourcesData
import uniffi.nz_ffi.Versions

/**
 * Réglages' state machine (lot MB16).
 *
 * Every rule the SPA taught, in the order the person meets it, as plain Kotlin
 * over [SettingsService] — `SettingsModelTest` proves each one on the JVM with a
 * fake, no JNA, no `.so`, no device.
 *
 * - **Each server card loads with `screenFlow`**: the device's copy paints at
 *   once, the network replaces it, a dead link keeps the copy and says « Hors
 *   ligne · version mémorisée », and with nothing to show the cause and
 *   « Réessayer » ([Section] holds all four states).
 * - **A click answers in the same frame.** A theme, an accent, a switch or a
 *   slider move in [state] BEFORE anything else; the write follows, and a
 *   refused SERVER write puts the server's own answer back with a toast.
 * - **One request per action**, never a storm: a source's arrow is one `PUT`,
 *   an import is one `POST` and the progress is read from the copy the next
 *   `accounts()` brings (never a poll of its own while nothing runs).
 * - **A local write is one `kv` upsert**, and the shell hears it through
 *   [NzAppearance], so a theme change repaints the whole app without a reload.
 *
 * The numbers this lot is held to: a cold *Sources* is the SPA's three
 * requests, a warm one ZERO (the device's copy), and every card that is only
 * this device's preferences (appearance, animations, downloads, assistance)
 * asks nothing at all.
 */
class SettingsModel(
    private val service: SettingsService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
    private val clock: () -> Long = { System.currentTimeMillis() },
) {
    /** One card's four states (rule 3): the copy, the fresh answer, the cause. */
    data class Section<T>(
        val data: T? = null,
        val loading: Boolean = false,
        /** What is drawn came from the device and the refresh failed. */
        val stale: Boolean = false,
        val failure: String? = null,
    ) {
        val ready: Boolean get() = data != null && failure == null
    }

    data class State(
        /** The category open, `null` on the list (the phone's two levels). */
        val section: String? = null,
        /** The menu's filter — it narrows the LIST, never the page. */
        val query: String = "",
        /** This device's settings, resolved (never `null`: the defaults are the SPA's). */
        val settings: SettingsData? = null,
        val sources: Section<SourcesData> = Section(),
        val accounts: Section<AccountsData> = Section(),
        val access: Section<AccessData> = Section(),
        val services: Section<ServicesData> = Section(),
        /** The administrator's two cards; absent for a member (the server refuses them). */
        val server: ServerSettingsData? = null,
        val serverFailure: String? = null,
        val deezer: DeezerStatusData? = null,
        val deezerChecking: Boolean = false,
        val dislikes: Section<ListHandle> = Section(),
        val versions: Versions? = null,
        /** The text *Partager le diagnostic* hands to the platform's share sheet. */
        val report: String? = null,
        /** The discovery level, from MB5's own store (no request to paint it). */
        val discoveryLevel: Double = 0.35,
        /** What the discovery slider's last move will save (`lib/discovery.js`'s settle). */
        val discoveryPending: Double? = null,
        /** One write is in flight; a second tap on the same control is ignored. */
        val busy: Boolean = false,
        /**
         * `/api/me`'s `user.admin`, read from the SESSION — no request.
         *
         * Why the menu needs it: on a phone the list is a ROUTE of its own, so
         * opening Réglages builds a fresh model that has neither `/access` nor
         * `/settings`; the two administrator categories were therefore never
         * drawn until *Sources* had been opened once (measured on the emulator
         * by MB17, whose pages a person could not reach at all). The session
         * already knows the answer, for free.
         */
        val sessionAdmin: Boolean = false,
    ) {
        val categories get() = filterCategories(categoriesFor(isAdmin = admin), query)

        val admin: Boolean get() = sessionAdmin || server != null || access.data?.admin == true

        /** The sources in the order the card draws them: main, fallbacks, off. */
        val orderedSources: List<SourceData>
            get() = sources.data?.sources.orEmpty()
                .sortedWith(compareBy<SourceData> { when (it.place) { "main" -> 0; "fallback" -> 1; else -> 2 } }.thenBy { it.rank ?: 0 })
    }

    var state by mutableStateOf(State())
        private set

    /** The rows a removal from "Je n'aime pas" has hidden, so the list agrees at once. */
    val hiddenDislikes = mutableStateListOf<String>()

    private var job: Job? = null
    private var discoveryJob: Job? = null

    /** The device's own settings, before any section asks for anything. */
    fun start() {
        if (!state.sessionAdmin) {
            state = state.copy(
                sessionAdmin = runCatching { fr.neezer.mobile.feature.admin.AppAdmin.service()?.identity()?.admin }
                    .getOrNull() == true,
            )
        }
        if (state.settings == null) {
            state = state.copy(settings = runCatching { service.settings() }.getOrNull())
            state.settings?.let(NzAppearance::apply)
        }
    }

    /** A category is opened (from the list, a deep link, or the wide rail). */
    fun open(section: String?) {
        if (state.section == section) return
        job?.cancel()
        state = state.copy(section = section, versions = service.versions())
        if (section == null) return
        when (section) {
            "appearance", "viz", "downloads" -> start()
            "sources" -> loadSources()
            "account" -> {
                loadServices()
                state = state.copy(versions = service.versions())
            }
            "discovery" -> {
                loadDislikes()
                state = state.copy(discoveryLevel = readDiscoveryLevel())
            }
            "help" -> state = state.copy(versions = service.versions())
        }
    }

    fun setQuery(query: String) {
        state = state.copy(query = query)
    }

    // --- the sources' card -------------------------------------------------

    private fun loadSources() {
        job = scope.launch {
            launch {
                screenFlow(
                    cached = { withContext(io) { service.cachedSources() } },
                    fresh = { withContext(io) { service.sources() } },
                ).collect { screen -> state = state.copy(sources = screen.asSection()) }
            }
            launch {
                screenFlow(
                    cached = { withContext(io) { service.cachedAccounts() } },
                    fresh = { withContext(io) { service.accounts() } },
                ).collect { screen -> state = state.copy(accounts = screen.asSection()) }
            }
            val access = launch {
                screenFlow(
                    cached = { withContext(io) { service.cachedAccess() } },
                    fresh = { withContext(io) { service.access() } },
                ).collect { screen -> state = state.copy(access = screen.asSection()) }
            }
            // The administrator's cards are the SPA's two (`/settings` and the
            // provider's verdict), and they are asked ONLY once `/access` said
            // this person is an administrator: a member's 403 is not an error to
            // shout about, and it is not a request worth sending either.
            access.join()
            if (state.access.data?.admin == true) admin()
        }
    }

    private suspend fun admin() {
        val server = withContext(io) {
            runCatching {
                service.cachedServerSettings() ?: service.serverSettings()
            }
        }
        val settings = server.getOrNull()
        state = state.copy(
            server = settings,
            serverFailure = server.exceptionOrNull()?.let(NzErrors::text),
            deezer = withContext(io) { runCatching { service.cachedDeezerStatus() }.getOrNull() },
        )
        if (settings == null) return
        state = state.copy(deezerChecking = true)
        val status = withContext(io) { runCatching { service.deezerStatus(false) } }
        state = state.copy(
            deezer = status.getOrNull() ?: state.deezer,
            deezerChecking = false,
        )
    }

    /**
     * One arrow, the lead source, a switch or *classer par qualité*. The card
     * moves in the SAME frame as the tap (the optimistic preview), and the
     * server's own answer replaces it; a refusal puts the old order back with
     * its cause.
     */
    fun saveSourceOrder(order: List<String>) {
        val before = state.sources
        state = state.copy(sources = before.previews(order))
        serverWrite {
            try {
                state = state.copy(sources = Section(service.saveSourcesOrder(order)))
            } catch (e: Exception) {
                state = state.copy(sources = before)
                throw e
            }
        }
    }

    fun setSourceEnabled(id: String, on: Boolean) = serverWrite {
        val before = state.sources.data
        if (before != null) {
            state = state.copy(
                sources = Section(before.copy(sources = before.sources.map { if (it.id == id) it.copy(enabled = on) else it })),
            )
        }
        state = state.copy(sources = Section(service.setSourceEnabled(id, on)))
    }

    /** `SourceOrder.svelte#byBestQuality`: the fallbacks, best quality first. */
    fun sortFallbacksByQuality() {
        val data = state.sources.data ?: return
        val main = data.sources.firstOrNull { it.place == "main" }
        val fallbacks = data.sources.filter { it.place == "fallback" }
            .sortedWith(compareByDescending<SourceData> { it.qualityLabel == "FLAC" }.thenByDescending { it.qualityLabel })
        val off = data.sources.filter { it.place == "off" }
        saveSourceOrder((listOfNotNull(main) + fallbacks + off).map { it.id })
    }

    // --- my accounts -------------------------------------------------------

    fun retry(section: String) {
        state = state.copy(section = null)
        open(section)
    }

    fun importNow(account: AccountData) = serverWrite {
        state = state.copy(accounts = Section(service.importAccount(account.id, account.kinds)))
        pollImports()
    }

    /**
     * While an import runs, its progress is read every 1.5 s — the SPA's own
     * cadence (`MyAccounts.svelte#schedule`) — and NOT AT ALL when nothing runs
     * (rule 2: a job that is running says so, a screen never polls for nothing).
     */
    private fun pollImports() {
        scope.launch {
            while (state.accounts.data?.accounts?.any { it.runningPhase != null } == true) {
                kotlinx.coroutines.delay(IMPORT_POLL_MS)
                val fresh = withContext(io) { runCatching { service.accounts() } }
                fresh.getOrNull()?.let { state = state.copy(accounts = Section(it)) }
                if (fresh.isFailure) return@launch
            }
        }
    }

    /** `MyAccounts.svelte#chooseDefault`: instant, and the server's answer replaces it. */
    fun chooseDefault(id: String?) = serverWrite {
        val before = state.accounts.data
        if (before != null) {
            state = state.copy(
                accounts = Section(
                    before.copy(
                        default = id,
                        accounts = before.accounts.map { it.copy(isDefault = it.id == id) },
                    ),
                ),
            )
        }
        state = state.copy(accounts = Section(service.setDefaultAccount(id)))
    }

    fun toggleAccountAuto(account: AccountData, kind: String) = serverWrite {
        val next = kind !in account.autoOn
        val before = state.accounts.data
        if (before != null) {
            state = state.copy(
                accounts = Section(
                    before.copy(
                        accounts = before.accounts.map {
                            if (it.id != account.id) it else it.copy(
                                autoOn = if (next) it.autoOn + kind else it.autoOn - kind,
                            )
                        },
                    ),
                ),
            )
        }
        state = state.copy(accounts = Section(service.setAccountAuto(account.id, listOf(kind), next)))
    }

    fun unlinkAccount(account: AccountData) = serverWrite {
        state = state.copy(accounts = Section(service.unlinkAccount(account.id)))
    }

    /** `MyAccounts.svelte#link`: the ARL is verified by the server before it is stored. */
    fun linkDeezer(arl: String, mode: String? = null) = serverWrite {
        if (arl.isBlank()) {
            toast("Collez d'abord un ARL")
            return@serverWrite
        }
        state = state.copy(accounts = Section(service.linkDeezer(arl.trim(), mode)))
        toast("Compte Deezer relié")
    }

    // --- who may use what --------------------------------------------------

    fun setGrants(provider: String, account: String, feature: String, subjects: List<String>) = serverWrite {
        val grants = service.setGrants(provider, account, feature, subjects)
        val before = state.access.data ?: return@serverWrite
        state = state.copy(
            access = Section(
                before.copy(
                    providers = before.providers.map { p ->
                        if (p.id != provider) p else p.copy(
                            accounts = p.accounts.map { a ->
                                if (a.id != account) a else a.copy(grants = grants)
                            },
                        )
                    },
                ),
            ),
        )
    }

    fun createGroup(name: String) = serverWrite {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) return@serverWrite
        service.createGroup(trimmed)
        state = state.copy(access = Section(service.access()))
    }

    fun setGroupMembers(id: String, members: List<String>) = serverWrite {
        val updated = service.setGroupMembers(id, members)
        val before = state.access.data ?: return@serverWrite
        state = state.copy(
            access = Section(before.copy(groups = before.groups.map { if (it.id == id) it.copy(members = updated) else it })),
        )
    }

    fun deleteGroup(id: String) = serverWrite {
        service.deleteGroup(id)
        state = state.copy(access = Section(service.access()))
    }

    // --- the export services ------------------------------------------------

    private fun loadServices() {
        job = scope.launch {
            screenFlow(
                cached = { withContext(io) { service.cachedServices() } },
                fresh = { withContext(io) { service.services() } },
            ).collect { screen -> state = state.copy(services = screen.asSection()) }
        }
    }

    fun disconnectService(id: String, name: String) = serverWrite {
        state = state.copy(services = Section(service.disconnectService(id)))
        toast("$name délié")
    }

    fun saveSpotifyApp(clientId: String) = serverWrite {
        state = state.copy(services = Section(service.setSpotifyApp(clientId.trim())))
        toast(if (clientId.isBlank()) "Spotify désactivé" else "Application Spotify enregistrée")
    }

    /** Answers the consent URL; the caller opens it in the platform's browser. */
    suspend fun spotifyAuthorize(): String? = withContext(io) {
        runCatching { service.spotifyAuthorize() }
            .onFailure { toast(NzErrors.text(it)) }
            .getOrNull()
    }

    /**
     * « Relier » on one service (`ServicesSettings.svelte#connect`): Spotify hands
     * back a consent URL the platform's browser opens (PKCE, never a password in
     * the app); a service with no door — YouTube Music's pasted headers — says so
     * instead of pretending.
     */
    fun linkService(id: String) {
        when (id) {
            "spotify" -> scope.launch {
                val url = spotifyAuthorize() ?: return@launch
                val answer = ServiceLink(url)
                emitServiceLink(answer)
            }
            "youtube" -> toast("YouTube Music se relie depuis la page Services du site : la session se colle en en-têtes de navigateur.")
            else -> toast("Ce service n'a pas de connexion depuis l'application.")
        }
    }

    /** The URL a screen must open, handed over once (the UI owns the browser). */
    val pendingLink = mutableStateListOf<ServiceLink>()

    private fun emitServiceLink(link: ServiceLink) {
        pendingLink.clear()
        pendingLink.add(link)
    }

    // --- the server's own settings (admin) ---------------------------------

    fun saveQuota(gb: Double) = serverWrite {
        if (gb < 0 || gb.isNaN()) {
            toast("Valeur de quota invalide")
            return@serverWrite
        }
        val saved = service.saveUploadQuota(gb)
        state = state.copy(server = saved)
        toast(if (gb == 0.0) "Quota d'upload désactivé (illimité)" else "Quota d'upload réglé à ${saved.uploadQuotaGb} Go")
    }

    fun saveDeezerArl(arl: String) = serverWrite {
        if (arl.isBlank()) {
            toast("Collez d'abord un ARL")
            return@serverWrite
        }
        state = state.copy(server = service.saveDeezerArl(arl.trim()))
        toast("ARL Deezer enregistré et vérifié")
        refreshDeezer(force = true)
    }

    fun clearDeezerArl() = serverWrite {
        state = state.copy(server = service.clearDeezerArl())
        toast("ARL personnalisé supprimé")
        refreshDeezer(force = true)
    }

    fun testDeezer() {
        if (state.deezerChecking) return
        scope.launch { refreshDeezer(force = true) }
    }

    private suspend fun refreshDeezer(force: Boolean) {
        state = state.copy(deezerChecking = true)
        val status = withContext(io) { runCatching { service.deezerStatus(force) } }
        state = state.copy(
            deezer = status.getOrNull() ?: state.deezer,
            deezerChecking = false,
        )
        status.exceptionOrNull()?.let { toast(NzErrors.text(it)) }
    }

    // --- Découverte -------------------------------------------------------

    private fun loadDislikes() {
        job = scope.launch {
            hiddenDislikes.clear()
            screenFlow(
                cached = { withContext(io) { service.cachedDislikes() } },
                fresh = { withContext(io) { service.dislikes() } },
            ).collect { screen -> state = state.copy(dislikes = screen.asSection()) }
        }
    }

    /** The level lives in MB5's own store; the slider shows it before any save. */
    private fun readDiscoveryLevel(): Double = state.discoveryLevel

    /**
     * `lib/discovery.js#setDiscovery`: the value moves at once, and the write
     * waits for the hand to settle (one request per drag, never one per pixel).
     */
    fun setDiscoveryLevel(level: Double) {
        val clamped = level.coerceIn(0.0, 1.0)
        state = state.copy(discoveryLevel = clamped, discoveryPending = clamped)
        discoveryJob?.cancel()
        discoveryJob = scope.launch {
            kotlinx.coroutines.delay(SETTLE_MS)
            val pending = state.discoveryPending ?: return@launch
            val saved = withContext(io) { runCatching { service.set(key = "discover.level", value = pending.toString()) } }
            if (saved.isFailure) toast(NzErrors.text(saved.exceptionOrNull()!!))
            state = state.copy(discoveryPending = null)
        }
    }

    /** *Retirer*: the row leaves the list at once, and comes back if the server refuses. */
    fun undislike(row: uniffi.nz_ffi.TrackRow) = serverWrite {
        hiddenDislikes.add(row.key)
        val answer = runCatching { service.setDisliked(row.id, false) }
        if (answer.isFailure) {
            hiddenDislikes.remove(row.key)
            throw answer.exceptionOrNull()!!
        }
    }

    // --- this device's settings --------------------------------------------

    /** `ui.theme` — the SPA's `setTheme`: instant, and the shell repaints. */
    fun setTheme(name: String) = localWrite("ui.theme", "\"$name\"")

    /** `ui.accent` — `null` means "back to the theme's own". */
    fun setAccent(hex: String?) = localWrite("ui.accent", hex?.let { "\"$it\"" } ?: "null")

    /** `viz.eco` — the eco switch: no animation anywhere. */
    fun setEco(on: Boolean) = localWrite("viz.eco", on.toString())

    fun setVizMode(mode: String) = localWrite("viz.mode", "\"$mode\"")

    fun setVizPalette(palette: String) = localWrite("viz.palette", "\"$palette\"")

    fun setVizIntensity(value: Double) = localWrite("viz.intensity", value.coerceIn(0.0, 1.0).toString())

    fun setVizFps(fps: Int) = localWrite("viz.fps", fps.toString())

    fun setVizBeat(on: Boolean) = localWrite("viz.beat", on.toString())

    fun setScopeOrientation(value: String) = localWrite("viz.scope.orientation", "\"$value\"")

    fun setScopeColour(value: String) = localWrite("viz.scope.colour", "\"$value\"")

    fun setPlaybackQuality(id: String) = localWrite("player.quality", "\"$id\"")

    fun setDownloadQuality(id: String) = localWrite("offline.quality", "\"$id\"")

    fun setOfflineOnly(on: Boolean) = localWrite("offline.onlyDownloaded", on.toString())

    fun setCacheLimit(bytes: Long) = localWrite("cache.limit", bytes.toString())

    fun setPrefetch(on: Boolean) = localWrite("cache.prefetch", on.toString())

    fun setPrefetchCount(count: Int) = localWrite("cache.prefetchCount", count.coerceIn(1, 10).toString())

    /**
     * One local write: the value applies in [SettingsData] BEFORE the store is
     * touched, so the switch and the whole app move in the same frame; a
     * refused key is the only thing that can fail, and it raises its cause.
     */
    private fun localWrite(key: String, value: String) {
        val before = state.settings ?: return
        val next = before.with(key, value)
        if (next == null) {
            toast("Réglage inconnu : $key")
            return
        }
        state = state.copy(settings = next)
        NzAppearance.apply(next)
        scope.launch {
            val written = withContext(io) { runCatching { service.set(key, value) } }
            if (written.isFailure) {
                state = state.copy(settings = before)
                NzAppearance.apply(before)
                toast(NzErrors.text(written.exceptionOrNull()!!))
            }
        }
    }

    // --- Assistance -------------------------------------------------------

    /** *Partager le diagnostic*: the report is built ONCE, then handed to a share sheet. */
    suspend fun buildReport(): String? {
        val report = withContext(io) { runCatching { service.diagnosticReport() } }
        report.exceptionOrNull()?.let { toast(NzErrors.text(it)) }
        val text = report.getOrNull()
        state = state.copy(report = text)
        return text
    }

    // --- the plumbing -----------------------------------------------------

    /** One server write: [block] sends it, and a failure is a toast with its cause. */
    private fun serverWrite(block: suspend () -> Unit) {
        if (state.busy) return
        state = state.copy(busy = true)
        scope.launch {
            try {
                block()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast(NzErrors.text(e))
            } finally {
                state = state.copy(busy = false)
            }
        }
    }

    /** `screenFlow`'s answer, as one card's [Section]. */
    private fun <T> ScreenState<T>.asSection(): Section<T> = when (this) {
        is ScreenState.Loading -> Section(loading = true)
        is ScreenState.Ready -> Section(data = data, stale = stale)
        is ScreenState.Failed -> Section(failure = cause)
    }

    companion object {
        /** `lib/discovery.js#SETTLE_MS`: one request per drag, not per pixel. */
        const val SETTLE_MS = 450L

        /** `MyAccounts.svelte#schedule`: how often a running import is read. */
        const val IMPORT_POLL_MS = 1500L
    }
}

/** A consent URL the platform's browser must open (Spotify's PKCE door). */
data class ServiceLink(val url: String)

/**
 * This device's settings with ONE key replaced, or `null` when [key] is not one
 * this crate stores (`SettingsCopy`'s table and the core's whitelist agree;
 * a mismatch must never silently drop a tap).
 */
fun SettingsData.with(key: String, json: String): SettingsData? {
    fun bool() = json.toBooleanStrictOrNull()
    fun number() = json.toDoubleOrNull()
    fun text() = json.trim('"')
    return when (key) {
        "ui.theme" -> if (text() in listOf("encre", "papier", "pochette")) copy(theme = text()) else null
        "ui.accent" -> if (json == "null") copy(accent = null) else copy(accent = text())
        "viz.eco" -> bool()?.let { copy(eco = it) }
        "viz.mode" -> copy(vizMode = text())
        "viz.palette" -> copy(vizPalette = text())
        "viz.intensity" -> number()?.let { copy(vizIntensity = it) }
        "viz.fps" -> number()?.let { copy(vizFps = it.toUInt()) }
        "viz.beat" -> bool()?.let { copy(vizBeat = it) }
        "viz.scope.orientation" -> copy(vizScopeOrientation = text())
        "viz.scope.colour" -> copy(vizScopeColour = text())
        "player.quality" -> copy(quality = text())
        "offline.quality" -> copy(downloadQuality = text())
        "offline.onlyDownloaded" -> bool()?.let { copy(offlineOnly = it) }
        "cache.limit" -> json.toLongOrNull()?.let { copy(cacheLimit = it.toULong()) }
        "cache.prefetch" -> bool()?.let { copy(prefetch = it) }
        "cache.prefetchCount" -> number()?.let { copy(prefetchCount = it.toUInt()) }
        else -> null
    }
}

/** The sources as they will be once [order] is saved — the optimistic card. */
fun SettingsModel.Section<SourcesData>.previews(order: List<String>): SettingsModel.Section<SourcesData> {
    val data = data ?: return this
    val ranked = order.withIndex().associate { (index, id) -> id to index }
    val sources = data.sources.map { source ->
        val index = ranked[source.id] ?: return@map source.copy(place = "off", rank = null)
        if (index == 0) source.copy(place = "main", rank = 0L) else source.copy(place = "fallback", rank = index.toLong())
    }.sortedBy { ranked[it.id] ?: Int.MAX_VALUE }
    return copy(data = data.copy(main = order.firstOrNull(), sources = sources))
}
