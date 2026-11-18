package fr.neezer.mobile.feature.settings

import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.graphics.Color
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzThemeKind
import uniffi.nz_ffi.AccessData
import uniffi.nz_ffi.AccountsData
import uniffi.nz_ffi.DeezerStatusData
import uniffi.nz_ffi.GrantData
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.PersonData
import uniffi.nz_ffi.ServerSettingsData
import uniffi.nz_ffi.ServicesData
import uniffi.nz_ffi.SettingsApi
import uniffi.nz_ffi.SettingsData
import uniffi.nz_ffi.SourcesData
import uniffi.nz_ffi.Versions

/**
 * What Réglages asks of the core (lot MB16).
 *
 * An interface, not the generated `SettingsApi`, so the screen's logic is plain
 * Kotlin that `SettingsModelTest` runs on the JVM with a fake — no JNA, no
 * `.so`, no device. [NzSettingsService] is the one implementation, a one-to-one
 * delegation to `nz-ffi/src/settings.rs`.
 */
interface SettingsService {
    /** This device's settings — disk only, no request (the shell reads it at launch). */
    fun settings(): SettingsData

    /** Writes one local key (its value as JSON text, validated by the core). */
    suspend fun set(key: String, value: String)

    /** The two versions Aide shows. */
    fun versions(): Versions

    /** What *Assistance* shares. */
    fun diagnosticReport(): String

    // -- sources ------------------------------------------------------------
    fun cachedSources(): SourcesData?
    suspend fun sources(): SourcesData
    suspend fun saveSourcesOrder(order: List<String>): SourcesData
    suspend fun setSourceEnabled(id: String, on: Boolean): SourcesData

    // -- my accounts --------------------------------------------------------
    fun cachedAccounts(): AccountsData?
    suspend fun accounts(): AccountsData
    suspend fun linkDeezer(arl: String, mode: String?): AccountsData
    suspend fun unlinkAccount(id: String): AccountsData
    suspend fun setAccountAuto(id: String, kinds: List<String>, on: Boolean): AccountsData
    suspend fun setDefaultAccount(id: String?): AccountsData
    suspend fun importAccount(id: String, kinds: List<String>): AccountsData

    // -- who may use what ---------------------------------------------------
    fun cachedAccess(): AccessData?
    suspend fun access(): AccessData
    suspend fun setGrants(provider: String, account: String, feature: String, subjects: List<String>): List<GrantData>
    suspend fun createGroup(name: String): String
    suspend fun setGroupMembers(id: String, members: List<String>): List<PersonData>
    suspend fun deleteGroup(id: String)

    // -- export services ----------------------------------------------------
    fun cachedServices(): ServicesData?
    suspend fun services(): ServicesData
    suspend fun disconnectService(id: String): ServicesData
    suspend fun setSpotifyApp(clientId: String): ServicesData
    suspend fun spotifyAuthorize(): String

    // -- the server's own settings (admin only) -----------------------------
    fun cachedServerSettings(): ServerSettingsData?
    suspend fun serverSettings(): ServerSettingsData
    suspend fun saveUploadQuota(gb: Double): ServerSettingsData
    suspend fun saveDeezerArl(arl: String): ServerSettingsData
    suspend fun clearDeezerArl(): ServerSettingsData
    fun cachedDeezerStatus(): DeezerStatusData?
    suspend fun deezerStatus(force: Boolean): DeezerStatusData

    // -- "Je n'aime pas" ----------------------------------------------------
    fun cachedDislikes(): ListHandle?
    suspend fun dislikes(): ListHandle
    suspend fun setDisliked(rowId: String, on: Boolean)
}

/** The one real implementation: this lot's `nz-ffi` methods. */
class NzSettingsService(private val api: SettingsApi) : SettingsService {
    override fun settings(): SettingsData = api.settings()

    override suspend fun set(key: String, value: String) = api.set(key, value)

    override fun versions(): Versions = api.versions()

    override fun diagnosticReport(): String = api.diagnosticReport()

    override fun cachedSources(): SourcesData? = api.cachedSources()

    override suspend fun sources(): SourcesData = api.sources()

    override suspend fun saveSourcesOrder(order: List<String>): SourcesData = api.saveSourcesOrder(order)

    override suspend fun setSourceEnabled(id: String, on: Boolean): SourcesData = api.setSourceEnabled(id, on)

    override fun cachedAccounts(): AccountsData? = api.cachedAccounts()

    override suspend fun accounts(): AccountsData = api.accounts()

    override suspend fun linkDeezer(arl: String, mode: String?): AccountsData = api.linkDeezer(arl, mode)

    override suspend fun unlinkAccount(id: String): AccountsData = api.unlinkAccount(id)

    override suspend fun setAccountAuto(id: String, kinds: List<String>, on: Boolean): AccountsData =
        api.setAccountAuto(id, kinds, on)

    override suspend fun setDefaultAccount(id: String?): AccountsData = api.setDefaultAccount(id)

    override suspend fun importAccount(id: String, kinds: List<String>): AccountsData =
        api.importAccount(id, kinds)

    override fun cachedAccess(): AccessData? = api.cachedAccess()

    override suspend fun access(): AccessData = api.access()

    override suspend fun setGrants(
        provider: String,
        account: String,
        feature: String,
        subjects: List<String>,
    ): List<GrantData> = api.setGrants(provider, account, feature, subjects)

    override suspend fun createGroup(name: String): String = api.createGroup(name)

