//! Which optional features a server has, asked once and cheaply.
//!
//! A server older than a client feature answers nothing for its routes: a
//! desktop that only finds out when the person opens the screen shows a menu
//! entry that cannot work (D17's acceptance, *an older server without a route
//! hides the feature*). The probe is one `OPTIONS` per feature at login, on the
//! `Prefetch` lane: a registered route answers `200` with an `Allow` header, an
//! unknown one `404`. Measured on `tools/perf_api.py --serve` (2026-11-18): the six
//! probes cost one request each, a registered route's answer has a **0-byte body**
//! (~700 B of headers) and the `404` a server without the feature answers carries
//! **207 bytes** of JSON — so the whole probe set costs nothing but a header round
//! trip, and it can never wait behind the screen being opened.
//!
//! `OPTIONS` and not `GET`: several of these features have no reading route
//! that is not also a legitimate `404` (a party id that does not exist, a
//! folder id that is not yours), so a `GET` could not tell "this server is old"
//! from "that thing is not there". The route's existence is what is asked.
//!
//! One feature is PROBED but not yet answered by the server: `Feature::Lan`
//! (`nz-lan`, `docs/agents/lan-nodes.md`) is declared here in LN0 because every
//! client lot compiles against it, and `/lan/status` lands with LN1. Until then
//! `nz-api`'s bench names it in `PENDING_SERVER_ROUTES`; LN1 removes it from
//! there and the same test requires `Some(true)` for it too.

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
    /// The LAN exchange between the person's devices, `/api/lan/*` (LN1;
    /// `nz-lan`, `docs/agents/lan-nodes.md`). Its absence is what keeps the
    /// whole feature — thread, screens, settings — out of the app.
    Lan,
}

impl Feature {
    /// Every feature the client asks about, in one probe each.
    pub const ALL: [Feature; 6] = [
        Feature::Party,
        Feature::Remote,
        Feature::Genres,
        Feature::Transcription,
        Feature::Folders,
        Feature::Lan,
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
            Feature::Lan => "/lan/status",
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
            Feature::Lan => "L'échange entre appareils proches",
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

    /// `Feature::Lan` is the note's contract, verbatim: the probe is
    /// `/lan/status` (the route LN1 registers) and the sentence is the one
    /// every client shows. A typo here hides the feature on a server that has
    /// it, or offers it on a server that does not.
    #[test]
    fn the_lan_feature_probes_the_route_ln1_registers() {
        assert_eq!(Feature::Lan.probe(), "/lan/status");
        assert_eq!(
            Feature::Lan.absent_fr(),
            "L'échange entre appareils proches"
        );
        assert!(Feature::ALL.contains(&Feature::Lan));
        // Six probes at login, and that is the count the doc comment states.
        assert_eq!(Feature::ALL.len(), 6);
    }
}
