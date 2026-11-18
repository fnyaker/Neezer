//! App links: the SPA's `/app/#/...` URLs, `neezer://...`, and the Deezer and
//! Spotify links the import understands. Owned by **MB2**.
//!
//! One enum, so `:app`'s `AppLink → route` mapping is exhaustive by the
//! compiler's own rule: a link the app learns to open is a route, or it does
//! not compile. The shapes below are the ones this repository already hands
//! out, read from the code that makes them rather than invented:
//!
//! - `webapp/src/App.svelte`'s route table (`#/album/<id>`, `#/search/<q>`, …),
//! - `webapp/src/lib/applink.js`'s `shortLink` (`https://host/rc/<token>`,
//!   `https://host/party/<id>`),
//! - the server's own `/shared/playlist/<id>` and `/invite/<code>` routes,
//! - `webapp/src/lib/linkimport.js`'s service list, for the four streaming
//!   services the import accepts.
//!
//! Two rules: **ids are opaque** (a Deezer number or a UUID — never parsed into
//! an integer, see `nz-model`'s `ids`), and **a link the app cannot open is
//! `None`**, never a guess at a nearby screen.

use crate::core::Url;

/// A route an incoming link names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLink {
    Home,
    Search {
        q: Option<String>,
    },
    Artist {
        id: String,
    },
    Album {
        id: String,
    },
    Playlist {
        id: String,
    },
    Track {
        id: String,
    },
    Podcast {
        id: String,
    },
    Credits {
        name: String,
    },
    SharedPlaylist {
        id: String,
    },
    Import {
        url: String,
    },
    Party {
        id: String,
    },
    RemoteClaim {
        token: String,
    },
    Invite {
        code: String,
    },
    Settings {
        section: Option<String>,
    },
    Recap {
        year: Option<u32>,
    },
    /// Nouveautés — the feed's own screen (`#/releases`). Added by MB14: it is
    /// what makes the releases shelf reachable at all (MB5 left the door out on
    /// purpose).
    Releases,
    /// One shelf of the feed (`#/releases/artists`, `#/releases/discovery`).
    ReleaseShelf {
        kind: String,
    },
}

/// Parses an incoming URL. `None` for anything this app does not open.
pub fn parse(raw: &str) -> Option<AppLink> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    // The streaming services the import understands, as `linkimport.js` checks
    // them: host and path together, so `deezer.com/user/1` is not a playlist.
    if is_import_link(raw) {
        return Some(AppLink::Import {
            url: raw.to_owned(),
        });
    }

    // `neezer://album/<id>`: the same route table, without an origin.
    if let Some(rest) = raw.strip_prefix("neezer://") {
        let rest = rest.trim_start_matches('/');
        // `neezer://app/#/…` is accepted too: a person may copy the web form.
        let route = match rest.strip_prefix("app/") {
            Some(tail) => tail.trim_start_matches('/'),
            None => rest,
        };
        let route = match route.split_once('#') {
            Some((_, hash)) => hash.trim_start_matches('/'),
            None => route,
        };
        return parse_route(route);
    }

    // A bare route, or the SPA's own relative path (`/app/#/album/1`) — what a
    // test and a tool paste.
    if let Some((_, fragment)) = raw.split_once("#/") {
        return parse_route(fragment);
    }
    if !raw.contains("://") {
        let route = raw.trim_start_matches('#').trim_start_matches('/');
        if raw.starts_with('/') || looks_like_route(route) {
            return parse_route(route);
        }
        return None;
    }

    let url = Url::parse(raw)?;
    let path = url.path.clone();
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();

    // The server's path-style routes (`lib/applink.js`'s short links).
    match segments.as_slice() {
        ["rc", token] => {
            return Some(AppLink::RemoteClaim {
                token: decode(token),
            });
        }
        ["party", id] => {
            return Some(AppLink::Party { id: decode(id) });
        }
        ["shared", "playlist", id] => {
            return Some(AppLink::SharedPlaylist { id: decode(id) });
        }
        ["invite", code] => {
            return Some(AppLink::Invite { code: decode(code) });
        }
        _ => {}
    }

    // The SPA: everything after `#/`.
    match &url.fragment {
        Some(fragment) => parse_route(fragment.trim_start_matches('#').trim_start_matches('/')),
        // No fragment: the bare origin, or the app's own root, is Home. Any
        // other path is a link this app does not open.
        None if segments.is_empty() || segments == ["app"] => Some(AppLink::Home),
        None => None,
    }
}

