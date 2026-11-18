package fr.neezer.mobile.feature.admin

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.AdminAnalysisJob
import uniffi.nz_ffi.AdminArchiveStatus
import uniffi.nz_ffi.AdminAutomatch
import uniffi.nz_ffi.AdminCleanup
import uniffi.nz_ffi.AdminCleanupRun
import uniffi.nz_ffi.AdminCredits
import uniffi.nz_ffi.AdminDeleteResult
import uniffi.nz_ffi.AdminEnrich
import uniffi.nz_ffi.AdminIdentity
import uniffi.nz_ffi.AdminInviteIssued
import uniffi.nz_ffi.AdminInviteRow
import uniffi.nz_ffi.AdminMigrate
import uniffi.nz_ffi.AdminPlan
import uniffi.nz_ffi.AdminRules
import uniffi.nz_ffi.AdminServerCard
import uniffi.nz_ffi.AdminSimilar
import uniffi.nz_ffi.AdminStorage
import uniffi.nz_ffi.AdminTranscribe
import uniffi.nz_ffi.AdminUserDetail
import uniffi.nz_ffi.AdminUserRow
import uniffi.nz_ffi.AdminUsersPage

/**
 * Ce que les pages d'administration demandent au noyau (lot MB17).
 *
 * Une interface, pas le `Screens` généré : la logique de l'écran est du Kotlin
 * ordinaire que `AdminModelTest` exécute sur la JVM avec un faux — ni JNA, ni
 * `.so`, ni appareil. [NzAdminService] est la seule implémentation, une
 * délégation d'une ligne à `nz-ffi/src/admin.rs` (`impl Screens`).
 *
 * Chaque carte suit la même paire : `cachedX()` (le disque seul, pour la
 * première image) puis `x()` (le réseau). `screenFlow` les enchaîne.
 */
interface AdminService {
    /** `/api/me`'s `user.admin`, lu de la session : **zéro requête**. */
    fun identity(): AdminIdentity?

    /**
     * `GET /api/me` : relit la session DANS le noyau. Une requête, et seulement
     * quand un écran n'a trouvé aucune session en mémoire (un lancement hors
     * ligne la laisse vide) — c'est ainsi que les catégories d'administration
     * reviennent quand le réseau revient. Hors ligne : `false`, zéro socket.
     */
    suspend fun restoreSession(): Boolean

    // -- les comptes --------------------------------------------------------
    fun cachedUsers(): AdminUsersPage?
    suspend fun users(q: String, after: String?, limit: UInt): AdminUsersPage
    suspend fun user(id: String): AdminUserDetail
    suspend fun setRole(id: String, on: Boolean): AdminUserRow
    suspend fun setDisabled(id: String, on: Boolean): AdminUserRow
    suspend fun deleteUser(id: String, name: String): AdminDeleteResult

    // -- les invitations ----------------------------------------------------
    fun cachedInvites(): List<AdminInviteRow>?
    suspend fun invites(state: String?): List<AdminInviteRow>
    suspend fun createInvite(note: String?, expiresIn: UInt): AdminInviteIssued
    suspend fun renewInvite(id: String): AdminInviteIssued
    suspend fun revokeInvite(id: String)

    // -- le serveur ---------------------------------------------------------
    fun cachedStorage(): AdminStorage?
    suspend fun storage(): AdminStorage
    fun cachedServerCard(): AdminServerCard?
    suspend fun serverCard(): AdminServerCard
    suspend fun testDeezer(force: Boolean): AdminServerCard
    suspend fun saveQuota(gb: Double): AdminServerCard
    suspend fun flushCache(): ULong

    // -- l'archive ----------------------------------------------------------
    fun cachedRules(): AdminRules?
    suspend fun rules(): AdminRules
    suspend fun saveRule(field: String, value: String): AdminRules
    fun cachedCleanup(): AdminCleanup?
    suspend fun cleanup(): AdminCleanup
    suspend fun runCleanup(): AdminCleanupRun
    fun cachedArchiveStatus(): AdminArchiveStatus?
    suspend fun archiveStatus(): AdminArchiveStatus
    suspend fun startArchive(scope: String): AdminArchiveStatus
    fun cachedMigrate(): AdminMigrate?
    suspend fun migrate(): AdminMigrate
    suspend fun startMigrate(force: Boolean): AdminMigrate

