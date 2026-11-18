package fr.neezer.mobile.feature.onboarding

import uniffi.nz_ffi.AccountImportOffer
import uniffi.nz_ffi.AccountOptions
import uniffi.nz_ffi.AccountRun
import uniffi.nz_ffi.ArtistCard
import uniffi.nz_ffi.ArtistPage
import uniffi.nz_ffi.ArtistSearch
import uniffi.nz_ffi.ArtistsDone
import uniffi.nz_ffi.DiversityBounds
import uniffi.nz_ffi.FeedState
import uniffi.nz_ffi.ImportItem
import uniffi.nz_ffi.ImportJob
import uniffi.nz_ffi.ImportState
import uniffi.nz_ffi.InviteInfo
import uniffi.nz_ffi.InviteRules
import uniffi.nz_ffi.LinkedAccount
import uniffi.nz_ffi.Me
import uniffi.nz_ffi.OnboardingData
import uniffi.nz_ffi.OnboardingState
import uniffi.nz_ffi.OnboardingStep
import uniffi.nz_ffi.ProviderOptions
import uniffi.nz_ffi.RedeemAnswer
import uniffi.nz_ffi.RedeemStatus
import uniffi.nz_ffi.RelatedAnswer
import uniffi.nz_ffi.RelatedState

/**
 * The FFI records the screen draws, built by hand for the JVM tests.
 *
 * A fake `Screens` is impossible (it is a native object), so the tests build the
 * same records the boundary returns. The values are the SPA's own fixtures where
 * it has them (`onboarding.test.mjs`'s `forest`, `invite.test.mjs`'s token).
 */
object OnboardingFixtures {
    val diversity = DiversityBounds(window = 6u, maxSameGenre = 3u, maxStreak = 2u)

    fun feed(
        state: String = "ready",
        building: Boolean = false,
        error: String? = null,
        source: String = "deezer",
    ) = FeedState(
        state = state,
        building = building,
        error = error,
        source = source,
        ready = state == "ready",
        stale = false,
        progress = null,
    )

    fun artist(
        id: String,
        name: String = "Artiste $id",
        liked: Boolean = false,
        via: String? = "chart",
        parent: String? = null,
        genres: List<String> = listOf("Pop"),
    ) = ArtistCard(
        id = id,
        deezerId = id,
        name = name,
        picture = null,
        pictureSmall = null,
        genres = genres,
        provider = "deezer",
        via = via,
        parent = parent,
        liked = liked,
        openable = true,
        hint = null,
    )

    fun forest(): List<ArtistCard> = (0 until 6).map { artist("10$it", "Artiste $it") }

    fun state(
        step: OnboardingStep = OnboardingStep.ARTISTS,
        liked: Int = 1,
        feed: FeedState = feed(),
    ) = OnboardingState(
        step = step,
        liked = liked.toUInt(),
        minArtists = 1u,
        feed = feed,
        diversity = diversity,
    )

    fun page(
        items: List<ArtistCard> = forest(),
        next: String? = null,
        building: Boolean = false,
    ) = ArtistPage(
        items = items,
        next = next,
        total = items.size.toUInt(),
        state = if (items.isEmpty() && building) "building" else "ready",
        source = "deezer",
        building = building,
        error = null,
        partial = false,
    )

    fun data(
        state: OnboardingState = state(),
        page: ArtistPage = page(),
    ) = OnboardingData(state = state, page = page)

    fun search(items: List<ArtistCard> = listOf(artist("90", "Aya Nakamura", via = "search"))) =
        ArtistSearch(
            query = "aya",
            items = items,
            local = items.size.toUInt(),
            partial = false,
            error = null,
        )

    fun relatedReady(vararg items: ArtistCard) = RelatedAnswer(
        state = RelatedState.READY,
        items = items.toList(),
        cause = null,
    )

    fun relatedPending() = RelatedAnswer(
        state = RelatedState.PENDING,
        items = emptyList(),
        cause = null,
    )

    fun relatedError(cause: String = "deezer-unreachable") = RelatedAnswer(
        state = RelatedState.ERROR,
        items = emptyList(),
        cause = cause,
    )

