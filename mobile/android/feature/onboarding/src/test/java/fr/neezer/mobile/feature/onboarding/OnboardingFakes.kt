package fr.neezer.mobile.feature.onboarding

import uniffi.nz_ffi.AccountOptions
import uniffi.nz_ffi.ArtistCard
import uniffi.nz_ffi.ArtistPage
import uniffi.nz_ffi.ArtistSearch
import uniffi.nz_ffi.ArtistsDone
import uniffi.nz_ffi.DiversityBounds
import uniffi.nz_ffi.FeedCause
import uniffi.nz_ffi.FinishBlock
import uniffi.nz_ffi.ImportJob
import uniffi.nz_ffi.InviteInfo
import uniffi.nz_ffi.InviteRules
import uniffi.nz_ffi.Me
import uniffi.nz_ffi.MergeResult
import uniffi.nz_ffi.OnboardingData
import uniffi.nz_ffi.OnboardingState
import uniffi.nz_ffi.OnboardingStep
import uniffi.nz_ffi.PasswordStrength
import uniffi.nz_ffi.RedeemAnswer
import uniffi.nz_ffi.RelatedAnswer
import uniffi.nz_ffi.StartedImport

/**
 * A fake [OnboardingService] that COUNTS every call and answers what a test
 * configures — the same seam `HomeModelTest` and `SearchModelTest` use, so the
 * state machine is proven without a `.so` and without a device.
 */
class FakeOnboardingService : OnboardingService {
    val calls = mutableListOf<String>()
    var cachedValue: OnboardingData? = OnboardingFixtures.data()
    var loadValue: OnboardingData = OnboardingFixtures.data()
    var loadError: Throwable? = null
    var stateValue: OnboardingState = OnboardingFixtures.state()
    var stateError: Throwable? = null
    var pageValue: ArtistPage = OnboardingFixtures.page()
    var pageError: Throwable? = null
    var searchValue: ArtistSearch = OnboardingFixtures.search()
    var searchError: Throwable? = null
    var likeValue: RelatedAnswer = OnboardingFixtures.relatedReady()
    var likeError: Throwable? = null
    var unlikeError: Throwable? = null
    var relatedValue: RelatedAnswer = OnboardingFixtures.relatedReady()
    var relatedError: Throwable? = null
    var artistsDoneValue: ArtistsDone = OnboardingFixtures.artistsDone()
    var artistsDoneError: Throwable? = null
    var feedRetryError: Throwable? = null
    var accountDoneValue: OnboardingStep = OnboardingStep.DONE
    var accountDoneError: Throwable? = null
    var optionsValue: AccountOptions = OnboardingFixtures.options()
    var optionsError: Throwable? = null
    var linkValue: AccountOptions = OnboardingFixtures.options()
    var linkError: Throwable? = null
    var startedValue: StartedImport = StartedImport(
        job = OnboardingFixtures.job(),
        options = OnboardingFixtures.options(),
    )
    var importError: Throwable? = null
    var jobStatus: ImportJob = OnboardingFixtures.job()
    var jobError: Throwable? = null
    var savedJob: ImportJob = OnboardingFixtures.finishedJob()
    var saveError: Throwable? = null

    override suspend fun cached(): OnboardingData? {
        calls += "cached"
        return cachedValue
    }

    override suspend fun load(): OnboardingData {
        calls += "load"
        loadError?.let { throw it }
        return loadValue
    }

    override suspend fun state(): OnboardingState {
        calls += "state"
        stateError?.let { throw it }
        return stateValue
    }

    override suspend fun page(after: String?, limit: Int): ArtistPage {
        calls += "page:${after ?: "first"}:$limit"
        pageError?.let { throw it }
        return pageValue
    }

    override suspend fun search(query: String, limit: Int): ArtistSearch {
        calls += "search:$query:$limit"
        searchError?.let { throw it }
        return searchValue
    }

    override suspend fun like(id: String, seen: List<String>): RelatedAnswer {
        calls += "like:$id"
        likeError?.let { throw it }
        return likeValue
    }

    override suspend fun unlike(id: String) {
        calls += "unlike:$id"
        unlikeError?.let { throw it }
    }

    override suspend fun related(id: String, seen: List<String>): RelatedAnswer {
        calls += "related:$id"
        relatedError?.let { throw it }
        return relatedValue
    }

    override suspend fun artistsDone(skip: Boolean): ArtistsDone {
        calls += "artistsDone:$skip"
        artistsDoneError?.let { throw it }
        return artistsDoneValue
    }

    override suspend fun feedRetry() {
        calls += "feedRetry"
        feedRetryError?.let { throw it }
    }

    override suspend fun accountDone(): OnboardingStep {
        calls += "accountDone"
        accountDoneError?.let { throw it }
        return accountDoneValue
    }

    override suspend fun options(): AccountOptions {
        calls += "options"
        optionsError?.let { throw it }
        return optionsValue
    }

    override suspend fun link(provider: String, secret: String): AccountOptions {
        calls += "link:$provider"
        linkError?.let { throw it }
        return linkValue
    }

    override suspend fun startImport(url: String): StartedImport {
        calls += "startImport"
        importError?.let { throw it }
        return startedValue
    }

    override suspend fun importStatus(id: String): ImportJob {
        calls += "importStatus:$id"
        jobError?.let { throw it }
        return jobStatus
    }

    override suspend fun importSave(id: String, title: String): ImportJob {
        calls += "importSave:$id"
        saveError?.let { throw it }
        return savedJob
    }

    override fun cacheList(items: List<ArtistCard>, max: Int): List<ArtistCard> = items
}

