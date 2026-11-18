//! `webapp/src/lib/linkimport.js#playlistLink`, ported: the search box is where
//! people paste a shared playlist link, and a link is an IMPORT offer, not a
//! query. Recognising it costs no request and stops the screen from spending two
//! on a search that cannot answer.
//!
//! Only RECOGNISES it: the server parses the link again and is the only judge of
//! what it reads (`neezer/linkimport.py`). Nothing here decides that a playlist
//! exists, and nothing here fetches it — MB9's import screen does.
//!
//! The service table is the SPA's, one row per host the app offers to import
//! from, with the same path shapes: a bare `open.spotify.com` home page is not a
//! playlist, and a `youtube.com/watch` without a list is not one either.

/// One recognised link: what the person pasted, which service it belongs to, and
/// the name the offer shows ("Importer cette playlist Deezer").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistLink {
    /// The URL as pasted, minus the trailing punctuation a sentence carries.
    pub url: String,
    /// `deezer`, `spotify`, `youtube` or `apple`.
    pub source: String,
    /// The service's name, as the offer says it.
    pub label: String,
}

/// `playlistLink(text)`: the first playlist link in `text`, or `None`.
pub fn playlist_link(text: &str) -> Option<PlaylistLink> {
    let url = find_url(text)?;
    // A `spotify:playlist:<22 base-62 chars>` URI, which has no host at all.
    if let Some(rest) = strip_prefix_ci(&url, "spotify:") {
        let mut parts = rest.split(':');
        let kind = parts.next().unwrap_or_default();
        let id = parts.next().unwrap_or_default();
        let base = (kind.eq_ignore_ascii_case("playlist") || kind.eq_ignore_ascii_case("album"))
            && parts.next().is_none()
            && id.len() == 22
            && id.chars().all(|ch| ch.is_ascii_alphanumeric());
        if base {
            return Some(link(url, "spotify"));
        }
        return None;
    }
    let after_scheme =
        strip_prefix_ci(&url, "https://").or_else(|| strip_prefix_ci(&url, "http://"))?;
    let (authority, rest) = match after_scheme.find(['/', '?', '#']) {
        Some(at) => (&after_scheme[..at], &after_scheme[at..]),
        None => (after_scheme, ""),
    };
    // `user@host:port` — the host is what a service table matches on.
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or(authority)
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let path_and_query = rest.to_ascii_lowercase();

    // The SPA's table, in ITS order (`linkimport.js#SERVICES`): the first row
    // whose host AND path match wins, which is why `link.deezer.com/s/abc`
    // reaches the short-link row after the page row's path refused it.
    for (host_match, path_match, source) in SERVICES {
        if !host_match.matches(&host) {
            continue;
        }
        let path_ok = match path_match {
            PathMatch::Any => true,
            PathMatch::DeezerPage => path_has_number(&path_and_query, &["/playlist/", "/album/"]),
            PathMatch::SpotifyPage => {
                path_has_spotify_id(&path_and_query, &["/playlist/", "/album/"])
            }
            PathMatch::YoutubeList => {
                path_and_query.contains("list=") || path_and_query.contains("/browse/vl")
            }
            PathMatch::ApplePage => {
                path_and_query.contains("/playlist/") || path_and_query.contains("/album/")
            }
        };
        if path_ok {
            return Some(link(url, source));
        }
    }
    None
}

/// How a row of the service table recognises its host.
#[derive(Clone, Copy)]
enum HostMatch {
    /// The host itself, or any of its subdomains (`deezer.com`, `www.deezer.com`).
    Domain(&'static str),
    /// These exact hosts only (`spoti.fi` and no subdomain of it).
    Exactly(&'static [&'static str]),
}

impl HostMatch {
    fn matches(self, host: &str) -> bool {
        match self {
            HostMatch::Domain(domain) => host == domain || host.ends_with(&format!(".{domain}")),
            HostMatch::Exactly(hosts) => hosts.contains(&host),
        }
    }
}

/// What the path must look like, in the terms the SPA writes as regexes.
#[derive(Clone, Copy)]
enum PathMatch {
    /// Any path at all (a shortener decides where it leads, not us).
    Any,
    DeezerPage,
    SpotifyPage,
    YoutubeList,
    ApplePage,
}

