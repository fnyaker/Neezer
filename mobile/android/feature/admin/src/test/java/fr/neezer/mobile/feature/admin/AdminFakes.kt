package fr.neezer.mobile.feature.admin

import kotlinx.coroutines.CompletableDeferred
import uniffi.nz_ffi.AdminAnalysisJob
import uniffi.nz_ffi.AdminArchiveStatus
import uniffi.nz_ffi.AdminAutomatch
import uniffi.nz_ffi.AdminAutomatchMiss
import uniffi.nz_ffi.AdminCause
import uniffi.nz_ffi.AdminCleanup
import uniffi.nz_ffi.AdminCleanupRun
import uniffi.nz_ffi.AdminCleanupTrack
import uniffi.nz_ffi.AdminCount
import uniffi.nz_ffi.AdminCredits
import uniffi.nz_ffi.AdminDeleteResult
import uniffi.nz_ffi.AdminEnrich
import uniffi.nz_ffi.AdminFailureRow
import uniffi.nz_ffi.AdminIdentity
import uniffi.nz_ffi.AdminInviteIssued
import uniffi.nz_ffi.AdminInviteRow
import uniffi.nz_ffi.AdminJobFailures
import uniffi.nz_ffi.AdminMigrate
import uniffi.nz_ffi.AdminPlan
import uniffi.nz_ffi.AdminReplaced
import uniffi.nz_ffi.AdminRuleSet
import uniffi.nz_ffi.AdminRuleSwitch
import uniffi.nz_ffi.AdminRules
import uniffi.nz_ffi.AdminSchedule
import uniffi.nz_ffi.AdminServerCard
import uniffi.nz_ffi.AdminSimilar
import uniffi.nz_ffi.AdminStage
import uniffi.nz_ffi.AdminStorage
import uniffi.nz_ffi.AdminTranscribe
import uniffi.nz_ffi.AdminUserDetail
import uniffi.nz_ffi.AdminUserRow
import uniffi.nz_ffi.AdminUsersPage

/**
 * Les fixtures des pages d'administration, une de chaque, avec les formes du
 * VRAI serveur (`admin_users.py`, `storage.py`, `automatch.py`,
 * `transcribe.py`) : un écran dessiné sur une forme inventée ne prouve rien.
 */
object AdminFixtures {
    fun user(
        id: String = "u-2",
        name: String = "Léo",
        admin: Boolean = false,
        disabled: Boolean = false,
        lastLogin: String? = "2026-11-16T09:30:00",
        mail: String? = null,
    ) = AdminUserRow(
        id = id,
        name = name,
        admin = admin,
        disabled = disabled,
        lastLogin = lastLogin,
        mail = mail,
        onboarding = "done",
    )

    fun usersPage(
        users: List<AdminUserRow> = listOf(
            user(id = "u-1", name = "Nadia", admin = true),
            user(id = "u-2", name = "Léo"),
        ),
        total: UInt = 3u,
        next: String? = "u-2",
    ) = AdminUsersPage(users = users, total = total, next = next)

    fun invite(
        id: String = "i-old",
        note: String? = "Léa",
        expires: String? = "2099-11-23T09:00:00",
        state: String = "pending",
    ) = AdminInviteRow(
        id = id,
        note = note,
        created = "2099-11-16T09:00:00",
        expires = expires,
        state = state,
        createdBy = "Nadia",
        usedBy = null,
    )

    fun issued(id: String = "i-new") = AdminInviteIssued(
        invite = invite(id = id),
        url = "https://neezer.example/app/#/invite/tok-abc123",
        token = "tok-abc123",
    )

    fun userDetail(user: AdminUserRow = user()) = AdminUserDetail(user = user, uploads = 4u, playlists = 2u)

    fun deleteResult() = AdminDeleteResult(uploadsReassigned = 4u, podcastsReassigned = 1u)

    fun storage() = AdminStorage(
        archiveDir = "/srv/neezer/archive",
        archiveBytes = 987_654_321uL,
        cacheBytes = 10_485_760uL,
        cacheLimit = 536_870_912uL,
        transcodeBytes = 52_428_800uL,
        transcodeLimit = 268_435_456uL,
        diskFree = 300_000_000_000uL,
        diskTotal = 500_000_000_000uL,
        tracksArchived = 812u,
        tracksTotal = 900u,
    )

