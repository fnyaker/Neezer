//! The wire of the entity area: the nine reads the SPA opens an artist, an album
//! or a credit page with, and the conversions into the screen-shaped records.
//!
//! | This file | The SPA |
//! |---|---|
//! | [`fetch_artist`] | `routes/Artist.svelte#load` → `api.artist()` |
//! | [`fetch_discography`] | `#load` → `api.discography()` |
//! | [`fetch_artist_on_deezer`] | `#loadOnDeezer` |
//! | [`fetch_artist_tracks`] | `#loadTracks` (the « Titres » tab, lazily) |
//! | [`fetch_artist_radio`] | `#startRadio` |
//! | [`fetch_album`] | `routes/Album.svelte#load` → `api.album()` |
//! | [`fetch_album_on_deezer`] | `#load` → `api.albumOnDeezer()` |
//! | [`fetch_track_credits`] | `components/CreditsSheet.svelte#load` |
//! | [`fetch_person_credits`] | `components/PersonCredits.svelte#load` |
//! | [`favorite`] | `lib/actions.js#toggleEntityFavorite` |
//!
//! Every call is `Lane::User` work: it is the screen the person just opened, or
//! the button they just pressed (rule 1). The bodies are read as BYTES and parsed
//! HERE rather than through a typed endpoint, for one reason: the raw text is
//! also what the device keeps ([`super::store`]), so a cached page and a fresh one
//! go through exactly the same conversion — a cache hit cannot drift from a live
//! answer.
//!
//! No rule lives in this file: fonts, labels and link choices are the screen's.

use serde::de::DeserializeOwned;

use nz_api::{Api, Lane};
use nz_model::{Album, AlbumList, Artist, Track, TrackList};

use crate::core::CoreError;
use crate::lists::Row;

/// `GET` one path as text, on the `User` lane. The request and its body bytes are
/// counted by `nz-api`'s own per-tag meter (SC3), so `NzCore::stats()` answers
/// what this screen cost.
pub(super) async fn fetch_text(api: &Api, path: &str) -> Result<String, CoreError> {
    let bytes = api.bytes(Lane::User, path).await?;
    String::from_utf8(bytes).map_err(|error| CoreError::Internal {
        detail: format!("{path} : {error}"),
    })
}

/// A body, parsed. A refusal to parse is an internal fault with the path in it —
/// never a silent empty screen.
fn parse<T: DeserializeOwned>(path: &str, body: &str) -> Result<T, CoreError> {
    serde_json::from_str(body).map_err(|error| CoreError::Internal {
        detail: format!("{path} : {error}"),
    })
}

