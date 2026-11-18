package fr.neezer.mobile.feature.onboarding

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.AccountOptions
import uniffi.nz_ffi.ArtistCard
import uniffi.nz_ffi.ArtistPage
import uniffi.nz_ffi.ArtistSearch
import uniffi.nz_ffi.ArtistsDone
import uniffi.nz_ffi.DiversityBounds
import uniffi.nz_ffi.FeedCause
import uniffi.nz_ffi.FinishBlock
import uniffi.nz_ffi.ImportJob
import uniffi.nz_ffi.ImportOffer
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
import uniffi.nz_ffi.Screens
import uniffi.nz_ffi.StartedImport
import uniffi.nz_ffi.nzInviteFoldName
import uniffi.nz_ffi.nzInviteFoldUsername
import uniffi.nz_ffi.nzInviteNeedsOnboarding
import uniffi.nz_ffi.nzInvitePasswordMessage
import uniffi.nz_ffi.nzInvitePasswordScore
import uniffi.nz_ffi.nzInvitePasswordStrength
import uniffi.nz_ffi.nzInviteRules
import uniffi.nz_ffi.nzInviteTokenOk
import uniffi.nz_ffi.nzInviteUsernameMessage
import uniffi.nz_ffi.nzOnboardingCanPass
import uniffi.nz_ffi.nzOnboardingCacheList
import uniffi.nz_ffi.nzOnboardingFeedCause
import uniffi.nz_ffi.nzOnboardingFinishBlocked
import uniffi.nz_ffi.nzOnboardingInsertSimilar
import uniffi.nz_ffi.nzOnboardingNormalizeQuery
import uniffi.nz_ffi.nzOnboardingPageFirst
import uniffi.nz_ffi.nzOnboardingPageNext
import uniffi.nz_ffi.nzOnboardingReconcile
import uniffi.nz_ffi.nzOnboardingRemoveSimilar
import uniffi.nz_ffi.nzOnboardingSearchMin

/** `idOf`: the reference a tile is known by — its Deezer id, else its row id. */
val ArtistCard.key: String get() = deezerId.ifEmpty { id }

/**
 * What the onboarding asks of the core (lot MB5b).
 *
 * An interface, not the generated `Screens`, so the screen's logic is plain
 * Kotlin that `OnboardingModelTest` runs on the JVM with a fake. [NzOnboardingService]
 * is the one implementation, a one-to-one delegation to `nz-ffi/src/onboarding.rs`.
 */
interface OnboardingService {
    /** The copy this device kept: disk only, no request. */
    suspend fun cached(): OnboardingData?

    /** The visit's two reads: **two requests cold, 0 warm**. */
    suspend fun load(): OnboardingData

    /** `GET /api/onboarding` alone: the feed's build poll. */
    suspend fun state(): OnboardingState

    /** `GET /api/onboarding/artists?after=&limit=`. */
    suspend fun page(after: String?, limit: Int): ArtistPage

    /** `GET /api/onboarding/search?q=&limit=`. */
    suspend fun search(query: String, limit: Int): ArtistSearch

    /** `POST /api/onboarding/artists/like` — one request, neighbours included. */
    suspend fun like(id: String, seen: List<String>): RelatedAnswer

    /** `DELETE /api/onboarding/artists/<id>/like`. */
    suspend fun unlike(id: String)

    /** `GET /api/onboarding/artists/<id>/related?seen=`. */
    suspend fun related(id: String, seen: List<String>): RelatedAnswer

    /** `POST /api/onboarding/artists/done` (or `{skip: true}`). */
    suspend fun artistsDone(skip: Boolean): ArtistsDone

    /** `POST /api/onboarding/feed/retry`. */
    suspend fun feedRetry()

    /** `POST /api/onboarding/account/done`. */
    suspend fun accountDone(): OnboardingStep

    /** `GET /api/onboarding/account/options` — one request. */
    suspend fun options(): AccountOptions

    /** `POST /api/onboarding/account/link` — one request. */
    suspend fun link(provider: String, secret: String): AccountOptions

    /** `POST /api/onboarding/account/import` — one request, then a job. */
    suspend fun startImport(url: String): StartedImport

