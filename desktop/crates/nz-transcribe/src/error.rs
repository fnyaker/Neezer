//! What can go wrong, under the server's own names for it.
//!
//! `neezer/transcribe/engine.py` and `docker/transcribe/worker.py` report a
//! failure as a short `code` ([`Error::code`]) plus a French sentence the person
//! reads ([`std::fmt::Display`]); `job.py#fail` stores the code and shows the
//! sentence. The local run keeps the same strings, so the same cause reads
//! identically whether the server or this GPU hit it, and a `{code, message}`
//! pair sent to the server needs no translation table.
//!
//! [`SidecarError`] is the other half: a transcript document that is not one.
//! It is deliberately separate — a sidecar is read off a disk and checked, never
//! reported to the server, and its messages are `sidecar.py`'s.

use thiserror::Error;

/// A transcription failure: the whisper binary, the model, the audio, the
/// engine's own exit.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// `engine.py#_check`: the configured binary does not exist or is not a file.
    #[error("moteur introuvable (whisper-cli)")]
    EngineMissing,
    /// `engine.py#_check`: the file is there but has no execute bit.
    #[error("le binaire whisper n'est pas exécutable")]
    EngineNotExecutable,
    /// `engine.py#probe`: the binary exists but `--version` failed.
    #[error("le binaire whisper ne répond pas")]
    EngineUnresponsive,
    /// `engine.py#_check`: no model file at the configured path.
    #[error("modèle introuvable")]
    ModelMissing,
    /// `engine.py#_check`: smaller than `MIN_MODEL_BYTES`, so not a model.
    #[error("modèle trop petit")]
    ModelTooSmall,
    /// `engine.py#_check`: no GGML magic in the first four bytes.
    #[error("modèle invalide (magie GGML absente)")]
    ModelInvalid,
    /// `engine.py#_check`: `ffmpeg` is neither configured nor on `PATH`.
    #[error("ffmpeg introuvable")]
    FfmpegMissing,
    /// `engine.py#parse_json`: the engine wrote no readable JSON.
    #[error("le JSON du moteur est illisible")]
    BadJson,
    /// `engine.py#parse_json`: the JSON is not an object.
    #[error("le JSON du moteur n'a pas la bonne forme")]
    BadJsonShape,
    /// `engine.py#parse_json`: no `transcription` list in it.
    #[error("le JSON du moteur n'a pas de transcription")]
    NoTranscription,
    /// `engine.py#extract_chunk`: the episode's audio could not be read.
    #[error("l'audio de l'épisode n'a pas pu être lu")]
    NoAudio,
    /// `engine.py#extract_chunk`: the extracted chunk is empty.
    #[error("l'extrait audio est vide")]
    EmptyAudio,
    /// The runner was asked to stop (`engine.Cancelled`).
    #[error("transcription annulée")]
    Cancelled,
    /// The engine outlived its deadline (`engine.TimedOut`).
    #[error("le moteur a dépassé son délai")]
    Timeout,
    /// The engine exited non-zero; the payload is [`ending`] plus the last line
    /// it wrote, when there was one.
    #[error("le moteur s'est arrêté : {0}")]
    Exit(String),
}

/// How a child process ended, in words: its exit code, or the signal that
/// killed it.
///
/// `ExitStatus::code()` answers `None` for a process a signal killed, and both
/// reports of a dead engine printed only that `None` — `code -1` in the
/// supervisor, `a échoué (None)` in the engine bench. A segfault inside ggml
/// (`SIGSEGV`), the kernel's OOM killer (`SIGKILL`), a driver's `abort()`
/// (`SIGABRT`) and the deadline's own `SIGTERM` were therefore the same line,
/// and the whisper log tail named none of them: CI run 44200808004 spent a red
/// `cargo test + window` on exactly that. The numbers are Linux's, like the
/// supervisor's `SIGSTOP`/`SIGTERM`/`SIGKILL` constants — the desktop app is
/// Linux-only (Omarchy).
pub fn ending(status: &std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;

    match (status.code(), status.signal()) {
        (Some(code), _) => format!("code {code}"),
        (None, Some(signal)) => format!("{} (signal {signal})", signal_name(signal)),
        // Both `None` cannot happen on Linux; say so rather than print `None`.
        (None, None) => "ni code ni signal".to_owned(),
    }
}

/// The name of a Linux signal, for [`ending`].
///
/// Only the ones a whisper child can plausibly die of are named; anything else
/// keeps its number, because a wrong name is worse than a number.
fn signal_name(signal: i32) -> &'static str {
    match signal {
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        15 => "SIGTERM",
        24 => "SIGXCPU",
        25 => "SIGXFSZ",
        _ => "signal inconnu",
    }
}

impl Error {
    /// The `code` the server stores and the UI switches on.
    pub fn code(&self) -> &'static str {
        match self {
            Error::EngineMissing | Error::EngineNotExecutable => "engine_missing",
            Error::EngineUnresponsive => "engine_error",
            Error::ModelMissing => "model_missing",
            // `engine.py#_check` reports both under `model_invalid`, the message
            // telling them apart ("trop petit" / "magie GGML absente").
            Error::ModelTooSmall | Error::ModelInvalid => "model_invalid",
            Error::FfmpegMissing => "ffmpeg_missing",
            Error::BadJson | Error::BadJsonShape | Error::NoTranscription => "bad_json",
            Error::NoAudio | Error::EmptyAudio => "no_audio",
            Error::Cancelled => "cancelled",
            Error::Timeout => "timeout",
            Error::Exit(_) => "exit",
        }
    }

    /// `(code, message)` exactly as `POST …/fail` sends them.
    pub fn failure(&self) -> (&'static str, String) {
        (self.code(), self.to_string())
    }
}