    override suspend fun setGroupMembers(id: String, members: List<String>): List<PersonData> =
        api.setGroupMembers(id, members)

    override suspend fun deleteGroup(id: String) = api.deleteGroup(id)

    override fun cachedServices(): ServicesData? = api.cachedServices()

    override suspend fun services(): ServicesData = api.services()

    override suspend fun disconnectService(id: String): ServicesData = api.disconnectService(id)

    override suspend fun setSpotifyApp(clientId: String): ServicesData = api.setSpotifyApp(clientId)

    override suspend fun spotifyAuthorize(): String = api.spotifyAuthorize()

    override fun cachedServerSettings(): ServerSettingsData? = api.cachedServerSettings()

    override suspend fun serverSettings(): ServerSettingsData = api.serverSettings()

    override suspend fun saveUploadQuota(gb: Double): ServerSettingsData = api.saveUploadQuota(gb)

    override suspend fun saveDeezerArl(arl: String): ServerSettingsData = api.saveDeezerArl(arl)

    override suspend fun clearDeezerArl(): ServerSettingsData = api.clearDeezerArl()

    override fun cachedDeezerStatus(): DeezerStatusData? = api.cachedDeezerStatus()

    override suspend fun deezerStatus(force: Boolean): DeezerStatusData = api.deezerStatus(force)

    override fun cachedDislikes(): ListHandle? = api.cachedDislikes()

    override suspend fun dislikes(): ListHandle = api.dislikes()

    override suspend fun setDisliked(rowId: String, on: Boolean) = api.setDisliked(rowId, on)
}

/**
 * The app's settings service, or `null` when the native core could not be built
 * (Aide says so instead of the app crashing).
 */
object AppSettings {
    fun api(): SettingsApi? = runCatching { NzBridge.core?.settings() }.getOrNull()

    fun service(): SettingsService? = api()?.let { NzSettingsService(it) }
}

/**
 * What the shell paints BEFORE the first frame, and whenever Réglages changes it.
 *
 * This is the one seam between Réglages and `:app`'s `NzTheme` (the note's MB16
 * section says so, and `MainActivity` reads it): the theme, the person's accent,
 * Pochette's sleeve colour and the eco switch. It is a Compose state, so a
 * change in Réglages repaints every screen at once — no restart, no reload, the
 * SPA's `data-theme` attribute rule ported to one object.
 *
 * **Why it is loaded synchronously, once, at launch**: a person who chose Papier
 * must not see Encre for a frame. The read is ONE round of `SELECT`s on the
 * already-open WAL store (`SettingsApi::settings`: 197.6 µs for its 17 keys,
 * measured in `nz-core`, MB16's number), executed in `MainActivity.onCreate`
 * before `setContent` — never during a frame, and never on a network call.
 */
object NzAppearance {
    /** One resolved appearance: the four values `NzTheme` takes. */
    data class Appearance(
        val theme: NzThemeKind = NzThemeKind.Encre,
        val accent: Color? = null,
        val coverAccent: Color? = null,
        val motion: NzMotionLevel? = null,
        /** The person's accent as they typed it (`#RRGGBB`), `null` for the theme's. */
        val accentHex: String? = null,
    ) {
        /** The accent Pochette paints: the sleeve, or the designed fallback. */
        val sleeve: Color get() = coverAccent ?: P0CHETTE_FALLBACK
    }

    /** `lib/theme.js#ACCENT_FALLBACK`, the colour a sleeve-less title paints. */
    val P0CHETTE_FALLBACK: Color = Color(0xFFFF6B4A)

    private val state = mutableStateOf(Appearance())

    /** What `NzTheme` reads. */
    val current: State<Appearance> get() = state

    /**
     * Reads the device's settings ONCE. Called by `MainActivity.onCreate` before
     * the first frame; `null` (no core) keeps the defaults, which is Encre.
     */
    fun load(service: SettingsService?) {
        if (service == null) return
        state.value = runCatching { of(service.settings()) }.getOrElse { Appearance() }
    }

    /**
     * Applies what Réglages just saved. Called from the model after every write,
     * so the switch lands in the same frame as the tap.
     */
    fun apply(settings: SettingsData) {
        state.value = of(settings)
    }

    /** Sets the sleeve colour Pochette paints (MB11's watcher calls this). */
    fun applyCoverAccent(hex: String?) {
        state.value = state.value.copy(coverAccent = hex?.let(::colourOf))
    }

    private fun of(settings: SettingsData): Appearance = Appearance(
        theme = themeKindOf(settings.theme),
        accent = settings.accent?.let(::colourOf),
        coverAccent = settings.coverAccent?.let(::colourOf),
        // `viz.eco` is the SPA's eco switch, and it means NO animation anywhere
        // (`lib/motion.js`, transitions.md): `NzMotionLevel.Off` is that value.
        motion = if (settings.eco) NzMotionLevel.Off else null,
        accentHex = settings.accent,
    )
}

/** `ui.theme`'s three names, as the design system's enum. */
fun themeKindOf(name: String?): NzThemeKind = when (name) {
    "papier" -> NzThemeKind.Papier
    "pochette" -> NzThemeKind.Pochette
    else -> NzThemeKind.Encre
}

/** `#RRGGBB` (the only shape the store accepts) as a Compose colour. */
fun colourOf(hex: String): Color? {
    val digits = hex.trim().removePrefix("#")
    if (digits.length != 6) return null
    val value = digits.toLongOrNull(16) ?: return null
    return Color(0xFF000000 or value)
}

