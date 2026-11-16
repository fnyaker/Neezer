//! The JSON-lines channel between the app and the supervisor — owned by T3.
//!
//! One JSON object per line in each direction, over the supervisor's stdin and
//! stdout (the note's *Control channel*). T1 pins the names, because three
//! places have to agree on them: the supervisor writes them, the app reads them
//! and shows a failure's cause (rule 2), and T4's settings screen switches on
//! `engine` and `error`.
//!
//! T3 pins the shapes. Both directions are serde enums tagged with the pinned
//! word (`{"cmd":"pause"}`, `{"event":"idle"}`), so a name in this file and a
//! name on the wire cannot drift apart, and an unknown line is a logged error
//! that does not kill the channel: the person may have run a newer app against
//! an older supervisor (rule 7).

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

use crate::engine::{Backend, FallbackCause};
use crate::segments::Segment;

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

/// App → supervisor: one of the four commands above.
///
/// `Local` carries the path of an archived audio file the app has already
/// resolved: the supervisor never reads a path from anywhere else, and a local
/// transcript writes to `$XDG_DATA_HOME/neezer/transcripts/<episode_id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// [`PAUSE`].
    Pause,
    /// [`RESUME`].
    Resume,
    /// [`STOP`].
    Stop,
    /// [`LOCAL`].
    Local {
        /// The episode, a UUID string.
        episode_id: String,
        /// The archived audio file.
        audio: String,
        /// `auto` (or absent) to let the engine decide.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
}

/// Supervisor → app: one of the five events above.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// [`ENGINE`]: which processor this run ended up on.
    Engine {
        /// `vulkan` or `cpu` ([`Backend::as_str`]).
        backend: String,
        /// The device whisper.cpp chose, when it says.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Why not the GPU ([`FallbackCause::code`]), when that is the answer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// [`PROGRESS`]: how far the current job is.
    Progress {
        /// The episode being transcribed.
        episode_id: String,
        /// Milliseconds of audio transcribed so far.
        done_ms: i64,
        /// The episode's duration.
        duration_ms: i64,
    },
    /// [`DONE`]: the transcript is whole.
    Done {
        /// The episode.
        episode_id: String,
        /// Its lines, so the app can show the text without reading a file.
        segments: Vec<Segment>,
    },
    /// [`ERROR`]: the cause the person reads (rule 2: never just a count).
    Error {
        /// The episode, or an empty string when no job was running.
        episode_id: String,
        /// The server's short code ([`crate::error::Error::code`]).
        code: String,
        /// The French sentence.
        message: String,
    },
    /// [`IDLE`]: the queue is empty and the supervisor waits.
    Idle,
}

impl Event {
    /// The [`Event::Engine`] line of a run that ended up on `backend`.
    pub fn engine(
        backend: Backend,
        device: Option<String>,
        reason: Option<FallbackCause>,
    ) -> Event {
        Event::Engine {
            backend: backend.as_str().to_owned(),
            device,
            reason: reason.map(|cause| cause.code().to_owned()),
        }
    }

