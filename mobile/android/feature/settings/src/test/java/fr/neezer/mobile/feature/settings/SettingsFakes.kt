package fr.neezer.mobile.feature.settings

import kotlinx.coroutines.CompletableDeferred
import uniffi.nz_ffi.AccessData
import uniffi.nz_ffi.AccessAccountData
import uniffi.nz_ffi.AccessData as Access
import uniffi.nz_ffi.AccountData
import uniffi.nz_ffi.AccountsData
import uniffi.nz_ffi.DeezerCard
import uniffi.nz_ffi.DeezerStatusData
import uniffi.nz_ffi.FeatureData
import uniffi.nz_ffi.GrantData
import uniffi.nz_ffi.GroupData
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.PersonData
import uniffi.nz_ffi.ProviderData
import uniffi.nz_ffi.ServerSettingsData
import uniffi.nz_ffi.ServiceData
import uniffi.nz_ffi.ServicesData
import uniffi.nz_ffi.SettingsData
import uniffi.nz_ffi.SourceData
import uniffi.nz_ffi.SourcesData
import uniffi.nz_ffi.Versions

/**
 * The screen's fixtures: the SPA's own shapes, one of each, so a test and a
 * screenshot draw what a real server sends (`sourceapi.py#sources_payload`,
 * `myaccounts.py`, `accessapi.py`, `services.py`).
 */
object SettingsFixtures {
    fun settings(
        theme: String = "encre",
        accent: String? = null,
        eco: Boolean = false,
        vizMode: String = "bars",
        downloadQuality: String = "OPUS_320",
        quality: String = "FLAC",
        prefetch: Boolean = true,
        prefetchCount: Int = 1,
        cacheLimit: Long = 1024L * 1024 * 1024,
        offlineOnly: Boolean = true,
        vizIntensity: Double = 0.7,
        vizFps: Int = 60,
    ) = SettingsData(
        theme = theme,
        accent = accent,
        coverAccent = null,
        eco = eco,
        vizMode = vizMode,
        vizPalette = "cover",
        vizIntensity = vizIntensity,
        vizFps = vizFps.toUInt(),
        vizBeat = true,
        vizScopeOrientation = "horizontal",
        vizScopeColour = "duo",
        quality = quality,
        downloadQuality = downloadQuality,
        offlineOnly = offlineOnly,
        cacheLimit = cacheLimit.toULong(),
        prefetch = prefetch,
        prefetchCount = prefetchCount.toUInt(),
    )

    fun source(
        id: String,
        name: String,
        place: String,
        rank: Long? = null,
        enabled: Boolean = true,
        configured: Boolean = true,
        canLead: Boolean = false,
        quality: String = "",
        lossless: Boolean = false,
        resting: Int = 0,
        restingWhy: String = "",
        setup: String = "",
    ) = SourceData(
        id = id,
        name = name,
        place = place,
        rank = rank,
        enabled = enabled,
        configured = configured,
        canLead = canLead,
        switchable = true,
        qualityLabel = quality,
        lossless = lossless,
        restingS = resting.toUInt(),
        restingWhy = restingWhy,
        setup = setup,
    )

    fun sources(admin: Boolean = true) = SourcesData(
        admin = admin,
        main = "deezer",
        sources = listOf(
            source("deezer", "Deezer", "main", quality = "FLAC", lossless = true, canLead = true),
            source("youtube", "YouTube", "fallback", rank = 1, quality = "Opus 160", resting = 120, restingWhy = "n'a pas répondu"),
            source("qobuz", "Qobuz", "off", enabled = false, configured = false, canLead = true, setup = "Ajoutez vos identifiants Qobuz."),
        ),
    )

    fun account(
        id: String = "acc-1",
        provider: String = "deezer",
        name: String = "Deezer",
        isDefault: Boolean = true,
        status: String = "ok",
        runningPhase: String? = null,
        lastAt: String? = "2026-11-17T10:00:00Z",
    ) = AccountData(
        id = id,
        provider = provider,
        name = name,
        label = "nadia",
        server = false,
        status = status,
        isDefault = isDefault,
        kinds = listOf("artists", "tracks"),
        autoOn = listOf("tracks"),
        runningPhase = runningPhase,
        runningKind = if (runningPhase == "import") "tracks" else null,
        runningDone = if (runningPhase == "import") 40u else 0u,
        runningTotal = if (runningPhase == "import") 120u else 0u,
        lastAt = lastAt,
        lastReason = null,
        lastError = null,
        lastAdded = emptyList(),
        lastKept = emptyList(),
        lossless = true,
        error = null,
    )

    fun accounts(accounts: List<AccountData> = listOf(account())) = AccountsData(
        accounts = accounts,
        canLink = listOf("youtube"),
        default = accounts.firstOrNull { it.isDefault }?.id,
    )

