//! The URLs the app hands to something that is not `Api`: a download manager, a
//! shell handler, an MPRIS client, an `<audio>`-shaped consumer.
//!
//! The SPA returns `BASE + "/export/..."` and lets the browser fetch it with the
//! session cookie; a native app instead needs an ABSOLUTE URL, so these build
//! one from the server root. They are paths, not requests: nothing here touches
//! the network.

use crate::client::{Api, Quality, TrackId, encode};
use crate::error::ApiError;
use crate::lanes::Lane;

impl Api {
    /// `/api/stream/<id>?q=&start=` — the SPA's `api.streamUrl`.
    ///
    /// An absent `q` means FLAC; `start` below 50 ms is dropped (the server
    /// would slice for nothing).
    pub fn stream_url(&self, id: &TrackId, quality: Quality, start: Option<f64>) -> String {
        self.url(&crate::client::stream_path(id, quality, start))
    }

    /// `/api/cover/<id>` — the archived sleeve, same origin.
    pub fn cover_url(&self, id: &str) -> String {
        self.url(&format!("/cover/{}", encode(id)))
    }

    /// `/api/share/file/<id>?fmt=` — the whole track as a file, with the server's
    /// `Content-Disposition`.
    pub fn share_file_url(&self, id: &str, fmt: Option<&str>) -> String {
        let suffix = match fmt {
            Some(fmt) => format!("?fmt={}", encode(fmt)),
            None => String::new(),
        };
        self.url(&format!("/share/file/{}{suffix}", encode(id)))
    }

    /// `/api/share/clip/<id>?start=&end=&fmt=` — a clip of the track.
    pub fn share_clip_url(&self, id: &str, start: f64, end: f64, fmt: &str) -> String {
        self.url(&format!(
            "/share/clip/{}?start={:.3}&end={:.3}&fmt={}",
            encode(id),
            start,
            end,
            encode(fmt)
        ))
    }

    /// `/api/export/<kind>/<id>?fmt=` — a whole playlist/album/favourites as one
    /// archive.
    pub fn export_url(&self, kind: &str, id: &str, fmt: &str) -> String {
        self.url(&format!(
            "/export/{}/{}{}",
            encode(kind),
            encode(id),
            crate::endpoints::query(&[("fmt", Some(fmt.to_owned()))])
        ))
    }

    /// `/api/services/list/<kind>/<id>?fmt=` — the plain list file for a service
    /// there is no door to (Apple Music…).
    pub fn list_file_url(&self, kind: &str, id: &str, fmt: &str) -> String {
        self.url(&format!(
            "/services/list/{}/{}{}",
            encode(kind),
            encode(id),
            crate::endpoints::query(&[("fmt", Some(fmt.to_owned()))])
        ))
    }

    /// `GET /api/share/waveform/<id>` — the peaks a clip editor draws before it
    /// asks for anything.
    pub async fn waveform(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        self.get(Lane::User, &format!("/share/waveform/{}", encode(id)))
            .await
            .map(|fresh| fresh.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api() -> Api {
        Api::new("http://127.0.0.1:5000").expect("base")
    }

    /// The SPA's own URL grammar, byte for byte: a q= only for a non-FLAC
    /// quality, `start` only past 50 ms and to three decimals, and the id
    /// percent-encoded (an upload's UUID is safe either way; a hostile id is
    /// not).
    #[test]
    fn stream_urls_match_api_stream_url() {
        let api = api();
        let id = TrackId::from("3000168");
        assert_eq!(
            api.stream_url(&id, Quality::Flac, None),
            "http://127.0.0.1:5000/api/stream/3000168"
        );
        assert_eq!(
            api.stream_url(&id, Quality::Opus128, None),
            "http://127.0.0.1:5000/api/stream/3000168?q=OPUS_128"
        );
        assert_eq!(
            api.stream_url(&id, Quality::Opus320, Some(6.0)),
            "http://127.0.0.1:5000/api/stream/3000168?q=OPUS_320&start=6.000"
        );
        // 50 ms or under: the server would slice for nothing.
        assert_eq!(
            api.stream_url(&id, Quality::Flac, Some(0.04)),
            "http://127.0.0.1:5000/api/stream/3000168"
        );
        let hostile = TrackId::from("a/b?c");
        assert_eq!(
            api.stream_url(&hostile, Quality::Flac, None),
            "http://127.0.0.1:5000/api/stream/a%2Fb%3Fc"
        );
    }

    #[test]
    fn share_and_export_urls_carry_their_query() {
        let api = api();
        assert_eq!(
            api.share_file_url("abc", None),
            "http://127.0.0.1:5000/api/share/file/abc"
        );
        assert_eq!(
            api.share_clip_url("abc", 1.5, 30.25, "mp3"),
            "http://127.0.0.1:5000/api/share/clip/abc?start=1.500&end=30.250&fmt=mp3"
        );
        assert_eq!(
            api.export_url("playlist", "8f-2b", "zip"),
            "http://127.0.0.1:5000/api/export/playlist/8f-2b?fmt=zip"
        );
        assert_eq!(
            api.cover_url("3000168"),
            "http://127.0.0.1:5000/api/cover/3000168"
        );
    }
}
