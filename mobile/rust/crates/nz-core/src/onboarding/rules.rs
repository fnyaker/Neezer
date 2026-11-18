//! The invitation form's and the onboarding's RULES, ported from the SPA
//! (`webapp/src/lib/invite.js`, `webapp/src/lib/onboarding.js`) so the phone
//! accepts, folds and orders exactly what the page does.
//!
//! Nothing here touches a socket, a file or a clock a test would have to wait
//! on: every function is a pure answer, and the SPA's own test cases
//! (`webapp/test/invite.test.mjs`, `webapp/test/onboarding.test.mjs`) are
//! repeated below one for one — the port is proven against the page, not against
//! a re-reading of it.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

/// `invite.js#MIN_PASSWORD` — the server's own floor (`check_invite_password`).
pub const MIN_PASSWORD: usize = 12;
pub const MAX_PASSWORD: usize = 128;
pub const MIN_USERNAME: usize = 3;
pub const MAX_USERNAME: usize = 32;
/// `invite.js#DRAFT_TTL`: ten minutes. Past it the request id is worthless (the
/// server's replay window is shorter), so the draft is forgotten.
pub const DRAFT_TTL_SECS: u64 = 10 * 60;
/// The server mints `<22 base64url>.<24 base64url>` (`neezer/invites.py`), and
/// the page refuses anything else without a request (`tokenShapeOk`).
pub const TOKEN_LEN: usize = 47;

// -- folding ------------------------------------------------------------------
//
// `foldName` is `normalize("NFKD")` then "drop every combining mark", then
// lowercase. This crate carries no ICU and no `unicode-normalization` (the
// manifest is shared), so the two halves are explicit: the precomposed Latin
// letters of Latin-1 and Latin Extended-A are decomposed by a table, and the
// combining blocks are dropped. A character outside both is kept as it is — and
// the server refuses it, with its own French sentence, which is the honest
// outcome for a name the two sides do not agree on.

/// The combining marks `NFKD` would separate: combining diacriticals, the
/// extended and supplement blocks, Cyrillic's, and the half marks.
fn is_combining(ch: char) -> bool {
    matches!(ch as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

/// The Latin letters a keyboard or a paste plausibly carries, decomposed the way
/// `NFKD` does (a ligature becomes two letters, `ß` becomes `ss`).
fn decompose(ch: char) -> Option<&'static str> {
    let base = match ch {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å'
        | 'Ā' | 'Ă' | 'Ą' => "a",
        'æ' | 'Æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => "c",
        'ď' | 'đ' | 'Ď' | 'Đ' | 'Ð' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ'
        | 'Ė' | 'Ę' | 'Ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' | 'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => "g",
        'ĥ' | 'ħ' | 'Ĥ' | 'Ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī'
        | 'Ĭ' | 'Į' | 'İ' => "i",
        'ĵ' | 'Ĵ' => "j",
        'ķ' | 'Ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' | 'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' | 'Ñ' | 'Ń' | 'Ņ' | 'Ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø'
        | 'Ō' | 'Ŏ' | 'Ő' => "o",
        'œ' | 'Œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' | 'Ŕ' | 'Ŗ' | 'Ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'Ś' | 'Ŝ' | 'Ş' | 'Š' => "s",
        'ß' | 'ẞ' => "ss",
        'ţ' | 'ť' | 'ŧ' | 'Ţ' | 'Ť' | 'Ŧ' => "t",
        'þ' | 'Þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ'
        | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => "u",
        'ŵ' | 'Ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' | 'Ý' | 'Ÿ' | 'Ŷ' => "y",
        'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => "z",
        _ => return None,
    };
    Some(base)
}

/// `foldName`: folded WITHOUT touching the edges, so a field can be folded while
/// somebody is typing a trailing space (`invite.js#foldName`).
pub fn fold_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if is_combining(ch) {
            continue;
        }
        if let Some(base) = decompose(ch) {
            out.push_str(base);
        } else {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        }
    }
    out
}

/// `foldUsername`: the same fold, trimmed.
pub fn fold_username(raw: &str) -> String {
    fold_name(raw.trim()).trim().to_owned()
}

// -- usernames ----------------------------------------------------------------

const ALLOWED: &str = "abcdefghijklmnopqrstuvwxyz0123456789_.-";

fn is_allowed(ch: char) -> bool {
    ch.is_ascii() && ALLOWED.contains(ch)
}

fn is_edge(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

/// Why a username cannot be sent, as the screen's own state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsernameProblem {
    TooShort,
    TooLong,
    Charset,
    Edge,
}

/// `usernameProblem`: 3–32 characters of `a–z 0–9 . - _`, edges alphanumeric.
pub fn username_problem(raw: &str) -> Option<UsernameProblem> {
    let folded = fold_username(raw);
    let length = folded.chars().count();
    if length == 0 || length < MIN_USERNAME {
        return Some(UsernameProblem::TooShort);
    }
    if length > MAX_USERNAME {
        return Some(UsernameProblem::TooLong);
    }
    if !folded.chars().all(is_allowed) {
        return Some(UsernameProblem::Charset);
    }
    let first = folded.chars().next().unwrap_or('-');
    let last = folded.chars().next_back().unwrap_or('-');
    if !is_edge(first) || !is_edge(last) {
        return Some(UsernameProblem::Edge);
    }
    None
}