/// `linkimport.js#SERVICES`, in its own order.
const SERVICES: [(HostMatch, PathMatch, &str); 8] = [
    (
        HostMatch::Domain("deezer.com"),
        PathMatch::DeezerPage,
        "deezer",
    ),
    (
        HostMatch::Exactly(&["link.deezer.com", "deezer.page.link", "dzr.page.link"]),
        PathMatch::Any,
        "deezer",
    ),
    (
        HostMatch::Exactly(&["open.spotify.com", "play.spotify.com"]),
        PathMatch::SpotifyPage,
        "spotify",
    ),
    (
        HostMatch::Exactly(&["spotify.link", "spoti.fi"]),
        PathMatch::Any,
        "spotify",
    ),
    (
        HostMatch::Domain("youtube.com"),
        PathMatch::YoutubeList,
        "youtube",
    ),
    (
        HostMatch::Exactly(&["youtu.be"]),
        PathMatch::YoutubeList,
        "youtube",
    ),
    (
        HostMatch::Domain("music.apple.com"),
        PathMatch::ApplePage,
        "apple",
    ),
    // The embed host is a subdomain of `music.apple.com`, so the domain row
    // above already covers it; this row keeps the SPA's own list one-for-one and
    // is reached only for a host the first row would have refused (none).
    (
        HostMatch::Exactly(&["embed.music.apple.com"]),
        PathMatch::ApplePage,
        "apple",
    ),
];

fn link(url: String, source: &str) -> PlaylistLink {
    PlaylistLink {
        url,
        source: source.to_owned(),
        label: match source {
            "deezer" => "Deezer",
            "spotify" => "Spotify",
            "youtube" => "YouTube",
            _ => "Apple Music",
        }
        .to_owned(),
    }
}

/// The first URL-looking token in `text`, without the punctuation that ends a
/// sentence around it (`linkimport.js#URL_RE`).
fn find_url(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let start = ["https://", "http://", "spotify:"]
        .iter()
        .filter_map(|scheme| lower.find(scheme))
        .min()?;
    let rest = &text[start..];
    let end = rest
        .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>'))
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches([')', '.', ',', ';', '!', '?', '»', '…']);
    // A scheme alone ("http://") names no host, so there is nothing to import.
    (!url.is_empty()).then(|| url.to_owned())
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &text[prefix.len()..])
}

/// `/(playlist|album)/<digits>` anywhere in the path — Deezer's shape.
fn path_has_number(path: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| {
        path.match_indices(prefix).any(|(at, _)| {
            path[at + prefix.len()..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit())
        })
    })
}

/// `/playlist/<22 base-62 chars>` — Spotify's shape.
fn path_has_spotify_id(path: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| {
        path.match_indices(prefix).any(|(at, _)| {
            let id: String = path[at + prefix.len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric())
                .collect();
            id.len() == 22
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playlist_page_of_each_service_is_recognised() {
        for (text, source, label) in [
            (
                "https://www.deezer.com/fr/playlist/1234567",
                "deezer",
                "Deezer",
            ),
            ("https://deezer.com/album/42", "deezer", "Deezer"),
            (
                "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
                "spotify",
                "Spotify",
            ),
            (
                "https://open.spotify.com/album/4aawyAB9vmqN3uQ7FjRGTy",
                "spotify",
                "Spotify",
            ),
            (
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "spotify",
                "Spotify",
            ),
            (
                "https://www.youtube.com/watch?v=abc&list=PL1234567890",
                "youtube",
                "YouTube",
            ),
            ("https://youtu.be/abc?list=PL1", "youtube", "YouTube"),
            (
                "https://music.apple.com/fr/playlist/ma-liste/pl.123",
                "apple",
                "Apple Music",
            ),
            ("https://link.deezer.com/s/abc", "deezer", "Deezer"),
            ("https://spoti.fi/3abc", "spotify", "Spotify"),
        ] {
            let found = playlist_link(text).unwrap_or_else(|| panic!("{text} is a playlist link"));
            assert_eq!(found.source, source, "{text}");
            assert_eq!(found.label, label);
        }
    }

    #[test]
    fn the_link_keeps_its_url_without_the_sentence_around_it() {
        let found = playlist_link(
            "Regarde ça : https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M).",
        )
        .expect("a link in a sentence");
        assert_eq!(
            found.url, "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "the closing bracket and the full stop are the sentence's"
        );
    }

    #[test]
    fn a_search_term_is_not_a_link() {
        for text in [
            "daft punk",
            "",
            "https://neezer.example/app/#/album/42",
            "https://open.spotify.com/track/4aawyAB9vmqN3uQ7FjRGTy",
            "https://www.youtube.com/watch?v=abc",
            "https://music.apple.com/fr/artist/123",
            "spotify:track:4aawyAB9vmqN3uQ7FjRGTy",
            "spotify:playlist:too-short",
            "http://",
        ] {
            assert_eq!(playlist_link(text), None, "{text}");
        }
    }

    #[test]
    fn an_uppercase_scheme_and_host_still_match() {
        let found = playlist_link("HTTPS://WWW.DEEZER.COM/FR/PLAYLIST/99").expect("a link");
        assert_eq!(found.source, "deezer");
    }
}
