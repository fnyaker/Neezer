//! Onboarding and invitation redemption.
//!
//! Owned by **MB5b** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this directory and edits no other file of the crate. MB0 created
//! the module empty so that no later lot ever edits `lib.rs`.
//!
//! The behaviour is `webapp/src/routes/Invite.svelte`,
//! `webapp/src/routes/Onboarding.svelte`, `webapp/src/components/Onboarding*`
//! and the two `lib/*.js` they rest on — PORTED, never re-invented. What lives
//! where:
//!
//! | This module | The SPA |
//! |---|---|
//! | [`rules`] | `lib/invite.js` (folding, the form's rules, the token) and `lib/onboarding.js` (insertion, diversity, finish, feed causes) |
//! | [`wire`] | the routes, the one snapshot the device keeps, the draft, the import job's shape |
//! | [`Core::inspect_invite`], [`Core::redeem_invite`] | `Invite.svelte#inspect`/`#submit`, `lib/invite.js#landingFor` |
//! | [`Core::onboarding`], [`Core::cached_onboarding`] | `Onboarding.svelte#load`, the visit's copy |
//! | [`Core::onboarding_artists`], [`Core::onboarding_search`] | `#loadPage`, `#runSearch` |
//! | [`Core::onboarding_like`], [`Core::onboarding_unlike`], [`Core::onboarding_related`] | `#like`, `#unlike`, `#mergeRelated`/`#pollRelated` |
//! | [`Core::account_options`], [`Core::link_account`], [`Core::start_import`] | `OnboardingAccount.svelte` |
//!
//! # The request budget (rule 3)
//!
//! The artists step costs **two** requests cold, the SPA's own two:
//! `GET /api/onboarding` and `GET /api/onboarding/artists?limit=36`, fired
//! CONCURRENTLY on the `User` lane (`Onboarding.svelte#load` asks them in
//! sequence; the answer is the same and the wait is halved). A warm visit paints
//! the device's copy with **0** requests. The account step is **one**
//! (`/onboarding/account/options`), a search **one**, a like **one** (its answer
//! already carries the neighbours), an import **one** to start and then the job's
//! own poll. The invitation page is **one** to inspect and **one** to redeem.
//!
//! Every request is counted under [`TAG_ONBOARDING`] (the account step and its
//! writes included) or [`TAG_INVITE`], so `NzCore::stats()` answers what the
//! screen cost — and the import's poll has [`TAG_IMPORT`] of its own, because it
//! keeps running after the screen is left.
//!
//! # What is kept, and where
//!
//! [`Core::cached_onboarding`] answers from `onboarding-<account hash>.json` in
//! the CACHE directory (evictable, like every cache): the person's step, the
//! feed's state and the first page of tiles. A warm visit therefore paints
//! before any request, and a dead link keeps what it painted with « Hors ligne ·
//! version mémorisée ». The like queue and the invitation draft live in `kv`,
//! both keyed by account (the draft by token): the SPA's `sessionStorage` and
//! `nsInvite.draft`, with the same ten-minute life and never the password.
//!
//! The import itself is the SERVER's job (rule 2): leaving the screen stops the
//! poll, never the work, and coming back reads `/onboarding/account/options`'s
//! `job` (or the draft's id) and resumes where it was.

pub mod rules;
mod wire;

#[cfg(test)]
mod testing;

use std::sync::Arc;

use nz_model::Me;
use serde::{Deserialize, Serialize};

pub use rules::{
    ArtistCard, Diversity, FeedCause, FinishBlock, InsertResult, PasswordProblem, Strength,
    UsernameProblem,
};
pub use wire::InviteDraft;

use crate::core::{Core, CoreError};

/// The tag every request of the onboarding is counted under (`NzCore::stats`).
pub const TAG_ONBOARDING: &str = "onboarding";
/// The invitation page's own tag: two requests at most, and only while a person
/// creates an account.
pub const TAG_INVITE: &str = "invite";
/// The import job's poll: it outlives the screen, so it is counted apart.
pub const TAG_IMPORT: &str = "import";

/// `onboarding.js#PAGE_FIRST`: the feed's first page, the SPA's 36 tiles.
pub const PAGE_FIRST: u32 = 36;
/// `onboarding.js#PAGE_NEXT`: what a scroll adds.
pub const PAGE_NEXT: u32 = 24;
/// `onboarding.js#SEARCH_LIMIT`: the search box's answer.
pub const SEARCH_LIMIT: u32 = 25;
/// `onboarding.js#SEARCH_MIN_CHARS`: two characters, or no query.
pub const SEARCH_MIN_CHARS: usize = 2;
/// `onboarding.js#SIMILAR_PER_LIKE`: two neighbours after a liked tile.
pub const SIMILAR_PER_LIKE: usize = 2;
/// `onboarding.js#SIMILAR_TOTAL_MAX`: twelve inserted tiles at most.
pub const SIMILAR_TOTAL_MAX: usize = 12;
/// `onboarding.js#CACHE_ITEMS_MAX`: what one visit keeps of the feed.
pub const CACHE_ITEMS_MAX: usize = 120;
/// `onboarding.js#CACHE_TTL`: thirty minutes, the page's own life.
pub const LIST_TTL_SECS: u64 = 30 * 60;

