//! Onboarding and invitation redemption.
//!
//! Owned by **MB5b** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records, its own listener — and edits no other file of the crate.
//!
//! Invitation links arrive as an `AppLink` (MB2's `links.rs`) and land on this area.
//!
//! # The shape of this boundary
//!
//! - **`inspect_invite`** is ONE request, and a malformed token is none: the
//!   page's own shape check runs in `nz-core::onboarding::rules`.
//! - **`redeem_invite`** is ONE request and three ANSWERS (`created`,
//!   `field`, `invalid`): a refused form is not a failure, so its sentence goes
//!   under the field it names instead of into a toast.
//! - **`cached_onboarding`** is synchronous and disk-only; **`onboarding`** is the
//!   visit's two requests, concurrently.
//! - **The account step is one request**, and the import is a JOB: `start_import`
//!   answers its handle, `import_status` its progress and the cause of each line.
//! - The pure rules that shape the list (which neighbours go where, and how many
//!   of one family) are `nz-core`'s, exported below so the Kotlin side never
//!   keeps a second copy of `lib/onboarding.js`.
//!
//! Every body converts to `nz_core::onboarding` and back; no rule lives in this
//! file, and no French sentence either — except the form's two, which ARE the
//! ported `lib/invite.js` messages (the screen shows them verbatim, as the page
//! does).

use nz_core::onboarding as core;

use crate::core::{Me, Screens};
use crate::errors::NzError;

/// Where the person stands in the first-run flow (`/api/onboarding`'s `step`).
#[derive(uniffi::Enum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OnboardingStep {
    Artists,
    Account,
    #[default]
    Done,
}

impl From<core::Step> for OnboardingStep {
    fn from(step: core::Step) -> Self {
        match step {
            core::Step::Artists => OnboardingStep::Artists,
            core::Step::Account => OnboardingStep::Account,
            core::Step::Done => OnboardingStep::Done,
        }
    }
}

/// The artists feed's own state (`/api/onboarding`'s `feed`).
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct FeedState {
    /// `"ready"`, `"building"`, `"unavailable"`…
    pub state: String,
    pub building: bool,
    /// The provider's cause code (`"deezer-unreachable"`, `"disabled"`…); the
    /// SCREEN writes the sentence from [`nz_onboarding_feed_cause`].
    pub error: Option<String>,
    pub source: String,
    pub ready: bool,
    pub stale: bool,
    pub progress: Option<f64>,
}

impl From<core::Feed> for FeedState {
    fn from(feed: core::Feed) -> Self {
        FeedState {
            state: feed.state,
            building: feed.building,
            error: feed.error,
            source: feed.source,
            ready: feed.ready,
            stale: feed.stale,
            progress: feed.progress,
        }
    }
}

/// The mix's bounds, as the server sent them: the phone inserts no more of one
/// family than the page would.
#[derive(uniffi::Record, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiversityBounds {
    pub window: u32,
    pub max_same_genre: u32,
    pub max_streak: u32,
}

impl From<core::Diversity> for DiversityBounds {
    fn from(diversity: core::Diversity) -> Self {
        DiversityBounds {
            window: diversity.window as u32,
            max_same_genre: diversity.max_same_genre as u32,
            max_streak: diversity.max_streak as u32,
        }
    }
}

/// `/api/onboarding` — where the first-run flow stands.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct OnboardingState {
    pub step: OnboardingStep,
    pub liked: u32,
    pub min_artists: u32,
    pub feed: FeedState,
    pub diversity: DiversityBounds,
}

impl From<core::OnboardingState> for OnboardingState {
    fn from(state: core::OnboardingState) -> Self {
        OnboardingState {
            step: state.step.into(),
            liked: state.liked,
            min_artists: state.min_artists,
            feed: state.feed.into(),
            diversity: state.diversity.into(),
        }
    }
}

/// One artist tile, as `OnboardingArtistCard.svelte` draws it and as the order
/// rules read it (`via`, `parent`, `liked`, the genres).
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct ArtistCard {
    /// The reference the like button sends.
    pub id: String,
    pub deezer_id: String,
    pub name: String,
    pub picture: Option<String>,
    pub picture_small: Option<String>,
    pub genres: Vec<String>,
    /// `"deezer"`, `"youtube"` or `"local"`.
    pub provider: String,
    /// `"chart"`, `"related"`, `"search"`, `"library"`…
    pub via: Option<String>,
    /// The tile this one is a neighbour of.
    pub parent: Option<String>,
    pub liked: bool,
    /// The artist has a page the library can open.
    pub openable: bool,
    /// Why this tile is here, in the server's words.
    pub hint: Option<String>,
}

impl From<core::ArtistCard> for ArtistCard {
    fn from(artist: core::ArtistCard) -> Self {
        ArtistCard {
            id: artist.id,
            deezer_id: artist.deezer_id,
            name: artist.name,
            picture: artist.picture,
            picture_small: artist.picture_small,
            genres: artist.genres,
            provider: artist.provider,
            via: artist.via,
            parent: artist.parent,
            liked: artist.liked,
            openable: artist.openable,
            hint: artist.hint,
        }
    }
}