/// A route argument, percent-encoded the way `encodeURIComponent` does it.
///
/// Every id here is a UUID or a provider number today, and a NAME reaches the
/// person-credits route through its own `?name=` (already encoded by
/// [`Api::person_credits_path`]). Encoding anyway is what makes a hostile route
/// argument (a `/`, a `?`, a `#`) one path segment instead of another route.
pub(super) fn seg(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `GET /api/artist/<id>` — the page's body, kept by the caller.
pub(super) async fn fetch_artist(api: &Api, id: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/artist/{}", seg(id))).await
}

/// `GET /api/artist/<id>/discography` — every release type in one answer.
pub(super) async fn fetch_discography(api: &Api, id: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/artist/{}/discography", seg(id))).await
}

/// `GET /api/artist/<id>/on-deezer` — what the provider has and the library does
/// not. Asked only for a library artist, after the page is on screen.
pub(super) async fn fetch_artist_on_deezer(api: &Api, id: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/artist/{}/on-deezer", seg(id))).await
}

/// `GET /api/artist/<id>/tracks` — everything of this artist in the library.
pub(super) async fn fetch_artist_tracks(api: &Api, id: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/artist/{}/tracks", seg(id))).await
}

/// `GET /api/radio/artist/<id>` — one station, seeded on the artist.
pub(super) async fn fetch_artist_radio(api: &Api, id: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/radio/artist/{}", seg(id))).await
}

/// `GET /api/album/<id>` — the album, its tracks in disc/track order.
pub(super) async fn fetch_album(api: &Api, id: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/album/{}", seg(id))).await
}

/// `GET /api/album/<id>/on-deezer` — the rest of the tracklist, from the provider.
pub(super) async fn fetch_album_on_deezer(api: &Api, id: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/album/{}/on-deezer", seg(id))).await
}

/// `GET /api/track/<id>/credits` — who wrote, composed, produced it.
pub(super) async fn fetch_track_credits(api: &Api, id: &str) -> Result<String, CoreError> {
    fetch_text(api, &format!("/track/{}/credits", seg(id))).await
}

/// `GET /api/credits/person?name=` — everything one person worked on.
pub(super) async fn fetch_person_credits(api: &Api, name: &str) -> Result<String, CoreError> {
    fetch_text(api, &Api::person_credits_path(name)).await
}

/// `POST /api/favorite/<kind>` — the heart of the artist and album pages. `meta`
/// is `trackmenu.js#entityMeta`'s body, which is what makes the row when the star
/// is the first thing that ever named the entity.
pub(super) async fn favorite(
    api: &Api,
    kind: &str,
    reference: &str,
    on: bool,
    meta: &serde_json::Value,
) -> Result<(), CoreError> {
    api.favorite_entity(kind, reference, on, meta).await?;
    Ok(())
}

// --- the bodies, as the screens draw them ------------------------------------

/// The artist page, assembled. `id` is what a link names the artist by
/// (`lib/entity.js#entityRef`): the row's UUID when there is one, else the
/// provider's id.
pub(super) fn artist(id: &str, body: &str) -> Result<super::ArtistData, CoreError> {
    let page: nz_model::ArtistPage = parse(&format!("/artist/{id}"), body)?;
    Ok(super::ArtistData {
        id: entity_id(page.artist.id.as_deref(), page.artist.deezer_id.as_str()),
        deezer_id: page.artist.deezer_id.to_string(),
        name: page.artist.name,
        picture: page.artist.picture,
        fans: page.artist.nb_fan,
        favorite: page.artist.is_favorite.unwrap_or(false),
        credited: page.credited,
        library: page.library,
        bio: page.bio.filter(|bio| !bio.trim().is_empty()),
        top: rows(&page.top),
        albums: page.albums.iter().map(album_card).collect(),
        related: page.related.iter().map(artist_card).collect(),
    })
}

/// The discography, grouped the way the page's shelves are. A group the answer
/// does not carry stays empty — the SPA's `disco?.single` is `[]` too.
pub(super) fn discography(id: &str, body: &str) -> Result<super::Discography, CoreError> {
    // The body is `{"discography": {group: [...]}}`; deserialising into a map
    // rather than `nz_model::Discography` keeps a group the shared model does not
    // name (`featured`, which the provider path sends) instead of dropping it.
    #[derive(serde::Deserialize)]
    struct Wire {
        #[serde(default)]
        discography: std::collections::BTreeMap<String, Vec<Album>>,
    }
    let wire: Wire = parse(&format!("/artist/{id}/discography"), body)?;
    let group = |name: &str| -> Vec<super::AlbumCard> {
        wire.discography
            .get(name)
            .map(|albums| albums.iter().map(album_card).collect())
            .unwrap_or_default()
    };
    // `nz-model`'s four groups keep their meaning; `all` falls back to the album
    // group when an older server sent only the latter.
    let albums = group("album");
    let all = {
        let all = group("all");
        if all.is_empty() { albums.clone() } else { all }
    };
    Ok(super::Discography {
        all,
        albums,
        singles: group("single"),
        eps: group("ep"),
        featured: group("featured"),
        ..super::Discography::default()
    })
}

/// `{"albums": [...]}` — the provider's part of an artist page.
pub(super) fn album_cards(body: &str) -> Result<Vec<super::AlbumCard>, CoreError> {
    let wire: AlbumList = parse("/artist/<id>/on-deezer", body)?;
    Ok(wire.albums.iter().map(album_card).collect())
}

/// `{"tracks": [...]}` — a list of titles, as a screen row list.
pub(super) fn track_rows(body: &str) -> Result<Vec<Row>, CoreError> {
    let wire: TrackList = parse("/tracks", body)?;
    Ok(rows(&wire.tracks))
}

/// The album page: its header, its tracks, and whether the library holds it.
pub(super) fn album(id: &str, body: &str) -> Result<super::AlbumData, CoreError> {
    let page: nz_model::AlbumPage = parse(&format!("/album/{id}"), body)?;
    // `nz_model::TrackArtist` keeps the provider id and the name; a LIBRARY
    // album's artist also carries its row UUID, and `entityRef` prefers that id
    // (an upload's artist has no provider id at all, so the UUID is the only
    // thing that makes its page reachable). One extra parse of a small body.
    #[derive(Default, serde::Deserialize)]
    struct ArtistId {
        #[serde(default)]
        id: Option<String>,
    }
    #[derive(Default, serde::Deserialize)]
    struct AlbumArtist {
        #[serde(default)]
        artist: ArtistId,
    }
    #[derive(Default, serde::Deserialize)]
    struct AlbumArtistPage {
        #[serde(default)]
        album: AlbumArtist,
    }
    let row_id = parse::<AlbumArtistPage>(&format!("/album/{id}"), body)
        .ok()
        .and_then(|page| page.album.artist.id);
    let tracks = rows(&page.tracks);
    let total_s = tracks.iter().map(|row| row.duration_s).sum();
    Ok(super::AlbumData {
        id: entity_id(
            page.album.id.as_ref().map(|id| id.to_string()).as_deref(),
            page.album.deezer_id.as_str(),
        ),
        deezer_id: page.album.deezer_id.to_string(),
        title: page.album.title,
        cover: page.album.cover,
        year: page.album.year,
        release_date: page.album.release_date,
        artist_name: page.album.artist.name,
        artist_id: entity_id(row_id.as_deref(), page.album.artist.deezer_id.as_str()),
        favorite: page.album.is_favorite.unwrap_or(false),
        library: page.library,
        nb_tracks: tracks.len() as u32,
        total_s,
        tracks,
    })
}

/// One track's credits, with the release line the panel prints under them.
pub(super) fn track_credits(id: &str, body: &str) -> Result<super::TrackCredits, CoreError> {
    let wire: nz_model::TrackCredits = parse(&format!("/track/{id}/credits"), body)?;
    let text = |key: &str| -> Option<String> {
        wire.info
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let credits: Vec<super::CreditGroup> = wire
        .credits
        .into_iter()
        .map(|group| super::CreditGroup {
            role: group.role,
            label: group.label,
            people: group
                .people
                .into_iter()
                .map(|person| super::CreditPerson {
                    artist_id: person.id.filter(|id| !id.is_empty()).or_else(|| {
                        person
                            .deezer_id
                            .as_ref()
                            .map(|id| id.to_string())
                            .filter(|id| !id.is_empty())
                    }),
                    name: person.name,
                    credited: person.credited,
                })
                .collect(),
        })
        .filter(|group| !group.people.is_empty())
        .collect();
    let info = super::CreditsInfo {
        isrc: text("isrc"),
        released: text("released"),
        label: text("label"),
        copyright: text("copyright"),
        album: text("album"),
    };
    Ok(super::TrackCredits {
        empty: credits.is_empty() && info.is_empty(),
        credits,
        info,
    })
}

/// A person's credits page: the roles they appear in, and the titles.
pub(super) fn person_credits(name: &str, body: &str) -> Result<super::PersonCredits, CoreError> {
    let wire: nz_model::PersonCredits = parse("/credits/person", body)?;
    let mut roles_by_key = std::collections::BTreeMap::new();
    let mut tracks = Vec::with_capacity(wire.tracks.len());
    for (position, track) in wire.tracks.iter().enumerate() {
        let row = row(track, position as u32);
        if !track.credit_roles.is_empty() {
            roles_by_key.insert(row.key.clone(), track.credit_roles.clone());
        }
        tracks.push(row);
    }
    Ok(super::PersonCredits {
        // The server answers the name the credits themselves spell ("NINA
        // SIMONE" for a page opened as "Nina Simone"); an empty one is not a name.
        name: if wire.name.trim().is_empty() {
            name.to_owned()
        } else {
            wire.name
        },
        roles: wire
            .roles
            .into_iter()
            .map(|role| super::PersonRole {
                role: role.role,
                label: role.label,
                count: role.count,
            })
            .collect(),
        roles_by_key,
        tracks,
    })
}

/// `entityRef(x)`: the row's id when it has one, else the provider's.
fn entity_id(row_id: Option<&str>, deezer_id: &str) -> String {
    match row_id.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => id.to_owned(),
        None => deezer_id.to_owned(),
    }
}

/// One release, as a shelf or a grid card draws it.
pub(super) fn album_card(album: &Album) -> super::AlbumCard {
    let deezer = album.deezer_id.to_string();
    super::AlbumCard {
        id: entity_id(
            album.id.as_ref().map(|id| id.to_string()).as_deref(),
            &deezer,
        ),
        deezer_id: deezer,
        title: album.title.clone(),
        cover: album.cover.clone(),
        year: album.year.clone(),
        release_date: album.release_date.clone(),
        record_type: album.record_type.clone(),
        nb_tracks: album.nb_tracks,
        artist_name: album.artist.name.clone(),
        artist_id: entity_id(None, album.artist.deezer_id.as_str()),
    }
}

/// One related artist, as the portrait shelf draws it.
pub(super) fn artist_card(artist: &Artist) -> super::ArtistCard {
    let deezer = artist.deezer_id.to_string();
    super::ArtistCard {
        id: entity_id(artist.id.as_deref(), &deezer),
        name: artist.name.clone(),
        picture: artist.picture.clone(),
    }
}

/// Every track of a list, as the rows a screen draws and queues.
pub(super) fn rows(tracks: &[Track]) -> Vec<Row> {
    tracks
        .iter()
        .enumerate()
        .map(|(position, track)| row(track, position as u32))
        .collect()
}

/// ONE track, as the shared row. The key is the queue's own
/// (`trackkey.js#trackRef`): the row's UUID when it has one, else the provider's
/// id — the same value a tap from a list and a queue entry agree on.
pub(super) fn row(track: &Track, position: u32) -> Row {
    let deezer = track.deezer_id.to_string();
    let row_id = track
        .id
        .as_ref()
        .map(|id| id.to_string())
        .filter(|id| !id.is_empty());
    let album_id = track.album.deezer_id.to_string();
    let source = match track.source.as_deref() {
        Some("youtube") => "youtube",
        _ if track.local => "upload",
        _ => "deezer",
    };
    Row {
        key: row_id.clone().unwrap_or_else(|| deezer.clone()),
        id: row_id.unwrap_or(deezer),
        title: track.title.clone(),
        artists: artist_line(track),
        album: track.album.title.clone(),
        album_id: (!album_id.is_empty()).then_some(album_id),
        duration_s: track.duration,
        cover: track.album.cover.clone(),
        explicit: track.explicit,
        // A like is MB10's state and reaches the row through `update_rows`; a
        // page never invents one.
        liked: false,
        disliked: false,
        unavailable: track.unavailable,
        downloaded: false,
        source: source.to_owned(),
        position,
        played_at: 0,
        added_at: 0,
    }
}

/// `format.js#artistLine`: the server's own line when it sent one, else the
/// credited names joined — the same sentence every list shows.
fn artist_line(track: &Track) -> String {
    if let Some(line) = track
        .display_artist
        .as_deref()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        return line.to_owned();
    }
    let names: Vec<&str> = track
        .artists
        .iter()
        .map(|artist| artist.name.trim())
        .filter(|name| !name.is_empty())
        .collect();
    if !names.is_empty() {
        return names.join(", ");
    }
    track.artist.name.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A route argument is one segment, whatever it holds.
    #[test]
    fn a_hostile_id_is_one_path_segment() {
        assert_eq!(
            seg("11111111-1111-4111-8111-111111111111"),
            "11111111-1111-4111-8111-111111111111"
        );
        assert_eq!(seg("42"), "42");
        assert_eq!(seg("a/b?c#d e"), "a%2Fb%3Fc%23d%20e");
        assert_eq!(
            seg(".."),
            "..",
            "a dot is unreserved: it cannot leave the route"
        );
        assert!(!seg("../x").contains('/'));
    }

    /// The page's id is the row's UUID when there is one, else the provider's —
    /// what `entityRef` answers, and therefore what a link names.
    #[test]
    fn an_entity_is_named_by_its_row_then_its_provider_id() {
        assert_eq!(entity_id(Some("uuid-1"), "42"), "uuid-1");
        assert_eq!(entity_id(Some(""), "42"), "42");
        assert_eq!(entity_id(None, "42"), "42");
    }

    /// A catalogue title's key is its provider id; a library title's is its UUID
    /// — the same key a queue entry and a list row agree on.
    #[test]
    fn a_track_becomes_the_shared_row() {
        let mut track = Track {
            deezer_id: nz_model::TrackId::new("3000168"),
            title: "Été".to_owned(),
            duration: 180,
            ..Track::default()
        };
        track.album.title = "Le matin".to_owned();
        track.album.deezer_id = nz_model::AlbumId::new("501");
        track.album.cover = Some("/api/cover/501".to_owned());
        track.artists = vec![
            nz_model::TrackArtistRole {
                name: "Ascenseur".to_owned(),
                role: "Main".to_owned(),
                ..Default::default()
            },
            nz_model::TrackArtistRole {
                name: "June".to_owned(),
                role: "Featured".to_owned(),
                ..Default::default()
            },
        ];
        let first = row(&track, 3);
        assert_eq!(first.key, "3000168");
        assert_eq!(first.id, "3000168");
        assert_eq!(first.artists, "Ascenseur, June");
        assert_eq!(first.album, "Le matin");
        assert_eq!(first.album_id.as_deref(), Some("501"));
        assert_eq!(first.cover.as_deref(), Some("/api/cover/501"));
        assert_eq!(first.position, 3);
        assert_eq!(first.source, "deezer");
        assert!(!first.liked && !first.downloaded, "a page invents no state");

        let mut library = track.clone();
        library.id = Some(nz_model::TrackRowId::new(
            "11111111-1111-4111-8111-111111111111",
        ));
        assert_eq!(row(&library, 0).key, "11111111-1111-4111-8111-111111111111");

        let mut upload = track;
        upload.local = true;
        assert_eq!(row(&upload, 0).source, "upload");
    }

    /// The discography keeps a group the shared model does not name, and falls
    /// back to `album` when an older server sent no `all`.
    #[test]
    fn the_discography_keeps_every_group_it_received() {
        let body = r#"{"discography":{
            "album":[{"deezer_id":"501","title":"Le matin","year":"2024","record_type":"album"}],
            "single":[{"deezer_id":"502","title":"Un titre","record_type":"single"}],
            "featured":[{"deezer_id":"503","title":"Chez June","record_type":"album"}]}}"#;
        let groups = discography("42", body).expect("a discography");
        assert_eq!(groups.albums.len(), 1);
        assert_eq!(
            groups.all, groups.albums,
            "no `all`: the albums are the shelf"
        );
        assert_eq!(groups.singles.len(), 1);
        assert_eq!(
            groups.featured.len(),
            1,
            "the provider's own group survives"
        );

        let older = r#"{"discography":{}}"#;
        let empty = discography("42", older).expect("an empty discography");
        assert!(empty.all.is_empty() && empty.albums.is_empty());
    }

    /// A track's credits: the release line is typed, the roles keep their French
    /// labels, and an empty answer says so instead of drawing four empty rows.
    #[test]
    fn credits_become_roles_and_one_release_line() {
        let body = r#"{"credits":[
            {"role":"composer","label":"Compositeur","people":[{"name":"Nina","id":"uuid-9","credited":3}]},
            {"role":"producer","label":"Producteur","people":[{"name":"Bob"}]}],
          "info":{"isrc":"FR123","released":"2001-03-12","label":"Blue Note","album":"Le matin"}}"#;
        let credits = track_credits("3000168", body).expect("credits");
        assert!(!credits.empty);
        assert_eq!(credits.credits[0].label, "Compositeur");
        assert_eq!(
            credits.credits[0].people[0].artist_id.as_deref(),
            Some("uuid-9")
        );
        assert_eq!(credits.credits[0].people[0].credited, Some(3));
        assert_eq!(credits.credits[1].people[0].artist_id, None, "plain text");
        assert_eq!(credits.info.isrc.as_deref(), Some("FR123"));
        assert_eq!(credits.info.released.as_deref(), Some("2001-03-12"));

        let empty: nz_model::TrackCredits = nz_model::TrackCredits::default();
        let body = serde_json::to_string(&empty).unwrap();
        assert!(track_credits("1", &body).expect("empty").empty);
    }

    /// A person's page keeps each title's roles, so a chip filters locally.
    #[test]
    fn a_person_page_keeps_the_roles_of_every_title() {
        let body = r#"{"name":"NINA SIMONE","roles":[{"role":"composer","label":"Compositeur","count":1}],
          "labels":{"composer":"Compositeur"},
          "tracks":[
            {"id":"11111111-1111-4111-8111-111111111111","deezer_id":"9001","title":"Sinnerman",
             "duration":600,"credit_roles":["composer","author"],
             "artist":{"deezer_id":"42","name":"Nina Simone"},
             "album":{"deezer_id":"501","title":"Pastel Blues"}}]}"#;
        let page = person_credits("Nina Simone", body).expect("a page");
        assert_eq!(page.name, "NINA SIMONE", "the name the credits spell");
        assert_eq!(page.roles.len(), 1);
        assert_eq!(page.tracks.len(), 1);
        let key = page.tracks[0].key.clone();
        assert_eq!(
            page.roles_by_key.get(&key).map(Vec::len),
            Some(2),
            "both roles are kept for the chip filter"
        );
        assert_eq!(page.tracks[0].album, "Pastel Blues");
    }
}