/// `refusedCharacters`: what survives the fold and is still not allowed, in the
/// order typed and without repeats — the screen names them.
pub fn refused_characters(raw: &str) -> Vec<char> {
    let mut seen: Vec<char> = Vec::new();
    for ch in fold_username(raw).chars() {
        if is_allowed(ch) || seen.contains(&ch) {
            continue;
        }
        seen.push(ch);
    }
    seen
}

/// `charLabel`: how a refused character is named, without printing a control
/// character raw.
fn char_label(ch: char) -> String {
    match ch {
        ' ' => "l'espace".to_owned(),
        '\t' => "la tabulation".to_owned(),
        '\n' => "le retour à la ligne".to_owned(),
        other => format!("« {other} »"),
    }
}

/// `usernameMessage`: the SPA's own sentence for each refusal, which is the one
/// the server repeats — the form says it before a request.
pub fn username_message(raw: &str) -> String {
    match username_problem(raw) {
        None => String::new(),
        Some(UsernameProblem::TooShort) => format!("Il faut au moins {MIN_USERNAME} caractères."),
        Some(UsernameProblem::TooLong) => format!("{MAX_USERNAME} caractères au plus."),
        Some(UsernameProblem::Edge) => {
            "Le nom doit commencer et finir par une lettre ou un chiffre.".to_owned()
        }
        Some(UsernameProblem::Charset) => {
            let bad = refused_characters(raw);
            let list: Vec<String> = bad.iter().take(3).copied().map(char_label).collect();
            let head = if bad.len() > 1 {
                "Ces caractères ne sont pas autorisés"
            } else {
                "Ce caractère n'est pas autorisé"
            };
            format!(
                "{head} : {}. Utilise a–z, 0–9, le point, le tiret ou le tiret bas.",
                list.join(", ")
            )
        }
    }
}

/// Why a password cannot be sent (`passwordProblem`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordProblem {
    Short,
    Long,
    SameAsUsername,
}

fn contains_name(password: &str, username: &str) -> bool {
    let name = fold_username(username);
    if name.is_empty() {
        return false;
    }
    let folded = fold_username(password);
    let reversed: String = name.chars().rev().collect();
    folded.contains(&name) || folded.contains(&reversed)
}

/// `passwordProblem`: 12–128 characters, no composition rule, and never the
/// username (forwards or backwards) — the server's own rule.
pub fn password_problem(password: &str, username: &str) -> Option<PasswordProblem> {
    let length = password.chars().count();
    if length < MIN_PASSWORD {
        return Some(PasswordProblem::Short);
    }
    if length > MAX_PASSWORD {
        return Some(PasswordProblem::Long);
    }
    if contains_name(password, username) {
        return Some(PasswordProblem::SameAsUsername);
    }
    None
}

/// `passwordScore`: a length ladder, capped when the name is inside or the whole
/// password is one repeated character. Never a composition rule — the SPA's own
/// choice, so a passphrase scores well.
pub fn password_score(password: &str, username: &str) -> u8 {
    let length = password.chars().count();
    if length == 0 {
        return 0;
    }
    if length < MIN_PASSWORD {
        return 1;
    }
    let mut score = if length >= 24 {
        4
    } else if length >= 16 {
        3
    } else {
        2
    };
    if contains_name(password, username) {
        score = score.min(2);
    }
    let repeated = length >= 8 && {
        let mut chars = password.chars();
        let first = chars.next();
        chars.all(|ch| Some(ch) == first)
    };
    if repeated {
        score = score.min(2);
    }
    score
}

/// The word under the meter (`passwordLabel`). The SCREEN writes it from this
/// enum; the core never carries UI copy for a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strength {
    TooShort,
    Weak,
    Fair,
    Good,
    VeryGood,
}

pub fn password_label(password: &str, username: &str) -> Strength {
    if password.chars().count() < MIN_PASSWORD {
        return Strength::TooShort;
    }
    match password_score(password, username) {
        0 | 1 => Strength::Weak,
        2 => Strength::Fair,
        3 => Strength::Good,
        _ => Strength::VeryGood,
    }
}

/// `passwordMessage`: the SPA's own sentence for each refusal.
pub fn password_message(password: &str, username: &str) -> String {
    match password_problem(password, username) {
        None => String::new(),
        Some(PasswordProblem::Short) => format!("{MIN_PASSWORD} caractères minimum."),
        Some(PasswordProblem::Long) => format!("{MAX_PASSWORD} caractères au plus."),
        Some(PasswordProblem::SameAsUsername) => {
            "Le mot de passe ne peut pas contenir ton nom d'utilisateur.".to_owned()
        }
    }
}

// -- the token and where a redeemed account lands ------------------------------

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

/// `tokenShapeOk`: exactly 47 characters, `22.token`, every one base64url. The
/// page refuses a malformed link BEFORE asking, so a pasted half-link costs no
/// request.
pub fn token_shape_ok(token: &str) -> bool {
    let chars: Vec<char> = token.chars().collect();
    chars.len() == TOKEN_LEN
        && chars.get(22) == Some(&'.')
        && chars
            .iter()
            .enumerate()
            .all(|(index, ch)| index == 22 || is_token_char(*ch))
}