    fun serverCard(quota: Double = 12.5, ok: Boolean? = true) = AdminServerCard(
        uploadQuotaGb = quota,
        deezerEnabled = true,
        arlSet = true,
        arlSource = "database",
        arlHint = "…a1b2",
        statusOk = ok,
        statusReason = if (ok == true) null else "arl",
        statusMessage = null,
    )

    fun ruleSet(cleanOn: Boolean = true) = AdminRuleSet(
        artistScope = "releases",
        artistLimit = 3u,
        chapters = true,
        chapterUrls = false,
        cleanOn = cleanOn,
        cleanMinFreeGb = 20.0,
        cleanStaleDays = 180u,
        cleanOrder = "oldest_play",
        cleanKeepFav = true,
        cleanKeepPlaylist = true,
        cleanKeepPodcast = false,
    )

    fun rules(albumOn: Boolean = false) = AdminRules(
        events = listOf(
            AdminRuleSwitch(id = "on_play_context", on = true),
            AdminRuleSwitch(id = "on_fav_track", on = true),
            AdminRuleSwitch(id = "on_fav_album", on = albumOn),
            AdminRuleSwitch(id = "on_fav_playlist", on = true),
            AdminRuleSwitch(id = "on_fav_artist", on = true),
            AdminRuleSwitch(id = "on_playlist_add", on = false),
            AdminRuleSwitch(id = "on_podcast", on = true),
        ),
        rules = ruleSet(),
        artistScopes = listOf("all", "releases", "top"),
        cleanupOrders = listOf("oldest_play", "least_played", "largest", "oldest"),
        archiveLibrary = true,
    )

    fun cleanup(needed: ULong = 2_147_483_648uL) = AdminCleanup(
        enabled = true,
        running = false,
        eligible = 9u,
        wouldFree = 24_000_000uL,
        freeBytes = 1_000_000uL,
        neededBytes = needed,
        tracks = listOf(
            AdminCleanupTrack(title = "Vieille prod", artist = "Anonyme", bytes = 12_000_000uL),
            AdminCleanupTrack(title = "Autre vieille", artist = "Quelqu'un", bytes = 12_000_000uL),
        ),
    )

    fun cleanupRun() = AdminCleanupRun(deleted = 2u, freed = 24_000_000uL)

    fun archive(running: Boolean = false) = AdminArchiveStatus(
        running = running,
        scope = "all",
        total = 120u,
        done = if (running) 40u else 120u,
        archived = if (running) 38u else 118u,
        failed = 1u,
        unavailable = 1u,
        error = null,
    )

    fun migrate(running: Boolean = false) = AdminMigrate(
        running = running,
        done = 40u,
        failed = 0u,
        error = null,
        counts = listOf(AdminCount(name = "tags", count = 40u), AdminCount(name = "bpm", count = 12u)),
        causes = emptyList(),
        hidden = 0u,
    )

    fun failures() = AdminJobFailures(
        failed = 2u,
        causes = listOf(AdminCause(reason = "ffmpeg a disparu", count = 1u)),
        rows = listOf(AdminFailureRow(track = "a.flac", path = "/srv/a.flac", reason = "ffmpeg a disparu")),
        hidden = 1u,
    )

    fun plan(place: String = "server") = AdminPlan(
        place = place,
        total = 900u,
        todo = 30u,
        never = 10u,
        current = 860u,
        stages = listOf(
            AdminStage(id = "measure", late = 15u, off = false),
            AdminStage(id = "embed", late = 0u, off = true),
            AdminStage(id = "decide", late = 25u, off = false),
        ),
        extractor = false,
        schedule = AdminSchedule(enabled = true, at = "03:00", hours = 6u),
    )

    fun analysis(running: Boolean = true) = AdminAnalysisJob(
        running = running,
        scanned = if (running) 30u else 50u,
        done = if (running) 28u else 50u,
        total = 50u,
        stage = "measure",
        paused = if (running) "listening" else null,
        error = null,
        workers = 2u,
        workersSetting = 2u,
        workersAuto = 8u,
        force = false,
        failures = failures(),
    )

