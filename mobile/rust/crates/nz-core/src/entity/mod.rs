//! Artist, album, discography and credits — `routes/Artist.svelte`,
//! `routes/Album.svelte`, `routes/Credits.svelte` and the three credit
//! components (`TrackCredits`, `CreditsSheet`, `PersonCredits`), PORTED.
//!
//! Owned by **MB7** (docs/agents/mobile.md, "Module and file ownership"): the lot
//! fills this directory and edits no other file of the crate.
//!
//! # What a screen asks, and what it costs
//!
//! Each page has the pair the screen standard names: a `cached_x()` that is
//! disk-only (zero requests, what a warm open paints in its first frame) and an
//! `x()` that goes to the network on the `User` lane. The counts are the SPA's
//! own, because the routes are:
//!
//! | Page | Cold | Warm (to paint) | The SPA's files |
//! |---|---|---|---|
//! | Artiste (Aperçu) | 3 | 0 | `Artist.svelte#load` (`/artist`, `/discography`, `/on-deezer`) |
//! | Artiste (Titres) | 1 | 0 | `#loadTracks` (`/artist/<id>/tracks`) |
//! | Artiste (Crédits) | 1 | 0 | `PersonCredits.svelte#load` (`/credits/person`) |
//! | Album | 2 | 0 | `Album.svelte#load` (`/album`, `/on-deezer`) |
//! | Crédits d'un titre | 1 | 0 | `CreditsSheet.svelte#load` (`/track/<id>/credits`) |
//!
//! Every request is one route, so a page never fans out; `/artist/<id>/on-deezer`
//! is asked only for a LIBRARY artist, and its failure is a sentence on the page,
//! never a failed page (the provider is optional, CLAUDE.md).
//!
//! # What is kept, and where
//!
//! The server's own body, per account, in the CACHE directory ([`store`]): the
//! SPA's `lib/apicache.js` caches exactly these paths, and a revisit paints from
//! it before revalidating. A name that would choose a path (a person's name) is
//! hashed, never used literally.
//!
//! # The links a page draws
//!
//! `lib/entity.js#entityRef`: an entity is named by its row's UUID when it has
//! one, else by the provider's id — both are what the server's routes accept, so
//! a tap and a deep link land on the same page. A credit's name opens its artist
//! page when the library holds one, else its own credits page once it is credited
//! on another title too (`TrackCredits.svelte#href`); with neither it is plain
//! text, because a link must lead somewhere.

mod store;
mod wire;

#[cfg(test)]
mod testing;

use std::collections::BTreeMap;

use crate::core::{Core, CoreError};
use crate::lists::Row;

/// The tag every request of this area is counted under (`NzCore::stats`).
pub const TAG_ENTITY: &str = "entity";

/// `Artist.svelte#latest`: the five most recent releases the shelf shows.
pub const LATEST: usize = 5;
/// `Artist.svelte`: `data.top.slice(0, 10)` — the popular shelf.
pub const TOP: usize = 10;

/// Which part of a discography a call asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscographyKind {
    /// `/artist/<id>/discography` — every release group in one answer.
    Groups,
    /// `/artist/<id>/on-deezer` — what the provider has and the library does not.
    OnDeezer,
}

impl DiscographyKind {
    /// The `kind` a caller hands across the boundary. Anything that is not
    /// `on-deezer` asks for the groups: an empty or unknown kind is the album
    /// page's own default, never an error a screen must handle.
    pub fn parse(kind: &str) -> DiscographyKind {
        if kind.trim().eq_ignore_ascii_case("on-deezer") {
            DiscographyKind::OnDeezer
        } else {
            DiscographyKind::Groups
        }
    }
}

/// One release, as a shelf card or a grid tile draws it. `id` is what a tap
/// opens (`entityRef`); `record_type` and `year` are raw because the French
/// label (« Album · 1998 ») is the screen's copy, not the core's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumCard {
    pub id: String,
    pub deezer_id: String,
    pub title: String,
    pub cover: Option<String>,
    pub year: Option<String>,
    pub release_date: Option<String>,
    /// `album`, `single`, `ep`, or absent for a provider card that names none.
    pub record_type: Option<String>,
    pub nb_tracks: u32,
    pub artist_name: String,
    /// The artist's ref, for the card's own sub-line link (empty when none).
    pub artist_id: String,
}

/// One related artist, as the portrait shelf draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistCard {
    pub id: String,
    pub name: String,
    pub picture: Option<String>,
}