/**
 * The rules, faked with the SAME shape the core answers: a neighbour block is
 * appended right after its parent (the placement and the diversity bounds are
 * `nz-core`'s own tests), removal drops a parent's block, and the two gates are
 * the page's.
 */
class FakeRules : OnboardingRules {
    override fun insertSimilar(
        items: List<ArtistCard>,
        index: Int,
        similar: List<ArtistCard>,
        liked: Set<String>,
        perLike: Int,
        budget: Int,
        diversity: DiversityBounds,
    ): MergeResult {
        if (index < 0 || index >= items.size || similar.isEmpty()) {
            return MergeResult(items = items, inserted = 0u)
        }
        val room = minOf(perLike, budget)
        val taken = items.map { it.key }.toMutableSet()
        val picked = similar
            .filter { it.key !in taken && it.key !in liked && it.key != items[index].key }
            .take(room)
            .map { it.copy(via = "related", parent = items[index].key) }
        if (picked.isEmpty()) return MergeResult(items = items, inserted = 0u)
        val merged = items.take(index + 1) + picked + items.drop(index + 1)
        return MergeResult(items = merged, inserted = picked.size.toUInt())
    }

    override fun removeSimilar(
        items: List<ArtistCard>,
        parent: String,
        liked: Set<String>,
    ): List<ArtistCard> = items.filterNot { it.via == "related" && it.parent == parent }

    override fun reconcile(items: List<ArtistCard>, page: List<ArtistCard>): List<ArtistCard> {
        val out = items.toMutableList()
        for (fresh in page) {
            val at = out.indexOfFirst { it.key == fresh.key }
            if (at >= 0) out[at] = fresh else if (out.none { it.key == fresh.key }) out += fresh
        }
        return out
    }

    override fun finishBlocked(
        liked: Int,
        pending: Int,
        queued: Int,
        online: Boolean,
        min: Int,
    ): FinishBlock? = when {
        liked < min -> FinishBlock.MIN
        !online -> FinishBlock.OFFLINE
        pending > 0 || queued > 0 -> FinishBlock.PENDING
        else -> null
    }

    override fun canPass(
        attempts: Int,
        feedState: String,
        building: Boolean,
        count: Int,
        min: Int,
    ): Boolean = count < min && attempts >= 2 && feedState == "unavailable" && !building

    override fun feedCause(code: String?): FeedCause = when (code) {
        "deezer-unreachable" -> FeedCause.DEEZER_UNREACHABLE
        "disabled" -> FeedCause.DISABLED
        "absent" -> FeedCause.ABSENT
        "error" -> FeedCause.ERROR
        else -> FeedCause.UNKNOWN
    }

    override fun normalizeQuery(raw: String): String =
        raw.trim().replace(Regex("\\s+"), " ").take(80)

    override val pageFirst: Int = 36
    override val pageNext: Int = 24
    override val searchMin: Int = 2
    override val similarTotalMax: Int = 12
    override val similarPerLike: Int = 2
}

/** A fake [InviteService]: the form's rules are the core's own shape, faked. */
class FakeInviteService : InviteService {
    val calls = mutableListOf<String>()
    var info: InviteInfo = OnboardingFixtures.invite()
    var inspectError: Throwable? = null
    var answer: RedeemAnswer = OnboardingFixtures.created()
    var redeemError: Throwable? = null
    var account: Me? = null
    var logoutError: Throwable? = null
    var tokenOk: Boolean = true

    override suspend fun inspect(code: String): InviteInfo {
        calls += "inspect"
        inspectError?.let { throw it }
        return info
    }

    override suspend fun redeem(code: String, username: String, password: String): RedeemAnswer {
        calls += "redeem:$username"
        redeemError?.let { throw it }
        return answer
    }

    override fun signedIn(): Me? = account

    override suspend fun logout() {
        calls += "logout"
        logoutError?.let { throw it }
        account = null
    }

    override fun tokenOk(token: String): Boolean = tokenOk

    override fun foldName(raw: String): String = raw.lowercase().replace("é", "e")

    override fun foldUsername(raw: String): String = foldName(raw.trim()).trim()

    override fun usernameMessage(name: String): String {
        val folded = foldUsername(name)
        return when {
            folded.length < 3 -> "Il faut au moins 3 caractères."
            folded.length > 32 -> "32 caractères au plus."
            folded.any { !(it.isLetterOrDigit() || it in "_.-") } ->
                "Ce caractère n'est pas autorisé : « @ »."
            else -> ""
        }
    }

    override fun passwordMessage(password: String, username: String): String {
        val folded = foldUsername(password)
        val name = foldUsername(username)
        return when {
            password.length < 12 -> "12 caractères minimum."
            password.length > 128 -> "128 caractères au plus."
            name.isNotEmpty() && folded.contains(name) ->
                "Le mot de passe ne peut pas contenir ton nom d'utilisateur."
            else -> ""
        }
    }

    override fun strength(password: String, username: String): PasswordStrength = when {
        password.length < 12 -> PasswordStrength.TOO_SHORT
        password.length >= 24 -> PasswordStrength.VERY_GOOD
        password.length >= 16 -> PasswordStrength.GOOD
        else -> PasswordStrength.FAIR
    }

    override fun score(password: String, username: String): Int = when {
        password.isEmpty() -> 0
        password.length < 12 -> 1
        password.length >= 24 -> 4
        password.length >= 16 -> 3
        else -> 2
    }

    override fun needsOnboarding(onboarding: String?): Boolean =
        onboarding != null && onboarding.isNotEmpty() && onboarding != "done"

    override val rules: InviteRules = OnboardingFixtures.rules
}