/// `landingFor`: a new account whose onboarding is not finished opens the
/// onboarding, and everybody else the home.
pub fn needs_onboarding(onboarding: Option<&str>) -> bool {
    matches!(onboarding, Some(step) if !step.is_empty() && step != "done")
}

/// `newRequestId`: unique across two attempts, bounded, and never a secret. The
/// server hashes it into its replay table, so any stable unique string works;
/// time plus a counter keeps it reproducible-looking in a log without a random
/// source the core does not carry.
pub fn new_request_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}-{:x}", nanos, count, std::process::id())
}

// -- the artists feed ---------------------------------------------------------

/// One artist tile, as the feed and the search answer it. The fields the ORDER
/// reads (`via`, `parent`, `liked`, the genres) and the ones the screen draws are
/// both here: this is also what the device keeps between two visits.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ArtistCard {
    /// The reference the like button sends (`deezer_id` for a catalogue row).
    pub id: String,
    pub deezer_id: String,
    pub name: String,
    pub picture: Option<String>,
    pub picture_small: Option<String>,
    pub genres: Vec<String>,
    /// `"deezer"`, `"youtube"` or `"local"`.
    pub provider: String,
    /// `"library"`, `"related"`, `"search"`, … — where the tile came from.
    pub via: Option<String>,
    /// The tile this one is a neighbour of (`insertSimilar`).
    pub parent: Option<String>,
    pub liked: bool,
    /// The artist has a page the library can open.
    pub openable: bool,
    /// Why this tile is here, in the server's words.
    pub hint: Option<String>,
}

impl ArtistCard {
    /// The identity the list keys on (`idOf`).
    pub fn key(&self) -> String {
        if !self.deezer_id.is_empty() {
            self.deezer_id.clone()
        } else {
            self.id.clone()
        }
    }

    /// `genreOf`: the first non-empty genre, trimmed and lowercased.
    pub fn genre(&self) -> Option<String> {
        self.genres.iter().find_map(|value| {
            let genre = value.trim().to_lowercase();
            (!genre.is_empty()).then_some(genre)
        })
    }

    fn is_inserted(&self) -> bool {
        self.via.as_deref() == Some("related")
    }
}

/// `countInserted`.
pub fn count_inserted(items: &[ArtistCard]) -> usize {
    items.iter().filter(|item| item.is_inserted()).count()
}

/// `budgetLeft`: how many neighbour tiles are still allowed in the list.
pub fn budget_left(items: &[ArtistCard], budget: usize) -> usize {
    budget.saturating_sub(count_inserted(items))
}

/// The diversity bounds the SERVER sent for this person (`/onboarding`'s
/// `diversity`), so the phone inserts exactly as many of one family as it would.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Diversity {
    pub window: usize,
    pub max_same_genre: usize,
    pub max_streak: usize,
}

impl Default for Diversity {
    fn default() -> Self {
        Diversity {
            window: 12,
            max_same_genre: 4,
            max_streak: 2,
        }
    }
}

/// What one insertion did: the new list and how many tiles it added.
#[derive(Debug, Clone, PartialEq)]
pub struct InsertResult {
    pub items: Vec<ArtistCard>,
    pub inserted: usize,
}

/// `insertSimilar`: at most [`SIMILAR_PER_LIKE`](super::SIMILAR_PER_LIKE)
/// neighbours right after the liked tile, never a duplicate, never one already
/// liked, never past the list's budget, and alternating families when it can.
pub fn insert_similar(
    items: &[ArtistCard],
    liked_index: isize,
    similar: &[ArtistCard],
    liked: &HashSet<String>,
    per_like: usize,
    budget: usize,
    diversity: Diversity,
) -> InsertResult {
    let index = liked_index;
    if index < 0 || index as usize >= items.len() {
        return InsertResult {
            items: items.to_vec(),
            inserted: 0,
        };
    }
    let index = index as usize;
    let room = per_like.min(budget_left(items, budget));
    if room == 0 {
        return InsertResult {
            items: items.to_vec(),
            inserted: 0,
        };
    }
    let parent = items[index].key();
    if parent.is_empty() {
        return InsertResult {
            items: items.to_vec(),
            inserted: 0,
        };
    }
    let mut taken: HashSet<String> = items
        .iter()
        .map(ArtistCard::key)
        .filter(|k| !k.is_empty())
        .collect();
    let mut candidates: Vec<ArtistCard> = Vec::new();
    for artist in similar {
        let key = artist.key();
        if key.is_empty()
            || key == parent
            || taken.contains(&key)
            || artist.liked
            || liked.contains(&key)
        {
            continue;
        }
        taken.insert(key);
        candidates.push(artist.clone());
    }
    if candidates.is_empty() {
        return InsertResult {
            items: items.to_vec(),
            inserted: 0,
        };
    }

    let window = diversity.window.max(1);
    let cap = diversity.max_same_genre.max(1);
    let from = index.saturating_sub(window - 1);
    let to = (index + window).min(items.len());
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for artist in &items[from..to] {
        if let Some(genre) = artist.genre() {
            *counts.entry(genre).or_default() += 1;
        }
    }
    let fits = |artist: &ArtistCard, counts: &std::collections::HashMap<String, usize>| match artist
        .genre()
    {
        Some(genre) => counts.get(&genre).copied().unwrap_or(0) < cap,
        None => true,
    };

    let mut picked: Vec<ArtistCard> = Vec::new();
    let mut deferred: Vec<ArtistCard> = Vec::new();
    for artist in candidates {
        if picked.len() >= room {
            break;
        }
        if !fits(&artist, &counts) {
            continue;
        }
        let previous = picked.last().and_then(ArtistCard::genre);
        if previous.is_some() && previous == artist.genre() {
            deferred.push(artist);
            continue;
        }
        if let Some(genre) = artist.genre() {
            *counts.entry(genre).or_default() += 1;
        }
        picked.push(artist);
    }
    for artist in deferred {
        if picked.len() >= room {
            break;
        }
        if !fits(&artist, &counts) {
            continue;
        }
        if let Some(genre) = artist.genre() {
            *counts.entry(genre).or_default() += 1;
        }
        picked.push(artist);
    }
    if picked.is_empty() {
        return InsertResult {
            items: items.to_vec(),
            inserted: 0,
        };
    }
    let block: Vec<ArtistCard> = picked
        .into_iter()
        .map(|mut artist| {
            artist.via = Some("related".to_owned());
            artist.parent = Some(parent.clone());
            artist
        })
        .collect();
    let mut out = Vec::with_capacity(items.len() + block.len());
    out.extend_from_slice(&items[..=index]);
    out.extend(block.iter().cloned());
    out.extend_from_slice(&items[index + 1..]);
    InsertResult {
        items: out,
        inserted: block.len(),
    }
}

