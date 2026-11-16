//! The JSON-lines channel between the app and the supervisor — owned by T3.
//!
//! One JSON object per line in each direction, over the supervisor's stdin and
//! stdout (the note's *Control channel*). T1 pins the names, because three
//! places have to agree on them: the supervisor writes them, the app reads them
//! and shows a failure's cause (rule 2), and T4's settings screen switches on
//! `engine` and `error`.

/// App → supervisor: stop the engine child, keep the job and its lease.
pub const PAUSE: &str = "pause";

/// App → supervisor: let the engine run again.
pub const RESUME: &str = "resume";

/// App → supervisor: give the job back and exit.
pub const STOP: &str = "stop";

/// App → supervisor: transcribe one episode from a local audio file, no server.
pub const LOCAL: &str = "local";

/// Supervisor → app: `{backend, device, reason}` — which processor, and why not
/// the GPU when the answer is `cpu` ([`crate::engine::FallbackCause`]).
pub const ENGINE: &str = "engine";

/// Supervisor → app: `{episode_id, done_ms, duration_ms}`.
pub const PROGRESS: &str = "progress";

/// Supervisor → app: `{episode_id, segments}` — the transcript is whole.
pub const DONE: &str = "done";

/// Supervisor → app: `{episode_id, code, message}` — the cause, in French.
pub const ERROR: &str = "error";

/// Supervisor → app: nothing left to do.
pub const IDLE: &str = "idle";

#[cfg(test)]
mod tests {
    use super::{DONE, ENGINE, ERROR, IDLE, LOCAL, PAUSE, PROGRESS, RESUME, STOP};

    /// Four commands in, five events out, and no two share a name: a supervisor
    /// that confused `stop` with `pause` would end a job the person only paused.
    #[test]
    fn the_channel_speaks_nine_distinct_words() {
        let words = [
            PAUSE, RESUME, STOP, LOCAL, ENGINE, PROGRESS, DONE, ERROR, IDLE,
        ];
        let mut sorted = words.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), words.len());
        assert!(words.iter().all(|word| !word.is_empty()));
    }
}