    /// The [`Event::Error`] line of a job that failed.
    pub fn error(
        episode_id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Event {
        Event::Error {
            episode_id: episode_id.into(),
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Write one event as one JSON line, and flush it.
///
/// The flush is the contract: the app reads the pipe as a stream, and a line
/// that sits in a buffer is a progress bar that does not move (rule 2).
pub fn write_event<W: Write>(writer: &mut W, event: &Event) -> io::Result<()> {
    let mut line =
        serde_json::to_vec(event).map_err(|error| io::Error::other(error.to_string()))?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

/// Read one command, or `None` at the end of the channel.
///
/// A blank line is skipped (a shell or a test may leave one), and a line that is
/// not a command is an `InvalidData` error carrying the line: the caller logs it
/// and keeps reading, because an app one version ahead must not wedge a
/// supervisor (rule 7).
pub fn read_command<R: BufRead>(reader: &mut R) -> io::Result<Option<Command>> {
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            continue;
        }
        return match serde_json::from_str::<Command>(&line) {
            Ok(command) => Ok(Some(command)),
            Err(error) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ligne de contrôle illisible ({error}): {}", line.trim()),
            )),
        };
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        Command, DONE, ENGINE, ERROR, Event, IDLE, LOCAL, PAUSE, PROGRESS, RESUME, STOP,
        read_command, write_event,
    };
    use crate::engine::{Backend, FallbackCause};
    use crate::segments::Segment;

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

    /// Each command and each event carries the pinned word as its tag, exactly
    /// once: the app switches on `event`, the supervisor on `cmd`.
    #[test]
    fn every_variant_is_tagged_with_its_pinned_word() {
        let commands = [
            (Command::Pause, PAUSE),
            (Command::Resume, RESUME),
            (Command::Stop, STOP),
            (
                Command::Local {
                    episode_id: "0".repeat(36),
                    audio: "/tmp/a.mp3".to_owned(),
                    language: None,
                },
                LOCAL,
            ),
        ];
        for (command, word) in commands {
            let value = serde_json::to_value(&command).expect("serialize");
            assert_eq!(value["cmd"], word);
            let back: Command = serde_json::from_value(value).expect("deserialize");
            assert_eq!(back, command);
        }

        let events = [
            (
                Event::engine(Backend::Vulkan, Some("RADV".to_owned()), None),
                ENGINE,
            ),
            (
                Event::engine(Backend::Cpu, None, Some(FallbackCause::NoVulkanDevice)),
                ENGINE,
            ),
            (
                Event::Progress {
                    episode_id: "0".repeat(36),
                    done_ms: 600_000,
                    duration_ms: 3_600_000,
                },
                PROGRESS,
            ),
            (
                Event::Done {
                    episode_id: "0".repeat(36),
                    segments: vec![Segment::new(0, 1000, "Bonjour.")],
                },
                DONE,
            ),
            (
                Event::error("", "model_missing", "modèle introuvable"),
                ERROR,
            ),
            (Event::Idle, IDLE),
        ];
        for (event, word) in events {
            let value = serde_json::to_value(&event).expect("serialize");
            assert_eq!(value["event"], word);
            let back: Event = serde_json::from_value(value).expect("deserialize");
            assert_eq!(back, event);
        }
    }

    /// The CPU fallback travels with its cause as the server's code, and an
    /// engine that found a device names it.
    #[test]
    fn a_fallback_carries_its_cause_in_the_servers_code() {
        let cpu = Event::engine(Backend::Cpu, None, Some(FallbackCause::GpuOutOfMemory));
        let value = serde_json::to_value(&cpu).expect("serialize");
        assert_eq!(value["backend"], "cpu");
        assert_eq!(value["reason"], "gpu_out_of_memory");
        assert!(
            value.get("device").is_none(),
            "no device is not a null device"
        );
        let gpu = Event::engine(Backend::Vulkan, Some("AMD Radeon 780M".to_owned()), None);
        let value = serde_json::to_value(&gpu).expect("serialize");
        assert_eq!(value["device"], "AMD Radeon 780M");
        assert!(value.get("reason").is_none());
    }

    /// One event is one line, flushed: the app reads a stream of lines.
    #[test]
    fn an_event_is_one_flushed_line() {
        let mut out: Vec<u8> = Vec::new();
        write_event(&mut out, &Event::Idle).expect("write");
        write_event(
            &mut out,
            &Event::Progress {
                episode_id: "id".to_owned(),
                done_ms: 0,
                duration_ms: 1,
            },
        )
        .expect("write");
        let text = String::from_utf8(out).expect("UTF-8");
        assert_eq!(
            text,
            "{\"event\":\"idle\"}\n{\"event\":\"progress\",\"episode_id\":\"id\",\"done_ms\":0,\"duration_ms\":1}\n"
        );
    }

    /// Commands arrive one per line; a blank line is skipped and the end of the
    /// pipe is `None`, so a supervisor with no app attached does not spin.
    #[test]
    fn commands_are_read_line_by_line() {
        let mut input = Cursor::new(
            "\n{\"cmd\":\"pause\"}\n{\"cmd\":\"resume\"}\n{\"cmd\":\"local\",\"episode_id\":\"x\",\"audio\":\"/a.mp3\"}\n"
                .as_bytes()
                .to_vec(),
        );
        assert_eq!(
            read_command(&mut input).expect("pause"),
            Some(Command::Pause)
        );
        assert_eq!(
            read_command(&mut input).expect("resume"),
            Some(Command::Resume)
        );
        assert_eq!(
            read_command(&mut input).expect("local"),
            Some(Command::Local {
                episode_id: "x".to_owned(),
                audio: "/a.mp3".to_owned(),
                language: None,
            })
        );
        assert_eq!(read_command(&mut input).expect("eof"), None);
    }

    /// A line this supervisor does not know is an error carrying the line, not a
    /// panic and not a silent drop: the caller logs it and keeps reading.
    #[test]
    fn an_unknown_line_is_an_error_that_names_it() {
        let mut input = Cursor::new(b"{\"cmd\":\"teleport\"}\n".to_vec());
        let error = read_command(&mut input).expect_err("unknown command");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("teleport"), "{error}");
    }
}