/// A `*.transcript.json` that is not a transcript this crate can trust.
///
/// The variants mirror `sidecar.py`'s checks one for one, in the same order and
/// with the same sentences: a document written by the server and read here (or
/// the reverse) must fail the same way on both sides.
#[derive(Debug, Error)]
pub enum SidecarError {
    /// The document is not a JSON object.
    #[error("le sidecar n'est pas un objet JSON")]
    NotAnObject,
    /// `format` is not [`crate::sidecar::FORMAT_VERSION`].
    #[error("version de sidecar inconnue")]
    UnknownVersion,
    /// No `segments` list (or it is not a list).
    #[error("le sidecar n'a pas de segments")]
    NoSegments,
    /// More than [`crate::sidecar::MAX_SEGMENTS`] of them.
    #[error("le sidecar a trop de segments")]
    TooManySegments,
    /// One entry of `segments` is not an object.
    #[error("un segment n'est pas un objet")]
    NotASegment,
    /// `start_ms` / `end_ms` / `duration_ms` is not a number.
    #[error("{what} n'est pas un nombre")]
    NotANumber { what: &'static str },
    /// … or a negative one.
    #[error("{what} est négatif")]
    Negative { what: &'static str },
    /// `end_ms <= start_ms`.
    #[error("un segment se termine avant de commencer")]
    EndBeforeStart,
    /// `text` is not a string.
    #[error("un segment n'a pas de texte")]
    NoText,
    /// `text` is empty once cleaned.
    #[error("un segment a un texte vide")]
    EmptyText,
    /// The file could not be read.
    #[error("sidecar illisible")]
    Unreadable(#[source] std::io::Error),
    /// Larger than [`crate::sidecar::MAX_SIDECAR_BYTES`].
    #[error("sidecar trop gros")]
    TooBig,
    /// Not JSON, or not UTF-8.
    #[error("sidecar JSON invalide")]
    InvalidJson(#[source] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::{Error, SidecarError};

    /// The server's codes are a wire contract: `job.py#fail` stores this string
    /// and the SPA's transcript panel switches on it.
    #[test]
    fn every_failure_carries_the_server_code() {
        let codes = [
            (Error::EngineMissing, "engine_missing"),
            (Error::EngineNotExecutable, "engine_missing"),
            (Error::EngineUnresponsive, "engine_error"),
            (Error::ModelMissing, "model_missing"),
            (Error::ModelTooSmall, "model_invalid"),
            (Error::ModelInvalid, "model_invalid"),
            (Error::FfmpegMissing, "ffmpeg_missing"),
            (Error::BadJson, "bad_json"),
            (Error::BadJsonShape, "bad_json"),
            (Error::NoTranscription, "bad_json"),
            (Error::NoAudio, "no_audio"),
            (Error::EmptyAudio, "no_audio"),
            (Error::Cancelled, "cancelled"),
            (Error::Timeout, "timeout"),
            (Error::Exit("code 9".to_owned()), "exit"),
        ];
        for (error, code) in codes {
            assert_eq!(error.code(), code, "{error}");
            assert_eq!(error.failure().0, code);
            assert!(!error.failure().1.is_empty());
        }
    }

    /// A model too small and a model without the GGML magic share the server's
    /// `model_invalid` code but not the sentence the person reads.
    #[test]
    fn a_failure_says_which_of_its_causes_it_was() {
        assert_eq!(Error::ModelTooSmall.code(), Error::ModelInvalid.code());
        assert_ne!(
            Error::ModelTooSmall.to_string(),
            Error::ModelInvalid.to_string()
        );
        assert_eq!(
            SidecarError::TooManySegments.to_string(),
            "le sidecar a trop de segments"
        );
        assert_eq!(
            SidecarError::NotANumber { what: "end_ms" }.to_string(),
            "end_ms n'est pas un nombre"
        );
    }

    /// A child a signal killed says WHICH signal: `code()` answers `None` for it
    /// and the reports printed that `None`, so a segfault in ggml, the kernel's
    /// OOM killer and a driver's `abort()` were one indistinguishable line (CI
    /// run 44200808004, 4 tests red on it).
    ///
    /// Measured: `/bin/sh -c 'kill -9 $$'` answers `SIGKILL (signal 9)` in one
    /// spawn, and `exit 3` answers `code 3` — the two halves of `ending`.
    #[test]
    fn a_dead_child_names_its_signal() {
        let killed = std::process::Command::new("/bin/sh")
            .args(["-c", "kill -9 $$"])
            .output()
            .expect("spawn");
        assert_eq!(super::ending(&killed.status), "SIGKILL (signal 9)");

        let aborted = std::process::Command::new("/bin/sh")
            .args(["-c", "kill -ABRT $$"])
            .output()
            .expect("spawn");
        assert_eq!(super::ending(&aborted.status), "SIGABRT (signal 6)");

        let exited = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .output()
            .expect("spawn");
        assert_eq!(super::ending(&exited.status), "code 3");
    }
}
