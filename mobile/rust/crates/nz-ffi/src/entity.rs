//! Artist, album, discography and credits — the FFI surface of lot MB7.
//!
//! Owned by **MB7** (docs/agents/mobile.md, "Module and file ownership"): the
//! lot fills this file — its own `#[uniffi::export] impl` blocks, its own
//! records — and edits no other file of the crate. UniFFI merges the `impl
//! Screens` block below into the ONE Kotlin `Screens` class, so nothing shared is
//! touched.
//!
//! # The shape of this boundary
//!
//! - **Every page has the pair the screen standard names**: `cached_x()` is
//!   synchronous and disk-only (the frame a warm open paints, 0 requests), `x()`
//!   is `async` and goes to the network on the `User` lane, so a screen never
//!   waits on the link to show something.
//! - **A list of titles never crosses whole**: `artist_tracks`, an album's
//!   tracklist and a person's credits are `ListHandle`s, windowed by the screen
//!   (`rememberListWindow`). The short ones (a popular shelf, a related-artist
//!   row, the provider's remainder of an album) cross as `Vec`s: they are one
//!   page by construction.
//! - **The French copy is the screen's.** Records carry what the page DRAWS (a
//!   release type, a year, a role's label as the server wrote it) and never a
//!   sentence: « Album · 1998 » is built by Kotlin.
//! - **A name opens somewhere or it is plain text**: a credit carries the artist
//!   page it can open and, failing that, the count that makes its own credits
//!   page worth a link (`TrackCredits.svelte#href`).
//!
//! Every body converts to `nz_core::entity` and back; no rule lives here.

use std::sync::Arc;

use nz_core::entity as core;

use crate::core::Screens;
use crate::errors::NzError;
use crate::lists::{ListHandle, TrackRow, open_rows};

/// One release, as a shelf card or a grid tile draws it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct AlbumCardData {
    /// What a tap opens: the row's UUID when the library holds it, else the
    /// provider's id (`entityRef`).
    pub id: String,
    pub deezer_id: String,
    pub title: String,
    pub cover: Option<String>,
    pub year: Option<String>,
    pub release_date: Option<String>,
    /// `album`, `single`, `ep`, or absent — the screen prints « Album », « EP »…
    pub record_type: Option<String>,
    pub nb_tracks: u32,
    pub artist_name: String,
    /// The artist a sub-line tap opens (empty when there is none).
    pub artist_id: String,
}

impl From<core::AlbumCard> for AlbumCardData {
    fn from(card: core::AlbumCard) -> Self {
        AlbumCardData {
            id: card.id,
            deezer_id: card.deezer_id,
            title: card.title,
            cover: card.cover,
            year: card.year,
            release_date: card.release_date,
            record_type: card.record_type,
            nb_tracks: card.nb_tracks,
            artist_name: card.artist_name,
            artist_id: card.artist_id,
        }
    }
}

fn album_cards(cards: Vec<core::AlbumCard>) -> Vec<AlbumCardData> {
    cards.into_iter().map(AlbumCardData::from).collect()
}

/// One related artist, as the portrait shelf draws it.
///
/// `ArtistTileData` and not `ArtistCardData`: the home's favourite-artist row
/// already owns that name in this crate (UniFFI requires one record name per
/// crate), and the two carry different things (that one counts plays).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ArtistTileData {
    pub id: String,
    pub name: String,
    pub picture: Option<String>,
}

impl From<core::ArtistCard> for ArtistTileData {
    fn from(card: core::ArtistCard) -> Self {
        ArtistTileData {
            id: card.id,
            name: card.name,
            picture: card.picture,
        }
    }
}

/// `/api/artist/<id>`: the header, the popular shelf, the albums and neighbours.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ArtistData {
    pub id: String,
    pub deezer_id: String,
    pub name: String,
    pub picture: Option<String>,
    /// The provider's fan count, when it sent one (the facts line falls back to
    /// the title count once the « Titres » tab is loaded).
    pub fans: Option<u32>,
    pub favorite: bool,
    /// Titles of the library that credit this artist as something else than a
    /// performer: the « Crédits » tab exists only when it is not zero.
    pub credited: u32,
    /// The library holds titles of theirs.
    pub library: bool,
    pub bio: Option<String>,
    /// The popular titles, at most the server's fifteen.
    pub top: Vec<TrackRow>,
    pub albums: Vec<AlbumCardData>,
    pub related: Vec<ArtistTileData>,
}

