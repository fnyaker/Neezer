//! The Rust port of the server's segment and sidecar rules, held to the Python
//! functions byte for byte.
//!
//! Every directory under `tests/golden/` is one case: `case.json` is what
//! whisper.cpp emitted for a chunk, where that chunk starts, and the metadata of
//! the transcript it belongs to. The three expected files were produced by
//! `desktop/tools/transcribe_golden.py` — that is, by
//! `neezer/transcribe/engine.py` and `neezer/transcribe/sidecar.py`, the code the
//! server actually runs — and never by the code under test, which would only
//! prove this crate agrees with itself.
//!
//! Regenerate with `python3 desktop/tools/transcribe_golden.py`, check with
//! `--check` (what the desktop job runs: a change to the Python rules then fails
//! here instead of drifting away silently). 16 cases, 62 files.

use std::fs;
use std::path::{Path, PathBuf};

use nz_transcribe::segments::{OVERLAP_MS, Segment, normalize_segments};
use nz_transcribe::sidecar::{self, Meta};
use serde::Deserialize;
use serde_json::Value;

/// The committed cases.
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");

/// The acceptance floor: the lot asks for at least twelve cases, and they must
/// cover the overlap cutoff, clamping, a tag-only line, control characters, the
/// 2 000-segment cap, an empty chunk and Unicode (see the case names).
const MIN_CASES: usize = 12;

#[derive(Debug, Deserialize)]
struct Case {
    /// The raw `transcription` array of whisper.cpp's `-oj` JSON.
    items: Vec<Value>,
    /// Where the chunk starts in the episode, in milliseconds.
    chunk_start_ms: i64,
    /// How long it is, in milliseconds.
    chunk_duration_ms: i64,
    /// The part of it the previous chunk already emitted.
    #[serde(default = "default_overlap")]
    overlap_ms: i64,
    /// The document's metadata, or `null` for a case that only checks lines.
    #[serde(default)]
    sidecar: Option<Meta>,
    /// Raw lines the document is built from, when the case exercises the
    /// sidecar's own cleaning instead of the engine's.
    #[serde(default)]
    sidecar_segments: Option<Vec<Segment>>,
}

fn default_overlap() -> i64 {
    OVERLAP_MS
}

/// Every case directory, in name order, so a failure names the case that broke.
fn case_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = fs::read_dir(GOLDEN)
        .unwrap_or_else(|error| panic!("{GOLDEN}: {error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("case.json").is_file())
        .collect();
    directories.sort();
    directories
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn assert_bytes(expected: &[u8], actual: &[u8], path: &Path) {
    assert_eq!(
        String::from_utf8_lossy(actual),
        String::from_utf8_lossy(expected),
        "{} differs from what the Python functions produce",
        path.display()
    );
    assert_eq!(actual, expected, "{} differs in bytes only", path.display());
}

/// `normalize_segments` and `Sidecar::build` reproduce the Python documents
/// exactly: same values, same order, same bytes.
#[test]
fn the_rust_port_is_byte_equal_to_the_python_functions() {
    let directories = case_directories();
    assert!(
        directories.len() >= MIN_CASES,
        "{} golden cases, at least {MIN_CASES} expected",
        directories.len()
    );
    let mut documents = 0;
    for directory in &directories {
        let case: Case = serde_json::from_slice(&read(&directory.join("case.json")))
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()));
        let segments = normalize_segments(
            &case.items,
            case.chunk_start_ms,
            case.chunk_duration_ms,
            case.overlap_ms,
        );
        let mut expected = serde_json::to_vec(&segments).expect("segments serialize");
        expected.push(b'\n');
        assert_bytes(
            &expected,
            &read(&directory.join("segments.json")),
            &directory.join("segments.json"),
        );
        let Some(meta) = case.sidecar else {
            continue;
        };
        documents += 1;
        let lines = case.sidecar_segments.unwrap_or(segments);
        let document = sidecar::build(meta, &lines).unwrap_or_else(|error| {
            panic!(
                "{}: a case's sidecar does not build: {error}",
                directory.display()
            )
        });
        assert_bytes(
            &sidecar::dumps(&document),
            &read(&directory.join("transcript.json")),
            &directory.join("transcript.json"),
        );
        assert_bytes(
            &sidecar::srt_bytes(&document.segments),
            &read(&directory.join("transcript.srt")),
            &directory.join("transcript.srt"),
        );
    }
    // Measured, and stated so a case that stops building a document is visible:
    // 16 cases, 15 of which produce a transcript and its subtitles.
    assert_eq!(directories.len(), 16);
    assert_eq!(documents, 15);
}

/// Every case directory is complete: a case with a document has both of its
/// files, and one without has neither (a stale `transcript.srt` from a case that
/// dropped its sidecar would otherwise never be read by anything).
#[test]
fn every_case_directory_holds_what_its_case_asks_for() {
    for directory in case_directories() {
        let case: Case = serde_json::from_slice(&read(&directory.join("case.json")))
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()));
        assert!(
            directory.join("segments.json").is_file(),
            "{}",
            directory.display()
        );
        let has_json = directory.join("transcript.json").is_file();
        let has_srt = directory.join("transcript.srt").is_file();
        assert_eq!(
            has_json,
            case.sidecar.is_some(),
            "{}: transcript.json is not the one the case asks for",
            directory.display()
        );
        assert_eq!(
            has_srt,
            case.sidecar.is_some(),
            "{}: transcript.srt is not the one the case asks for",
            directory.display()
        );
    }
}