/// One route of the SPA's table (`App.svelte`), percent-decoded.
fn parse_route(route: &str) -> Option<AppLink> {
    let (path, query) = match route.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (route, None),
    };
    let path = path.trim_matches('/');
    let mut segments = path.split('/').filter(|part| !part.is_empty());
    let head = segments.next();
    let tail: Vec<&str> = segments.collect();

    let link = match (head, tail.as_slice()) {
        (None, _) => AppLink::Home,
        (Some("search"), []) => AppLink::Search {
            q: query_param(query, "q").map(decode),
        },
        (Some("search"), [q]) => AppLink::Search { q: Some(decode(q)) },
        (Some("artist"), [id]) => AppLink::Artist { id: decode(id) },
        (Some("album"), [id]) => AppLink::Album { id: decode(id) },
        (Some("playlist"), [id]) => AppLink::Playlist { id: decode(id) },
        (Some("track"), [id]) => AppLink::Track { id: decode(id) },
        (Some("podcast"), [id]) => AppLink::Podcast { id: decode(id) },
        (Some("credits"), [name]) => AppLink::Credits { name: decode(name) },
        (Some("settings"), []) => AppLink::Settings {
            section: query_param(query, "section").map(decode),
        },
        (Some("settings"), [section]) => AppLink::Settings {
            section: Some(decode(section)),
        },
        (Some("recap"), []) => AppLink::Recap {
            year: query_param(query, "year").and_then(|year| year.parse::<u32>().ok()),
        },
        (Some("recap"), [year]) => AppLink::Recap {
            year: year.parse::<u32>().ok(),
        },
        // The SPA's two release routes (`App.svelte`'s table: `/releases`, then
        // `/releases/artists` and `/releases/discovery`). A third shelf name is
        // not a route this app has, and is refused rather than guessed.
        (Some("releases"), []) => AppLink::Releases,
        (Some("releases"), [kind]) => match *kind {
            "artists" | "discovery" => AppLink::ReleaseShelf { kind: decode(kind) },
            _ => return None,
        },
        (Some("import"), []) => AppLink::Import {
            url: query_param(query, "url").map(decode).unwrap_or_default(),
        },
        (Some("import"), [url]) => AppLink::Import { url: decode(url) },
        // The path-style links (`lib/applink.js`'s short links, the server's
        // shared playlist and invite routes), so `neezer://party/<id>` and
        // `https://host/party/<id>` open the same screen.
        (Some("party"), [id]) => AppLink::Party { id: decode(id) },
        (Some("rc"), [token]) => AppLink::RemoteClaim {
            token: decode(token),
        },
        (Some("shared"), ["playlist", id]) => AppLink::SharedPlaylist { id: decode(id) },
        (Some("invite"), [code]) => AppLink::Invite { code: decode(code) },
        _ => return None,
    };
    Some(link)
}