/// Where the person stands in the first-run flow (`/api/onboarding`'s `step`).
/// An unknown or missing step is [`Step::Done`], exactly as `stepFor` reads it:
/// an older server must never trap somebody in an unfinished onboarding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    Artists,
    Account,
    #[default]
    Done,
}

impl Step {
    pub fn slug(self) -> &'static str {
        match self {
            Step::Artists => "artists",
            Step::Account => "account",
            Step::Done => "done",
        }
    }

    pub fn parse(raw: &str) -> Step {
        match raw {
            "artists" => Step::Artists,
            "account" => Step::Account,
            _ => Step::Done,
        }
    }
}

/// The artists feed's own state (`/api/onboarding`'s `feed`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    pub state: String,
    pub building: bool,
    pub error: Option<String>,
    pub source: String,
    pub ready: bool,
    pub stale: bool,
    pub progress: Option<f64>,
}

impl Feed {
    /// `onboarding.js#canPassArtists`'s `feed` half: the server says there is
    /// nothing to propose AND it is not still preparing one.
    pub fn unavailable(&self) -> bool {
        self.state == "unavailable" && !self.building
    }
}

/// `/api/onboarding` — where the first-run flow stands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingState {
    pub step: Step,
    pub liked: u32,
    pub min_artists: u32,
    pub feed: Feed,
    pub diversity: Diversity,
}

/// One page of the feed (`/api/onboarding/artists`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

/// What one visit paints: the person's step and the feed's first page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingData {
    pub state: OnboardingState,
    pub page: ArtistPage,
}

/// `/api/onboarding/search` — the box's own answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArtistSearch {
    pub query: String,
    pub items: Vec<ArtistCard>,
    /// How many hits the LIBRARY itself had.
    pub local: u32,
    pub partial: bool,
    /// The PROVIDER's cause (`FeedCause`), never a failed request: the answer is
    /// 200 with the library's rows and this code beside them.
    pub error: Option<String>,
}

/// The neighbours' state (`similar_payload`'s `state`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RelatedState {
    /// The provider has no neighbours for this tile; nothing to draw.
    #[default]
    None,
    /// The server is preparing them: poll (the page's 900 ms then 1 500 ms).
    Pending,
    /// They are here.
    Ready,
    /// The provider refused: the tile's own line with « Réessayer ».
    Error,
}

/// `POST /api/onboarding/artists/like`'s `related`, and the related route.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelatedAnswer {
    pub state: RelatedState,
    pub items: Vec<ArtistCard>,
    pub cause: Option<String>,
}

/// `/api/onboarding/artists/done` and `/api/onboarding/account/done`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArtistsDone {
    pub step: Step,
    pub skipped: bool,
}

/// The invitation as inspected: whether the link can be used, and what the
/// screen says above the form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InviteInfo {
    pub valid: bool,
    pub label: String,
    /// ISO-8601, as the server says it. The SCREEN formats it in French.
    pub expires: Option<String>,
    /// The name a previous attempt typed, from the device's draft.
    pub draft_name: Option<String>,
}

/// Which field the server refused, so the screen puts the sentence under it
/// (`Invite.svelte#handleFailure`'s `map`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteField {
    Username,
    Password,
    Confirm,
}

/// What redeeming did, in the three outcomes the page draws. A network failure
/// is the `Err` side; these three are ANSWERS.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum RedeemOutcome {
    /// The account exists: the session cookie is kept and the person is signed
    /// in.
    Created(Box<Redeemed>),
    /// The form was refused: the server's own sentence, under its field.
    Field { field: InviteField, message: String },
    /// The link is gone (404): the page switches to "plus valide".
    #[default]
    Invalid,
}

/// A person whose account was just created.
#[derive(Debug, Clone, PartialEq)]
pub struct Redeemed {
    pub me: Me,
    /// `"artists"`, `"account"` or `"done"` — where the account lands
    /// (`landingFor`).
    pub onboarding: Step,
}

/// One provider's card at the account step (`/onboarding/account/options`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderOptions {
    pub available: bool,
    pub can_link: bool,
    pub linked: bool,
    pub denied: bool,
    /// Why it cannot be offered (`"absent"`, `"deezer_disabled"`, `"disabled"`).
    pub reason: Option<String>,
    /// The account already linked, when one is.
    pub account: Option<LinkedAccount>,
}

/// The account already linked to this person, as the card summarises it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkedAccount {
    pub id: String,
    pub label: String,
    /// `"arl"`, `"native"`, `"headers"`, `"server"`.
    pub mode: String,
    /// `"ok"`, `"auth"`, `"network"`.
    pub status: String,
    /// Whether the server itself holds this account (`mode == "server"`).
    pub server: bool,
    /// What the last import added (`last.added`).
    pub added_artists: u32,
    pub added_tracks: u32,
    /// `"auth"` or `"network"` when the last import failed (`last.reason`).
    pub last_reason: Option<String>,
    /// The import in flight on the server, if any (`li.status`).
    pub running: Option<AccountRun>,
}