/// `removeSimilar`: takes the neighbour block of one parent out, keeping a tile
/// that became liked as an ordinary grid tile.
pub fn remove_similar(
    items: &[ArtistCard],
    parent: &str,
    liked: &HashSet<String>,
) -> Vec<ArtistCard> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let of_this_parent =
            item.is_inserted() && item.parent.as_deref().unwrap_or_default() == parent;
        if !of_this_parent {
            out.push(item.clone());
            continue;
        }
        if liked.contains(&item.key()) || item.liked {
            let mut kept = item.clone();
            kept.via = Some("grid".to_owned());
            kept.parent = None;
            out.push(kept);
        }
    }
    out
}

/// `reconcilePage`: a fresh page updates rows in place (keeping where a tile
/// sits and why it is there), appends what is new, and drops a duplicate.
pub fn reconcile_page(items: &[ArtistCard], page: &[ArtistCard]) -> Vec<ArtistCard> {
    let mut list = items.to_vec();
    let mut places: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, item) in list.iter().enumerate() {
        places.insert(item.key(), index);
    }
    let mut seen: HashSet<String> = HashSet::new();
    for fresh in page {
        let key = fresh.key();
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        match places.get(&key).copied() {
            Some(index) => {
                let current = &list[index];
                let mut merged = fresh.clone();
                merged.via = current.via.clone();
                merged.parent = current.parent.clone();
                list[index] = merged;
            }
            None => {
                places.insert(key, list.len());
                list.push(fresh.clone());
            }
        }
    }
    list
}

/// `cacheList`: what the device keeps between two visits, bounded, and never a
/// placeholder.
pub fn cache_list(items: &[ArtistCard], max: usize) -> Vec<ArtistCard> {
    let list: Vec<ArtistCard> = items
        .iter()
        .filter(|item| !item.name.is_empty())
        .cloned()
        .collect();
    if max > 0 && list.len() > max {
        list[..max].to_vec()
    } else {
        list
    }
}

/// `normalizeQuery`: one line, edges trimmed, 80 characters at most.
pub fn normalize_query(raw: &str) -> String {
    let joined = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(80).collect()
}

/// Why the finish button is refused (`finishBlocked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishBlock {
    /// Fewer than `min` artists kept.
    Min,
    /// The link is down: a like made offline waits for it.
    Offline,
    /// A like is still in flight (or queued).
    Pending,
}

/// `finishBlocked`: the artist step finishes only with one kept artist, a link,
/// and nothing still being written.
pub fn finish_blocked(
    liked: usize,
    pending: usize,
    queued: usize,
    online: bool,
    min: usize,
) -> Option<FinishBlock> {
    if liked < min {
        return Some(FinishBlock::Min);
    }
    if !online {
        return Some(FinishBlock::Offline);
    }
    if pending > 0 || queued > 0 {
        return Some(FinishBlock::Pending);
    }
    None
}

/// `canPassArtists`: "Passer pour l'instant" appears only after two attempts at a
/// feed the server says is UNAVAILABLE — never while it is still building.
pub fn can_pass_artists(
    attempts: u32,
    feed_state: &str,
    building: bool,
    count: usize,
    min: usize,
) -> bool {
    if count >= min {
        return false;
    }
    if attempts < 2 {
        return false;
    }
    feed_state == "unavailable" && !building
}

/// `isImportRunning`: the three phases that still have work to show.
pub fn import_running(state: &str) -> bool {
    matches!(state, "reading" | "matching" | "saving")
}