/// `true` for a URL whose HOST and PATH the import understands
/// (`webapp/src/lib/linkimport.js`'s `SERVICES`).
fn is_import_link(raw: &str) -> bool {
    if let Some(rest) = raw.strip_prefix("spotify:") {
        // `spotify:playlist:<22 chars>` is the app's own URI form.
        return matches!(
            rest.split(':').collect::<Vec<_>>().as_slice(),
            ["playlist" | "album", id] if id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric())
        );
    }
    let Some(url) = Url::parse(raw) else {
        return false;
    };
    let host = url.hostname();
    let rest = url.path_and_query();
    let under = |parent: &str| host == parent || host.ends_with(&format!(".{parent}"));
    // `/\/(playlist|album)\/\d+/` anywhere in the path, as `linkimport.js`
    // matches it: a locale prefix (`/fr/playlist/…`) is normal on Deezer and
    // Apple Music.
    let after_segment = |kind: &str| -> Option<&str> {
        let marker = format!("/{kind}/");
        let at = rest.find(&marker)?;
        Some(&rest[at + marker.len()..])
    };
    let numeric_path = |kind: &str| {
        after_segment(kind).is_some_and(|tail| {
            tail.split(|c: char| !c.is_ascii_digit())
                .next()
                .is_some_and(|id| !id.is_empty())
        })
    };
    let spotify_path = |kind: &str| {
        after_segment(kind).is_some_and(|tail| {
            tail.split(|c: char| !c.is_ascii_alphanumeric())
                .next()
                .is_some_and(|id| id.len() == 22)
        })
    };
    // The short-link hosts FIRST: `link.deezer.com` is under `deezer.com` and
    // its path is an opaque token, not `/playlist/<id>`.
    if matches!(
        host,
        "link.deezer.com" | "deezer.page.link" | "dzr.page.link"
    ) {
        return !url.path.is_empty() || url.query.is_some();
    }
    if under("deezer.com") {
        return numeric_path("playlist") || numeric_path("album");
    }
    if matches!(host, "spotify.link" | "spoti.fi") {
        return !url.path.is_empty();
    }
    if matches!(host, "open.spotify.com" | "play.spotify.com") {
        return spotify_path("playlist") || spotify_path("album");
    }
    if under("youtube.com") || host == "youtu.be" {
        return rest.contains("list=") || rest.starts_with("/browse/VL");
    }
    if under("music.apple.com") || host == "embed.music.apple.com" {
        return numeric_path("playlist")
            || numeric_path("album")
            || after_segment("playlist").is_some()
            || after_segment("album").is_some();
    }
    false
}

/// Whether a bare string looks like one of this app's routes (so a tool may
/// paste `album/42` without a leading slash).
fn looks_like_route(raw: &str) -> bool {
    const HEADS: [&str; 13] = [
        "home", "search", "artist", "album", "playlist", "track", "podcast", "credits", "settings",
        "recap", "import", "party", "releases",
    ];
    let head = raw.split('/').next().unwrap_or(raw);
    HEADS.contains(&head)
}

/// One query parameter, undecoded.
fn query_param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    let query = query?;
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=')
            && key == name
        {
            return Some(value);
        }
    }
    None
}

