//! App links: the SPA's `/app/#/...` URLs, `neezer://...`, and the Deezer and
//! Spotify links the import understands. Owned by **MB2**.
//!
//! One enum, so `:app`'s `AppLink → route` mapping is exhaustive by the
//! compiler's own rule: a link the app learns to open is a route, or it does
//! not compile.

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
}

/// Parses an incoming URL. MB2 owns the rule (and its tests against the SPA's
/// own URL shapes); MB0 answers nothing so no link opens a wrong screen.
#[uniffi::export]
pub fn parse_app_link(url: String) -> Option<AppLink> {
    let _ = url;
    None
}
