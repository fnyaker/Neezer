//! Covers and metadata copied from a peer, so a playlist taken on the LAN looks
//! exactly the same offline (docs/agents/lan-nodes.md, *Downloads and playlists*).
//!
//! **LN11 filled it.** For every set taken from a peer: the tracks whose
//! `TrackMeta` this device does not have are asked with `Meta` (200 ids a frame),
//! and their covers with `Cover` — one `Bulk` request each, on this node's own
//! lane, so they yield to the tapped track (rule 1). Covers are stored PINNED
//! (`Covers::put_pinned`, so the LRU never evicts a download's art), and the
//! titles and artists travel back to the caller, which hands them to
//! `nz-offline` as its `Choice` labels — that is what makes the downloads screen
//! and the playlist page read the same offline.
//!
//! Nothing here writes anything the person did not ask for: the caller only runs
//! this for a set it is queueing, or one already in the downloads queue.
//!
//! # The two things the wire can make this module do, and the caps
//!
//! A peer is authenticated, not trusted: every count it announces is capped
//! BEFORE an allocation (`MAX_META_TRACKS` per frame, `MAX_COVER_BYTES` per
//! sleeve, `MAX_IMPORT_BYTES` per set), and nothing it sends becomes a path —
//! `Covers::put_pinned` names a sleeve by the digest of its URL, and a track id
//! only ever lands in a `Choice` and a database row.

use std::sync::Arc;

use nz_api::lanes::Lane as ApiLane;

use crate::proto;
use crate::service::Lan;
use crate::transport::PeerConn;
use crate::types::{DevicePub, LanError, Urgency};

/// How many track ids one `Meta` frame may ask for (`proto.rs`'s own cap).
pub const MAX_META_TRACKS: usize = 200;
/// The most one sleeve may be (`proto.rs`'s own `Cover` cap).
pub const MAX_COVER_BYTES: u64 = 2 << 20;
/// What one set's sleeves may cost this device together: a hostile peer must not
/// fill the disk through a "playlist".
pub const MAX_IMPORT_BYTES: u64 = 64 << 20;
/// The tracks one import may ask about; a set larger than this is refused rather
/// than walked for minutes.
pub const MAX_IMPORT_TRACKS: usize = crate::sets::MAX_TRACKS_PER_SET;

/// What one import brought back, counted (rule 2: the screen says what happened).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Metadata rows stored.
    pub meta: usize,
    /// Covers fetched and pinned.
    pub covers: usize,
    /// Bytes those covers took.
    pub bytes: u64,
}

/// The rows AND the report: `download_peer_sets` needs the titles for its
/// `Choice` labels, and the report is what the screen draws.
#[derive(Debug, Default)]
pub(crate) struct Imported {
    pub rows: Vec<proto::TrackMeta>,
    pub report: ImportReport,
}