    fun access(admin: Boolean = true) = AccessData(
        admin = admin,
        users = listOf(PersonData("u2", "Léo")),
        groups = listOf(GroupData("g1", "Maison", listOf(PersonData("u2", "Léo")))),
        providers = listOf(
            ProviderData(
                id = "deezer",
                name = "Deezer",
                role = "main",
                accounts = listOf(
                    AccessAccountData(
                        id = "acc-1",
                        mode = "server",
                        label = "serveur",
                        server = true,
                        owner = null,
                        mine = true,
                        editable = true,
                        grants = listOf(GrantData("*", listOf("search", "stream"))),
                    ),
                ),
                allowed = listOf("search", "stream"),
                anonymousGrants = emptyList(),
                anonymousEditable = false,
                features = listOf(FeatureData("search", "shared"), FeatureData("stream", "shared")),
            ),
        ),
    )

    fun services() = ServicesData(
        services = listOf(
            ServiceData("deezer", "Deezer", linked = false, ready = true, account = null, file = false, clientId = null),
            ServiceData("spotify", "Spotify", linked = false, ready = false, account = null, file = false, clientId = ""),
            ServiceData("youtube", "YouTube Music", linked = true, ready = true, account = "nadia", file = false, clientId = null),
        ),
        formats = listOf("M3U", "CSV (ISRC)"),
    )

    fun serverSettings(quota: Double = 12.5) = ServerSettingsData(
        uploadQuotaGb = quota,
        deezer = DeezerCard(
            arlSet = true,
            arlSource = "database",
            arlHint = "…a1b2",
            enabled = true,
            archiveDirSet = true,
        ),
    )

    fun deezerStatus(ok: Boolean = true) = DeezerStatusData(
        ok = ok,
        reason = if (ok) null else "arl",
        message = null,
        admin = true,
    )

    fun versions() = Versions(app = "0.1.0", server = "9.9.9")
}

/**
 * The core, faked: every call counted, every answer replaceable, and one gate
 * that holds a write so a test can read the state BEFORE the store answered
 * (the optimistic rule) without a clock.
 */