    // -- l'analyse ----------------------------------------------------------
    fun cachedPlan(): AdminPlan?
    suspend fun plan(): AdminPlan
    fun cachedAnalysis(): AdminAnalysisJob?
    suspend fun analysis(): AdminAnalysisJob
    suspend fun startAnalysis(force: Boolean, workers: UInt): AdminAnalysisJob
    suspend fun stopAnalysis(): AdminAnalysisJob
    suspend fun saveSchedule(enabled: Boolean, at: String, hours: UInt): AdminPlan
    suspend fun saveAnalysisPlace(place: String): AdminPlan

    // -- les rattrapages ----------------------------------------------------
    fun cachedEnrich(): AdminEnrich?
    suspend fun enrich(): AdminEnrich
    suspend fun startEnrich(force: Boolean): AdminEnrich
    fun cachedCredits(): AdminCredits?
    suspend fun credits(): AdminCredits
    suspend fun startCredits(force: Boolean): AdminCredits
    fun cachedSimilar(): AdminSimilar?
    suspend fun similar(): AdminSimilar
    suspend fun startSimilar(force: Boolean): AdminSimilar

    // -- le remplacement automatique ---------------------------------------
    fun cachedAutomatch(): AdminAutomatch?
    suspend fun automatch(): AdminAutomatch
    suspend fun setAutomatch(on: Boolean): AdminAutomatch
    suspend fun runAutomatch(): AdminAutomatch
    suspend fun cancelAutomatch(): AdminAutomatch

    // -- la transcription ---------------------------------------------------
    fun cachedTranscribe(): AdminTranscribe?
    suspend fun transcribe(probe: Boolean): AdminTranscribe
    suspend fun saveTranscribe(field: String, value: String): AdminTranscribe
}

/** La seule implémentation réelle : les méthodes de ce lot dans `nz-ffi`. */
class NzAdminService(private val screens: uniffi.nz_ffi.Screens) : AdminService {
    override fun identity(): AdminIdentity? = screens.adminIdentity()

    override suspend fun restoreSession(): Boolean = screens.adminRestoreSession()

    override fun cachedUsers(): AdminUsersPage? = screens.cachedAdminUsers()

    override suspend fun users(q: String, after: String?, limit: UInt): AdminUsersPage =
        screens.adminUsers(q, after, limit)

    override suspend fun user(id: String): AdminUserDetail = screens.adminUser(id)

    override suspend fun setRole(id: String, on: Boolean): AdminUserRow = screens.adminSetRole(id, on)

    override suspend fun setDisabled(id: String, on: Boolean): AdminUserRow =
        screens.adminSetDisabled(id, on)

    override suspend fun deleteUser(id: String, name: String): AdminDeleteResult =
        screens.adminDeleteUser(id, name)

    override fun cachedInvites(): List<AdminInviteRow>? = screens.cachedAdminInvites()

    override suspend fun invites(state: String?): List<AdminInviteRow> = screens.adminInvites(state)

    override suspend fun createInvite(note: String?, expiresIn: UInt): AdminInviteIssued =
        screens.adminCreateInvite(note, expiresIn)

    override suspend fun renewInvite(id: String): AdminInviteIssued = screens.adminRenewInvite(id)

    override suspend fun revokeInvite(id: String) = screens.adminRevokeInvite(id)

    override fun cachedStorage(): AdminStorage? = screens.cachedAdminStorage()

    override suspend fun storage(): AdminStorage = screens.adminStorage()

    override fun cachedServerCard(): AdminServerCard? = screens.cachedAdminServerCard()

    override suspend fun serverCard(): AdminServerCard = screens.adminServerCard()

    override suspend fun testDeezer(force: Boolean): AdminServerCard = screens.adminTestDeezer(force)

    override suspend fun saveQuota(gb: Double): AdminServerCard = screens.adminSaveQuota(gb)

    override suspend fun flushCache(): ULong = screens.adminFlushCache()