/// Runs `future` on this node's `nz-lan` runtime from a caller that is NOT one of
/// its tasks (the engine's decoder thread, the offline worker's), and answers
/// `None` when that cannot be done rather than panicking.
///
/// - On a plain thread: `Handle::block_on`, which is what the adapters do.
/// - Inside ANOTHER runtime (a test, the mobile core): the future is spawned and
///   its answer comes back through a channel — the calling runtime must never be
///   asked to `block_on` anything.
/// - Inside the `nz-lan` runtime itself: `None`. One worker thread means a
///   `block_on` here would wait for the driver it is standing on.
pub(crate) fn run_on<T, F>(handle: &tokio::runtime::Handle, future: F) -> Option<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = T> + Send + 'static,
{
    if let Ok(current) = tokio::runtime::Handle::try_current() {
        if current.id() == handle.id() {
            return None;
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        handle.spawn(async move {
            let _ = sender.send(future.await);
        });
        return receiver.recv().ok();
    }
    Some(handle.block_on(future))
}

/// The `TrackMeta` rows of `tracks`, 200 ids a frame.
pub(crate) async fn meta_rows(
    conn: &PeerConn,
    tracks: &[String],
) -> Result<Vec<proto::TrackMeta>, LanError> {
    let mut rows = Vec::with_capacity(tracks.len().min(MAX_META_TRACKS));
    for chunk in tracks.chunks(MAX_META_TRACKS) {
        let reply = conn
            .request(proto::Request::Meta {
                tracks: chunk.to_vec(),
            })
            .await?;
        match reply.head {
            proto::Response::Meta { rows: mut page } => {
                if page.len() > MAX_META_TRACKS {
                    return Err(LanError::Proto(format!(
                        "le pair a répondu {} lignes pour {MAX_META_TRACKS} titres demandés",
                        page.len()
                    )));
                }
                rows.append(&mut page);
            }
            proto::Response::Busy { retry_in_ms } => {
                return Err(LanError::Busy { retry_in_ms });
            }
            proto::Response::NotFound => return Err(LanError::NotFound),
            proto::Response::Error { code } => {
                return Err(LanError::Proto(format!("le pair a répondu « {code} »")));
            }
            other => {
                return Err(LanError::Proto(format!(
                    "une réponse inattendue à des métadonnées : {other:?}"
                )));
            }
        }
    }
    Ok(rows)
}

/// One sleeve, capped before anything is allocated.
pub(crate) async fn cover_bytes(conn: &PeerConn, url: &str) -> Result<Vec<u8>, LanError> {
    if url.is_empty() || url.len() > 2_048 {
        return Err(LanError::Proto(
            "une adresse de pochette invalide".to_owned(),
        ));
    }
    let reply = conn
        .request(proto::Request::Cover {
            url: url.to_owned(),
        })
        .await?;
    let len = match reply.head {
        proto::Response::Cover { len, .. } => u64::from(len),
        proto::Response::Busy { retry_in_ms } => return Err(LanError::Busy { retry_in_ms }),
        proto::Response::NotFound => return Err(LanError::NotFound),
        proto::Response::Error { code } => {
            return Err(LanError::Proto(format!("le pair a répondu « {code} »")));
        }
        other => {
            return Err(LanError::Proto(format!(
                "une réponse inattendue à une pochette : {other:?}"
            )));
        }
    };
    if len > MAX_COVER_BYTES {
        return Err(LanError::Proto(format!(
            "une pochette de {len} octets dépasse {MAX_COVER_BYTES}"
        )));
    }
    let mut body = reply
        .body
        .ok_or_else(|| LanError::Proto("une pochette sans octets".to_owned()))?;
    // With the head's length, so the reservation is the annonced size and not a
    // growth a hostile peer drives.
    let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    while let Some(chunk) = body.chunk(64 * 1024).await? {
        out.extend_from_slice(&chunk);
        if out.len() as u64 > MAX_COVER_BYTES {
            return Err(LanError::Proto(
                "une pochette plus longue que son annonce".to_owned(),
            ));
        }
    }
    if out.is_empty() {
        return Err(LanError::Proto("une pochette vide".to_owned()));
    }
    Ok(out)
}

/// Fetches the metadata of `tracks` and pins every sleeve this device does not
/// already hold. `cache` is cloned into the future, which is what lets the
/// caller run it on the node's runtime from any thread.
pub(crate) async fn collect(
    cache: nz_cache::Cache,
    gates: Arc<nz_api::lanes::Gates>,
    conn: PeerConn,
    tracks: Vec<String>,
) -> Result<Imported, LanError> {
    if tracks.is_empty() {
        return Ok(Imported::default());
    }
    if tracks.len() > MAX_IMPORT_TRACKS {
        return Err(LanError::Proto(format!(
            "un ensemble de {} titres dépasse {MAX_IMPORT_TRACKS}",
            tracks.len()
        )));
    }
    // Rule 1: one Bulk request at a time across server AND LAN, so a set's
    // metadata and sleeves wait behind what the person just tapped.
    let _bulk = gates.acquire(ApiLane::Bulk).await;
    let rows = meta_rows(&conn, &tracks).await?;
    let mut imported = Imported {
        report: ImportReport {
            meta: rows.len(),
            ..ImportReport::default()
        },
        rows,
    };
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for row in &imported.rows {
        let Some(url) = row.cover.as_deref() else {
            continue;
        };
        if !seen.insert(url) {
            continue;
        }
        if cache.covers().get(url).is_some() {
            continue;
        }
        if imported.report.bytes >= MAX_IMPORT_BYTES {
            break;
        }
        let bytes = match cover_bytes(&conn, url).await {
            Ok(bytes) => bytes,
            // A sleeve this peer cannot serve is not a reason to fail the set:
            // the titles still arrive, and the LRU fetches the art later.
            Err(LanError::Busy { .. } | LanError::NotFound | LanError::PeerGone) => continue,
            Err(error) => return Err(error),
        };
        if bytes.len() as u64 + imported.report.bytes > MAX_IMPORT_BYTES {
            break;
        }
        match cache.covers().put_pinned(url, &bytes) {
            Ok(_) => {
                imported.report.covers += 1;
                imported.report.bytes += bytes.len() as u64;
            }
            Err(error) => {
                tracing::debug!(%error, "LAN : une pochette n'a pas pu être épinglée");
            }
        }
    }
    Ok(imported)
}

/// Brings the missing metadata and covers of `tracks` over from `peer`, for a set
/// already being downloaded.
///
/// A track the peer answers nothing about is simply absent from the report (the
/// downloads screen keeps the label it already had), and a sleeve already in the
/// cache costs **0** requests.
pub async fn import_for_set(
    lan: &Lan,
    cache: &nz_cache::Cache,
    peer: &DevicePub,
    tracks: &[String],
) -> Result<ImportReport, LanError> {
    let conn = lan.conn(peer).ok_or(LanError::PeerGone)?;
    let imported = collect(cache.clone(), lan.api().gates(), conn, tracks.to_vec()).await?;
    Ok(imported.report)
}

/// The urgency an adapter's lane maps to (`User → Play`, the rest 1:1): the same
/// table the opener uses, named once.
pub(crate) fn urgency_of(lane: nz_audio::Lane) -> Urgency {
    match lane {
        nz_audio::Lane::User => Urgency::Play,
        nz_audio::Lane::Prefetch => Urgency::Prefetch,
        nz_audio::Lane::Bulk => Urgency::Bulk,
    }
}

/// A `nz_audio` lane in this crate's words, for the same reason.
#[cfg(test)]
pub(crate) fn lane_of(urgency: Urgency) -> nz_audio::Lane {
    match urgency {
        Urgency::Play => nz_audio::Lane::User,
        Urgency::Prefetch => nz_audio::Lane::Prefetch,
        Urgency::Bulk => nz_audio::Lane::Bulk,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh report counts nothing: a screen that draws it shows "0" rather than
    /// a number the import did not produce.
    #[test]
    fn an_empty_report_counts_nothing() {
        let report = ImportReport::default();
        assert_eq!((report.meta, report.covers, report.bytes), (0, 0, 0));
    }

    /// The caps are the protocol's own: a `Meta` frame asks 200 ids and a sleeve
    /// is 2 MiB, so `import.rs` cannot ask for more than `proto.rs` allows.
    #[test]
    fn the_caps_are_the_protocol_caps() {
        assert_eq!(MAX_META_TRACKS, 200);
        assert_eq!(MAX_COVER_BYTES, 2 << 20);
    }

    /// Rule 1's mapping, both ways: the engine's `User` is the wire's `Play`, and
    /// nothing gets promoted on the way out.
    #[test]
    fn a_lane_is_an_urgency_and_back() {
        for urgency in [Urgency::Play, Urgency::Prefetch, Urgency::Bulk] {
            assert_eq!(urgency_of(lane_of(urgency)), urgency);
        }
        assert_eq!(urgency_of(nz_audio::Lane::User), Urgency::Play);
    }
}