class FakeSettingsService(
    var settings: SettingsData = SettingsFixtures.settings(),
    var sources: SourcesData = SettingsFixtures.sources(),
    var accounts: AccountsData = SettingsFixtures.accounts(),
    var access: AccessData = SettingsFixtures.access(),
    var services: ServicesData = SettingsFixtures.services(),
    var server: ServerSettingsData? = SettingsFixtures.serverSettings(),
    var deezer: DeezerStatusData? = SettingsFixtures.deezerStatus(),
) : SettingsService {
    /** What was asked, by route — the acceptance counts these. */
    val calls = mutableListOf<String>()

    /** Failure to raise for one call, by route (`"sources"`, `"set"`, …). */
    val failures = mutableMapOf<String, Throwable>()

    /** When set, a local write waits on it: the optimistic state is observable. */
    var writeGate: CompletableDeferred<Unit>? = null

    /** When set, a server READ waits on it: the kept copy is observable first. */
    var readGate: CompletableDeferred<Unit>? = null

    /** The values written, in order, for a test that reads what travelled. */
    val writes = mutableListOf<Pair<String, String>>()

    private fun record(route: String) {
        calls.add(route)
        failures[route]?.let { throw it }
    }

    override fun settings(): SettingsData = settings

    override suspend fun set(key: String, value: String) {
        // The gate FIRST: a test reads the optimistic state while the store is
        // still thinking, and the failure (if any) lands after it.
        writeGate?.await()
        record("set")
        writes.add(key to value)
        settings = settings.with(key, value) ?: settings
    }

    override fun versions(): Versions = SettingsFixtures.versions()

    override fun diagnosticReport(): String {
        record("diagnosticReport")
        return "Neezer — diagnostic\napplication : 0.1.0\n" +
            "mémoire comptée : file 0 o, listes 0 o\nrebuffers : 0\n" +
            "requêtes par écran :\n"
    }

    override fun cachedSources(): SourcesData? = sourcesCache

    override suspend fun sources(): SourcesData {
        record("sources")
        readGate?.await()
        return sources
    }

    /** The server's own re-placing (`sourcesapi.py#sources_put` + a re-read). */
    override suspend fun saveSourcesOrder(order: List<String>): SourcesData {
        record("saveSourcesOrder")
        val ranked = order.withIndex().associate { (index, id) -> id to index }
        sources = sources.copy(
            main = order.firstOrNull(),
            sources = sources.sources
                .map { source ->
                    val index = ranked[source.id] ?: return@map source.copy(place = "off", rank = null)
                    if (index == 0) {
                        source.copy(place = "main", rank = 0L)
                    } else {
                        source.copy(place = "fallback", rank = index.toLong())
                    }
                }
                .sortedBy { ranked[it.id] ?: Int.MAX_VALUE },
        )
        return sources
    }

    override suspend fun setSourceEnabled(id: String, on: Boolean): SourcesData {
        record("setSourceEnabled")
        sources = sources.copy(sources = sources.sources.map { if (it.id == id) it.copy(enabled = on) else it })
        return sources
    }

    override fun cachedAccounts(): AccountsData? = accountsCache

    override suspend fun accounts(): AccountsData {
        record("accounts")
        return accounts
    }

    override suspend fun linkDeezer(arl: String, mode: String?): AccountsData {
        record("linkDeezer")
        return accounts
    }

    override suspend fun unlinkAccount(id: String): AccountsData {
        record("unlinkAccount")
        return accounts
    }

    override suspend fun setAccountAuto(id: String, kinds: List<String>, on: Boolean): AccountsData {
        record("setAccountAuto")
        return accounts
    }

    override suspend fun setDefaultAccount(id: String?): AccountsData {
        record("setDefaultAccount")
        accounts = accounts.copy(default = id, accounts = accounts.accounts.map { it.copy(isDefault = it.id == id) })
        return accounts
    }

    override suspend fun importAccount(id: String, kinds: List<String>): AccountsData {
        record("importAccount")
        accounts = accounts.copy(accounts = accounts.accounts.map { it.copy(runningPhase = "import") })
        return accounts
    }

    override fun cachedAccess(): AccessData? = accessCache

    override suspend fun access(): AccessData {
        record("access")
        return access
    }

    override suspend fun setGrants(
        provider: String,
        account: String,
        feature: String,
        subjects: List<String>,
    ): List<GrantData> {
        record("setGrants")
        return subjects.map { GrantData(it, listOf(feature)) }
    }

    override suspend fun createGroup(name: String): String {
        record("createGroup")
        return "g2"
    }

    override suspend fun setGroupMembers(id: String, members: List<String>): List<PersonData> {
        record("setGroupMembers")
        return members.map { PersonData(it, it) }
    }

    override suspend fun deleteGroup(id: String) {
        record("deleteGroup")
    }

    override fun cachedServices(): ServicesData? = servicesCache

    override suspend fun services(): ServicesData {
        record("services")
        return services
    }

    override suspend fun disconnectService(id: String): ServicesData {
        record("disconnectService")
        services = services.copy(services = services.services.map { if (it.id == id) it.copy(linked = false) else it })
        return services
    }

    override suspend fun setSpotifyApp(clientId: String): ServicesData {
        record("setSpotifyApp")
        return services
    }

    override suspend fun spotifyAuthorize(): String {
        record("spotifyAuthorize")
        return "https://accounts.spotify.com/authorize?x=1"
    }

    override fun cachedServerSettings(): ServerSettingsData? = serverCache

    override suspend fun serverSettings(): ServerSettingsData {
        record("serverSettings")
        return server ?: throw IllegalStateException("403")
    }

    override suspend fun saveUploadQuota(gb: Double): ServerSettingsData {
        record("saveUploadQuota")
        server = SettingsFixtures.serverSettings(gb)
        return server!!
    }

    override suspend fun saveDeezerArl(arl: String): ServerSettingsData {
        record("saveDeezerArl")
        return server!!
    }

    override suspend fun clearDeezerArl(): ServerSettingsData {
        record("clearDeezerArl")
        return server!!
    }

    override fun cachedDeezerStatus(): DeezerStatusData? = deezerCache

    override suspend fun deezerStatus(force: Boolean): DeezerStatusData {
        record("deezerStatus")
        return deezer ?: throw IllegalStateException("hors ligne")
    }

    override fun cachedDislikes(): ListHandle? = null

    override suspend fun dislikes(): ListHandle {
        record("dislikes")
        throw IllegalStateException("un handle ne se fabrique pas sur la JVM")
    }

    override suspend fun setDisliked(rowId: String, on: Boolean) {
        record("setDisliked")
    }

    /** The device's copies a warm open paints from (the SPA's `localStorage`). */
    var sourcesCache: SourcesData? = null
    var accountsCache: AccountsData? = null
    var accessCache: AccessData? = null
    var servicesCache: ServicesData? = null
    var serverCache: ServerSettingsData? = null
    var deezerCache: DeezerStatusData? = null

    /** After one successful read, the device has a copy (a second visit paints it). */
    fun keepCopies() {
        sourcesCache = sources
        accountsCache = accounts
        accessCache = access
        servicesCache = services
        serverCache = server
        deezerCache = deezer
    }
}
