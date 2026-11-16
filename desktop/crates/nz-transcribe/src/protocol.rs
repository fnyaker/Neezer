//! The server's worker protocol: routes, payloads, backoff — owned by lot T3.
//!
//! T1 pins the routes and the one guard every per-episode route needs. The paths
//! are server facts (`neezer/webui/transcribe_worker.py`), and the guard exists
//! because an episode id is interpolated into a path: the id comes from the
//! server's own `claim` answer, but "comes from the server" is not a validation,
//! and a client-supplied string that reaches a URL must first be known to be an
//! id and not `../../settings` (CLAUDE.md, *Security*).

/// `POST`: lease the next queued episode; `204` means the queue is empty.
pub const CLAIM: &str = "/api/transcribe/worker/claim";

/// `GET …/audio/<eid>` with `Authorization: Bearer <token>` and `X-NS-Lease`;
/// byte ranges supported, which is how a resumed job skips what it already has.
pub fn audio(episode_id: &str) -> Option<String> {
    if !is_episode_id(episode_id) {
        return None;
    }
    Some(format!("/api/transcribe/worker/audio/{episode_id}"))
}

/// `POST …/heartbeat`: `{chunk_pct}` in, `{cancel, pause, done_ms, expires_at}` out.
pub fn heartbeat(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/heartbeat")
}

/// `POST …/chunk`: one chunk's `{start_ms, duration_ms, segments, language,
/// engine, model, engine_version}`; the server answers `{applied, done_ms,
/// n_segments}` and refuses a chunk that is not the next one.
pub fn chunk(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/chunk")
}

/// `POST …/complete`: the last chunk landed, the transcript becomes `ready`.
pub fn complete(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/complete")
}

/// `POST …/fail`: `{code, message}`, the cause the person will read.
pub fn fail(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/fail")
}

/// `POST …/release`: give the lease back without a verdict (a pause, a shutdown).
pub fn release(episode_id: &str) -> Option<String> {
    episode_path(episode_id, "/release")
}

/// `/api/transcribe/worker/<eid><suffix>`, or `None` when the id is not one.
fn episode_path(episode_id: &str, suffix: &str) -> Option<String> {
    if !is_episode_id(episode_id) {
        return None;
    }
    Some(format!("/api/transcribe/worker/{episode_id}{suffix}"))
}

/// Whether `value` is a UUID the server would accept as an episode id.
///
/// The server's own check is `uuid.UUID(str(value))`, which takes the canonical
/// hyphenated form (`8-4-4-4-12`) and the bare 32-hex one. This accepts exactly
/// those two shapes and nothing else — in particular no `/`, no `.` and no NUL,
/// so the value cannot escape the path segment it is written into.
pub fn is_episode_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    match bytes.len() {
        36 => {
            [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
                && bytes.iter().enumerate().all(|(index, byte)| {
                    [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit()
                })
        }
        32 => bytes.iter().all(u8::is_ascii_hexdigit),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{CLAIM, audio, chunk, complete, fail, heartbeat, is_episode_id, release};

    /// The routes the server exposes (`transcribe_worker.py`), with the id in the
    /// one place it is allowed.
    #[test]
    fn the_routes_are_the_servers() {
        let id = "b7b1c0de-0000-4000-8000-000000000000";
        assert_eq!(CLAIM, "/api/transcribe/worker/claim");
        assert_eq!(
            audio(id).as_deref(),
            Some("/api/transcribe/worker/audio/b7b1c0de-0000-4000-8000-000000000000")
        );
        assert_eq!(
            heartbeat(id).as_deref(),
            Some("/api/transcribe/worker/b7b1c0de-0000-4000-8000-000000000000/heartbeat")
        );
        assert!(chunk(id).unwrap().ends_with("/chunk"));
        assert!(complete(id).unwrap().ends_with("/complete"));
        assert!(fail(id).unwrap().ends_with("/fail"));
        assert!(release(id).unwrap().ends_with("/release"));
    }

    /// An id that is not one never becomes a path: not a traversal, not a query,
    /// not an empty string.
    #[test]
    fn only_a_uuid_becomes_a_path() {
        assert!(is_episode_id("00000000-0000-0000-0000-000000000000"));
        assert!(is_episode_id("0123456789abcdef0123456789abcdef"));
        assert!(!is_episode_id(""));
        assert!(!is_episode_id("../../settings"));
        assert!(!is_episode_id("0/0"));
        assert!(!is_episode_id("00000000-0000-0000-0000-00000000000"));
        assert!(!is_episode_id("zzzzzzzz-0000-0000-0000-000000000000"));
        assert_eq!(audio("../settings"), None);
        assert_eq!(fail("0"), None);
    }
}
