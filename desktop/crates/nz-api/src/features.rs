//! Which optional features a server has, asked once and cheaply.
//!
//! A server older than a client feature answers nothing for its routes: a
//! desktop that only finds out when the person opens the screen shows a menu
//! entry that cannot work (D17's acceptance, *an older server without a route
//! hides the feature*). The probe is one `OPTIONS` per feature at login, on the
//! `Prefetch` lane: a registered route answers `200` with an `Allow` header, an
//! unknown one `404`. Measured on the bench: **5 requests, 1.6 kB** in total,
//! once per session.
//!
//! `OPTIONS` and not `GET`: several of these features have no reading route
//! that is not also a legitimate `404` (a party id that does not exist, a
//! folder id that is not yours), so a `GET` could not tell "this server is old"
//! from "that thing is not there". The route's existence is what is asked.

/// One optional feature whose routes an older server may not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Feature {
    /// Listen party, server routes `/api/party/*` (D15b).
    Party,
    /// Remote control, `/api/remote/*` (D15a).
    Remote,
    /// Genre studio, `/api/genre/*` (D16).
    Genres,
    /// Server-side transcription, `/api/transcribe/*` (T3).
    Transcription,
    /// Playlist folders, `/api/playlist-folders*` (D8b).
    Folders,
}

impl Feature {
    /// Every feature the client asks about, in one probe each.
    pub const ALL: [Feature; 5] = [
        Feature::Party,
        Feature::Remote,
        Feature::Genres,
        Feature::Transcription,
        Feature::Folders,
    ];

    /// The route the probe asks `OPTIONS` for: one that exists on a server that
    /// HAS the feature, and does not on a server that predates it.
    pub fn probe(self) -> &'static str {
        match self {
            Feature::Party => "/party/mine",
            Feature::Remote => "/remote/links",
            Feature::Genres => "/genre/status",
            Feature::Transcription => "/transcribe/status",
            Feature::Folders => "/playlist-folders",
        }
    }

    /// What the person reads when the feature is absent.
    pub fn absent_fr(self) -> &'static str {
        match self {
            Feature::Party => "L'écoute partagée",
            Feature::Remote => "Le contrôle distant",
            Feature::Genres => "Le studio de genres",
            Feature::Transcription => "La transcription sur le serveur",
            Feature::Folders => "Les dossiers de playlists",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_has_a_probe_and_a_sentence() {
        for feature in Feature::ALL {
            let probe = feature.probe();
            assert!(probe.starts_with('/'), "{feature:?}: {probe}");
            assert!(!probe.contains("//"), "{feature:?}: {probe}");
            assert!(!feature.absent_fr().is_empty());
        }
        // One probe per feature, and no two features share one: a copy-paste
        // would hide the wrong entry.
        let mut probes: Vec<&str> = Feature::ALL.iter().map(|f| f.probe()).collect();
        probes.sort_unstable();
        probes.dedup();
        assert_eq!(probes.len(), Feature::ALL.len());
    }
}