impl From<ArtistCard> for core::ArtistCard {
    fn from(artist: ArtistCard) -> Self {
        core::ArtistCard {
            id: artist.id,
            deezer_id: artist.deezer_id,
            name: artist.name,
            picture: artist.picture,
            picture_small: artist.picture_small,
            genres: artist.genres,
            provider: artist.provider,
            via: artist.via,
            parent: artist.parent,
            liked: artist.liked,
            openable: artist.openable,
            hint: artist.hint,
        }
    }
}

/// One page of the feed (`/api/onboarding/artists`).
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct ArtistPage {
    pub items: Vec<ArtistCard>,
    /// The cursor of the next page, `None` on the last one.
    pub next: Option<String>,
    pub total: u32,
    pub state: String,
    pub source: String,
    pub building: bool,
    pub error: Option<String>,
    pub partial: bool,
}

impl From<core::ArtistPage> for ArtistPage {
    fn from(page: core::ArtistPage) -> Self {
        ArtistPage {
            items: page.items.into_iter().map(ArtistCard::from).collect(),
            next: page.next,
            total: page.total,
            state: page.state,
            source: page.source,
            building: page.building,
            error: page.error,
            partial: page.partial,
        }
    }
}

/// What one visit paints: the person's step and the feed's first page.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct OnboardingData {
    pub state: OnboardingState,
    pub page: ArtistPage,
}

impl From<core::OnboardingData> for OnboardingData {
    fn from(data: core::OnboardingData) -> Self {
        OnboardingData {
            state: data.state.into(),
            page: data.page.into(),
        }
    }
}

/// `/api/onboarding/search` — the box's own answer.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct ArtistSearch {
    pub query: String,
    pub items: Vec<ArtistCard>,
    pub local: u32,
    pub partial: bool,
    /// The PROVIDER's cause when the library answered alone.
    pub error: Option<String>,
}

impl From<core::ArtistSearch> for ArtistSearch {
    fn from(found: core::ArtistSearch) -> Self {
        ArtistSearch {
            query: found.query,
            items: found.items.into_iter().map(ArtistCard::from).collect(),
            local: found.local,
            partial: found.partial,
            error: found.error,
        }
    }
}

/// The neighbours' state (`similar_payload`'s `state`).
#[derive(uniffi::Enum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RelatedState {
    #[default]
    None,
    Pending,
    Ready,
    Error,
}

impl From<core::RelatedState> for RelatedState {
    fn from(state: core::RelatedState) -> Self {
        match state {
            core::RelatedState::None => RelatedState::None,
            core::RelatedState::Pending => RelatedState::Pending,
            core::RelatedState::Ready => RelatedState::Ready,
            core::RelatedState::Error => RelatedState::Error,
        }
    }
}

/// `POST /api/onboarding/artists/like`'s `related`, and the related route.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct RelatedAnswer {
    pub state: RelatedState,
    pub items: Vec<ArtistCard>,
    /// The provider's cause code, when the neighbours failed.
    pub cause: Option<String>,
}

impl From<core::RelatedAnswer> for RelatedAnswer {
    fn from(answer: core::RelatedAnswer) -> Self {
        RelatedAnswer {
            state: answer.state.into(),
            items: answer.items.into_iter().map(ArtistCard::from).collect(),
            cause: answer.cause,
        }
    }
}

/// `/api/onboarding/artists/done` and `/api/onboarding/account/done`.
#[derive(uniffi::Record, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArtistsDone {
    pub step: OnboardingStep,
    pub skipped: bool,
}

impl From<core::ArtistsDone> for ArtistsDone {
    fn from(done: core::ArtistsDone) -> Self {
        ArtistsDone {
            step: done.step.into(),
            skipped: done.skipped,
        }
    }
}

/// The invitation as inspected.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq, Eq)]
pub struct InviteInfo {
    pub valid: bool,
    pub label: String,
    /// ISO-8601; the SCREEN formats it in French (`expiryLabel`).
    pub expires: Option<String>,
    /// The name a previous attempt typed, from the device's draft.
    pub draft_name: Option<String>,
}

impl From<core::InviteInfo> for InviteInfo {
    fn from(info: core::InviteInfo) -> Self {
        InviteInfo {
            valid: info.valid,
            label: info.label,
            expires: info.expires,
            draft_name: info.draft_name,
        }
    }
}

/// Which field a refusal belongs under (`Invite.svelte#handleFailure`).
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteField {
    Username,
    Password,
    Confirm,
}

impl From<core::InviteField> for InviteField {
    fn from(field: core::InviteField) -> Self {
        match field {
            core::InviteField::Username => InviteField::Username,
            core::InviteField::Password => InviteField::Password,
            core::InviteField::Confirm => InviteField::Confirm,
        }
    }
}

/// What redeeming did, in the three states the page draws.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedeemStatus {
    /// The account exists: `me` and `onboarding` say where it lands.
    Created,
    /// The form was refused: `field` names where, `message` is the sentence.
    Field,
    /// The link is gone: the page's "plus valide".
    Invalid,
}