/// An import the server is running for a linked account.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountRun {
    /// `"connect"`, `"list"`, `"import"`.
    pub phase: String,
    pub done: u32,
    pub total: Option<u32>,
}

/// One way to bring a library in (the account step's third card).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportOffer {
    pub source: String,
    pub label: String,
    pub available: bool,
    pub reason: Option<String>,
}

/// The import job's phase (`playlist_import._RUNNING` + its finals).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImportState {
    #[default]
    Reading,
    Matching,
    Saving,
    Done,
    Saved,
    Error,
}

impl ImportState {
    pub fn slug(self) -> &'static str {
        match self {
            ImportState::Reading => "reading",
            ImportState::Matching => "matching",
            ImportState::Saving => "saving",
            ImportState::Done => "done",
            ImportState::Saved => "saved",
            ImportState::Error => "error",
        }
    }

    pub fn parse(raw: &str) -> ImportState {
        match raw {
            "reading" => ImportState::Reading,
            "matching" => ImportState::Matching,
            "saving" => ImportState::Saving,
            "saved" => ImportState::Saved,
            "error" => ImportState::Error,
            _ => ImportState::Done,
        }
    }

    /// `isImportRunning`: the three phases with work left.
    pub fn running(self) -> bool {
        rules::import_running(self.slug())
    }
}

/// One line of the pasted playlist, and whether a sound was found for it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportItem {
    pub title: String,
    /// The credited artists, joined as the line names them.
    pub artists: String,
    pub album: String,
    pub duration_s: u32,
    /// Whether the line was matched to something this server can play.
    pub matched: bool,
    /// `"local"`, `"deezer"`, `"youtube"` — the source that answered.
    pub via: Option<String>,
    /// Whether the match is sure (`sure: false` is a close guess).
    pub sure: bool,
}

/// A playlist import's progress, exactly what a poll reads
/// (`playlist_import._snapshot`), and the CAUSE of each line that was not found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportJob {
    pub id: String,
    pub state: ImportState,
    pub source: String,
    pub name: String,
    pub total: u32,
    pub done: u32,
    pub matched: u32,
    pub truncated: bool,
    /// The job's own cause when it failed, or the note a finished import keeps
    /// (« Deezer ne répondait pas : certains titres… »).
    pub error: Option<String>,
    /// Only once the job is done or saved: a poll of a long import must not
    /// re-send a growing list (`_snapshot`'s own rule).
    pub items: Vec<ImportItem>,
}

impl ImportJob {
    /// `unmatchedItems`: the lines no sound was found for — each one's cause is
    /// its own title and artists.
    pub fn unmatched(&self) -> Vec<&ImportItem> {
        self.items.iter().filter(|item| !item.matched).collect()
    }

    pub fn unmatched_count(&self) -> usize {
        self.unmatched().len()
    }
}

/// `/api/onboarding/account/options` — what the step may offer now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountOptions {
    pub step: Step,
    pub deezer: ProviderOptions,
    pub youtube: ProviderOptions,
    pub imports: Vec<ImportOffer>,
    pub can_save_imports: bool,
    /// The import job the step is already running, when one is.
    pub job: Option<ImportJob>,
}

/// `POST /api/onboarding/account/import`'s answer: the job to poll, and the
/// options as the server re-read them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StartedImport {
    pub job: ImportJob,
    pub options: AccountOptions,
}

impl Core {
    // -- invitation ---------------------------------------------------------

    /// `POST /api/invites/inspect` — what an invitation link is for, BEFORE the
    /// form. A malformed token costs **0** requests (`tokenShapeOk`), and a
    /// `404` is an ANSWER (`valid: false`), not a failure: the page's "plus
    /// valide" state has no « Réessayer ».
    pub async fn inspect_invite(&self, code: &str) -> Result<InviteInfo, CoreError> {
        if !rules::token_shape_ok(code) {
            return Ok(InviteInfo {
                valid: false,
                ..InviteInfo::default()
            });
        }
        let api = Arc::new(self.api()?.with_tag(TAG_INVITE));
        let before = wire::totals(&api);
        let answer = wire::fetch_invite_inspect(&api, code).await;
        wire::note_traffic(self, TAG_INVITE, &api, before);
        let info = answer?;
        let draft_name = self
            .invite_draft(code)
            .map(|draft| draft.name)
            .filter(|name| !name.is_empty());
        Ok(InviteInfo { draft_name, ..info })
    }