impl From<core::ArtistData> for ArtistData {
    fn from(data: core::ArtistData) -> Self {
        ArtistData {
            id: data.id,
            deezer_id: data.deezer_id,
            name: data.name,
            picture: data.picture,
            fans: data.fans,
            favorite: data.favorite,
            credited: data.credited,
            library: data.library,
            bio: data.bio,
            top: rows(data.top),
            albums: album_cards(data.albums),
            related: data.related.into_iter().map(ArtistTileData::from).collect(),
        }
    }
}

/// The artist's releases, grouped as the page's shelves are — and the provider's
/// part, which the page asks for separately.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct DiscographyData {
    /// The five newest releases, in the server's order (« Dernières sorties »).
    pub latest: Vec<AlbumCardData>,
    /// The albums grid (the page's own `albums` when the discography has none).
    pub albums: Vec<AlbumCardData>,
    pub singles: Vec<AlbumCardData>,
    pub eps: Vec<AlbumCardData>,
    /// « Apparaît dans ».
    pub featured: Vec<AlbumCardData>,
    /// « Disponible sur Deezer »: what the library does not hold.
    pub on_deezer: Vec<AlbumCardData>,
    /// Whether the provider's half was ever fetched: the section's skeleton waits
    /// on this, and a page from the copy says what it knew.
    pub on_deezer_loaded: bool,
    /// The contract's paging token. `None` today: the server sends whole groups.
    pub next_cursor: Option<String>,
}

impl From<core::Discography> for DiscographyData {
    fn from(groups: core::Discography) -> Self {
        DiscographyData {
            latest: album_cards(groups.latest()),
            albums: album_cards(groups.albums),
            singles: album_cards(groups.singles),
            eps: album_cards(groups.eps),
            featured: album_cards(groups.featured),
            on_deezer: album_cards(groups.on_deezer),
            on_deezer_loaded: groups.on_deezer_loaded,
            next_cursor: None,
        }
    }
}

/// `/api/album/<id>`: the sleeve, the facts the band states, and the tracklist.
///
/// No `PartialEq`: the record carries a `ListHandle` (a UniFFI object), which is
/// identity, not a value — comparing two windows is not what a screen does.
#[derive(uniffi::Record, Clone)]
pub struct AlbumData {
    pub id: String,
    pub deezer_id: String,
    pub title: String,
    pub cover: Option<String>,
    pub year: Option<String>,
    pub release_date: Option<String>,
    pub artist_name: String,
    /// The artist a tap opens (the row's UUID when the server sent one).
    pub artist_id: String,
    pub favorite: bool,
    pub library: bool,
    pub nb_tracks: u32,
    /// The whole tracklist's duration, in seconds — the band's « 43:12 ».
    pub total_s: u32,
    /// The library's tracks, windowed: an album can be a box set.
    pub tracks: Arc<ListHandle>,
}

/// One person of one role.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct CreditPersonData {
    pub name: String,
    /// The artist page to open; `None` means the name is not an artist page.
    pub artist_id: Option<String>,
    /// Titles of the library this person is credited on: at two or more the name
    /// also opens its own credits page.
    pub credited: Option<u32>,
}

impl From<core::CreditPerson> for CreditPersonData {
    fn from(person: core::CreditPerson) -> Self {
        CreditPersonData {
            name: person.name,
            artist_id: person.artist_id,
            credited: person.credited,
        }
    }
}

/// One role of a track's credits, in a sleeve's order.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct CreditGroupData {
    /// The machine role (`composer`, `producer`, …) — the page's own key.
    pub role: String,
    /// The French label the server chose (« Compositeur »).
    pub label: String,
    pub people: Vec<CreditPersonData>,
}

impl From<core::CreditGroup> for CreditGroupData {
    fn from(group: core::CreditGroup) -> Self {
        CreditGroupData {
            role: group.role,
            label: group.label,
            people: group
                .people
                .into_iter()
                .map(CreditPersonData::from)
                .collect(),
        }
    }
}

/// The release line under the credits.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct CreditsInfoData {
    pub isrc: Option<String>,
    pub released: Option<String>,
    pub label: Option<String>,
    pub copyright: Option<String>,
    pub album: Option<String>,
}

impl From<core::CreditsInfo> for CreditsInfoData {
    fn from(info: core::CreditsInfo) -> Self {
        CreditsInfoData {
            isrc: info.isrc,
            released: info.released,
            label: info.label,
            copyright: info.copyright,
            album: info.album,
        }
    }
}