/// `POST /api/invites/redeem`'s answer. One record, because UniFFI has no
/// sum type with payloads that Kotlin reads as neatly as three nullable fields.
#[derive(uniffi::Record, Debug, Clone)]
pub struct RedeemAnswer {
    pub status: RedeemStatus,
    /// The signed-in person, when `status` is `Created`.
    pub me: Option<Me>,
    /// Where a created account lands (`landingFor`).
    pub onboarding: OnboardingStep,
    /// Which field the sentence belongs under, when `status` is `Field`.
    pub field: Option<InviteField>,
    /// The server's own French sentence, when `status` is `Field`.
    pub message: String,
}

/// The account already linked to this person, as the card summarises it.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct LinkedAccount {
    pub id: String,
    pub label: String,
    /// `"arl"`, `"native"`, `"headers"`, `"server"`.
    pub mode: String,
    /// `"ok"`, `"auth"`, `"network"`.
    pub status: String,
    pub server: bool,
    pub added_artists: u32,
    pub added_tracks: u32,
    /// `"auth"` or `"network"` when the last import failed.
    pub last_reason: Option<String>,
    /// The import in flight on the server, if any.
    pub running: Option<AccountRun>,
}

/// An import the server is running for a linked account.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct AccountRun {
    /// `"connect"`, `"list"`, `"import"`.
    pub phase: String,
    pub done: u32,
    pub total: Option<u32>,
}

/// One provider's card at the account step.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct ProviderOptions {
    pub available: bool,
    pub can_link: bool,
    pub linked: bool,
    pub denied: bool,
    /// Why it cannot be offered (`"absent"`, `"deezer_disabled"`, `"disabled"`).
    pub reason: Option<String>,
    pub account: Option<LinkedAccount>,
}

impl From<core::ProviderOptions> for ProviderOptions {
    fn from(provider: core::ProviderOptions) -> Self {
        ProviderOptions {
            available: provider.available,
            can_link: provider.can_link,
            linked: provider.linked,
            denied: provider.denied,
            reason: provider.reason,
            account: provider.account.map(|account| LinkedAccount {
                id: account.id,
                label: account.label,
                mode: account.mode,
                status: account.status,
                server: account.server,
                added_artists: account.added_artists,
                added_tracks: account.added_tracks,
                last_reason: account.last_reason,
                running: account.running.map(|running| AccountRun {
                    phase: running.phase,
                    done: running.done,
                    total: running.total,
                }),
            }),
        }
    }
}

/// One way to bring a library in (the account step's third card).
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct AccountImportOffer {
    pub source: String,
    pub label: String,
    pub available: bool,
    pub reason: Option<String>,
}

impl From<core::ImportOffer> for AccountImportOffer {
    fn from(offer: core::ImportOffer) -> Self {
        AccountImportOffer {
            source: offer.source,
            label: offer.label,
            available: offer.available,
            reason: offer.reason,
        }
    }
}

/// The import job's phase.
#[derive(uniffi::Enum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImportState {
    #[default]
    Reading,
    Matching,
    Saving,
    Done,
    Saved,
    Error,
}

impl From<core::ImportState> for ImportState {
    fn from(state: core::ImportState) -> Self {
        match state {
            core::ImportState::Reading => ImportState::Reading,
            core::ImportState::Matching => ImportState::Matching,
            core::ImportState::Saving => ImportState::Saving,
            core::ImportState::Done => ImportState::Done,
            core::ImportState::Saved => ImportState::Saved,
            core::ImportState::Error => ImportState::Error,
        }
    }
}

/// One line of the pasted playlist, and the cause when nothing was found.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct ImportItem {
    pub title: String,
    /// The credited artists, joined as the line names them.
    pub artists: String,
    pub album: String,
    pub duration_s: u32,
    pub matched: bool,
    pub via: Option<String>,
    pub sure: bool,
}

impl From<core::ImportItem> for ImportItem {
    fn from(item: core::ImportItem) -> Self {
        ImportItem {
            title: item.title,
            artists: item.artists,
            album: item.album,
            duration_s: item.duration_s,
            matched: item.matched,
            via: item.via,
            sure: item.sure,
        }
    }
}

/// A playlist import's progress, and the cause of each line nobody found.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct ImportJob {
    pub id: String,
    pub state: ImportState,
    pub source: String,
    pub name: String,
    pub total: u32,
    pub done: u32,
    pub matched: u32,
    pub truncated: bool,
    /// The job's own cause, or the note a finished import keeps.
    pub error: Option<String>,
    /// Only once the job is done or saved (`_snapshot`'s own rule).
    pub items: Vec<ImportItem>,
    /// How many of `items` found nothing: the screen's count, without walking
    /// the list again in Kotlin.
    pub unmatched: u32,
}

impl From<core::ImportJob> for ImportJob {
    fn from(job: core::ImportJob) -> Self {
        let unmatched = job.unmatched_count() as u32;
        ImportJob {
            id: job.id,
            state: job.state.into(),
            source: job.source,
            name: job.name,
            total: job.total,
            done: job.done,
            matched: job.matched,
            truncated: job.truncated,
            error: job.error,
            items: job.items.into_iter().map(ImportItem::from).collect(),
            unmatched,
        }
    }
}