/// `/artist/<id>`: the header, the popular shelf, the albums and the neighbours.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArtistData {
    /// What a link names this artist by (`entityRef`).
    pub id: String,
    pub deezer_id: String,
    pub name: String,
    /// `/api/cover/<id>`, or `None` for the initialled disc.
    pub picture: Option<String>,
    /// The provider's fan count, when it sent one.
    pub fans: Option<u32>,
    /// The heart's state, as the server reported it.
    pub favorite: bool,
    /// How many library titles credit this artist as something else than a
    /// performer: the « Crédits » tab exists only when it is not zero.
    pub credited: u32,
    /// The library holds titles of theirs (`/artist/<id>/on-deezer` completes it).
    pub library: bool,
    pub bio: Option<String>,
    /// The popular titles, at most the server's fifteen.
    pub top: Vec<Row>,
    pub albums: Vec<AlbumCard>,
    pub related: Vec<ArtistCard>,
}

/// `/artist/<id>/discography` and `/artist/<id>/on-deezer`, as the page draws
/// them: one answer carries every group it has, so the shelves never cost a
/// second request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Discography {
    /// The newest-first shelf source (`Artist.svelte#latest`).
    pub all: Vec<AlbumCard>,
    pub albums: Vec<AlbumCard>,
    pub singles: Vec<AlbumCard>,
    pub eps: Vec<AlbumCard>,
    /// « Apparaît dans » — the provider's own group.
    pub featured: Vec<AlbumCard>,
    /// « Disponible sur Deezer »: what the library does not hold.
    pub on_deezer: Vec<AlbumCard>,
    /// Whether the on-deezer half was ever fetched (the page's own section waits
    /// on it, and a cached page says what it knew).
    pub on_deezer_loaded: bool,
}

impl Discography {
    /// The shelf's five newest releases: `all` when the server sorted it, else
    /// the album group (an older server sends only that one).
    pub fn latest(&self) -> Vec<AlbumCard> {
        let source = if self.all.is_empty() {
            &self.albums
        } else {
            &self.all
        };
        source.iter().take(LATEST).cloned().collect()
    }
}

/// `/album/<id>`: the sleeve, the facts the band states, and the tracklist.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AlbumData {
    pub id: String,
    pub deezer_id: String,
    pub title: String,
    pub cover: Option<String>,
    pub year: Option<String>,
    pub release_date: Option<String>,
    pub artist_name: String,
    /// The artist a tap opens (`entityRef`: the row's UUID when the server sent
    /// one, else the provider's id).
    pub artist_id: String,
    pub favorite: bool,
    pub library: bool,
    pub nb_tracks: u32,
    /// The whole tracklist's duration, in seconds — the band's « 43:12 ».
    pub total_s: u32,
    /// The library's tracks, in disc/track order.
    pub tracks: Vec<Row>,
}

/// One person of one role (`components/TrackCredits.svelte`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreditPerson {
    pub name: String,
    /// The artist page to open, when the library holds an artist of that name
    /// (a row UUID or a provider id); `None` means the name is not an artist
    /// page — it may still open a credits page ([`CreditPerson::credited`]).
    pub artist_id: Option<String>,
    /// How many library titles this person is credited on: at two or more, the
    /// name opens its own credits page (`credits.py#PAGE_FROM`).
    pub credited: Option<u32>,
}

/// One role of a track's credits, in a sleeve's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreditGroup {
    pub role: String,
    /// The French label the server chose.
    pub label: String,
    pub people: Vec<CreditPerson>,
}

/// The release line under the credits: what the sleeve also says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreditsInfo {
    pub isrc: Option<String>,
    pub released: Option<String>,
    pub label: Option<String>,
    pub copyright: Option<String>,
    pub album: Option<String>,
}

impl CreditsInfo {
    /// Whether the release line has anything to say.
    pub fn is_empty(&self) -> bool {
        self.isrc.is_none()
            && self.released.is_none()
            && self.label.is_none()
            && self.copyright.is_none()
            && self.album.is_none()
    }
}

/// `/track/<id>/credits`: the roles, then the release line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackCredits {
    pub credits: Vec<CreditGroup>,
    pub info: CreditsInfo,
    /// Nothing at all: the sheet says « Aucun crédit connu pour ce titre ».
    pub empty: bool,
}

/// One role a person appears in, with how many titles (`PersonCredits.svelte`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonRole {
    pub role: String,
    pub label: String,
    pub count: u32,
}

/// `/credits/person?name=`: what one person wrote, composed or produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PersonCredits {
    /// The name the credits themselves spell (the server's answer), which a
    /// differently-cased query adopts.
    pub name: String,
    pub roles: Vec<PersonRole>,
    /// Each title's roles, by row key: a chip filters the list locally, without
    /// asking the server again (`PersonCredits.svelte#shown`).
    pub roles_by_key: BTreeMap<String, Vec<String>>,
    pub tracks: Vec<Row>,
}