    override fun cachedRules(): AdminRules? = screens.cachedAdminRules()

    override suspend fun rules(): AdminRules = screens.adminRules()

    override suspend fun saveRule(field: String, value: String): AdminRules =
        screens.adminSaveRule(field, value)

    override fun cachedCleanup(): AdminCleanup? = screens.cachedAdminCleanup()

    override suspend fun cleanup(): AdminCleanup = screens.adminCleanup()

    override suspend fun runCleanup(): AdminCleanupRun = screens.adminRunCleanup()

    override fun cachedArchiveStatus(): AdminArchiveStatus? = screens.cachedAdminArchiveStatus()

    override suspend fun archiveStatus(): AdminArchiveStatus = screens.adminArchiveStatus()

    override suspend fun startArchive(scope: String): AdminArchiveStatus =
        screens.adminStartArchive(scope)

    override fun cachedMigrate(): AdminMigrate? = screens.cachedAdminMigrate()

    override suspend fun migrate(): AdminMigrate = screens.adminMigrate()

    override suspend fun startMigrate(force: Boolean): AdminMigrate = screens.adminStartMigrate(force)

    override fun cachedPlan(): AdminPlan? = screens.cachedAdminPlan()

    override suspend fun plan(): AdminPlan = screens.adminPlan()

    override fun cachedAnalysis(): AdminAnalysisJob? = screens.cachedAdminAnalysis()

    override suspend fun analysis(): AdminAnalysisJob = screens.adminAnalysis()

    override suspend fun startAnalysis(force: Boolean, workers: UInt): AdminAnalysisJob =
        screens.adminStartAnalysis(force, workers)

    override suspend fun stopAnalysis(): AdminAnalysisJob = screens.adminStopAnalysis()

    override suspend fun saveSchedule(enabled: Boolean, at: String, hours: UInt): AdminPlan =
        screens.adminSaveSchedule(enabled, at, hours)

    override suspend fun saveAnalysisPlace(place: String): AdminPlan =
        screens.adminSaveAnalysisPlace(place)

    override fun cachedEnrich(): AdminEnrich? = screens.cachedAdminEnrich()

    override suspend fun enrich(): AdminEnrich = screens.adminEnrich()

    override suspend fun startEnrich(force: Boolean): AdminEnrich = screens.adminStartEnrich(force)

    override fun cachedCredits(): AdminCredits? = screens.cachedAdminCredits()

    override suspend fun credits(): AdminCredits = screens.adminCredits()

    override suspend fun startCredits(force: Boolean): AdminCredits = screens.adminStartCredits(force)

    override fun cachedSimilar(): AdminSimilar? = screens.cachedAdminSimilar()

    override suspend fun similar(): AdminSimilar = screens.adminSimilar()

    override suspend fun startSimilar(force: Boolean): AdminSimilar = screens.adminStartSimilar(force)

    override fun cachedAutomatch(): AdminAutomatch? = screens.cachedAdminAutomatch()

    override suspend fun automatch(): AdminAutomatch = screens.adminAutomatch()

    override suspend fun setAutomatch(on: Boolean): AdminAutomatch = screens.adminSetAutomatch(on)

    override suspend fun runAutomatch(): AdminAutomatch = screens.adminRunAutomatch()

    override suspend fun cancelAutomatch(): AdminAutomatch = screens.adminCancelAutomatch()

    override fun cachedTranscribe(): AdminTranscribe? = screens.cachedAdminTranscribe()

    override suspend fun transcribe(probe: Boolean): AdminTranscribe = screens.adminTranscribe(probe)

    override suspend fun saveTranscribe(field: String, value: String): AdminTranscribe =
        screens.adminSaveTranscribe(field, value)
}

/**
 * Le service des pages d'administration, ou `null` quand le noyau natif n'a pas
 * pu être chargé (l'écran le dit au lieu de planter).
 */
object AppAdmin {
    fun screens(): uniffi.nz_ffi.Screens? = runCatching { NzBridge.core?.screens() }.getOrNull()

    fun service(): AdminService? = screens()?.let { NzAdminService(it) }
}