    /** `GET /api/import/playlist/<id>` — the job's progress and its causes. */
    suspend fun importStatus(id: String): ImportJob

    /** `POST /api/import/playlist/<id>/save`. */
    suspend fun importSave(id: String, title: String): ImportJob

    /** The cached-card trimming the page does before keeping the feed. */
    fun cacheList(items: List<ArtistCard>, max: Int): List<ArtistCard>
}

/** The one real implementation: this lot's `nz-ffi` methods, one for one. */
class NzOnboardingService(private val screens: Screens) : OnboardingService {
    override suspend fun cached(): OnboardingData? = screens.cachedOnboarding()

    override suspend fun load(): OnboardingData = screens.onboarding()

    override suspend fun state(): OnboardingState = screens.onboardingState()

    override suspend fun page(after: String?, limit: Int): ArtistPage =
        screens.onboardingArtists(after, limit.toUInt())

    override suspend fun search(query: String, limit: Int): ArtistSearch =
        screens.onboardingSearch(query, limit.toUInt())

    override suspend fun like(id: String, seen: List<String>): RelatedAnswer =
        screens.onboardingLike(id, seen)

    override suspend fun unlike(id: String) = screens.onboardingUnlike(id)

    override suspend fun related(id: String, seen: List<String>): RelatedAnswer =
        screens.onboardingRelated(id, seen)

    override suspend fun artistsDone(skip: Boolean): ArtistsDone =
        screens.onboardingArtistsDone(skip)

    override suspend fun feedRetry() = screens.onboardingFeedRetry()

    override suspend fun accountDone(): OnboardingStep = screens.onboardingAccountDone()

    override suspend fun options(): AccountOptions = screens.accountOptions()

    override suspend fun link(provider: String, secret: String): AccountOptions =
        screens.linkAccount(provider, secret)

    override suspend fun startImport(url: String): StartedImport = screens.startImport(url)

    override suspend fun importStatus(id: String): ImportJob = screens.importStatus(id)

    override suspend fun importSave(id: String, title: String): ImportJob =
        screens.importSave(id, title)

    override fun cacheList(items: List<ArtistCard>, max: Int): List<ArtistCard> =
        nzOnboardingCacheList(items, max.toUInt())
}

/**
 * The ported rules of `lib/onboarding.js`, as a seam.
 *
 * The implementation is `nz-core`'s (one copy, tested there); the seam exists so
 * `OnboardingModelTest` proves the state machine without a `.so`, exactly as
 * [OnboardingService] does.
 */
interface OnboardingRules {
    fun insertSimilar(
        items: List<ArtistCard>,
        index: Int,
        similar: List<ArtistCard>,
        liked: Set<String>,
        perLike: Int,
        budget: Int,
        diversity: DiversityBounds,
    ): MergeResult

    fun removeSimilar(items: List<ArtistCard>, parent: String, liked: Set<String>): List<ArtistCard>

    fun reconcile(items: List<ArtistCard>, page: List<ArtistCard>): List<ArtistCard>

    fun finishBlocked(liked: Int, pending: Int, queued: Int, online: Boolean, min: Int): FinishBlock?

    fun canPass(attempts: Int, feedState: String, building: Boolean, count: Int, min: Int): Boolean

    fun feedCause(code: String?): FeedCause

    fun normalizeQuery(raw: String): String

    val pageFirst: Int
    val pageNext: Int
    val searchMin: Int
    val similarTotalMax: Int
    val similarPerLike: Int
}

/** The real rules: `nz-core::onboarding::rules`, through the boundary. */
object NzOnboardingRules : OnboardingRules {
    override fun insertSimilar(
        items: List<ArtistCard>,
        index: Int,
        similar: List<ArtistCard>,
        liked: Set<String>,
        perLike: Int,
        budget: Int,
        diversity: DiversityBounds,
    ): MergeResult = nzOnboardingInsertSimilar(
        items,
        index.coerceAtLeast(0).toUInt(),
        similar,
        liked.toList(),
        perLike.toUInt(),
        budget.toUInt(),
        diversity.window,
        diversity.maxSameGenre,
        diversity.maxStreak,
    )