/// `/api/track/<id>/credits`: the roles, then the release line.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct TrackCreditsData {
    pub credits: Vec<CreditGroupData>,
    pub info: CreditsInfoData,
    /// Nothing at all: the sheet says « Aucun crédit connu pour ce titre ».
    pub empty: bool,
}

impl From<core::TrackCredits> for TrackCreditsData {
    fn from(credits: core::TrackCredits) -> Self {
        TrackCreditsData {
            credits: credits
                .credits
                .into_iter()
                .map(CreditGroupData::from)
                .collect(),
            info: credits.info.into(),
            empty: credits.empty,
        }
    }
}

/// One role a person appears in, with how many titles.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PersonRoleData {
    pub role: String,
    pub label: String,
    pub count: u32,
}

/// `/api/credits/person?name=`: the roles, and the titles.
#[derive(uniffi::Record, Clone)]
pub struct PersonCreditsData {
    /// The name the credits themselves spell.
    pub name: String,
    pub roles: Vec<PersonRoleData>,
    pub count: u32,
    /// Every title, windowed. A role chip asks [`Screens::person_credits_role`]
    /// for the same list narrowed — no request.
    pub tracks: Arc<ListHandle>,
}

/// The rows a list of titles crosses as — the screen's own `TrackRow`.
fn rows(rows: Vec<nz_core::lists::Row>) -> Vec<TrackRow> {
    rows.into_iter().map(TrackRow::from).collect()
}

/// The same, as a windowed handle (registered, so a later patch reaches it).
fn handle(screens: &Screens, rows: Vec<nz_core::lists::Row>) -> Arc<ListHandle> {
    open_rows(screens.core(), rows)
}

#[uniffi::export(async_runtime = "tokio")]
impl Screens {
    // -- the artist page ------------------------------------------------------

    /// The artist page as this device last saw it — disk only, 0 requests. `None`
    /// the first time, which is the one moment the screen shows a skeleton.
    pub fn cached_artist(&self, id: String) -> Option<ArtistData> {
        self.core().cached_artist(&id).map(ArtistData::from)
    }

    /// `GET /api/artist/<id>` — ONE request on the `User` lane, kept for the next
    /// visit. `NzError::Offline` in zero sockets when the link is dead.
    pub async fn artist(&self, id: String) -> Result<ArtistData, NzError> {
        self.core()
            .artist(&id)
            .await
            .map(ArtistData::from)
            .map_err(NzError::from)
    }

    /// The releases this device last saw, both halves merged — disk only.
    pub fn cached_discography(&self, id: String) -> Option<DiscographyData> {
        self.core()
            .cached_discography(&id)
            .map(DiscographyData::from)
    }

    /// `GET /api/artist/<id>/discography` (`kind` other than `on-deezer`) or
    /// `/api/artist/<id>/on-deezer` (`kind` = `on-deezer`) — ONE request, and the
    /// answer carries every group the device knows.
    ///
    /// `cursor` is the paging token the contract names: the server sends whole
    /// groups today, so it is `None` and `next_cursor` stays `None` too; a paged
    /// discography lands without a second method.
    pub async fn discography(
        &self,
        id: String,
        kind: String,
        cursor: Option<String>,
    ) -> Result<DiscographyData, NzError> {
        self.core()
            .discography(&id, core::DiscographyKind::parse(&kind), cursor)
            .await
            .map(DiscographyData::from)
            .map_err(NzError::from)
    }

    /// Every title of this artist the library holds, as a windowed list — disk
    /// only; `None` before the « Titres » tab was ever opened.
    pub fn cached_artist_tracks(&self, id: String) -> Option<Arc<ListHandle>> {
        self.core()
            .cached_artist_tracks(&id)
            .map(|rows| handle(self, rows))
    }

    /// `GET /api/artist/<id>/tracks` — ONE request, asked lazily when the person
    /// opens the tab.
    pub async fn artist_tracks(&self, id: String) -> Result<Arc<ListHandle>, NzError> {
        self.core()
            .artist_tracks(&id)
            .await
            .map(|rows| handle(self, rows))
            .map_err(NzError::from)
    }

    /// `GET /api/radio/artist/<id>` — the « Radio » button. ONE request for the
    /// first titles; the player's own refill continues the station.
    pub async fn artist_radio(&self, id: String) -> Result<Vec<TrackRow>, NzError> {
        self.core()
            .artist_radio(&id)
            .await
            .map(rows)
            .map_err(NzError::from)
    }