/// `/api/onboarding/account/options` — what the step may offer now.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct AccountOptions {
    pub step: OnboardingStep,
    pub deezer: ProviderOptions,
    pub youtube: ProviderOptions,
    pub imports: Vec<AccountImportOffer>,
    pub can_save_imports: bool,
    /// The import job the step is already running, when one is.
    pub job: Option<ImportJob>,
}

impl From<core::AccountOptions> for AccountOptions {
    fn from(options: core::AccountOptions) -> Self {
        AccountOptions {
            step: options.step.into(),
            deezer: options.deezer.into(),
            youtube: options.youtube.into(),
            imports: options
                .imports
                .into_iter()
                .map(AccountImportOffer::from)
                .collect(),
            can_save_imports: options.can_save_imports,
            job: options.job.map(ImportJob::from),
        }
    }
}

/// `POST /api/onboarding/account/import`'s answer: the job to poll, and the
/// options as the server re-read them.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct StartedImport {
    pub job: ImportJob,
    pub options: AccountOptions,
}

/// `lib/onboarding.js#finishBlocked`'s answer: why the step cannot end yet.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishBlock {
    /// Fewer than `min` artists kept.
    Min,
    /// The link is down.
    Offline,
    /// A like is still being written.
    Pending,
}

/// `lib/onboarding.js#feedCause`'s codes, so the screen writes the sentence.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedCause {
    DeezerUnreachable,
    Disabled,
    Held,
    Absent,
    Busy,
    Error,
    Unknown,
}

/// What one insertion of neighbours did.
#[derive(uniffi::Record, Debug, Clone, Default, PartialEq)]
pub struct MergeResult {
    pub items: Vec<ArtistCard>,
    pub inserted: u32,
}

/// The invitation form's numbers, so the screen's hints cannot drift from the
/// core's rules.
#[derive(uniffi::Record, Debug, Clone, Copy, PartialEq, Eq)]
pub struct InviteRules {
    pub min_username: u32,
    pub max_username: u32,
    pub min_password: u32,
    pub max_password: u32,
}