    override fun removeSimilar(
        items: List<ArtistCard>,
        parent: String,
        liked: Set<String>,
    ): List<ArtistCard> = nzOnboardingRemoveSimilar(items, parent, liked.toList())

    override fun reconcile(items: List<ArtistCard>, page: List<ArtistCard>): List<ArtistCard> =
        nzOnboardingReconcile(items, page)

    override fun finishBlocked(
        liked: Int,
        pending: Int,
        queued: Int,
        online: Boolean,
        min: Int,
    ): FinishBlock? = nzOnboardingFinishBlocked(
        liked.coerceAtLeast(0).toUInt(),
        pending.coerceAtLeast(0).toUInt(),
        queued.coerceAtLeast(0).toUInt(),
        online,
        min.coerceAtLeast(0).toUInt(),
    )

    override fun canPass(
        attempts: Int,
        feedState: String,
        building: Boolean,
        count: Int,
        min: Int,
    ): Boolean = nzOnboardingCanPass(
        attempts.coerceAtLeast(0).toUInt(),
        feedState,
        building,
        count.coerceAtLeast(0).toUInt(),
        min.coerceAtLeast(0).toUInt(),
    )

    override fun feedCause(code: String?): FeedCause = nzOnboardingFeedCause(code)

    override fun normalizeQuery(raw: String): String = nzOnboardingNormalizeQuery(raw)

    override val pageFirst: Int get() = nzOnboardingPageFirst().toInt()

    override val pageNext: Int get() = nzOnboardingPageNext().toInt()

    override val searchMin: Int get() = nzOnboardingSearchMin().toInt()

    override val similarTotalMax: Int = 12

    override val similarPerLike: Int = 2
}

/**
 * The invitation page's asks of the core (lot MB5b).
 *
 * Same seam, same reason: the form's rules are `lib/invite.js`'s (in Rust), and
 * the page's state machine is proven on the JVM over this.
 */
interface InviteService {
    suspend fun inspect(code: String): InviteInfo

    /** One request, and three ANSWERS: created, a refused field, or a gone link. */
    suspend fun redeem(code: String, username: String, password: String): RedeemAnswer

    /** The person already signed in, for the page's own state — 0 requests. */
    fun signedIn(): Me?

    /** « Utiliser cette invitation » closes the session first. */
    suspend fun logout()

    fun tokenOk(token: String): Boolean

    fun foldName(raw: String): String

    fun foldUsername(raw: String): String

    fun usernameMessage(name: String): String

    fun passwordMessage(password: String, username: String): String

    fun strength(password: String, username: String): PasswordStrength

    fun score(password: String, username: String): Int

    fun needsOnboarding(onboarding: String?): Boolean

    val rules: InviteRules
}

/** The one real implementation of [InviteService]. */
class NzInviteService(private val screens: Screens) : InviteService {
    override suspend fun inspect(code: String): InviteInfo = screens.inspectInvite(code)

    override suspend fun redeem(
        code: String,
        username: String,
        password: String,
    ): RedeemAnswer = screens.redeemInvite(code, username, password)

    override fun signedIn(): Me? = screens.inviteAccount()

    override suspend fun logout() {
        NzBridge.core?.logout()
    }

    override fun tokenOk(token: String): Boolean = nzInviteTokenOk(token)

    override fun foldName(raw: String): String = nzInviteFoldName(raw)

    override fun foldUsername(raw: String): String = nzInviteFoldUsername(raw)

    override fun usernameMessage(name: String): String = nzInviteUsernameMessage(name)

    override fun passwordMessage(password: String, username: String): String =
        nzInvitePasswordMessage(password, username)

    override fun strength(password: String, username: String): PasswordStrength =
        nzInvitePasswordStrength(password, username)

    override fun score(password: String, username: String): Int =
        nzInvitePasswordScore(password, username).toInt()

    override fun needsOnboarding(onboarding: String?): Boolean =
        nzInviteNeedsOnboarding(onboarding)

    override val rules: InviteRules get() = nzInviteRules()
}