    // -- the album page -------------------------------------------------------

    /// The album page as this device last saw it — disk only.
    pub fn cached_album(&self, id: String) -> Option<AlbumData> {
        self.core().cached_album(&id).map(|page| AlbumData {
            id: page.id,
            deezer_id: page.deezer_id,
            title: page.title,
            cover: page.cover,
            year: page.year,
            release_date: page.release_date,
            artist_name: page.artist_name,
            artist_id: page.artist_id,
            favorite: page.favorite,
            library: page.library,
            nb_tracks: page.nb_tracks,
            total_s: page.total_s,
            tracks: handle(self, page.tracks),
        })
    }

    /// `GET /api/album/<id>` — ONE request, kept for the next visit.
    pub async fn album(&self, id: String) -> Result<AlbumData, NzError> {
        let page = self.core().album(&id).await.map_err(NzError::from)?;
        Ok(AlbumData {
            id: page.id,
            deezer_id: page.deezer_id,
            title: page.title,
            cover: page.cover,
            year: page.year,
            release_date: page.release_date,
            artist_name: page.artist_name,
            artist_id: page.artist_id,
            favorite: page.favorite,
            library: page.library,
            nb_tracks: page.nb_tracks,
            total_s: page.total_s,
            tracks: handle(self, page.tracks),
        })
    }

    /// `GET /api/album/<id>/on-deezer` — ONE request, after the page is on screen
    /// and only for a library album. A provider that cannot be reached is an
    /// error the screen turns into its own sentence, never a failed album.
    pub async fn album_on_deezer(&self, id: String) -> Result<Vec<TrackRow>, NzError> {
        self.core()
            .album_on_deezer(&id)
            .await
            .map(rows)
            .map_err(NzError::from)
    }

    // -- credits --------------------------------------------------------------

    /// One track's credits as this device last saw them — disk only, which is
    /// what makes the sheet open instantly on a title already visited.
    pub fn cached_track_credits(&self, id: String) -> Option<TrackCreditsData> {
        self.core()
            .cached_track_credits(&id)
            .map(TrackCreditsData::from)
    }

    /// `GET /api/track/<id>/credits` — ONE request, what the full-screen player's
    /// panel and the track menu's sheet both read.
    pub async fn track_credits(&self, id: String) -> Result<TrackCreditsData, NzError> {
        self.core()
            .track_credits(&id)
            .await
            .map(TrackCreditsData::from)
            .map_err(NzError::from)
    }

    /// A person's credits page as this device last saw it — disk only.
    pub fn cached_person_credits(&self, name: String) -> Option<PersonCreditsData> {
        self.core()
            .cached_person_credits(&name)
            .map(|page| person_credits_data(self, page))
    }

    /// `GET /api/credits/person?name=` — ONE request, however many roles the page
    /// then offers.
    pub async fn person_credits(&self, name: String) -> Result<PersonCreditsData, NzError> {
        self.core()
            .person_credits(&name)
            .await
            .map(|page| person_credits_data(self, page))
            .map_err(NzError::from)
    }

    /// The titles one role shows on a page already read — ZERO requests (the
    /// SPA's chips filter the list they have). `None` when the page was never
    /// read; an empty role answers every title.
    pub fn person_credits_role(&self, name: String, role: String) -> Option<Arc<ListHandle>> {
        self.core()
            .person_credits_role(&name, &role)
            .map(|rows| handle(self, rows))
    }

    // -- the one write these pages do -----------------------------------------

    /// The heart of an artist or an album page: `POST /api/favorite/<kind>`, ONE
    /// request, never retried. The device's copy flips FIRST (a page reopened
    /// while the request flies shows the new state) and rolls back if the server
    /// refuses, which is the `Err` the screen turns into a toast and its own
    /// rollback.
    ///
    /// `title` (and, for an album, `artist`/`artist_id`) is
    /// `trackmenu.js#entityMeta`'s body: what makes the row when the star is the
    /// first thing that ever named the entity.
    pub async fn set_entity_favorite(
        &self,
        kind: String,
        id: String,
        on: bool,
        title: String,
        artist: Option<String>,
        artist_id: Option<String>,
    ) -> Result<(), NzError> {
        let meta = nz_core::core::Core::favorite_meta(
            &kind,
            &title,
            artist.as_deref(),
            artist_id.as_deref(),
        );
        self.core()
            .set_favorite(&kind, &id, on, meta)
            .await
            .map_err(NzError::from)
    }
}