    fun artistsDone(step: OnboardingStep = OnboardingStep.ACCOUNT, skipped: Boolean = false) =
        ArtistsDone(step = step, skipped = skipped)

    fun linkedAccount(
        running: AccountRun? = null,
        status: String = "ok",
        addedArtists: Int = 0,
        addedTracks: Int = 0,
        lastReason: String? = null,
    ) = LinkedAccount(
        id = "7",
        label = "Deezer",
        mode = "arl",
        status = status,
        server = false,
        addedArtists = addedArtists.toUInt(),
        addedTracks = addedTracks.toUInt(),
        lastReason = lastReason,
        running = running,
    )

    fun provider(
        available: Boolean = true,
        canLink: Boolean = true,
        linked: Boolean = false,
        denied: Boolean = false,
        reason: String? = null,
        account: LinkedAccount? = null,
    ) = ProviderOptions(
        available = available,
        canLink = canLink,
        linked = linked,
        denied = denied,
        reason = reason,
        account = account,
    )

    fun options(
        deezer: ProviderOptions = provider(),
        youtube: ProviderOptions = provider(),
        imports: List<AccountImportOffer> = listOf(
            AccountImportOffer(
                source = "deezer",
                label = "Deezer",
                available = true,
                reason = null,
            ),
            AccountImportOffer(
                source = "spotify",
                label = "Spotify",
                available = true,
                reason = null,
            ),
        ),
        step: OnboardingStep = OnboardingStep.ACCOUNT,
        job: ImportJob? = null,
    ) = AccountOptions(
        step = step,
        deezer = deezer,
        youtube = youtube,
        imports = imports,
        canSaveImports = true,
        job = job,
    )

    fun item(title: String, matched: Boolean = true, artists: String = "Quelqu'un") = ImportItem(
        title = title,
        artists = artists,
        album = "",
        durationS = 200u,
        matched = matched,
        via = if (matched) "deezer" else null,
        sure = matched,
    )

    fun job(
        id: String = "job-1",
        state: ImportState = ImportState.MATCHING,
        total: Int = 0,
        done: Int = 0,
        matched: Int = 0,
        error: String? = null,
        items: List<ImportItem> = emptyList(),
    ) = ImportJob(
        id = id,
        state = state,
        source = "spotify",
        name = "Mes hits",
        total = total.toUInt(),
        done = done.toUInt(),
        matched = matched.toUInt(),
        truncated = false,
        error = error,
        items = items,
        unmatched = items.count { !it.matched }.toUInt(),
    )

    fun finishedJob() = job(
        state = ImportState.DONE,
        total = 3,
        done = 3,
        matched = 1,
        error = "Deezer ne répondait pas : certains titres n'ont pas pu être cherchés",
        items = listOf(item("Get Lucky"), item("Inconnu 1", matched = false), item("Inconnu 2", matched = false)),
    )

    fun invite(valid: Boolean = true, draftName: String? = null) = InviteInfo(
        valid = valid,
        label = "Invitation",
        expires = "2026-11-21T18:00:00",
        draftName = draftName,
    )

    fun me(name: String = "elodie", onboarding: String = "artists") = Me(
        id = "u-new",
        name = name,
        admin = false,
        avatar = null,
    )

    fun created(
        step: OnboardingStep = OnboardingStep.ARTISTS,
        name: String = "elodie",
    ) = RedeemAnswer(
        status = RedeemStatus.CREATED,
        me = me(name),
        onboarding = step,
        field = null,
        message = "",
    )

    fun fieldAnswer(field: uniffi.nz_ffi.InviteField, message: String) = RedeemAnswer(
        status = RedeemStatus.FIELD,
        me = null,
        onboarding = OnboardingStep.DONE,
        field = field,
        message = message,
    )

    fun invalidAnswer() = RedeemAnswer(
        status = RedeemStatus.INVALID,
        me = null,
        onboarding = OnboardingStep.DONE,
        field = null,
        message = "",
    )

    val rules = InviteRules(
        minUsername = 3u,
        maxUsername = 32u,
        minPassword = 12u,
        maxPassword = 128u,
    )

    fun token() = "a".repeat(22) + "." + "b".repeat(24)
}