impl PersonCredits {
    /// The titles one role shows, or all of them for an empty role. Local on
    /// purpose: the SPA's chips filter the list it already has.
    pub fn with_role(&self, role: &str) -> Vec<Row> {
        if role.trim().is_empty() {
            return self.tracks.clone();
        }
        self.tracks
            .iter()
            .filter(|row| {
                self.roles_by_key
                    .get(&row.key)
                    .is_some_and(|roles| roles.iter().any(|known| known == role))
            })
            .cloned()
            .collect()
    }
}

/// What one tag spent, as `nz-api` counts it (SC3): requests really sent, and the
/// response bodies' bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Totals {
    requests: u64,
    bytes_in: u64,
    bytes_out: u64,
}

/// The `entity` tag's running totals on this client.
fn totals(api: &nz_api::Api) -> Totals {
    api.stats()
        .into_iter()
        .find(|row| row.tag == TAG_ENTITY)
        .map(|row| Totals {
            requests: row.requests,
            bytes_in: row.bytes_in,
            bytes_out: row.bytes_out,
        })
        .unwrap_or_default()
}

impl Core {
    // -- the artist page ------------------------------------------------------

    /// The artist page as this device last saw it — disk only, no request. `None`
    /// the first time, which is the one moment the screen shows a skeleton.
    pub fn cached_artist(&self, id: &str) -> Option<ArtistData> {
        wire::artist(id, &store::read(self, store::ARTIST, id)?).ok()
    }