/// A person's page, with its titles as a windowed handle.
fn person_credits_data(screens: &Screens, page: core::PersonCredits) -> PersonCreditsData {
    PersonCreditsData {
        name: page.name,
        roles: page
            .roles
            .into_iter()
            .map(|role| PersonRoleData {
                role: role.role,
                label: role.label,
                count: role.count,
            })
            .collect(),
        count: page.tracks.len() as u32,
        tracks: handle(screens, page.tracks),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_core::lists::Row;

    /// A page crosses screen-shaped: what it DRAWS, never the server's JSON, and
    /// a release's type and year stay raw (the French label is Kotlin's).
    #[test]
    fn an_artist_crosses_as_screen_shaped_records() {
        let data = core::ArtistData {
            id: "uuid-artist".to_owned(),
            name: "Ascenseur".to_owned(),
            picture: Some("/api/cover/uuid-artist".to_owned()),
            favorite: true,
            credited: 2,
            library: true,
            top: vec![Row::test_row(0, "Ouverture")],
            albums: vec![core::AlbumCard {
                id: "uuid-album".to_owned(),
                title: "Le matin".to_owned(),
                record_type: Some("single".to_owned()),
                year: Some("1998".to_owned()),
                ..core::AlbumCard::default()
            }],
            related: vec![core::ArtistCard {
                id: "uuid-june".to_owned(),
                name: "June".to_owned(),
                picture: None,
            }],
            ..core::ArtistData::default()
        };
        let crossed = ArtistData::from(data);
        assert_eq!(crossed.top.len(), 1);
        assert_eq!(crossed.top[0].title, "Ouverture");
        assert_eq!(crossed.top[0].position, 0);
        assert_eq!(crossed.albums[0].record_type.as_deref(), Some("single"));
        assert_eq!(crossed.albums[0].year.as_deref(), Some("1998"));
        assert_eq!(crossed.related[0].name, "June");
        assert!(crossed.favorite && crossed.library);
    }

    /// The shelf is computed by the core (five, newest first) and crosses as
    /// cards; a group the provider sends survives.
    #[test]
    fn a_discography_crosses_with_its_shelf_ready() {
        let card = |id: &str| core::AlbumCard {
            id: id.to_owned(),
            title: format!("Sortie {id}"),
            ..core::AlbumCard::default()
        };
        let groups = core::Discography {
            all: (0..7).map(|n| card(&n.to_string())).collect(),
            albums: vec![card("1")],
            singles: vec![card("2")],
            featured: vec![card("3")],
            on_deezer: vec![card("4")],
            on_deezer_loaded: true,
            ..core::Discography::default()
        };
        let crossed = DiscographyData::from(groups);
        assert_eq!(crossed.latest.len(), core::LATEST);
        assert_eq!(crossed.albums.len(), 1);
        assert_eq!(crossed.singles.len(), 1);
        assert_eq!(crossed.featured.len(), 1);
        assert_eq!(crossed.on_deezer.len(), 1);
        assert!(crossed.on_deezer_loaded);
        assert_eq!(crossed.next_cursor, None, "the server sends whole groups");
    }

    /// A credit's name says where it can go: an artist page when the library
    /// holds one, else the count that makes its own credits page a link.
    #[test]
    fn a_credit_says_where_its_name_opens() {
        let credits = core::TrackCredits {
            credits: vec![core::CreditGroup {
                role: "composer".to_owned(),
                label: "Compositeur".to_owned(),
                people: vec![
                    core::CreditPerson {
                        name: "Nina Simone".to_owned(),
                        artist_id: Some("uuid-nina".to_owned()),
                        credited: Some(4),
                    },
                    core::CreditPerson {
                        name: "Bob Power".to_owned(),
                        artist_id: None,
                        credited: Some(1),
                    },
                ],
            }],
            ..core::TrackCredits::default()
        };
        let crossed = TrackCreditsData::from(credits);
        assert_eq!(crossed.credits[0].label, "Compositeur");
        assert_eq!(
            crossed.credits[0].people[0].artist_id.as_deref(),
            Some("uuid-nina")
        );
        assert_eq!(
            crossed.credits[0].people[1].artist_id, None,
            "one title credited is not a page: the screen draws plain text"
        );
        assert!(!crossed.empty);
    }
}