/// The password meter's word (`passwordLabel`); the SCREEN writes the French.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordStrength {
    TooShort,
    Weak,
    Fair,
    Good,
    VeryGood,
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    // -- invitation ---------------------------------------------------------

    /// `POST /api/invites/inspect`. A malformed token costs **0 requests**; a
    /// gone link is `valid: false` (the page's invalid state has no retry).
    pub async fn inspect_invite(&self, code: String) -> Result<InviteInfo, NzError> {
        self.core()
            .inspect_invite(&code)
            .await
            .map(InviteInfo::from)
            .map_err(NzError::from)
    }

    /// `POST /api/invites/redeem` — ONE request, three answers. A refusal names
    /// its field; a `404` is the invalid link; only a network failure is `Err`.
    pub async fn redeem_invite(
        &self,
        code: String,
        username: String,
        password: String,
    ) -> Result<RedeemAnswer, NzError> {
        let outcome = self
            .core()
            .redeem_invite(&code, &username, &password)
            .await
            .map_err(NzError::from)?;
        Ok(match outcome {
            core::RedeemOutcome::Created(redeemed) => RedeemAnswer {
                status: RedeemStatus::Created,
                me: Some(redeemed.me.into()),
                onboarding: redeemed.onboarding.into(),
                field: None,
                message: String::new(),
            },
            core::RedeemOutcome::Field { field, message } => RedeemAnswer {
                status: RedeemStatus::Field,
                me: None,
                onboarding: OnboardingStep::Done,
                field: Some(field.into()),
                message,
            },
            core::RedeemOutcome::Invalid => RedeemAnswer {
                status: RedeemStatus::Invalid,
                me: None,
                onboarding: OnboardingStep::Done,
                field: None,
                message: String::new(),
            },
        })
    }

    /// The person already signed in, for the page's "tu es déjà connecté" state —
    /// **0 requests** (the SPA reads its store).
    pub fn invite_account(&self) -> Option<Me> {
        core::signed_in(self.core()).map(Me::from)
    }

    // -- the artists step ---------------------------------------------------

    /// The copy this device kept: disk only, no request. `None` the first time.
    pub fn cached_onboarding(&self) -> Option<OnboardingData> {
        self.core().cached_onboarding().map(OnboardingData::from)
    }

    /// The visit's two reads, concurrently: **two requests cold, 0 warm**.
    pub async fn onboarding(&self) -> Result<OnboardingData, NzError> {
        self.core()
            .onboarding()
            .await
            .map(OnboardingData::from)
            .map_err(NzError::from)
    }

    /// `GET /api/onboarding` alone: the feed's build poll, one request.
    pub async fn onboarding_state(&self) -> Result<OnboardingState, NzError> {
        self.core()
            .onboarding_state()
            .await
            .map(OnboardingState::from)
            .map_err(NzError::from)
    }

    /// `GET /api/onboarding/artists?after=&limit=` — one page, one request.
    pub async fn onboarding_artists(
        &self,
        after: Option<String>,
        limit: Option<u32>,
    ) -> Result<ArtistPage, NzError> {
        self.core()
            .onboarding_artists(after.as_deref(), limit)
            .await
            .map(ArtistPage::from)
            .map_err(NzError::from)
    }

    /// `GET /api/onboarding/search?q=&limit=` — one request per settled term.
    pub async fn onboarding_search(
        &self,
        query: String,
        limit: Option<u32>,
    ) -> Result<ArtistSearch, NzError> {
        self.core()
            .onboarding_search(&query, limit)
            .await
            .map(ArtistSearch::from)
            .map_err(NzError::from)
    }

    /// `POST /api/onboarding/artists/like` — one request, neighbours included.
    pub async fn onboarding_like(
        &self,
        artist: String,
        seen: Vec<String>,
    ) -> Result<RelatedAnswer, NzError> {
        self.core()
            .onboarding_like(&artist, &seen)
            .await
            .map(RelatedAnswer::from)
            .map_err(NzError::from)
    }

    /// `DELETE /api/onboarding/artists/<id>/like` — one request.
    pub async fn onboarding_unlike(&self, artist: String) -> Result<(), NzError> {
        self.core()
            .onboarding_unlike(&artist)
            .await
            .map_err(NzError::from)
    }

    /// `GET /api/onboarding/artists/<id>/related?seen=` — the poll.
    pub async fn onboarding_related(
        &self,
        artist: String,
        seen: Vec<String>,
    ) -> Result<RelatedAnswer, NzError> {
        self.core()
            .onboarding_related(&artist, &seen)
            .await
            .map(RelatedAnswer::from)
            .map_err(NzError::from)
    }

    /// `POST /api/onboarding/artists/done` (or `{skip: true}`).
    pub async fn onboarding_artists_done(&self, skip: bool) -> Result<ArtistsDone, NzError> {
        self.core()
            .onboarding_artists_done(skip)
            .await
            .map(ArtistsDone::from)
            .map_err(NzError::from)
    }

    /// `POST /api/onboarding/feed/retry` — one request, rate-limited by the
    /// server (`NotReady` carries the wait).
    pub async fn onboarding_feed_retry(&self) -> Result<(), NzError> {
        self.core()
            .onboarding_feed_retry()
            .await
            .map_err(NzError::from)
    }

    /// `POST /api/onboarding/account/done`.
    pub async fn onboarding_account_done(&self) -> Result<OnboardingStep, NzError> {
        self.core()
            .onboarding_account_done()
            .await
            .map(OnboardingStep::from)
            .map_err(NzError::from)
    }

    // -- the account step ---------------------------------------------------

    /// `GET /api/onboarding/account/options` — one request, no provider asked.
    pub async fn account_options(&self) -> Result<AccountOptions, NzError> {
        self.core()
            .account_options()
            .await
            .map(AccountOptions::from)
            .map_err(NzError::from)
    }

    /// `POST /api/onboarding/account/link` — one request. `provider` is
    /// `"deezer"` (the secret is an ARL) or `"youtube"` (the headers of a
    /// signed-in browse request).
    pub async fn link_account(
        &self,
        provider: String,
        secret: String,
    ) -> Result<AccountOptions, NzError> {
        self.core()
            .link_account(&provider, &secret)
            .await
            .map(AccountOptions::from)
            .map_err(NzError::from)
    }

    /// `POST /api/onboarding/account/import` — one request, then the job's poll.
    pub async fn start_import(&self, url: String) -> Result<StartedImport, NzError> {
        let started = self
            .core()
            .start_import(&url)
            .await
            .map_err(NzError::from)?;
        Ok(StartedImport {
            job: started.job.into(),
            options: started.options.into(),
        })
    }

    /// `GET /api/import/playlist/<id>` — progress, phase and each line's cause.
    pub async fn import_status(&self, id: String) -> Result<ImportJob, NzError> {
        self.core()
            .import_status(&id)
            .await
            .map(ImportJob::from)
            .map_err(NzError::from)
    }

    /// `POST /api/import/playlist/<id>/save` — keeps the import as a LOCAL
    /// playlist (never a provider write).
    pub async fn import_save(&self, id: String, title: String) -> Result<ImportJob, NzError> {
        self.core()
            .import_save(&id, &title)
            .await
            .map(ImportJob::from)
            .map_err(NzError::from)
    }
}

// -- the ported rules, as free functions --------------------------------------
//
// `lib/invite.js` and `lib/onboarding.js` live in `nz-core`, so the phone never
// keeps a second copy of a fold, a threshold or an insertion policy. A JVM test
// drives the SCREEN's state machine through its ports; these are the ports'
// implementations.

/// The invitation form's numbers.
#[uniffi::export]
pub fn nz_invite_rules() -> InviteRules {
    InviteRules {
        min_username: core::rules::MIN_USERNAME as u32,
        max_username: core::rules::MAX_USERNAME as u32,
        min_password: core::rules::MIN_PASSWORD as u32,
        max_password: core::rules::MAX_PASSWORD as u32,
    }
}