    fun enrich() = AdminEnrich(
        enabled = true,
        running = false,
        acoustid = true,
        found = listOf(AdminCount(name = "isrc", count = 9u)),
        tried = 12u,
        untried = 3u,
        error = null,
        failures = AdminJobFailures(failed = 0u, causes = emptyList(), rows = emptyList(), hidden = 0u),
    )

    fun credits(running: Boolean = false) = AdminCredits(
        running = running,
        done = 3u,
        failed = 0u,
        total = 50u,
        tracks = 7u,
        completed = true,
        error = null,
        sources = listOf(AdminCount(name = "deezer", count = 5u)),
        failures = failures(),
    )

    fun similar(running: Boolean = false) = AdminSimilar(running = running, tracks = 12u, at = null)

    fun automatch(running: Boolean = false, replaced: UInt = 2u) = AdminAutomatch(
        available = true,
        enabled = true,
        switchedOff = false,
        youtube = true,
        threshold = 0.99,
        running = running,
        total = if (running) 40u else null,
        done = if (running) 12u else 0u,
        matched = if (running) 3u else 0u,
        cancel = false,
        lastCounts = if (running) emptyList() else listOf(AdminCount(name = "match", count = replaced)),
        lastStopped = null,
        misses = listOf(
            AdminAutomatchMiss(
                title = "Perdu",
                artist = "Anonyme",
                reason = "nothing with the same title and artist",
                status = "no_candidate",
            ),
        ),
        recent = listOf(
            AdminReplaced(
                at = "2026-11-17T10:00:00",
                fromTitle = "Perdu",
                fromArtist = "Anonyme",
                toTitle = "Raretés",
                toSource = "youtube",
                score = 0.995,
            ),
        ),
    )

    fun transcribe(maxMinutes: UInt = 180u) = AdminTranscribe(
        available = true,
        enabled = true,
        runner = "server",
        reason = null,
        language = "fr",
        languages = listOf("auto", "fr", "en"),
        maxMinutes = maxMinutes,
        dailyMinutes = 60u,
        searchSegments = 100_000uL,
        threads = 4u,
        engineName = "whisper.cpp",
        engineVersion = "1.7.4",
        engineOk = true,
        modelName = "ggml-small.bin",
        modelSizeMb = 466u,
        modelOk = true,
        runningNow = 1u,
        queued = 3u,
        ready = 9u,
        errored = 2u,
        budgetLimitMs = 3_600_000uL,
        budgetUsedMs = 600_000uL,
    )
}

/**
 * The screen's port, with what a test needs to hold it still: every call is
 * COUNTED, any call can be made to fail, and the account page's answer can be
 * held at a gate so "the copy paints before the network" is an assertion rather
 * than a hope.
 */