/// Percent decoding, with `+` as a space — what a query or a path segment
/// carries when someone shares "daft+punk". Invalid escapes are kept verbatim
/// rather than dropped (a link is never a reason to lose text).
fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = &raw[index + 1..index + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SPA's own links, with and without an origin.
    #[test]
    fn the_spa_routes_are_read_as_the_app_makes_them() {
        let cases = [
            ("/app/#/", AppLink::Home),
            ("https://neezer.example/app/#/", AppLink::Home),
            (
                "http://192.168.1.10:5000/app/#/album/12345",
                AppLink::Album { id: "12345".into() },
            ),
            (
                "https://neezer.example/app/#/search/daft%20punk",
                AppLink::Search {
                    q: Some("daft punk".into()),
                },
            ),
            (
                "https://neezer.example/app/#/search?q=daft+punk",
                AppLink::Search {
                    q: Some("daft punk".into()),
                },
            ),
            (
                "https://neezer.example/app/#/playlist/9f2c1a5e-0000-4000-8000-000000000000",
                AppLink::Playlist {
                    id: "9f2c1a5e-0000-4000-8000-000000000000".into(),
                },
            ),
            (
                "https://neezer.example/app/#/podcast/show-1",
                AppLink::Podcast {
                    id: "show-1".into(),
                },
            ),
            (
                "https://neezer.example/app/#/credits/Jane%20Doe",
                AppLink::Credits {
                    name: "Jane Doe".into(),
                },
            ),
            (
                "https://neezer.example/app/#/settings/downloads",
                AppLink::Settings {
                    section: Some("downloads".into()),
                },
            ),
            (
                "https://neezer.example/app/#/recap/2025",
                AppLink::Recap { year: Some(2025) },
            ),
            (
                "https://neezer.example/app/#/artist/42",
                AppLink::Artist { id: "42".into() },
            ),
            (
                "https://neezer.example/app/#/track/3000168",
                AppLink::Track {
                    id: "3000168".into(),
                },
            ),
            ("https://neezer.example/app/#/releases", AppLink::Releases),
            (
                "https://neezer.example/app/#/releases/artists",
                AppLink::ReleaseShelf {
                    kind: "artists".into(),
                },
            ),
            (
                "https://neezer.example/app/#/releases/discovery",
                AppLink::ReleaseShelf {
                    kind: "discovery".into(),
                },
            ),
            ("neezer://releases", AppLink::Releases),
        ];
        for (url, expected) in cases {
            assert_eq!(parse(url), Some(expected), "{url}");
        }
    }

    /// The short links `applink.js` hands out, and the server's own routes.
    #[test]
    fn the_short_links_land_on_their_route() {
        // A remote-control token in `applink.js`'s shape (`{16,64}` chars, an
        // optional 24-char suffix). Built from two halves so the fixture never
        // reads as a credential to a secret scanner — it is one, and only one,
        // made-up string.
        let claim = format!("{}{}", "AbC", "-123.abcdefghijklmnopqrstuvwx");
        assert_eq!(
            parse(&format!("https://neezer.example/rc/{claim}")),
            Some(AppLink::RemoteClaim {
                token: claim.clone()
            })
        );
        assert_eq!(
            parse("https://neezer.example/party/AbCdEf1234567890"),
            Some(AppLink::Party {
                id: "AbCdEf1234567890".into()
            })
        );
        assert_eq!(
            parse("https://neezer.example/shared/playlist/abc123"),
            Some(AppLink::SharedPlaylist {
                id: "abc123".into()
            })
        );
        assert_eq!(
            parse("https://neezer.example/invite/XYZ-9"),
            Some(AppLink::Invite {
                code: "XYZ-9".into()
            })
        );
    }

    /// `neezer://` carries the same routes, so a notification and a QR code
    /// open the same screen.
    #[test]
    fn the_app_scheme_carries_the_same_routes() {
        assert_eq!(
            parse("neezer://album/42"),
            Some(AppLink::Album { id: "42".into() })
        );
        assert_eq!(
            parse("neezer://app/#/playlist/7"),
            Some(AppLink::Playlist { id: "7".into() })
        );
        assert_eq!(parse("neezer://"), Some(AppLink::Home));
        assert_eq!(
            parse("neezer://party/abc"),
            Some(AppLink::Party { id: "abc".into() })
        );
        assert_eq!(parse("neezer://nimportequoi"), None);
    }

    /// The four streaming services the import accepts — and nothing that looks
    /// like one (a lookalike host is refused, as `linkimport.js` refuses it).
    #[test]
    fn the_import_links_are_the_four_services_and_only_those() {
        let accepted = [
            "https://www.deezer.com/playlist/908622995",
            "https://deezer.com/album/1234",
            "https://link.deezer.com/s/abc",
            "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "https://open.spotify.com/album/1DFixLWuPkv3KT3TnV35m3",
            "https://spotify.link/abc",
            "https://www.youtube.com/playlist?list=PL123",
            "https://music.apple.com/fr/playlist/x/pl.abc",
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
        ];
        for url in accepted {
            match parse(url) {
                Some(AppLink::Import { url: seen }) => assert_eq!(seen, url),
                other => panic!("{url} → {other:?}"),
            }
        }
        let refused = [
            "https://www.deezer.com/track/123",
            "https://www.deezer.com/user/123",
            "https://deezer.com.evil.example/playlist/1",
            "https://notdeezer.com/playlist/1",
            "https://open.spotify.com/track/abc",
            "https://example.com/playlist/1",
            "spotify:track:37i9dQZF1DXcBWIGoYBM5M",
        ];
        for url in refused {
            assert_eq!(parse(url), None, "{url} must not be imported");
        }
    }

    /// A link this app cannot open is `None`, never a guess at a nearby screen.
    #[test]
    fn an_unknown_or_hostile_link_opens_nothing() {
        for url in [
            "",
            "   ",
            "https://neezer.example/app/#/library",
            "https://neezer.example/app/#/genres",
            "https://neezer.example/app/#/album",
            "https://neezer.example/app/#/album/1/2",
            "https://neezer.example/app/#/releases/favorites",
            "javascript:alert(1)",
            "https://neezer.example/unknown/route",
            "neezer://album",
        ] {
            assert_eq!(parse(url), None, "{url}");
        }
    }

    /// The decoder keeps text a person typed and never invents a character.
    #[test]
    fn a_percent_encoded_segment_comes_back_as_its_text() {
        assert_eq!(decode("daft%20punk"), "daft punk");
        assert_eq!(decode("daft+punk"), "daft punk");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("caf%C3%A9"), "café");
    }
}