    /// `POST /api/invites/redeem` — creates the account, keeps the session
    /// cookie, and answers where it lands. **One request**, never retried by the
    /// client (a replay would create two accounts); the server's own
    /// `request_id` makes a person's second press the SAME attempt.
    pub async fn redeem_invite(
        &self,
        code: &str,
        username: &str,
        password: &str,
    ) -> Result<RedeemOutcome, CoreError> {
        // The form's own rules run here too: a value the server would refuse
        // must not cost a request (the page validates before submitting).
        let folded = rules::fold_username(username);
        if rules::username_problem(&folded).is_some() {
            return Ok(RedeemOutcome::Field {
                field: InviteField::Username,
                message: rules::username_message(&folded),
            });
        }
        if rules::password_problem(password, &folded).is_some() {
            return Ok(RedeemOutcome::Field {
                field: InviteField::Password,
                message: rules::password_message(password, &folded),
            });
        }
        let draft = self.invite_draft(code);
        let request_id = draft
            .map(|draft| draft.request_id)
            .filter(|id| !id.is_empty())
            .unwrap_or_else(rules::new_request_id);
        // Remembered BEFORE the request: a connection that dies after the server
        // created the account leaves the id that replays it (`Invite.svelte`).
        self.remember_invite_draft(code, &folded, &request_id);

        let api = Arc::new(self.api()?.with_tag(TAG_INVITE));
        let before = wire::totals(&api);
        let answer = wire::post_redeem(&api, code, &folded, password, &request_id).await;
        wire::note_traffic(self, TAG_INVITE, &api, before);

        match answer {
            Ok(me) => {
                let onboarding = Step::parse(me.user.onboarding.as_deref().unwrap_or_default());
                self.clear_invite_draft(code);
                self.adopt_session(me.clone());
                Ok(RedeemOutcome::Created(Box::new(Redeemed {
                    me,
                    onboarding,
                })))
            }
            Err(wire::RedeemFailure::Field { field, message }) => {
                Ok(RedeemOutcome::Field { field, message })
            }
            Err(wire::RedeemFailure::Invalid) => {
                self.clear_invite_draft(code);
                Ok(RedeemOutcome::Invalid)
            }
            Err(wire::RedeemFailure::Other(cause)) => Err(cause),
        }
    }

    /// The name a previous attempt typed for this link, while the draft lives
    /// (`readDraft`: ten minutes, and never the password).
    pub fn invite_draft(&self, code: &str) -> Option<InviteDraft> {
        wire::read_draft(self, code)
    }

    /// `rememberDraft`: the token, the name and the request id — nothing else.
    pub fn remember_invite_draft(&self, code: &str, name: &str, request_id: &str) {
        wire::write_draft(self, code, name, request_id);
    }

    /// `clearDraft`.
    pub fn clear_invite_draft(&self, code: &str) {
        wire::clear_draft(self, code);
    }

    /// Adopts a session the server just handed over (a redeemed invitation sets
    /// its cookie in the jar), so the shell and every screen see the person
    /// WITHOUT a second `GET /api/me`. `set_me` is MB2's `core/session.rs`; this
    /// lot made it `pub(crate)` — the one line it changed outside its files,
    /// named in its section of `docs/agents/mobile.md`.
    fn adopt_session(&self, me: Me) {
        self.set_me(Some(me));
    }

    // -- the artists step ---------------------------------------------------

    /// The copy this device kept for the person: disk only, no request. `None`
    /// the first time, which is the one moment the screen has nothing to paint.
    pub fn cached_onboarding(&self) -> Option<OnboardingData> {
        wire::read_snapshot(self)
    }