/// `tokenShapeOk`: the link's `22.token` shape, before any request.
#[uniffi::export]
pub fn nz_invite_token_ok(token: String) -> bool {
    core::rules::token_shape_ok(&token)
}

/// `foldName`: folded without touching the edges, for a field being typed.
#[uniffi::export]
pub fn nz_invite_fold_name(raw: String) -> String {
    core::rules::fold_name(&raw)
}

/// `foldUsername`: folded and trimmed, for the value that is sent.
#[uniffi::export]
pub fn nz_invite_fold_username(raw: String) -> String {
    core::rules::fold_username(&raw)
}

/// `usernameMessage` — empty when the name is acceptable.
#[uniffi::export]
pub fn nz_invite_username_message(raw: String) -> String {
    core::rules::username_message(&raw)
}

/// `passwordMessage` — empty when the password is acceptable.
#[uniffi::export]
pub fn nz_invite_password_message(password: String, username: String) -> String {
    core::rules::password_message(&password, &username)
}

/// `passwordLabel`, as the enum the screen words.
#[uniffi::export]
pub fn nz_invite_password_strength(password: String, username: String) -> PasswordStrength {
    match core::rules::password_label(&password, &username) {
        core::rules::Strength::TooShort => PasswordStrength::TooShort,
        core::rules::Strength::Weak => PasswordStrength::Weak,
        core::rules::Strength::Fair => PasswordStrength::Fair,
        core::rules::Strength::Good => PasswordStrength::Good,
        core::rules::Strength::VeryGood => PasswordStrength::VeryGood,
    }
}

/// `passwordScore` (0–4), for the four-segment meter.
#[uniffi::export]
pub fn nz_invite_password_score(password: String, username: String) -> u32 {
    u32::from(core::rules::password_score(&password, &username))
}

/// `landingFor`: whether a fresh account opens the onboarding.
#[uniffi::export]
pub fn nz_invite_needs_onboarding(onboarding: Option<String>) -> bool {
    core::rules::needs_onboarding(onboarding.as_deref())
}

/// The feed's first page, the SPA's 36 tiles.
#[uniffi::export]
pub fn nz_onboarding_page_first() -> u32 {
    core::PAGE_FIRST
}

/// What a scroll adds.
#[uniffi::export]
pub fn nz_onboarding_page_next() -> u32 {
    core::PAGE_NEXT
}

/// The search box's minimum: two characters.
#[uniffi::export]
pub fn nz_onboarding_search_min() -> u32 {
    core::SEARCH_MIN_CHARS as u32
}

/// `insertSimilar`: the neighbours of a liked tile, placed by the page's own
/// rules (two at most, never a duplicate or an already-liked artist, never past
/// the budget, alternating families).
#[uniffi::export]
#[allow(clippy::too_many_arguments)]
pub fn nz_onboarding_insert_similar(
    items: Vec<ArtistCard>,
    index: u32,
    similar: Vec<ArtistCard>,
    liked: Vec<String>,
    per_like: u32,
    budget: u32,
    window: u32,
    max_same_genre: u32,
    max_streak: u32,
) -> MergeResult {
    let liked: std::collections::HashSet<String> = liked.into_iter().collect();
    let items: Vec<core::rules::ArtistCard> = items.into_iter().map(Into::into).collect();
    let similar: Vec<core::rules::ArtistCard> = similar.into_iter().map(Into::into).collect();
    let out = core::rules::insert_similar(
        &items,
        index as isize,
        &similar,
        &liked,
        per_like as usize,
        budget as usize,
        core::rules::Diversity {
            window: window.max(1) as usize,
            max_same_genre: max_same_genre.max(1) as usize,
            max_streak: max_streak.max(1) as usize,
        },
    );
    MergeResult {
        items: out.items.into_iter().map(ArtistCard::from).collect(),
        inserted: out.inserted as u32,
    }
}

/// `removeSimilar`: takes the neighbour block of one parent out, keeping a tile
/// that became liked as an ordinary grid tile.
#[uniffi::export]
pub fn nz_onboarding_remove_similar(
    items: Vec<ArtistCard>,
    parent: String,
    liked: Vec<String>,
) -> Vec<ArtistCard> {
    let liked: std::collections::HashSet<String> = liked.into_iter().collect();
    let items: Vec<core::rules::ArtistCard> = items.into_iter().map(Into::into).collect();
    core::rules::remove_similar(&items, &parent, &liked)
        .into_iter()
        .map(ArtistCard::from)
        .collect()
}

/// `reconcilePage`: updates in place, appends what is new, drops duplicates.
#[uniffi::export]
pub fn nz_onboarding_reconcile(items: Vec<ArtistCard>, page: Vec<ArtistCard>) -> Vec<ArtistCard> {
    let items: Vec<core::rules::ArtistCard> = items.into_iter().map(Into::into).collect();
    let page: Vec<core::rules::ArtistCard> = page.into_iter().map(Into::into).collect();
    core::rules::reconcile_page(&items, &page)
        .into_iter()
        .map(ArtistCard::from)
        .collect()
}