class FakeAdminService(
    var admin: Boolean = true,
    var name: String = "Nadia",
    private val copy: AdminUsersPage? = null,
    private val copyInvites: List<AdminInviteRow>? = null,
) : AdminService {
    /** Every call this screen made, in order — the acceptance's request counter. */
    val calls: MutableList<String> = mutableListOf()

    /** A gate the account page waits on, when a test holds it. */
    var usersGate: CompletableDeferred<Unit>? = null

    /** A gate a WRITE waits on, so "it answered before the server did" is observable. */
    var writeGate: CompletableDeferred<Unit>? = null

    /** A failure injected on one call, by its name. */
    val failures: MutableMap<String, Throwable> = mutableMapOf()

    var page: AdminUsersPage = AdminFixtures.usersPage()
    var secondPage: AdminUsersPage = AdminFixtures.usersPage(
        users = listOf(AdminFixtures.user(id = "u-3", name = "Sacha", disabled = true)),
        total = 3u,
        next = null,
    )
    val invites: MutableList<AdminInviteRow> = mutableListOf(AdminFixtures.invite())
    var rules: AdminRules = AdminFixtures.rules()
    var serverCard: AdminServerCard = AdminFixtures.serverCard()
    var storage: AdminStorage = AdminFixtures.storage()
    var plan: AdminPlan = AdminFixtures.plan()
    var analysis: AdminAnalysisJob = AdminFixtures.analysis()
    var automatch: AdminAutomatch = AdminFixtures.automatch()
    var transcribe: AdminTranscribe = AdminFixtures.transcribe()
    var creditJob: AdminCredits = AdminFixtures.credits()
    var enrichJob: AdminEnrich = AdminFixtures.enrich()
    var similarJob: AdminSimilar = AdminFixtures.similar()
    var archiveJob: AdminArchiveStatus = AdminFixtures.archive()
    var migrateJob: AdminMigrate = AdminFixtures.migrate()
    var cleanup: AdminCleanup = AdminFixtures.cleanup()

    /** How many times the analysis job's state was read (the poll's number). */
    var analysisReads: Int = 0

    /** How many times the session was asked who is signed in (0 requests). */
    var identityReads: Int = 0

    private fun record(name: String) {
        calls += name
        failures[name]?.let { throw it }
    }

    /** `/api/me` is in memory: read for the gate, counted as NO request. */
    override fun identity(): AdminIdentity? {
        identityReads += 1
        return AdminIdentity(id = "u-1", name = name, admin = admin)
    }

    override suspend fun restoreSession(): Boolean {
        record("restoreSession")
        return admin
    }

    override fun cachedUsers(): AdminUsersPage? = copy

    override suspend fun users(q: String, after: String?, limit: UInt): AdminUsersPage {
        record("users")
        usersGate?.await()
        return if (after == null) page else secondPage
    }

    override suspend fun user(id: String): AdminUserDetail {
        record("user")
        return AdminFixtures.userDetail()
    }

    override suspend fun setRole(id: String, on: Boolean): AdminUserRow {
        record("setRole")
        writeGate?.await()
        val row = page.users.first { it.id == id }
        return row.copy(admin = on)
    }

    override suspend fun setDisabled(id: String, on: Boolean): AdminUserRow {
        record("setDisabled")
        writeGate?.await()
        val row = page.users.first { it.id == id }
        return row.copy(disabled = on)
    }

    override suspend fun deleteUser(id: String, name: String): AdminDeleteResult {
        record("deleteUser")
        return AdminFixtures.deleteResult()
    }

    override fun cachedInvites(): List<AdminInviteRow>? = copyInvites

    override suspend fun invites(state: String?): List<AdminInviteRow> {
        record("invites")
        return invites.toList()
    }

    override suspend fun createInvite(note: String?, expiresIn: UInt): AdminInviteIssued {
        record("createInvite")
        invites.add(0, AdminFixtures.invite(id = "i-new", note = note))
        return AdminFixtures.issued()
    }

    override suspend fun renewInvite(id: String): AdminInviteIssued {
        record("renewInvite")
        return AdminFixtures.issued(id = "i-renewed")
    }

    override suspend fun revokeInvite(id: String) {
        record("revokeInvite")
        invites.removeAll { it.id == id }
    }

    override fun cachedStorage(): AdminStorage? = null

    override suspend fun storage(): AdminStorage {
        record("storage")
        return storage
    }

    override fun cachedServerCard(): AdminServerCard? = null

    override suspend fun serverCard(): AdminServerCard {
        record("serverCard")
        return serverCard
    }

    override suspend fun testDeezer(force: Boolean): AdminServerCard {
        record("testDeezer")
        return serverCard
    }

    override suspend fun saveQuota(gb: Double): AdminServerCard {
        record("saveQuota")
        serverCard = serverCard.copy(uploadQuotaGb = gb)
        return serverCard
    }

    override suspend fun flushCache(): ULong {
        record("flushCache")
        return 1_048_576uL
    }

    override fun cachedRules(): AdminRules? = null

    override suspend fun rules(): AdminRules {
        record("rules")
        return rules
    }

    override suspend fun saveRule(field: String, value: String): AdminRules {
        record("saveRule:$field")
        rules = previewRule(rules, field, value.trim('"'))
        return rules
    }

    override fun cachedCleanup(): AdminCleanup? = null

    override suspend fun cleanup(): AdminCleanup {
        record("cleanup")
        return cleanup
    }

    override suspend fun runCleanup(): AdminCleanupRun {
        record("runCleanup")
        cleanup = cleanup.copy(neededBytes = 0uL, tracks = emptyList())
        return AdminFixtures.cleanupRun()
    }

    override fun cachedArchiveStatus(): AdminArchiveStatus? = null

    override suspend fun archiveStatus(): AdminArchiveStatus {
        record("archiveStatus")
        return archiveJob
    }

    override suspend fun startArchive(scope: String): AdminArchiveStatus {
        record("startArchive:$scope")
        archiveJob = AdminFixtures.archive(running = true)
        return archiveJob
    }

    override fun cachedMigrate(): AdminMigrate? = null

    override suspend fun migrate(): AdminMigrate {
        record("migrate")
        return migrateJob
    }

    override suspend fun startMigrate(force: Boolean): AdminMigrate {
        record("startMigrate")
        migrateJob = AdminFixtures.migrate(running = true)
        return migrateJob
    }

    override fun cachedPlan(): AdminPlan? = null

    override suspend fun plan(): AdminPlan {
        record("plan")
        return plan
    }

    override fun cachedAnalysis(): AdminAnalysisJob? = null

    override suspend fun analysis(): AdminAnalysisJob {
        record("analysis")
        analysisReads += 1
        // The first poll still runs, the second is over — a job that ends while
        // the screen watches, which is what "it says when it ends" means.
        if (analysisReads >= 2) analysis = AdminFixtures.analysis(running = false)
        return analysis
    }

    override suspend fun startAnalysis(force: Boolean, workers: UInt): AdminAnalysisJob {
        record("startAnalysis")
        analysis = AdminFixtures.analysis(running = true)
        // The job starts NOW: its own first poll is the first read (the page's
        // open-time read counted before the launch, not after).
        analysisReads = 0
        return analysis
    }

    override suspend fun stopAnalysis(): AdminAnalysisJob {
        record("stopAnalysis")
        analysis = AdminFixtures.analysis(running = false)
        return analysis
    }

    override suspend fun saveSchedule(enabled: Boolean, at: String, hours: UInt): AdminPlan {
        record("saveSchedule")
        plan = plan.copy(schedule = AdminSchedule(enabled = enabled, at = at, hours = hours))
        return plan
    }

    override suspend fun saveAnalysisPlace(place: String): AdminPlan {
        record("saveAnalysisPlace")
        plan = plan.copy(place = place)
        return plan
    }

    override fun cachedEnrich(): AdminEnrich? = null

    override suspend fun enrich(): AdminEnrich {
        record("enrich")
        return enrichJob
    }

    override suspend fun startEnrich(force: Boolean): AdminEnrich {
        record("startEnrich")
        enrichJob = AdminFixtures.enrich().copy(running = true)
        return enrichJob
    }

    override fun cachedCredits(): AdminCredits? = null

    override suspend fun credits(): AdminCredits {
        record("credits")
        return creditJob
    }

    override suspend fun startCredits(force: Boolean): AdminCredits {
        record("startCredits")
        creditJob = AdminFixtures.credits(running = true)
        return creditJob
    }

    override fun cachedSimilar(): AdminSimilar? = null

    override suspend fun similar(): AdminSimilar {
        record("similar")
        return similarJob
    }

    override suspend fun startSimilar(force: Boolean): AdminSimilar {
        record("startSimilar")
        similarJob = AdminFixtures.similar(running = true)
        return similarJob
    }

    override fun cachedAutomatch(): AdminAutomatch? = null

    override suspend fun automatch(): AdminAutomatch {
        record("automatch")
        return automatch
    }

    override suspend fun setAutomatch(on: Boolean): AdminAutomatch {
        record("setAutomatch")
        automatch = automatch.copy(enabled = on)
        return automatch
    }

    override suspend fun runAutomatch(): AdminAutomatch {
        record("runAutomatch")
        automatch = AdminFixtures.automatch(running = true)
        return automatch
    }

    override suspend fun cancelAutomatch(): AdminAutomatch {
        record("cancelAutomatch")
        automatch = automatch.copy(cancel = true)
        return automatch
    }

    override fun cachedTranscribe(): AdminTranscribe? = null

    override suspend fun transcribe(probe: Boolean): AdminTranscribe {
        record(if (probe) "transcribe:probe" else "transcribe")
        return transcribe
    }

    override suspend fun saveTranscribe(field: String, value: String): AdminTranscribe {
        record("saveTranscribe:$field")
        if (field == "max_minutes") transcribe = transcribe.copy(maxMinutes = value.toUInt())
        if (field == "enabled") transcribe = transcribe.copy(enabled = value == "true")
        return transcribe
    }
}