    /// The visit's two reads, CONCURRENTLY: `GET /api/onboarding` and
    /// `GET /api/onboarding/artists?limit=36`. **Two requests cold, 0 warm.**
    /// A `404` on `/onboarding` is a server older than the feature (rule 7).
    pub async fn onboarding(&self) -> Result<OnboardingData, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let state = wire::fetch_state(&api);
        let page = wire::fetch_page(&api, None, PAGE_FIRST);
        let (state, page) = tokio::join!(state, page);
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        let data = OnboardingData {
            state: state?,
            page: page?,
        };
        if let Err(cause) = wire::write_snapshot(self, &data) {
            tracing::debug!(%cause, "the onboarding snapshot was not kept");
        }
        Ok(data)
    }

    /// `GET /api/onboarding` alone: the feed's build poll (2 s, max 15 tries in
    /// the SPA) and the step check between the two screens. One request.
    pub async fn onboarding_state(&self) -> Result<OnboardingState, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::fetch_state(&api).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `GET /api/onboarding/artists?after=&limit=` — one page, one request.
    pub async fn onboarding_artists(
        &self,
        after: Option<&str>,
        limit: Option<u32>,
    ) -> Result<ArtistPage, CoreError> {
        let limit = limit.unwrap_or(PAGE_NEXT).clamp(1, 50);
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::fetch_page(&api, after, limit).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `GET /api/onboarding/search?q=&limit=` — one request, and one only: a new
    /// key CANCELS the previous call (the FFI future is dropped), so a fast typist
    /// pays for the term they settled on.
    pub async fn onboarding_search(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<ArtistSearch, CoreError> {
        let query = rules::normalize_query(query);
        if query.chars().count() < SEARCH_MIN_CHARS {
            return Ok(ArtistSearch {
                query,
                ..ArtistSearch::default()
            });
        }
        let limit = limit.unwrap_or(SEARCH_LIMIT).clamp(1, 50);
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::fetch_search(&api, &query, limit).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `POST /api/onboarding/artists/like` — one request, and its answer already
    /// carries the neighbours, so a like costs ONE and no follow-up.
    pub async fn onboarding_like(
        &self,
        artist: &str,
        seen: &[String],
    ) -> Result<RelatedAnswer, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::post_like(&api, artist, seen).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `DELETE /api/onboarding/artists/<id>/like` — one request.
    pub async fn onboarding_unlike(&self, artist: &str) -> Result<(), CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::delete_like(&api, artist).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `GET /api/onboarding/artists/<id>/related?seen=` — the neighbours' poll.
    pub async fn onboarding_related(
        &self,
        artist: &str,
        seen: &[String],
    ) -> Result<RelatedAnswer, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::fetch_related(&api, artist, seen).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `POST /api/onboarding/artists/done` (or `{skip: true}`): the step the
    /// server says is next. A `409` is the server's own refusal (one kept artist
    /// is the minimum, or the feed does have tiles to pick).
    pub async fn onboarding_artists_done(&self, skip: bool) -> Result<ArtistsDone, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::post_artists_done(&api, skip).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `POST /api/onboarding/feed/retry` — rebuild the feed now (3 minutes
    /// between two asks on the server; a `429` is `NotReady`).
    pub async fn onboarding_feed_retry(&self) -> Result<(), CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::post_feed_retry(&api).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `POST /api/onboarding/account/done` — the flow is over.
    pub async fn onboarding_account_done(&self) -> Result<Step, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::post_account_done(&api).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    // -- the account step ---------------------------------------------------

    /// `GET /api/onboarding/account/options` — ONE request, and it asks no
    /// provider: the server answers from its own state, 200 even with Deezer
    /// switched off, so the step always paints.
    pub async fn account_options(&self) -> Result<AccountOptions, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::fetch_account_options(&api).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `POST /api/onboarding/account/link` — the person's own Deezer (its ARL)
    /// or YouTube Music (the headers of a signed-in browse). **One request**;
    /// the server starts the import itself (rule 2), so the card polls the
    /// account's own state next.
    pub async fn link_account(
        &self,
        provider: &str,
        secret: &str,
    ) -> Result<AccountOptions, CoreError> {
        if provider != "deezer" && provider != "youtube" {
            return Err(CoreError::Invalid {
                detail: "Choisis Deezer ou YouTube Music".to_owned(),
            });
        }
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::post_account_link(&api, provider, secret).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `POST /api/onboarding/account/import` — reads and matches a public
    /// playlist link as a BACKGROUND job (rule 2). **One request** to start it,
    /// then [`Core::import_status`]'s poll.
    pub async fn start_import(&self, url: &str) -> Result<StartedImport, CoreError> {
        let url = url.trim().to_owned();
        if url.is_empty() {
            return Err(CoreError::Invalid {
                detail: "Colle le lien d'une playlist".to_owned(),
            });
        }
        let api = Arc::new(self.api()?.with_tag(TAG_ONBOARDING));
        let before = wire::totals(&api);
        let answer = wire::post_import(&api, &url).await;
        wire::note_traffic(self, TAG_ONBOARDING, &api, before);
        answer
    }

    /// `GET /api/import/playlist/<id>` — the job's progress, its phase and the
    /// cause of each line that was not found. One request per poll, and the poll
    /// stops with the screen: the job keeps running on the server.
    pub async fn import_status(&self, id: &str) -> Result<ImportJob, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_IMPORT));
        let before = wire::totals(&api);
        let answer = wire::fetch_import(&api, id).await;
        wire::note_traffic(self, TAG_IMPORT, &api, before);
        answer
    }

    /// `POST /api/import/playlist/<id>/save` — makes a LOCAL playlist and queues
    /// its tracks for archiving (never a provider write). One request.
    pub async fn import_save(&self, id: &str, title: &str) -> Result<ImportJob, CoreError> {
        let api = Arc::new(self.api()?.with_tag(TAG_IMPORT));
        let before = wire::totals(&api);
        let answer = wire::post_import_save(&api, id, title).await;
        wire::note_traffic(self, TAG_IMPORT, &api, before);
        answer
    }
}

/// The people a screen's tiles belong to, for the snapshot's file name: two
/// accounts never read each other's onboarding.
pub(crate) fn owner(core: &Core) -> String {
    core.me()
        .map(|me| me.user.id.as_str().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_default()
}

/// FNV-1a, as the search's own file name uses: a stable, short name for what was
/// typed or for whom — never a server string on disk.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Whether the person is signed in already, for the invitation page's "tu es
/// déjà connecté" state — **0 requests** (the SPA reads its store).
pub fn signed_in(core: &Core) -> Option<Me> {
    core.me()
}

#[cfg(test)]
mod lot_tests {
    use super::*;
    use crate::core::NetworkState;

    /// The artists step is the SPA's TWO requests, none warm, and one
    /// `onboarding` row in `NzCore::stats()` carrying both.
    #[tokio::test]
    async fn the_artists_step_costs_two_requests_cold_and_none_warm() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();

        let data = core.onboarding().await.expect("the artists step");
        let mut routes = server.routes();
        routes.sort();
        assert_eq!(
            routes,
            vec!["GET /api/onboarding", "GET /api/onboarding/artists"],
            "the state and the first page, and nothing else"
        );
        assert_eq!(data.state.step, Step::Artists);
        assert_eq!(data.state.liked, 1);
        assert_eq!(data.state.diversity.window, 6);
        assert_eq!(data.page.items.len(), 6);
        assert_eq!(data.page.next.as_deref(), Some("c2"));
        let stat = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_ONBOARDING)
            .expect("an onboarding row");
        assert_eq!(stat.requests, 2, "{:?}", core.stats());

        // The warm open: the copy this device kept, in ZERO requests.
        server.forget();
        let kept = core.cached_onboarding().expect("the copy");
        assert_eq!(kept.page.items.len(), 6);
        assert_eq!(kept.state.step, Step::Artists);
        assert_eq!(server.count(), 0, "a kept answer opens no socket");
    }

    /// A dead link keeps the copy; without one the cause is the server's, never
    /// a spinner.
    #[tokio::test]
    async fn a_failed_refresh_keeps_the_copy_and_says_so() {
        let (server, _dir, core) = testing::configured().await;
        core.onboarding().await.expect("a first visit");
        server.break_route("/api/onboarding/artists", 500);
        let cause = core.onboarding().await.expect_err("the refresh fails");
        assert!(
            matches!(cause, CoreError::Server { status: 500, .. }),
            "{cause:?}"
        );
        assert!(core.cached_onboarding().is_some(), "the copy survives");

        let (offline_server, _dir2, offline) = testing::configured().await;
        offline.set_network(NetworkState::None);
        let cause = offline.onboarding().await.expect_err("offline");
        assert_eq!(cause, CoreError::Offline);
        assert_eq!(
            offline_server.count(),
            2,
            "the setup's version and login, and nothing for the offline screen"
        );
    }

    /// A `/onboarding` a server does not have is `Unsupported`: the screen hides
    /// the step, it does not fail forever (rule 7).
    #[tokio::test]
    async fn a_server_older_than_the_feature_is_unsupported() {
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/onboarding", 404);
        let cause = core.onboarding_state().await.expect_err("no route");
        assert_eq!(
            cause,
            CoreError::Unsupported {
                feature: "onboarding".to_owned()
            }
        );
    }

    /// A like is ONE request and its answer already carries the neighbours; the
    /// unlike and the neighbours' poll are one each.
    #[tokio::test]
    async fn a_like_costs_one_request_and_brings_its_neighbours() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let related = core.onboarding_like("103", &[]).await.expect("a like");
        assert_eq!(server.routes(), vec!["POST /api/onboarding/artists/like"]);
        assert_eq!(related.state, RelatedState::Ready);
        assert_eq!(related.items.len(), 2);
        assert_eq!(related.items[0].parent.as_deref(), Some("103"));

        server.forget();
        core.onboarding_unlike("103").await.expect("an unlike");
        assert_eq!(
            server.routes(),
            vec!["DELETE /api/onboarding/artists/103/like"]
        );

        server.forget();
        let polled = core
            .onboarding_related("103", &["201".to_owned()])
            .await
            .expect("the neighbours");
        assert_eq!(server.count(), 1);
        assert_eq!(polled.items.len(), 1);
    }

    /// The search box is one request, and a term under two characters asks
    /// NOTHING.
    #[tokio::test]
    async fn a_search_is_one_request_and_a_short_term_is_none() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let found = core.onboarding_search("aya", None).await.expect("a search");
        assert_eq!(server.routes(), vec!["GET /api/onboarding/search"]);
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.items[0].name, "Aya Nakamura");
        assert_eq!(found.local, 1);

        server.forget();
        let short = core.onboarding_search("a", None).await.expect("too short");
        assert!(short.items.is_empty());
        assert_eq!(server.count(), 0, "one letter asks nothing");
    }

    /// The step's own transition: the artists' end answers the account step, and
    /// the account's end answers done.
    #[tokio::test]
    async fn the_steps_tell_the_server_where_they_are() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let done = core.onboarding_artists_done(false).await.expect("done");
        assert_eq!(done.step, Step::Account);
        let finished = core.onboarding_account_done().await.expect("finished");
        assert_eq!(finished, Step::Done);
        assert_eq!(
            server.routes(),
            vec![
                "POST /api/onboarding/artists/done",
                "POST /api/onboarding/account/done"
            ]
        );
    }

    /// A `409` from the artists' end is the server's refusal, and the screen says
    /// it: one kept artist is the minimum.
    #[tokio::test]
    async fn a_refused_finish_is_the_servers_own_sentence() {
        let (server, _dir, core) = testing::configured().await;
        server.answer(
            "POST",
            "/api/onboarding/artists/done",
            409,
            r#"{"error":"Suis au moins un artiste pour continuer.","code":"min_artists"}"#,
        );
        let cause = core
            .onboarding_artists_done(false)
            .await
            .expect_err("refused");
        match cause {
            CoreError::Conflict { detail } => {
                assert!(detail.contains("au moins un artiste"), "{detail}")
            }
            other => panic!("expected a conflict, got {other:?}"),
        }
    }

    /// The account step is ONE request, and its four cards are painted even with
    /// Deezer switched off; the import already running is read from it.
    #[tokio::test]
    async fn the_account_step_is_one_request_and_shows_the_running_import() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let options = core.account_options().await.expect("the options");
        assert_eq!(server.routes(), vec!["GET /api/onboarding/account/options"]);
        assert_eq!(options.step, Step::Account);
        assert!(options.deezer.can_link);
        assert!(!options.deezer.linked);
        assert_eq!(options.imports.len(), 4);
        let job = options.job.expect("the running import");
        assert_eq!(job.state, ImportState::Matching);
        assert_eq!(job.done, 42);
        assert_eq!(job.total, 120);
    }

    /// Linking an account is ONE request, and the card learns both the account
    /// and its import from that answer.
    #[tokio::test]
    async fn linking_an_account_is_one_request() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let options = core
            .link_account("deezer", "un-arl-de-test")
            .await
            .expect("a link");
        assert_eq!(server.routes(), vec!["POST /api/onboarding/account/link"]);
        assert!(options.deezer.linked);
        let account = options.deezer.account.expect("the account");
        assert_eq!(account.mode, "arl");
        let running = account.running.expect("its import");
        assert_eq!(running.phase, "import");
        assert_eq!(running.total, Some(42));
    }

    /// A started import is ONE request and an `Accepted`; its poll carries the
    /// progress AND the cause of each line nobody found.
    #[tokio::test]
    async fn an_import_starts_in_one_request_and_reports_each_line() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();
        let started = core
            .start_import("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M")
            .await
            .expect("an import");
        assert_eq!(server.routes(), vec!["POST /api/onboarding/account/import"]);
        assert_eq!(started.job.id, "job-1");
        assert_eq!(started.job.state, ImportState::Reading);

        server.forget();
        let job = core.import_status("job-1").await.expect("the poll");
        assert_eq!(server.routes(), vec!["GET /api/import/playlist/job-1"]);
        assert_eq!(job.state, ImportState::Done);
        assert_eq!(job.matched, 1);
        assert_eq!(job.unmatched_count(), 2);
        assert_eq!(
            job.unmatched()
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            ["Inconnu 1", "Inconnu 2"]
        );
        assert!(job.error.as_deref().unwrap_or_default().contains("Deezer"));
    }

    /// A job the server forgot is a cause the screen can print, and the import
    /// can be started again — never an endless poll.
    #[tokio::test]
    async fn an_expired_import_says_so() {
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/import/playlist/job-1", 404);
        let cause = core.import_status("job-1").await.expect_err("gone");
        assert!(
            matches!(&cause, CoreError::Invalid { detail } if detail.contains("expiré")),
            "{cause:?}"
        );
    }

    /// A malformed token costs NOTHING; a valid one costs one; a gone link is an
    /// ANSWER, not an error.
    #[tokio::test]
    async fn inspecting_an_invitation_costs_one_request_and_a_bad_shape_none() {
        let (server, _dir, core) = testing::anonymous().await;
        server.forget();
        let malformed = core.inspect_invite("trop-court").await.expect("shape");
        assert!(!malformed.valid);
        assert_eq!(server.count(), 0, "a malformed link asks nothing");

        let info = core.inspect_invite(&testing::token()).await.expect("valid");
        assert!(info.valid);
        assert_eq!(info.label, "Invitation");
        assert_eq!(info.expires.as_deref(), Some("2026-11-21T18:00:00"));
        assert_eq!(server.routes(), vec!["POST /api/invites/inspect"]);

        server.answer("POST", "/api/invites/inspect", 404, r#"{"error":"gone"}"#);
        let gone = core.inspect_invite(&testing::token()).await.expect("gone");
        assert!(!gone.valid, "a 404 is the page's invalid state");
    }

    /// Redeeming costs ONE request, adopts the server's own session (no `/me`),
    /// and lands where the account's onboarding says.
    #[tokio::test]
    async fn redeeming_costs_one_request_and_signs_the_person_in() {
        let (server, _dir, core) = testing::anonymous().await;
        server.forget();
        let outcome = core
            .redeem_invite(&testing::token(), "Élodie", "une phrase de passe simple")
            .await
            .expect("redeemed");
        match outcome {
            RedeemOutcome::Created(redeemed) => {
                assert_eq!(redeemed.me.user.name, "elodie");
                assert_eq!(redeemed.onboarding, Step::Artists);
            }
            other => panic!("expected a created account, got {other:?}"),
        }
        assert_eq!(server.routes(), vec!["POST /api/invites/redeem"]);
        assert_eq!(
            core.me().map(|me| me.user.name),
            Some("elodie".to_owned()),
            "the shell sees the person without a second request"
        );
        let body = &server.hits()[0].body;
        assert!(body.contains("\"username\":\"elodie\""), "{body}");
        assert!(body.contains("\"password_confirm\":\"une phrase"), "{body}");
        assert!(body.contains("\"request_id\":\""), "{body}");
    }

    /// The form's own rules run before the request: a short password or a name
    /// with a space costs ZERO requests.
    #[tokio::test]
    async fn the_forms_rules_run_before_any_request() {
        let (server, _dir, core) = testing::anonymous().await;
        server.forget();
        let short = core
            .redeem_invite(&testing::token(), "elodie", "court")
            .await
            .expect("an answer");
        match short {
            RedeemOutcome::Field { field, message } => {
                assert_eq!(field, InviteField::Password);
                assert!(message.contains("12 caractères"), "{message}");
            }
            other => panic!("expected a field answer, got {other:?}"),
        }
        let spaced = core
            .redeem_invite(&testing::token(), "élo die", "une phrase de passe simple")
            .await
            .expect("an answer");
        match spaced {
            RedeemOutcome::Field { field, message } => {
                assert_eq!(field, InviteField::Username);
                assert!(message.contains("l'espace"), "{message}");
            }
            other => panic!("expected a field answer, got {other:?}"),
        }
        assert_eq!(server.count(), 0);
    }

    /// The server's own refusal names the field, so the sentence goes under it.
    #[tokio::test]
    async fn a_refused_form_names_its_field() {
        let (server, _dir, core) = testing::anonymous().await;
        server.answer(
            "POST",
            "/api/invites/redeem",
            400,
            r#"{"error":"Ce nom d'utilisateur n'est pas disponible.","code":"username_unavailable","field":"username"}"#,
        );
        match core
            .redeem_invite(&testing::token(), "elodie", "une phrase de passe simple")
            .await
            .expect("an answer")
        {
            RedeemOutcome::Field { field, message } => {
                assert_eq!(field, InviteField::Username);
                assert!(message.contains("disponible"), "{message}");
            }
            other => panic!("expected a field answer, got {other:?}"),
        }
        server.answer("POST", "/api/invites/redeem", 404, r#"{"error":"gone"}"#);
        assert_eq!(
            core.redeem_invite(&testing::token(), "elodie", "une phrase de passe simple")
                .await
                .expect("an answer"),
            RedeemOutcome::Invalid
        );
    }

    /// A draft remembers the token, the name and the request id — never the
    /// password — and dies after ten minutes.
    #[tokio::test]
    async fn the_draft_never_holds_the_password_and_expires() {
        let (_server, _dir, core) = testing::anonymous().await;
        let token = testing::token();
        core.remember_invite_draft(&token, "elodie", "cle-1");
        let draft = core.invite_draft(&token).expect("the draft");
        assert_eq!(draft.name, "elodie");
        assert_eq!(draft.request_id, "cle-1");
        assert_eq!(draft.token, token);
        let raw = format!("{draft:?}");
        assert!(!raw.contains("secret"), "a draft never holds a password");

        // Read with the clock handed in: the ten-minute life is an assertion, not
        // a wait.
        let at = super::wire::read_draft_at(&core, &token, draft.at).expect("fresh");
        assert_eq!(at.request_id, "cle-1");
        assert!(
            super::wire::read_draft_at(&core, &token, draft.at + super::rules::DRAFT_TTL_SECS + 1)
                .is_none()
        );
        core.clear_invite_draft(&token);
        assert!(core.invite_draft(&token).is_none());
    }

    /// A `429` on the inspect is the page's "Trop de tentatives", with the
    /// server's wait — not a generic failure.
    #[tokio::test]
    async fn a_rate_limited_inspection_carries_the_wait() {
        let (server, _dir, core) = testing::anonymous().await;
        server.answer(
            "POST",
            "/api/invites/inspect",
            429,
            r#"{"error":"Trop de tentatives. Réessaie dans quelques minutes.","code":"rate_limited"}"#,
        );
        let cause = core
            .inspect_invite(&testing::token())
            .await
            .expect_err("limited");
        assert!(
            matches!(
                cause,
                CoreError::NotReady { .. } | CoreError::Server { status: 429, .. }
            ),
            "{cause:?}"
        );
    }

    /// A stale snapshot is not drawn at all: half an hour later the step it names
    /// may no longer be the server's.
    #[tokio::test]
    async fn a_stale_snapshot_is_not_drawn() {
        let (_server, _dir, core) = testing::configured().await;
        core.onboarding().await.expect("a visit");
        assert!(core.cached_onboarding().is_some());
        let path = {
            let dir = core.config().cache_dir.clone();
            std::fs::read_dir(&dir)
                .expect("the cache dir")
                .flatten()
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().starts_with("onboarding-"))
                        .unwrap_or(false)
                })
                .expect("the snapshot")
        };
        // Rewrite its timestamp a day back: the reader must refuse it.
        let body = std::fs::read_to_string(&path).expect("read");
        let mut value: serde_json::Value = serde_json::from_str(&body).expect("json");
        value["at"] = serde_json::json!(1);
        std::fs::write(&path, serde_json::to_vec(&value).expect("json")).expect("write");
        assert!(core.cached_onboarding().is_none());
    }
}