/// `cacheList`: what the device keeps between two visits, bounded.
#[uniffi::export]
pub fn nz_onboarding_cache_list(items: Vec<ArtistCard>, max: u32) -> Vec<ArtistCard> {
    let items: Vec<core::rules::ArtistCard> = items.into_iter().map(Into::into).collect();
    core::rules::cache_list(&items, max as usize)
        .into_iter()
        .map(ArtistCard::from)
        .collect()
}

/// `finishBlocked`: why the artist step cannot end yet.
#[uniffi::export]
pub fn nz_onboarding_finish_blocked(
    liked: u32,
    pending: u32,
    queued: u32,
    online: bool,
    min: u32,
) -> Option<FinishBlock> {
    core::rules::finish_blocked(
        liked as usize,
        pending as usize,
        queued as usize,
        online,
        min as usize,
    )
    .map(|blocked| match blocked {
        core::rules::FinishBlock::Min => FinishBlock::Min,
        core::rules::FinishBlock::Offline => FinishBlock::Offline,
        core::rules::FinishBlock::Pending => FinishBlock::Pending,
    })
}

/// `canPassArtists`: "Passer pour l'instant" appears only after two attempts at
/// a feed the server itself says is unavailable.
#[uniffi::export]
pub fn nz_onboarding_can_pass(
    attempts: u32,
    feed_state: String,
    building: bool,
    count: u32,
    min: u32,
) -> bool {
    core::rules::can_pass_artists(
        attempts,
        &feed_state,
        building,
        count as usize,
        min as usize,
    )
}

/// `feedCause`: the provider's cause, as the enum the screen words.
#[uniffi::export]
pub fn nz_onboarding_feed_cause(code: Option<String>) -> FeedCause {
    match core::rules::feed_cause(code.as_deref()) {
        core::rules::FeedCause::DeezerUnreachable => FeedCause::DeezerUnreachable,
        core::rules::FeedCause::Disabled => FeedCause::Disabled,
        core::rules::FeedCause::Held => FeedCause::Held,
        core::rules::FeedCause::Absent => FeedCause::Absent,
        core::rules::FeedCause::Busy => FeedCause::Busy,
        core::rules::FeedCause::Error => FeedCause::Error,
        core::rules::FeedCause::Unknown => FeedCause::Unknown,
    }
}

