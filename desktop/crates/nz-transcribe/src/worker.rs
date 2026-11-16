//! The enrolled-client worker: claim a job, stream its audio, deliver chunks —
//! owned by lot T3.
//!
//! T1 pins the two numbers of that loop which are not in a payload: how often a
//! heartbeat must arrive before the server gives the job away, and which version
//! of the protocol the server speaks. Both are read from the code the server
//! already runs (`docker/transcribe/worker.py`), not guessed.

/// Seconds between two heartbeats (`worker.heartbeat_seconds`, and the
/// `HEARTBEAT_WRITE_SECONDS` the server keeps a lease valid with).
///
/// A lease lives 15 minutes, but the *server* only rewrites it every 60 s, so a
/// 5 s beat is twelve chances per write: one dropped or slow request never loses
/// a job, and a `SIGSTOP`ped engine keeps its lease for the whole pause.
pub const HEARTBEAT_SECONDS: u64 = 5;

/// The protocol version this client speaks (`worker.PROTOCOL`).
pub const PROTOCOL: u32 = 1;

#[cfg(test)]
mod tests {
    use super::{HEARTBEAT_SECONDS, PROTOCOL};

    /// Twelve heartbeats inside one server-side write window (60 s), and the
    /// version the server's `worker.py` announces.
    #[test]
    fn the_worker_beats_twelve_times_per_minute() {
        assert_eq!(HEARTBEAT_SECONDS, 5);
        assert_eq!(60 / HEARTBEAT_SECONDS, 12);
        assert_eq!(PROTOCOL, 1);
    }
}