/// `feedCause`'s codes, as a closed set. The SCREEN writes the French sentence
/// from it (the core never carries UI copy) — and an unknown code is the last
/// variant, never a raw string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedCause {
    DeezerUnreachable,
    Disabled,
    Held,
    Absent,
    Busy,
    Error,
    Unknown,
}

pub fn feed_cause(code: Option<&str>) -> FeedCause {
    match code.unwrap_or_default() {
        "deezer-unreachable" => FeedCause::DeezerUnreachable,
        "disabled" => FeedCause::Disabled,
        "held" => FeedCause::Held,
        "absent" => FeedCause::Absent,
        "busy" => FeedCause::Busy,
        "error" => FeedCause::Error,
        _ => FeedCause::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artist(id: &str, name: &str, genre: Option<&str>) -> ArtistCard {
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

    fn forest() -> Vec<ArtistCard> {
        (0..20)
            .map(|index| {
                let genres = ["Pop", "Rap", "Rock"];
                let genre = genres[index % 3];
                artist(
                    &format!("{}", 100 + index),
                    &format!("Artiste {index}"),
                    Some(genre),
                )
            })
            .collect()
    }

    // -- invite.test.mjs, one for one ------------------------------------

    #[test]
    fn fold_name_folds_without_touching_the_edges_while_typing() {
        assert_eq!(fold_name(" Élodie "), " elodie ");
        assert_eq!(fold_name("ÉLODIE"), "elodie");
    }

    #[test]
    fn fold_username_folds_case_accents_and_edges() {
        assert_eq!(fold_username("Élodie"), "elodie");
        assert_eq!(fold_username("ALICE"), "alice");
        assert_eq!(fold_username("Jean-Luc"), "jean-luc");
        assert_eq!(fold_username("  Zoé  "), "zoe");
        assert_eq!(fold_username("e\u{0301}"), "e");
        assert_eq!(fold_username(""), "");
    }

    #[test]
    fn username_problem_accepts_three_to_thirty_two_and_refuses_the_rest() {
        assert_eq!(username_problem("elodie"), None);
        assert_eq!(username_problem("Élodie"), None);
        assert_eq!(username_problem("a.b-c_d"), None);
        assert_eq!(username_problem("abc"), None);
        assert_eq!(username_problem(&"a".repeat(32)), None);
        assert_eq!(username_problem(""), Some(UsernameProblem::TooShort));
        assert_eq!(username_problem("ab"), Some(UsernameProblem::TooShort));
        assert_eq!(
            username_problem(&"a".repeat(33)),
            Some(UsernameProblem::TooLong)
        );
        assert_eq!(username_problem("elodie!"), Some(UsernameProblem::Charset));
        assert_eq!(
            username_problem("elodie smith"),
            Some(UsernameProblem::Charset)
        );
        assert_eq!(username_problem("@elodie"), Some(UsernameProblem::Charset));
        assert_eq!(username_problem("-elodie"), Some(UsernameProblem::Edge));
        assert_eq!(username_problem("elodie."), Some(UsernameProblem::Edge));
        assert_eq!(username_problem("_elodie"), Some(UsernameProblem::Edge));
    }

    #[test]
    fn refused_characters_name_only_what_survives_the_fold() {
        assert_eq!(refused_characters("Élodie"), Vec::<char>::new());
        assert_eq!(refused_characters("élodie @"), vec![' ', '@']);
        assert_eq!(refused_characters("a!a?"), vec!['!', '?']);
    }

    #[test]
    fn password_problem_applies_twelve_to_a_hundred_and_twenty_eight() {
        assert_eq!(password_problem("court", ""), Some(PasswordProblem::Short));
        assert_eq!(
            password_problem(&"a".repeat(11), ""),
            Some(PasswordProblem::Short)
        );
        assert_eq!(password_problem(&"a".repeat(12), ""), None);
        assert_eq!(password_problem("une phrase de passe simple", ""), None);
        assert_eq!(
            password_problem(&"a".repeat(129), ""),
            Some(PasswordProblem::Long)
        );
        assert_eq!(
            password_problem("elodieelodie", "elodie"),
            Some(PasswordProblem::SameAsUsername)
        );
        assert_eq!(
            password_problem("xxeidoleeidole", "elodie"),
            Some(PasswordProblem::SameAsUsername)
        );
        assert_eq!(
            password_problem("ÉLODIEELODIE", "elodie"),
            Some(PasswordProblem::SameAsUsername)
        );
    }

    #[test]
    fn password_score_rises_with_length_and_never_notes_composition() {
        assert_eq!(password_score("", ""), 0);
        assert_eq!(password_score("court", ""), 1);
        let base = "abcdefghijklmnopqrstuvwxyz0123456789";
        let mut previous = 0;
        for length in 0..=36 {
            let score = password_score(&base[..length], "");
            assert!(score >= previous, "score {score} < {previous} at {length}");
            previous = score;
        }
        assert_eq!(password_score("abcdefghijkl", ""), 2);
        assert_eq!(password_score("abcdefghijklmnop", ""), 3);
        assert_eq!(password_score("abcdefghijklmnopqrstuvwx", ""), 4);
        assert!(password_score("une phrase de passe tres simple", "") >= 3);
        assert_eq!(password_score("aaaaaaaaaaaaaaaaaaaa", ""), 2);
        assert_eq!(password_score("elodieelodieelodie", "elodie"), 2);
    }

    #[test]
    fn password_label_words_the_meter() {
        assert_eq!(password_label("", ""), Strength::TooShort);
        assert_eq!(password_label("court", ""), Strength::TooShort);
        assert_eq!(password_label("abcdefghijkl", ""), Strength::Fair);
        assert_eq!(password_label("abcdefghijklmnop", ""), Strength::Good);
        assert_eq!(
            password_label("abcdefghijklmnopqrstuvwx", ""),
            Strength::VeryGood
        );
    }

    #[test]
    fn the_forms_sentences_are_the_pages() {
        assert_eq!(username_message("elodie"), "");
        assert_eq!(username_message("ab"), "Il faut au moins 3 caractères.");
        assert_eq!(username_message(&"a".repeat(33)), "32 caractères au plus.");
        assert_eq!(
            username_message("-elodie"),
            "Le nom doit commencer et finir par une lettre ou un chiffre."
        );
        assert_eq!(
            username_message("élodie @"),
            "Ces caractères ne sont pas autorisés : l'espace, « @ ». Utilise a–z, 0–9, le point, le tiret ou le tiret bas."
        );
        assert_eq!(
            username_message("elodie@"),
            "Ce caractère n'est pas autorisé : « @ ». Utilise a–z, 0–9, le point, le tiret ou le tiret bas."
        );
        assert_eq!(password_message("court", ""), "12 caractères minimum.");
        assert_eq!(
            password_message(&"a".repeat(129), ""),
            "128 caractères au plus."
        );
        assert_eq!(
            password_message("elodieelodie", "elodie"),
            "Le mot de passe ne peut pas contenir ton nom d'utilisateur."
        );
        assert_eq!(password_message("une phrase de passe simple", ""), "");
    }

    #[test]
    fn token_shape_accepts_only_the_servers_id_mac_form() {
        let token = format!("{}.{}", "a".repeat(22), "b".repeat(24));
        assert!(token_shape_ok(&token));
        assert!(!token_shape_ok(&"a".repeat(22)));
        assert!(!token_shape_ok(&"a".repeat(32)));
        assert!(!token_shape_ok(&format!(
            "{}.{}",
            "a".repeat(21),
            "b".repeat(24)
        )));
        assert!(!token_shape_ok(&format!(
            "{}.{}",
            "a".repeat(22),
            "b".repeat(23)
        )));
        assert!(!token_shape_ok(&format!("{token}\n")));
        assert!(!token_shape_ok(""));
    }

    #[test]
    fn landing_opens_the_onboarding_for_a_new_account() {
        assert!(needs_onboarding(Some("artists")));
        assert!(needs_onboarding(Some("account")));
        assert!(!needs_onboarding(Some("done")));
        assert!(!needs_onboarding(None));
        assert!(!needs_onboarding(Some("")));
    }

    #[test]
    fn request_ids_are_unique_bounded_and_plain() {
        let mut seen = HashSet::new();
        for _ in 0..50 {
            let id = new_request_id();
            assert!(!id.is_empty() && id.len() <= 128);
            assert!(id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-'));
            assert!(seen.insert(id));
        }
    }

    // -- onboarding.test.mjs, one for one --------------------------------

    #[test]
    fn insert_similar_places_at_most_two_neighbours_after_the_liked_tile() {
        let list = forest()[..6].to_vec();
        let similar = vec![
            artist("900", "Proche A", Some("Pop")),
            artist("901", "Proche B", Some("Rap")),
        ];
        let out = insert_similar(
            &list,
            2,
            &similar,
            &HashSet::new(),
            2,
            12,
            Diversity::default(),
        );
        assert_eq!(out.inserted, 2);
        let ids: Vec<&str> = out
            .items
            .iter()
            .map(|item| item.deezer_id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["100", "101", "102", "900", "901", "103", "104", "105"]
        );
        assert_eq!(out.items[3].via.as_deref(), Some("related"));
        assert_eq!(out.items[3].parent.as_deref(), Some("102"));
        assert_eq!(list.len(), 6, "the input is not modified");
    }

    #[test]
    fn insert_similar_never_exceeds_two_neighbours_per_like() {
        let list = forest()[..4].to_vec();
        let similar = vec![
            artist("900", "A", Some("Pop")),
            artist("901", "B", Some("Rap")),
            artist("902", "C", Some("Rock")),
        ];
        let out = insert_similar(
            &list,
            1,
            &similar,
            &HashSet::new(),
            2,
            12,
            Diversity::default(),
        );
        assert_eq!(out.inserted, 2);
        let ids: Vec<&str> = out
            .items
            .iter()
            .map(|item| item.deezer_id.as_str())
            .collect();
        assert_eq!(ids, ["100", "101", "900", "901", "102", "103"]);
    }

    #[test]
    fn insert_similar_refuses_the_artist_itself_duplicates_and_liked_ones() {
        let mut list = forest()[..4].to_vec();
        let similar = vec![
            artist("101", "Lui-même", None),
            artist("102", "Déjà dans la grille", None),
            artist("900", "Déjà aimé", None),
            artist("901", "Bon", None),
        ];
        let mut liked = HashSet::new();
        liked.insert("900".to_owned());
        for item in list.iter_mut() {
            if liked.contains(&item.key()) {
                item.liked = true;
            }
        }
        let out = insert_similar(&list, 0, &similar, &liked, 2, 12, Diversity::default());
        assert_eq!(out.inserted, 1);
        let ids: Vec<&str> = out
            .items
            .iter()
            .map(|item| item.deezer_id.as_str())
            .collect();
        assert_eq!(ids, ["100", "901", "101", "102", "103"]);
    }

    #[test]
    fn insert_similar_respects_the_twelve_inserted_tiles_ceiling() {
        let list = forest()[..4].to_vec();
        let twelve: Vec<ArtistCard> = (0..12)
            .map(|index| ArtistCard {
                via: Some("related".to_owned()),
                parent: Some("100".to_owned()),
                ..artist(
                    &format!("{}", 800 + index),
                    &format!("Inséré {index}"),
                    Some(if index % 2 == 0 { "Rap" } else { "Pop" }),
                )
            })
            .collect();
        let full: Vec<ArtistCard> = list[..2]
            .iter()
            .cloned()
            .chain(twelve.iter().cloned())
            .chain(list[2..].iter().cloned())
            .collect();
        assert_eq!(budget_left(&full, 12), 0);
        let blocked = insert_similar(
            &full,
            0,
            &[artist("950", "Nouveau", Some("Rock"))],
            &HashSet::new(),
            2,
            12,
            Diversity::default(),
        );
        assert_eq!(blocked.inserted, 0);
        assert_eq!(blocked.items.len(), full.len());

        let eleven: Vec<ArtistCard> = full
            .iter()
            .filter(|item| item.via.as_deref() == Some("related"))
            .take(11)
            .cloned()
            .collect();
        let nearly: Vec<ArtistCard> = list[..2]
            .iter()
            .cloned()
            .chain(eleven)
            .chain(list[2..].iter().cloned())
            .collect();
        let one = insert_similar(
            &nearly,
            0,
            &[
                artist("951", "Un", Some("Rock")),
                artist("952", "Deux", Some("Pop")),
            ],
            &HashSet::new(),
            2,
            12,
            Diversity::default(),
        );
        assert_eq!(one.inserted, 1);
    }

    #[test]
    fn insert_similar_keeps_the_servers_diversity_window() {
        let list: Vec<ArtistCard> = (0..8)
            .map(|index| {
                artist(
                    &format!("{}", 200 + index),
                    &format!("P{index}"),
                    Some("Pop"),
                )
            })
            .collect();
        let out = insert_similar(
            &list,
            4,
            &[artist("900", "Pop encore", Some("Pop"))],
            &HashSet::new(),
            2,
            12,
            Diversity {
                window: 6,
                max_same_genre: 4,
                max_streak: 2,
            },
        );
        assert_eq!(out.inserted, 0, "the window is already at the Pop cap");

        let mixed = vec![
            artist("300", "A", Some("Pop")),
            artist("301", "B", Some("Rap")),
            artist("302", "C", Some("Rock")),
        ];
        let ok = insert_similar(
            &mixed,
            0,
            &[artist("901", "Pop 2", Some("Pop"))],
            &HashSet::new(),
            2,
            12,
            Diversity {
                window: 6,
                max_same_genre: 3,
                max_streak: 2,
            },
        );
        assert_eq!(ok.inserted, 1);
    }

    #[test]
    fn insert_similar_alternates_two_families_when_it_can() {
        let list = vec![
            artist("100", "A", Some("Pop")),
            artist("101", "B", Some("Rock")),
        ];
        let similar = vec![
            artist("900", "Pop 1", Some("Pop")),
            artist("901", "Pop 2", Some("Pop")),
            artist("902", "Rap", Some("Rap")),
        ];
        let out = insert_similar(
            &list,
            0,
            &similar,
            &HashSet::new(),
            2,
            12,
            Diversity::default(),
        );
        let ids: Vec<&str> = out
            .items
            .iter()
            .map(|item| item.deezer_id.as_str())
            .collect();
        assert_eq!(ids, ["100", "900", "902", "101"]);
    }

    #[test]
    fn insert_similar_leaves_an_empty_list_or_a_wrong_index_alone() {
        let empty: Vec<ArtistCard> = Vec::new();
        let out = insert_similar(
            &empty,
            0,
            &[artist("1", "A", None)],
            &HashSet::new(),
            2,
            12,
            Diversity::default(),
        );
        assert_eq!(out.inserted, 0);
        assert!(out.items.is_empty());
        let list = forest()[..2].to_vec();
        assert_eq!(
            insert_similar(
                &list,
                9,
                &[artist("900", "A", None)],
                &HashSet::new(),
                2,
                12,
                Diversity::default()
            )
            .inserted,
            0
        );
        assert_eq!(
            insert_similar(
                &list,
                -1,
                &[artist("900", "A", None)],
                &HashSet::new(),
                2,
                12,
                Diversity::default()
            )
            .inserted,
            0
        );
        assert_eq!(
            insert_similar(&list, 0, &[], &HashSet::new(), 2, 12, Diversity::default()).inserted,
            0
        );
    }

    #[test]
    fn remove_similar_takes_the_parents_block_and_keeps_a_liked_tile() {
        let first_three = forest()[..3].to_vec();
        let base = insert_similar(
            &first_three,
            0,
            &[
                artist("900", "A", Some("Pop")),
                artist("901", "B", Some("Rap")),
            ],
            &HashSet::new(),
            2,
            12,
            Diversity::default(),
        )
        .items;
        let out = remove_similar(&base, "100", &HashSet::new());
        let ids: Vec<&str> = out.iter().map(|item| item.deezer_id.as_str()).collect();
        assert_eq!(ids, ["100", "101", "102"]);

        let mut liked = HashSet::new();
        liked.insert("901".to_owned());
        let kept = remove_similar(&base, "100", &liked);
        let ids: Vec<&str> = kept.iter().map(|item| item.deezer_id.as_str()).collect();
        assert_eq!(ids, ["100", "901", "101", "102"]);
        assert_eq!(kept[1].via.as_deref(), Some("grid"));
        assert_eq!(kept[1].parent, None);

        let other = remove_similar(&base, "999", &HashSet::new());
        assert_eq!(other.len(), base.len());
    }

    #[test]
    fn reconcile_page_updates_in_place_adds_new_and_deduplicates() {
        let mut inserted = artist("9", "Inséré", None);
        inserted.via = Some("related".to_owned());
        inserted.parent = Some("1".to_owned());
        let current = vec![
            artist("1", "Ancien nom", None),
            inserted,
            artist("2", "Deux", None),
        ];
        let page = vec![
            ArtistCard {
                picture: Some("p".to_owned()),
                ..artist("1", "Nouveau nom", None)
            },
            artist("3", "Trois", None),
            artist("1", "Doublon", None),
        ];
        let out = reconcile_page(&current, &page);
        let pairs: Vec<(&str, &str)> = out
            .iter()
            .map(|item| (item.deezer_id.as_str(), item.name.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("1", "Nouveau nom"),
                ("9", "Inséré"),
                ("2", "Deux"),
                ("3", "Trois")
            ]
        );
        assert_eq!(out[1].via.as_deref(), Some("related"));
        assert_eq!(out[0].picture.as_deref(), Some("p"));
    }

    #[test]
    fn cache_list_bounds_what_is_kept() {
        let items: Vec<ArtistCard> = (0..200)
            .map(|index| artist(&index.to_string(), &format!("A{index}"), None))
            .collect();
        assert_eq!(cache_list(&items, 120).len(), 120);
        assert_eq!(cache_list(&items, 0).len(), 200);
        assert_eq!(cache_list(&[], 120).len(), 0);
    }

    #[test]
    fn normalize_query_applies_the_two_character_minimum() {
        assert_eq!(normalize_query("  Aya   Nakamura  "), "Aya Nakamura");
        assert_eq!(normalize_query(""), "");
        assert_eq!(normalize_query(&"x".repeat(100)).chars().count(), 80);
    }

    #[test]
    fn finish_blocked_and_its_messages_are_the_plans() {
        assert_eq!(finish_blocked(0, 0, 0, true, 1), Some(FinishBlock::Min));
        assert_eq!(
            finish_blocked(1, 0, 0, false, 1),
            Some(FinishBlock::Offline)
        );
        assert_eq!(finish_blocked(1, 1, 0, true, 1), Some(FinishBlock::Pending));
        assert_eq!(finish_blocked(1, 0, 1, true, 1), Some(FinishBlock::Pending));
        assert_eq!(finish_blocked(1, 0, 0, true, 1), None);
    }

    #[test]
    fn feed_cause_names_every_code_the_server_sends() {
        assert_eq!(
            feed_cause(Some("deezer-unreachable")),
            FeedCause::DeezerUnreachable
        );
        assert_eq!(feed_cause(Some("disabled")), FeedCause::Disabled);
        assert_eq!(feed_cause(Some("held")), FeedCause::Held);
        assert_eq!(feed_cause(Some("absent")), FeedCause::Absent);
        assert_eq!(feed_cause(Some("error")), FeedCause::Error);
        assert_eq!(feed_cause(None), FeedCause::Unknown);
    }

    #[test]
    fn pass_waits_for_two_attempts_and_an_empty_server() {
        assert!(!can_pass_artists(0, "unavailable", false, 0, 1));
        assert!(!can_pass_artists(1, "unavailable", false, 0, 1));
        assert!(can_pass_artists(2, "unavailable", false, 0, 1));
        assert!(!can_pass_artists(2, "unavailable", false, 1, 1));
        assert!(!can_pass_artists(2, "unavailable", true, 0, 1));
        assert!(!can_pass_artists(2, "ready", false, 0, 1));
    }

    #[test]
    fn an_import_is_running_only_during_its_three_work_phases() {
        assert!(import_running("reading"));
        assert!(import_running("matching"));
        assert!(import_running("saving"));
        assert!(!import_running("done"));
        assert!(!import_running("error"));
        assert!(!import_running(""));
    }
}