/// `normalizeQuery`: one line, edges trimmed, 80 characters at most.
#[uniffi::export]
pub fn nz_onboarding_normalize_query(raw: String) -> String {
    core::rules::normalize_query(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, name: &str, genre: Option<&str>) -> ArtistCard {
        ArtistCard {
            id: id.to_owned(),
            deezer_id: id.to_owned(),
            name: name.to_owned(),
            genres: genre
                .map(|genre| vec![genre.to_owned()])
                .unwrap_or_default(),
            ..ArtistCard::default()
        }
    }

    /// Every enum crosses both ways: a screen never maps a string, and a new core
    /// variant is a compile error here.
    #[test]
    fn every_enum_crosses_both_ways() {
        for step in [
            OnboardingStep::Artists,
            OnboardingStep::Account,
            OnboardingStep::Done,
        ] {
            let inner: core::Step = match step {
                OnboardingStep::Artists => core::Step::Artists,
                OnboardingStep::Account => core::Step::Account,
                OnboardingStep::Done => core::Step::Done,
            };
            assert_eq!(OnboardingStep::from(inner), step);
        }
        for state in [
            RelatedState::None,
            RelatedState::Pending,
            RelatedState::Ready,
            RelatedState::Error,
        ] {
            let inner = match state {
                RelatedState::None => core::RelatedState::None,
                RelatedState::Pending => core::RelatedState::Pending,
                RelatedState::Ready => core::RelatedState::Ready,
                RelatedState::Error => core::RelatedState::Error,
            };
            assert_eq!(RelatedState::from(inner), state);
        }
        for state in [
            ImportState::Reading,
            ImportState::Matching,
            ImportState::Saving,
            ImportState::Done,
            ImportState::Saved,
            ImportState::Error,
        ] {
            let inner = match state {
                ImportState::Reading => core::ImportState::Reading,
                ImportState::Matching => core::ImportState::Matching,
                ImportState::Saving => core::ImportState::Saving,
                ImportState::Done => core::ImportState::Done,
                ImportState::Saved => core::ImportState::Saved,
                ImportState::Error => core::ImportState::Error,
            };
            assert_eq!(ImportState::from(inner), state);
        }
        for field in [
            InviteField::Username,
            InviteField::Password,
            InviteField::Confirm,
        ] {
            let inner = match field {
                InviteField::Username => core::InviteField::Username,
                InviteField::Password => core::InviteField::Password,
                InviteField::Confirm => core::InviteField::Confirm,
            };
            assert_eq!(InviteField::from(inner), field);
        }
    }

    /// The screen's insertion rule IS the page's: two tiles, right after the
    /// liked one, never a duplicate.
    #[test]
    fn the_boundary_inserts_neighbours_the_pages_way() {
        let items = vec![
            card("100", "A", Some("Pop")),
            card("101", "B", Some("Rock")),
            card("102", "C", Some("Pop")),
        ];
        let similar = vec![
            card("900", "Proche A", Some("Rap")),
            card("901", "Proche B", Some("Rock")),
            card("101", "Déjà là", Some("Rock")),
        ];
        let out = nz_onboarding_insert_similar(items, 1, similar, vec![], 2, 12, 12, 4, 2);
        assert_eq!(out.inserted, 2);
        let ids: Vec<&str> = out
            .items
            .iter()
            .map(|item| item.deezer_id.as_str())
            .collect();
        assert_eq!(ids, ["100", "101", "900", "901", "102"]);
        assert_eq!(out.items[2].via.as_deref(), Some("related"));
        assert_eq!(out.items[2].parent.as_deref(), Some("101"));
    }

    /// A liked neighbour is never inserted, and removing a block keeps a tile
    /// that became liked.
    #[test]
    fn a_liked_neighbour_is_not_inserted_twice() {
        let items = vec![card("100", "A", Some("Pop"))];
        let similar = vec![card("900", "Déjà aimé", Some("Rap"))];
        let out = nz_onboarding_insert_similar(
            items,
            0,
            similar,
            vec!["900".to_owned()],
            2,
            12,
            12,
            4,
            2,
        );
        assert_eq!(out.inserted, 0);

        let merged = vec![
            card("100", "A", Some("Pop")),
            ArtistCard {
                via: Some("related".to_owned()),
                parent: Some("100".to_owned()),
                ..card("900", "Proche", Some("Rap"))
            },
        ];
        let kept = nz_onboarding_remove_similar(merged.clone(), "100".into(), vec![]);
        assert_eq!(kept.len(), 1);
        let kept = nz_onboarding_remove_similar(merged, "100".into(), vec!["900".to_owned()]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[1].via.as_deref(), Some("grid"));
    }

    /// The form's rules cross: the screen's messages and its meter are the
    /// page's own sentences.
    #[test]
    fn the_form_rules_cross() {
        assert_eq!(
            nz_invite_rules(),
            InviteRules {
                min_username: 3,
                max_username: 32,
                min_password: 12,
                max_password: 128,
            }
        );
        assert!(nz_invite_token_ok(format!(
            "{}.{}",
            "a".repeat(22),
            "b".repeat(24)
        )));
        assert!(!nz_invite_token_ok("court".into()));
        assert_eq!(nz_invite_fold_username("Élodie".into()), "elodie");
        assert_eq!(nz_invite_username_message("elodie".into()), "");
        assert!(nz_invite_username_message("ab".into()).contains("3"));
        assert_eq!(
            nz_invite_password_strength("abcdefghijklmnopqrstuvwx".into(), String::new()),
            PasswordStrength::VeryGood
        );
        assert_eq!(
            nz_invite_password_strength("court".into(), String::new()),
            PasswordStrength::TooShort
        );
        assert_eq!(
            nz_invite_password_score("abcdefghijkl".into(), String::new()),
            2
        );
        assert!(nz_invite_needs_onboarding(Some("artists".into())));
        assert!(!nz_invite_needs_onboarding(Some("done".into())));
        assert!(!nz_invite_needs_onboarding(None));
    }

    /// A finish is refused for the reason the screen prints, and "Passer" waits
    /// for a feed the server itself calls unavailable.
    #[test]
    fn the_finish_and_the_pass_follow_the_plans_rules() {
        assert_eq!(
            nz_onboarding_finish_blocked(0, 0, 0, true, 1),
            Some(FinishBlock::Min)
        );
        assert_eq!(
            nz_onboarding_finish_blocked(1, 0, 0, false, 1),
            Some(FinishBlock::Offline)
        );
        assert_eq!(
            nz_onboarding_finish_blocked(1, 1, 0, true, 1),
            Some(FinishBlock::Pending)
        );
        assert_eq!(nz_onboarding_finish_blocked(1, 0, 0, true, 1), None);
        assert!(!nz_onboarding_can_pass(
            1,
            "unavailable".into(),
            false,
            0,
            1
        ));
        assert!(nz_onboarding_can_pass(2, "unavailable".into(), false, 0, 1));
        assert!(!nz_onboarding_can_pass(2, "unavailable".into(), true, 0, 1));
        assert_eq!(
            nz_onboarding_feed_cause(Some("deezer-unreachable".into())),
            FeedCause::DeezerUnreachable
        );
        assert_eq!(nz_onboarding_feed_cause(None), FeedCause::Unknown);
    }

    /// The constants the screen's copy depends on are the core's own.
    #[test]
    fn the_screens_constants_are_the_cores() {
        assert_eq!(nz_onboarding_page_first(), 36);
        assert_eq!(nz_onboarding_page_next(), 24);
        assert_eq!(nz_onboarding_search_min(), 2);
        assert_eq!(nz_onboarding_page_first(), core::PAGE_FIRST);
        assert_eq!(nz_onboarding_page_next(), core::PAGE_NEXT);
        assert_eq!(nz_onboarding_search_min() as usize, core::SEARCH_MIN_CHARS);
    }
}