    /// `GET /api/artist/<id>` — ONE request, counted under [`TAG_ENTITY`], and
    /// the answer is kept for the next visit.
    pub async fn artist(&self, id: &str) -> Result<ArtistData, CoreError> {
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_artist(&api, id).await?;
            let page = wire::artist(id, &body)?;
            self.keep(store::ARTIST, id, &body);
            Ok(page)
        }
        .await;
        self.book(&api, before);
        answer
    }

    /// The discography this device last saw, both halves merged (the groups and
    /// the provider's part). Disk only.
    pub fn cached_discography(&self, id: &str) -> Option<Discography> {
        let mut discography: Discography = store::read(self, store::DISCOGRAPHY, id)
            .and_then(|body| wire::discography(id, &body).ok())
            .unwrap_or_default();
        if let Some(body) = store::read(self, store::ARTIST_ON_DEEZER, id)
            && let Ok(cards) = wire::album_cards(&body)
        {
            discography.on_deezer = cards;
            discography.on_deezer_loaded = true;
        }
        Some(discography)
    }

    /// `GET /api/artist/<id>/discography` (one request) or
    /// `GET /api/artist/<id>/on-deezer` (one request). The answer carries BOTH
    /// halves the device knows, so the shelves of the page never ask again.
    ///
    /// `cursor` is the contract's paging token: the server sends whole groups
    /// today, so no page ever has one (`None`), and a future paged discography
    /// reads it without a second method.
    pub async fn discography(
        &self,
        id: &str,
        kind: DiscographyKind,
        _cursor: Option<String>,
    ) -> Result<Discography, CoreError> {
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before = totals(&api);
        let answer = async {
            match kind {
                DiscographyKind::Groups => {
                    let body = wire::fetch_discography(&api, id).await?;
                    wire::discography(id, &body)?;
                    self.keep(store::DISCOGRAPHY, id, &body);
                }
                DiscographyKind::OnDeezer => {
                    let body = wire::fetch_artist_on_deezer(&api, id).await?;
                    wire::album_cards(&body)?;
                    self.keep(store::ARTIST_ON_DEEZER, id, &body);
                }
            }
            Ok::<(), CoreError>(())
        }
        .await;
        self.book(&api, before);
        answer?;
        Ok(self.cached_discography(id).unwrap_or_default())
    }

    /// Every title of this artist the library holds — the « Titres » tab. Disk
    /// only, `None` before the tab was ever opened.
    pub fn cached_artist_tracks(&self, id: &str) -> Option<Vec<Row>> {
        wire::track_rows(&store::read(self, store::ARTIST_TRACKS, id)?).ok()
    }

    /// `GET /api/artist/<id>/tracks` — ONE request, asked lazily when the person
    /// opens the tab (`Artist.svelte#loadTracks`).
    pub async fn artist_tracks(&self, id: &str) -> Result<Vec<Row>, CoreError> {
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_artist_tracks(&api, id).await?;
            let rows = wire::track_rows(&body)?;
            self.keep(store::ARTIST_TRACKS, id, &body);
            Ok(rows)
        }
        .await;
        self.book(&api, before);
        answer
    }

    /// `GET /api/radio/artist/<id>` — the « Radio » button: ONE request for the
    /// first titles, then the player's own refill continues the station.
    pub async fn artist_radio(&self, id: &str) -> Result<Vec<Row>, CoreError> {
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_artist_radio(&api, id).await?;
            wire::track_rows(&body)
        }
        .await;
        self.book(&api, before);
        answer
    }

    // -- the album page -------------------------------------------------------

    /// The album page as this device last saw it — disk only.
    pub fn cached_album(&self, id: &str) -> Option<AlbumData> {
        wire::album(id, &store::read(self, store::ALBUM, id)?).ok()
    }

    /// `GET /api/album/<id>` — ONE request; kept for the next visit.
    pub async fn album(&self, id: &str) -> Result<AlbumData, CoreError> {
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_album(&api, id).await?;
            let page = wire::album(id, &body)?;
            self.keep(store::ALBUM, id, &body);
            Ok(page)
        }
        .await;
        self.book(&api, before);
        answer
    }

    /// The rest of an album's tracklist the provider has: disk only.
    pub fn cached_album_on_deezer(&self, id: &str) -> Option<Vec<Row>> {
        wire::track_rows(&store::read(self, store::ALBUM_ON_DEEZER, id)?).ok()
    }

    /// `GET /api/album/<id>/on-deezer` — ONE request, asked after the page is on
    /// screen and only for a library album. A provider that cannot be reached is
    /// `Err`, and the screen says so without condemning the album.
    pub async fn album_on_deezer(&self, id: &str) -> Result<Vec<Row>, CoreError> {
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_album_on_deezer(&api, id).await?;
            let rows = wire::track_rows(&body)?;
            self.keep(store::ALBUM_ON_DEEZER, id, &body);
            Ok(rows)
        }
        .await;
        self.book(&api, before);
        answer
    }

    // -- credits --------------------------------------------------------------

    /// One track's credits as this device last saw them — disk only, which is
    /// what makes the sheet open instantly on a track already visited.
    pub fn cached_track_credits(&self, id: &str) -> Option<TrackCredits> {
        wire::track_credits(id, &store::read(self, store::TRACK_CREDITS, id)?).ok()
    }

    /// `GET /api/track/<id>/credits` — ONE request (`CreditsSheet.svelte`).
    pub async fn track_credits(&self, id: &str) -> Result<TrackCredits, CoreError> {
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_track_credits(&api, id).await?;
            let credits = wire::track_credits(id, &body)?;
            self.keep(store::TRACK_CREDITS, id, &body);
            Ok(credits)
        }
        .await;
        self.book(&api, before);
        answer
    }

    /// A person's credits page as this device last saw it — disk only.
    pub fn cached_person_credits(&self, name: &str) -> Option<PersonCredits> {
        wire::person_credits(name, &store::read(self, store::PERSON_CREDITS, name)?).ok()
    }

    /// `GET /api/credits/person?name=` — ONE request, however many roles the page
    /// then offers.
    pub async fn person_credits(&self, name: &str) -> Result<PersonCredits, CoreError> {
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before = totals(&api);
        let answer = async {
            let body = wire::fetch_person_credits(&api, name).await?;
            let page = wire::person_credits(name, &body)?;
            self.keep(store::PERSON_CREDITS, name, &body);
            Ok(page)
        }
        .await;
        self.book(&api, before);
        answer
    }

    /// The titles one role shows on an already-read credits page — ZERO
    /// requests: the SPA's chips filter the list they have.
    pub fn person_credits_role(&self, name: &str, role: &str) -> Option<Vec<Row>> {
        self.cached_person_credits(name)
            .map(|page| page.with_role(role))
    }

    // -- the one write these pages do -----------------------------------------

    /// The heart: `POST /api/favorite/<kind>` — ONE request, never retried
    /// (a mutation, `nz-api`'s own rule).
    ///
    /// The device's copy flips FIRST, so a page reopened while the request flies
    /// already shows the new state; a refusal puts the old body back and the
    /// screen rolls its own state back from the returned `Err`. `meta` is
    /// `trackmenu.js#entityMeta`'s body: what makes the row when the star is the
    /// first thing that ever named the entity.
    pub async fn set_favorite(
        &self,
        kind: &str,
        id: &str,
        on: bool,
        meta: serde_json::Value,
    ) -> Result<(), CoreError> {
        let (store_kind, section) = match kind {
            "artist" => (store::ARTIST, "artist"),
            "album" => (store::ALBUM, "album"),
            other => {
                return Err(CoreError::Invalid {
                    detail: format!("Type inconnu : {other}"),
                });
            }
        };
        let before = store::read(self, store_kind, id);
        if let Some(body) = &before
            && let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body)
        {
            value[section]["is_favorite"] = serde_json::Value::Bool(on);
            let _ = store::write(self, store_kind, id, &value.to_string());
        }
        let api = self.api()?.with_tag(TAG_ENTITY);
        let before_traffic = totals(&api);
        let answer = wire::favorite(&api, kind, id, on, &meta).await;
        self.book(&api, before_traffic);
        match answer {
            Ok(()) => Ok(()),
            Err(cause) => {
                // The star did not move: the copy the next visit paints says so.
                if let Some(body) = before {
                    let _ = store::write(self, store_kind, id, &body);
                }
                Err(cause)
            }
        }
    }

    /// The heart, when the screen has the request's `meta` at hand.
    ///
    /// `Artist.svelte`/`Album.svelte` send the entity's title (and, for an album,
    /// its artist) so the server can create the row the star names. A page that
    /// only has the id still works: the body is then just the reference.
    pub fn favorite_meta(
        kind: &str,
        title: &str,
        artist: Option<&str>,
        artist_id: Option<&str>,
    ) -> serde_json::Value {
        let mut meta = serde_json::Map::new();
        if !title.is_empty() {
            meta.insert(
                "title".to_owned(),
                serde_json::Value::String(title.to_owned()),
            );
        }
        if kind == "album" {
            if let Some(artist) = artist.filter(|value| !value.is_empty()) {
                meta.insert(
                    "artist".to_owned(),
                    serde_json::Value::String(artist.to_owned()),
                );
            }
            if let Some(artist_id) = artist_id.filter(|value| !value.is_empty()) {
                meta.insert(
                    "artist_id".to_owned(),
                    serde_json::Value::String(artist_id.to_owned()),
                );
            }
        }
        serde_json::Value::Object(meta)
    }

    /// Keeps a body, and never lets a cache that cannot be written cost the
    /// screen: the fresh answer is already in hand.
    fn keep(&self, kind: &str, key: &str, body: &str) {
        if let Err(cause) = store::write(self, kind, key, body) {
            tracing::debug!(%cause, "the entity copy was not kept");
        }
    }

    /// Books what `nz-api` counted for this tag since `before` in the core's own
    /// meter, so `NzCore::stats()` answers what the page cost without the UI
    /// counting anything itself (rule 3's "counted, not timed").
    fn book(&self, api: &nz_api::Api, before: Totals) {
        let after = totals(api);
        let requests = after.requests.saturating_sub(before.requests);
        if requests == 0 {
            return;
        }
        // `Meter::record` counts ONE request and its bytes; the whole run's bytes
        // go with the first, the remaining N−1 are zeros.
        self.meter().record(
            TAG_ENTITY,
            after.bytes_in.saturating_sub(before.bytes_in),
            after.bytes_out.saturating_sub(before.bytes_out),
        );
        for _ in 1..requests {
            self.meter().record(TAG_ENTITY, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album_card(id: &str, year: &str) -> AlbumCard {
        AlbumCard {
            id: id.to_owned(),
            deezer_id: id.to_owned(),
            title: format!("Sortie {id}"),
            year: Some(year.to_owned()),
            record_type: Some("album".to_owned()),
            ..AlbumCard::default()
        }
    }

    /// The shelf is five, the newest first, and it falls back to the album group
    /// when an older server sent no `all` — `Artist.svelte#latest`.
    #[test]
    fn the_latest_shelf_is_five_releases() {
        let groups = Discography {
            all: (0..8).map(|n| album_card(&n.to_string(), "2024")).collect(),
            ..Discography::default()
        };
        let latest = groups.latest();
        assert_eq!(latest.len(), LATEST);
        assert_eq!(latest[0].title, "Sortie 0", "the server's own order");

        let older = Discography {
            albums: vec![album_card("1", "1998"), album_card("2", "2001")],
            ..Discography::default()
        };
        assert_eq!(older.latest().len(), 2);
        assert_eq!(older.latest()[0].id, "1");
    }

    /// The two kinds a caller may name: anything else is the groups, never an
    /// error a screen has to handle.
    #[test]
    fn a_discography_kind_is_read_leniently() {
        assert_eq!(
            DiscographyKind::parse("on-deezer"),
            DiscographyKind::OnDeezer
        );
        assert_eq!(
            DiscographyKind::parse(" On-Deezer "),
            DiscographyKind::OnDeezer
        );
        for other in ["", "all", "album", "nonsense"] {
            assert_eq!(
                DiscographyKind::parse(other),
                DiscographyKind::Groups,
                "{other:?} asks for the groups"
            );
        }
    }

    /// A chip filters the titles it already has: no request, and a title that
    /// carries two roles shows under both.
    #[test]
    fn a_role_filters_the_page_it_already_has() {
        let mut page = PersonCredits {
            name: "Nina".to_owned(),
            roles: vec![PersonRole {
                role: "composer".to_owned(),
                label: "Compositeur".to_owned(),
                count: 2,
            }],
            ..PersonCredits::default()
        };
        page.tracks = vec![Row::test_row(0, "A"), Row::test_row(1, "B")];
        page.roles_by_key
            .insert("test:0".to_owned(), vec!["composer".to_owned()]);
        page.roles_by_key.insert(
            "test:1".to_owned(),
            vec!["composer".to_owned(), "author".to_owned()],
        );
        assert_eq!(page.with_role("").len(), 2, "Tout");
        assert_eq!(page.with_role("composer").len(), 2);
        assert_eq!(page.with_role("author").len(), 1);
        assert_eq!(page.with_role("producer").len(), 0);
    }

    /// The release line says whether it has anything: an ISRC alone is enough to
    /// print it, and nothing at all is not an empty line.
    #[test]
    fn a_release_line_knows_when_it_is_empty() {
        assert!(CreditsInfo::default().is_empty());
        assert!(
            !CreditsInfo {
                isrc: Some("FR123".to_owned()),
                ..CreditsInfo::default()
            }
            .is_empty()
        );
    }

    /// The heart's body is `entityMeta`'s: the title, plus an album's artist.
    #[test]
    fn the_heart_body_is_the_spas_entity_meta() {
        assert_eq!(
            Core::favorite_meta("artist", "Nina Simone", Some("ignored"), Some("ignored")),
            serde_json::json!({"title": "Nina Simone"})
        );
        assert_eq!(
            Core::favorite_meta("album", "Le matin", Some("Ascenseur"), Some("42")),
            serde_json::json!({"title": "Le matin", "artist": "Ascenseur", "artist_id": "42"})
        );
        assert_eq!(
            Core::favorite_meta("album", "Le matin", None, None),
            serde_json::json!({"title": "Le matin"})
        );
    }
}

/// The lot's own acceptance: the pages' request budgets, their failure shapes and
/// the one write they do — against a server that COUNTS, never against a
/// duration.
#[cfg(test)]
mod lot_tests {
    use super::*;
    use crate::core::NetworkState;

    /// A cold artist page costs the SPA's THREE requests and not one more —
    /// `/artist/<id>`, `/artist/<id>/discography`, `/artist/<id>/on-deezer` — and
    /// a warm one costs ZERO: the body is on the device.
    ///
    /// Numbers this test holds: 3 requests / 0 bytes from the copy, one `entity`
    /// row in `NzCore::stats()` with the same three.
    #[tokio::test]
    async fn a_cold_artist_page_asks_the_spas_three_requests_and_a_warm_one_asks_none() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();

        let artist = core.artist("42").await.expect("an artist");
        let groups = core
            .discography("42", DiscographyKind::Groups, None)
            .await
            .expect("a discography");
        let with_deezer = core
            .discography("42", DiscographyKind::OnDeezer, None)
            .await
            .expect("the provider's part");

        let mut asked = server.routes();
        asked.sort();
        assert_eq!(
            asked,
            vec![
                "GET /api/artist/42".to_owned(),
                "GET /api/artist/42/discography".to_owned(),
                "GET /api/artist/42/on-deezer".to_owned(),
            ],
            "exactly the SPA's three: {} requests",
            server.count()
        );
        assert_eq!(artist.name, "Ascenseur");
        assert!(artist.favorite, "the heart is the server's own state");
        assert_eq!(artist.credited, 3, "the Crédits tab exists");
        assert!(artist.library, "the provider's part is worth asking");
        assert_eq!(artist.top.len(), 2);
        assert_eq!(artist.albums.len(), 1);
        assert_eq!(artist.related.len(), 1, "« Artistes similaires »");
        assert_eq!(artist.id, "aaaaaaaa-1111-4111-8111-111111111111");
        assert_eq!(groups.albums.len(), 1);
        assert_eq!(groups.latest().len(), 2, "the shelf is the `all` group");
        assert_eq!(groups.singles.len(), 1);
        assert_eq!(groups.featured.len(), 1, "« Apparaît dans »");
        assert!(
            groups.on_deezer.is_empty(),
            "the discography route is not the provider's part"
        );
        assert_eq!(with_deezer.on_deezer.len(), 1);
        assert!(with_deezer.on_deezer_loaded);
        assert_eq!(with_deezer.on_deezer[0].title, "Seulement sur Deezer");

        let row = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_ENTITY)
            .expect("one entity row");
        assert_eq!(row.requests, 3, "NzCore::stats() counts the same three");
        assert!(
            row.bytes_in > 0,
            "the response bodies are counted, not timed: {} bytes",
            row.bytes_in
        );
        println!(
            "MEASURE artist cold: {} requests, {} bytes in, {} bytes out",
            row.requests, row.bytes_in, row.bytes_out
        );

        // A warm open paints from the device: no request, and the same page.
        server.forget();
        let cached = core.cached_artist("42").expect("the copy of the visit");
        let discography = core
            .cached_discography("42")
            .expect("the discography of the visit");
        assert_eq!(server.count(), 0, "a warm artist asks nothing");
        assert_eq!(cached.name, "Ascenseur");
        assert!(cached.favorite);
        assert_eq!(discography.albums.len(), 1);
        assert_eq!(
            discography.on_deezer.len(),
            1,
            "both halves come back from the copy"
        );
    }

    /// A cold album costs TWO requests (`/album`, `/album/<id>/on-deezer`), a warm
    /// one ZERO; the artist link is the artist's ROW id, which is what
    /// `entityRef` prefers.
    #[tokio::test]
    async fn a_cold_album_asks_two_requests_and_a_warm_one_asks_none() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();

        let album = core.album("501").await.expect("an album");
        let rest = core.album_on_deezer("501").await.expect("the rest");

        let mut asked = server.routes();
        asked.sort();
        assert_eq!(
            asked,
            vec![
                "GET /api/album/501".to_owned(),
                "GET /api/album/501/on-deezer".to_owned(),
            ],
            "two requests, {}",
            server.count()
        );
        assert_eq!(album.title, "Le matin");
        assert_eq!(
            album.artist_id, "aaaaaaaa-1111-4111-8111-111111111111",
            "the row's UUID, not the provider's id"
        );
        assert_eq!(album.nb_tracks, 2);
        assert_eq!(album.total_s, 380, "the band's « 6:20 »");
        assert!(album.library);
        assert!(!album.favorite);
        assert_eq!(album.tracks[0].title, "Ouverture");
        assert_eq!(rest.len(), 1);

        server.forget();
        let cached = core.cached_album("501").expect("the copy");
        assert_eq!(server.count(), 0, "a warm album asks nothing");
        assert_eq!(cached.title, "Le matin");
        assert_eq!(
            core.cached_album_on_deezer("501").map(|rows| rows.len()),
            Some(1)
        );

        let row = core
            .stats()
            .into_iter()
            .find(|row| row.tag == TAG_ENTITY)
            .expect("one entity row");
        println!(
            "MEASURE album cold: {} requests over both pages, {} bytes in",
            row.requests, row.bytes_in
        );
    }

    /// A track's credits and a person's page are ONE request each; a role chip is
    /// ZERO (the SPA filters the list it already has), and the lazy « Titres » tab
    /// and the radio button are one request each.
    #[tokio::test]
    async fn the_sub_pages_are_one_request_each_and_a_chip_is_none() {
        let (server, _dir, core) = testing::configured().await;
        server.forget();

        let credits = core
            .track_credits("11111111-1111-4111-8111-111111111111")
            .await
            .expect("credits");
        assert_eq!(server.count(), 1, "the sheet: one request");
        assert!(!credits.empty);
        assert_eq!(credits.credits.len(), 2);
        assert_eq!(credits.credits[0].label, "Compositeur");
        assert_eq!(
            credits.credits[0].people[0].artist_id.as_deref(),
            Some("dddddddd-1111-4111-8111-111111111111"),
            "a name the library holds opens its artist page"
        );
        assert_eq!(credits.credits[0].people[0].credited, Some(3));
        assert_eq!(credits.info.isrc.as_deref(), Some("FR1234567890"));
        assert_eq!(credits.info.label.as_deref(), Some("Blue Note"));

        server.forget();
        let page = core.person_credits("Nina Simone").await.expect("a page");
        assert_eq!(server.count(), 1, "the page: one request");
        assert_eq!(
            server.hits()[0].query("name").as_deref(),
            Some("Nina Simone"),
            "the name arrives as one query value"
        );
        assert!(
            server.hits()[0].path.contains("name=Nina%20Simone"),
            "and it is percent-encoded (`encodeURIComponent`): {}",
            server.hits()[0].path
        );
        assert_eq!(page.name, "Nina Simone");
        assert_eq!(page.tracks.len(), 1);
        assert_eq!(page.roles.len(), 1);

        // The chip: the titles it already has, no request at all.
        server.forget();
        assert_eq!(
            core.person_credits_role("Nina Simone", "composer")
                .map(|rows| rows.len()),
            Some(1)
        );
        assert_eq!(
            core.person_credits_role("Nina Simone", "producer")
                .map(|rows| rows.len()),
            Some(0),
            "the role this title does not carry"
        );
        assert_eq!(server.count(), 0, "a chip filters locally");

        // The lazy tab and the radio button.
        server.forget();
        assert!(core.cached_artist_tracks("42").is_none());
        let tracks = core.artist_tracks("42").await.expect("the tab");
        assert_eq!(
            server.routes(),
            vec!["GET /api/artist/42/tracks".to_owned()]
        );
        assert_eq!(tracks.len(), 2);
        server.forget();
        assert_eq!(
            core.artist_tracks("42").await.map(|rows| rows.len()),
            Ok(2),
            "a second open asks the server again (SWR), the copy paints first"
        );
        assert_eq!(
            core.cached_artist_tracks("42").map(|rows| rows.len()),
            Some(2)
        );
        server.forget();
        assert_eq!(core.artist_radio("42").await.map(|rows| rows.len()), Ok(1));
        assert_eq!(server.routes(), vec!["GET /api/radio/artist/42".to_owned()]);
    }

    /// A dead link fails fast and keeps what the person already saw: the copy
    /// answers, and NO socket is opened to learn there is no link.
    #[tokio::test]
    async fn a_dead_link_opens_no_socket_and_the_copy_still_paints() {
        let (server, _dir, core) = testing::configured().await;
        core.album("501").await.expect("one good visit");
        assert!(core.cached_album("501").is_some());

        server.forget();
        let before = server.accepted();
        core.set_network(NetworkState::None);
        assert_eq!(
            core.album("501").await.expect_err("offline"),
            CoreError::Offline,
            "the cause is the link, not a timeout"
        );
        assert_eq!(server.accepted(), before, "not one socket was opened");
        assert_eq!(server.count(), 0);
        assert!(
            core.cached_album("501").is_some(),
            "the copy is what the screen keeps"
        );
    }

    /// A provider that refuses costs its own sentence, never the page; a page
    /// that refuses is the page's cause.
    #[tokio::test]
    async fn a_refused_provider_leaves_the_page_standing() {
        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/artist/42/on-deezer", 503);
        let artist = core.artist("42").await.expect("the page still opens");
        assert!(artist.library);
        assert!(
            core.discography("42", DiscographyKind::OnDeezer, None)
                .await
                .is_err(),
            "the Deezer part failed, and the page says so"
        );
        assert_eq!(
            core.cached_discography("42")
                .map(|groups| groups.on_deezer.len()),
            Some(0),
            "nothing was kept from a refusal"
        );

        let (server, _dir, core) = testing::configured().await;
        server.break_route("/api/album/501", 500);
        let cause = core.album("501").await.expect_err("the album failed");
        assert!(
            matches!(cause, CoreError::Server { status: 500, .. }),
            "the server's own status, a cause a screen can print: {cause:?}"
        );
        assert!(core.cached_album("501").is_none(), "nothing invented");
    }

    /// The heart is the lot's ONE write: the device's copy flips first, the
    /// request is one POST with `entityMeta`'s body, and a refusal puts the copy
    /// back so a page reopened while offline is not lying.
    #[tokio::test]
    async fn the_heart_flips_the_copy_first_and_rolls_back_when_refused() {
        let (server, _dir, core) = testing::configured().await;
        core.artist("42").await.expect("the page");
        assert!(core.cached_artist("42").expect("the copy").favorite);

        server.forget();
        core.set_favorite(
            "artist",
            "42",
            false,
            Core::favorite_meta("artist", "Ascenseur", None, None),
        )
        .await
        .expect("unfollowed");
        assert_eq!(
            server.routes(),
            vec!["POST /api/favorite/artist".to_owned()],
            "one request"
        );
        let body = server.last_body("/api/favorite/artist").expect("its body");
        assert!(
            body.contains("\"deezer_id\""),
            "a provider id is named deezer_id (`api.js#favoriteEntity`): {body}"
        );
        assert!(body.contains("\"on\":false"), "{body}");
        assert!(
            body.contains("Ascenseur"),
            "the title makes the row: {body}"
        );
        assert!(
            !core.cached_artist("42").expect("the copy").favorite,
            "the copy flipped first"
        );

        server.break_route("/api/favorite/artist", 500);
        assert!(
            core.set_favorite(
                "artist",
                "42",
                true,
                Core::favorite_meta("artist", "Ascenseur", None, None),
            )
            .await
            .is_err()
        );
        assert!(
            !core.cached_artist("42").expect("the copy").favorite,
            "a refusal rolled the copy back"
        );
    }
}
