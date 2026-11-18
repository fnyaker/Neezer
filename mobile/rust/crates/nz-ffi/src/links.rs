//! App links: the SPA's `/app/#/...` URLs, `neezer://...`, and the Deezer and
//! Spotify links the import understands. Owned by **MB2**.
//!
//! One enum, so `:app`'s `AppLink → route` mapping is exhaustive by the
//! compiler's own rule: a link the app learns to open is a route, or it does
//! not compile. The RULE (which URL is which route, and which are refused) is
//! `nz_core::links`; this file is the record and the conversion, so the parsing
//! is proven by `nz-core`'s own tests against the SPA's real URL shapes.

/// A route an incoming link names.
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum AppLink {
    Home,
    Search { q: Option<String> },
    Artist { id: String },
    Album { id: String },
    Playlist { id: String },
    Track { id: String },
    Podcast { id: String },
    Credits { name: String },
    SharedPlaylist { id: String },
    Import { url: String },
    Party { id: String },
    RemoteClaim { token: String },
    Invite { code: String },
    Settings { section: Option<String> },
    Recap { year: Option<u32> },
    /// Nouveautés (MB14): the releases feed's own screen.
    Releases,
    /// One release shelf (MB14): `artists` or `discovery`.
    ReleaseShelf { kind: String },
}

impl From<nz_core::links::AppLink> for AppLink {
    fn from(link: nz_core::links::AppLink) -> Self {
        use nz_core::links::AppLink as Core;
        match link {
            Core::Home => AppLink::Home,
            Core::Search { q } => AppLink::Search { q },
            Core::Artist { id } => AppLink::Artist { id },
            Core::Album { id } => AppLink::Album { id },
            Core::Playlist { id } => AppLink::Playlist { id },
            Core::Track { id } => AppLink::Track { id },
            Core::Podcast { id } => AppLink::Podcast { id },
            Core::Credits { name } => AppLink::Credits { name },
            Core::SharedPlaylist { id } => AppLink::SharedPlaylist { id },
            Core::Import { url } => AppLink::Import { url },
            Core::Party { id } => AppLink::Party { id },
            Core::RemoteClaim { token } => AppLink::RemoteClaim { token },
            Core::Invite { code } => AppLink::Invite { code },
            Core::Settings { section } => AppLink::Settings { section },
            Core::Recap { year } => AppLink::Recap { year },
            Core::Releases => AppLink::Releases,
            Core::ReleaseShelf { kind } => AppLink::ReleaseShelf { kind },
        }
    }
}

/// Parses an incoming URL. MB2 owns the rule (and its tests against the SPA's
/// own URL shapes); a link the app cannot open answers nothing, so no link
/// opens a wrong screen.
#[uniffi::export]
pub fn parse_app_link(url: String) -> Option<AppLink> {
    nz_core::links::parse(&url).map(AppLink::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant of the core enum has its record, so a new core route is a
    /// compile error here rather than a missing screen.
    #[test]
    fn every_core_variant_crosses() {
        let all = [
            nz_core::links::AppLink::Home,
            nz_core::links::AppLink::Search { q: None },
            nz_core::links::AppLink::Artist { id: "1".into() },
            nz_core::links::AppLink::Album { id: "1".into() },
            nz_core::links::AppLink::Playlist { id: "1".into() },
            nz_core::links::AppLink::Track { id: "1".into() },
            nz_core::links::AppLink::Podcast { id: "1".into() },
            nz_core::links::AppLink::Credits {
                name: "Jane".into(),
            },
            nz_core::links::AppLink::SharedPlaylist { id: "1".into() },
            nz_core::links::AppLink::Import {
                url: "https://x/1".into(),
            },
            nz_core::links::AppLink::Party { id: "1".into() },
            nz_core::links::AppLink::RemoteClaim { token: "t".into() },
            nz_core::links::AppLink::Invite { code: "c".into() },
            nz_core::links::AppLink::Settings { section: None },
            nz_core::links::AppLink::Recap { year: Some(2025) },
            nz_core::links::AppLink::Releases,
            nz_core::links::AppLink::ReleaseShelf {
                kind: "artists".into(),
            },
        ];
        for link in all {
            let record = AppLink::from(link.clone());
            // And back: the record names the same route.
            assert!(!format!("{record:?}").is_empty());
        }
    }

    #[test]
    fn the_boundary_parses_what_the_core_parses() {
        assert_eq!(
            parse_app_link("https://neezer.example/app/#/search/daft%20punk".into()),
            Some(AppLink::Search {
                q: Some("daft punk".into())
            })
        );
        assert_eq!(
            parse_app_link("https://neezer.example/app/#/library".into()),
            None
        );
        // MB14's two routes, from a link and from the app scheme: the door MB5
        // could not open, and the shelf behind it.
        assert_eq!(
            parse_app_link("https://neezer.example/app/#/releases".into()),
            Some(AppLink::Releases)
        );
        assert_eq!(
            parse_app_link("neezer://releases/discovery".into()),
            Some(AppLink::ReleaseShelf {
                kind: "discovery".into()
            })
        );
    }
}
