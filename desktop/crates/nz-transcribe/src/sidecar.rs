//! The transcript document, byte for byte the server's.
//!
//! `<episode>.transcript.json` is what a transcription *is*
//! (`neezer/transcribe/sidecar.py`): format version, episode, language, engine
//! and model, duration, creation and update instants, and the timed lines.
//! `<episode>.transcript.srt` is the same text as subtitles, always regenerated
//! from the document. Both are written to a temporary file in the same
//! directory and renamed, so a crash or a full disk leaves the previous copy
//! intact instead of half a JSON file.
//!
//! The bytes matter, not just the values: an episode transcribed on this laptop
//! lands in the same archive and the same database as one transcribed on the
//! server, the SPA re-reads both with `json.load`, and a diff of the two files
//! must be empty. Hence `dumps` writes what Python's
//! `json.dumps(ensure_ascii=False, separators=(",", ":")) + "\n"` writes: the
//! same key order (this struct's field order is `sidecar.normalize`'s insertion
//! order), no space after `:` or `,`, raw UTF-8, and one trailing newline. The
//! golden files under `tests/golden/` compare the bytes.
//!
//! Two fields are deliberately typed here where Python passes anything through
//! (`language`, `engine`, `model`, `engine_version`, `created`, `updated`, and
//! `episode_id`): this crate accepts a string or null, and a number for
//! `episode_id`; a producer that writes, say, `"language": 3` gets `null`, where
//! Python would copy the number. No producer does — the server's columns are
//! text — and a typed field is what lets the app show the language without
//! re-checking it at every use.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::SidecarError;
use crate::segments::{MAX_TEXT, Segment};

/// The only document version this crate reads or writes (`sidecar.FORMAT_VERSION`).
pub const FORMAT_VERSION: u32 = 1;

/// Segments one document may hold (`sidecar.MAX_SEGMENTS`): ten hours of dense
/// speech. Beyond that the file is not a transcript but a denial of service.
pub const MAX_SEGMENTS: usize = 100_000;

/// Bytes one document may take (`sidecar.MAX_SIDECAR_BYTES`): an hour of speech
/// is about 100 kB, so 8 MiB is a hundred hours of margin.
pub const MAX_SIDECAR_BYTES: u64 = 8 * 1024 * 1024;

/// Everything a document says besides its lines: `sidecar.build`'s arguments.
///
/// Field for field what `job.sidecar_payload` passes the Python builder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    /// The episode's UUID as a string (never parsed to a number).
    pub episode_id: String,
    /// The detected or forced language, `None` when nobody knows it yet.
    pub language: Option<String>,
    /// [`crate::engine::FORMAT_ENGINE`] in practice.
    pub engine: Option<String>,
    /// The model's file name, e.g. `ggml-large-v3-turbo-q8_0.bin`.
    pub model: Option<String>,
    /// The engine's own version string, cut at 32 characters by the server.
    pub engine_version: Option<String>,
    /// The episode's duration in milliseconds.
    pub duration_ms: i64,
    /// ISO-8601, naive local time, as `job.sidecar_payload` writes it.
    pub created: Option<String>,
    /// Same, for the last chunk written.
    pub updated: Option<String>,
}

/// A transcript document, in the shape `sidecar.normalize` returns.
///
/// The field order is the serialization order on purpose: it is the order
/// `sidecar.normalize` inserts its keys in, and the golden files are compared
/// byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    /// Always [`FORMAT_VERSION`].
    pub format: u32,
    /// The episode this text belongs to, as a UUID string.
    pub episode_id: String,
    /// Language of the speech, `None` when the engine did not say.
    pub language: Option<String>,
    /// The engine's name, `whisper.cpp` for ours.
    pub engine: Option<String>,
    /// The model file the engine ran.
    pub model: Option<String>,
    /// That engine's version string.
    pub engine_version: Option<String>,
    /// The episode's duration in milliseconds.
    pub duration_ms: i64,
    /// When the transcription started.
    pub created: Option<String>,
    /// When it last changed.
    pub updated: Option<String>,
    /// The lines, sorted by start then end.
    pub segments: Vec<Segment>,
}

impl Sidecar {
    /// The metadata of this document, for a caller that replaces its lines.
    pub fn meta(&self) -> Meta {
        Meta {
            episode_id: self.episode_id.clone(),
            language: self.language.clone(),
            engine: self.engine.clone(),
            model: self.model.clone(),
            engine_version: self.engine_version.clone(),
            duration_ms: self.duration_ms,
            created: self.created.clone(),
            updated: self.updated.clone(),
        }
    }
}

/// Build a document from its metadata and its lines, then validate it.
///
/// The Rust half of `sidecar.build` → `sidecar.normalize`: the same cleaning and
/// the same sorting, so a caller cannot write a document the server would reject.
pub fn build(meta: Meta, segments: &[Segment]) -> Result<Sidecar, SidecarError> {
    let payload = serde_json::json!({
        "format": FORMAT_VERSION,
        "episode_id": meta.episode_id,
        "language": meta.language,
        "engine": meta.engine,
        "model": meta.model,
        "engine_version": meta.engine_version,
        "duration_ms": meta.duration_ms,
        "created": meta.created,
        "updated": meta.updated,
        "segments": segments,
    });
    normalize(&payload)
}

/// Validate a document and return it with its lines cleaned and sorted.
///
/// `sidecar.normalize`, one check for one check and in the same order: an
/// unknown version, no `segments` list, too many of them, a line that is not an
/// object, a timestamp that is not a non-negative number, an end at or before
/// its start, a text that is missing or empty once clean — each one a named
/// [`SidecarError`].
pub fn normalize(payload: &Value) -> Result<Sidecar, SidecarError> {
    let object = payload.as_object().ok_or(SidecarError::NotAnObject)?;
    if !is_format_one(object.get("format")) {
        return Err(SidecarError::UnknownVersion);
    }
    let list = object
        .get("segments")
        .and_then(Value::as_array)
        .ok_or(SidecarError::NoSegments)?;
    if list.len() > MAX_SEGMENTS {
        return Err(SidecarError::TooManySegments);
    }
    let mut segments = Vec::with_capacity(list.len());
    for item in list {
        let item = item.as_object().ok_or(SidecarError::NotASegment)?;
        let start = milliseconds(item.get("start_ms"), "start_ms")?;
        let end = milliseconds(item.get("end_ms"), "end_ms")?;
        if end <= start {
            return Err(SidecarError::EndBeforeStart);
        }
        segments.push(Segment {
            start_ms: start,
            end_ms: end,
            text: clean_segment_text(item.get("text"))?,
        });
    }
    segments.sort_by_key(|segment| (segment.start_ms, segment.end_ms));
    Ok(Sidecar {
        format: FORMAT_VERSION,
        episode_id: episode_id(object.get("episode_id")),
        language: text_field(object.get("language")),
        engine: text_field(object.get("engine")),
        model: text_field(object.get("model")),
        engine_version: text_field(object.get("engine_version")),
        duration_ms: duration_ms(object.get("duration_ms"))?,
        created: text_field(object.get("created")),
        updated: text_field(object.get("updated")),
        segments,
    })
}

/// The document as the bytes on disk: compact JSON, UTF-8, one trailing newline.
///
/// `sidecar.dumps`, which is `json.dumps(…, ensure_ascii=False, sort_keys=False,
/// separators=(",", ":")) + "\n"` encoded as UTF-8.
pub fn dumps(document: &Sidecar) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(document)
        .expect("a sidecar is strings, integers and a list of segments");
    bytes.push(b'\n');
    bytes
}

/// Milliseconds as an SRT timestamp: `HH:MM:SS,mmm`, hours unbounded.
pub fn srt_time(milliseconds: i64) -> String {
    let total = milliseconds.max(0);
    let hours = total / 3_600_000;
    let minutes = (total % 3_600_000) / 60_000;
    let seconds = (total % 60_000) / 1000;
    let millis = total % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

/// The document's lines as SRT subtitles: `sidecar.srt_bytes`.
///
/// Cue `n` covers `[start_ms, max(start_ms + 1, end_ms))`, so a one-millisecond
/// line is still a line; an empty document is zero bytes (not a lone newline),
/// which is what makes "no transcript" and "a transcript with no line" the same
/// absence on disk.
pub fn srt_bytes(segments: &[Segment]) -> Vec<u8> {
    if segments.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::with_capacity(segments.len() * 4);
    for (index, segment) in segments.iter().enumerate() {
        let start = segment.start_ms;
        let end = segment.end_ms.max(start + 1);
        lines.push((index + 1).to_string());
        lines.push(format!("{} --> {}", srt_time(start), srt_time(end)));
        lines.push(segment.text.clone());
        lines.push(String::new());
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out.into_bytes()
}

/// The two files one transcript is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// `<base>.transcript.json`
    pub json: PathBuf,
    /// `<base>.transcript.srt`
    pub srt: PathBuf,
}

/// `<base>.transcript.json`: the last extension is replaced, as Python's
/// `os.path.splitext` splits it.
///
/// `base` is an archived audio path (the server's naming) or a bare episode id
/// (the local store's, `$XDG_DATA_HOME/neezer/transcripts/<episode_id>`).
pub fn json_path(base: &Path) -> PathBuf {
    base.with_extension("transcript.json")
}

/// `<base>.transcript.srt`, next to [`json_path`].
pub fn srt_path(base: &Path) -> PathBuf {
    base.with_extension("transcript.srt")
}

/// Both sidecar paths of one episode.
pub fn pair_for(base: &Path) -> Pair {
    Pair {
        json: json_path(base),
        srt: srt_path(base),
    }
}

/// Write the document and its SRT, each atomically, JSON first.
///
/// A temporary file in the same directory, fsynced, renamed, and the directory
/// fsynced: after a crash the file is either the previous transcript or the new
/// one, never a truncated JSON. The SRT follows the JSON, so a reader that sees
/// a new SRT always has the JSON that produced it.
pub fn write(pair: &Pair, document: &Sidecar) -> io::Result<()> {
    let json = dumps(document);
    let srt = srt_bytes(&document.segments);
    write_atomic(&pair.json, &json)?;
    write_atomic(&pair.srt, &srt)
}

/// Read and validate a document off the disk.
///
/// The size cap is checked before the read, so a hostile 4 GiB file costs one
/// `stat` and not four gigabytes of memory.
pub fn load(path: &Path) -> Result<Sidecar, SidecarError> {
    let metadata = fs::metadata(path).map_err(SidecarError::Unreadable)?;
    if metadata.len() > MAX_SIDECAR_BYTES {
        return Err(SidecarError::TooBig);
    }
    let bytes = fs::read(path).map_err(SidecarError::Unreadable)?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(SidecarError::TooBig);
    }
    let payload: Value = serde_json::from_slice(&bytes).map_err(SidecarError::InvalidJson)?;
    normalize(&payload)
}

/// Remove a transcript's two files, never the audio; a missing one is fine.
pub fn remove(pair: &Pair) -> io::Result<()> {
    for path in [&pair.json, &pair.srt] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// One temporary name per write in this process: two writers on the same
/// directory (a resumed job and a fresh one) must not share a `.part` file.
fn temporary_name() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)?;
    let (temporary, mut file) = create_temporary(directory)?;
    let written = file.write_all(data).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    // The rename itself is metadata: without this fsync a power cut can leave
    // the old file in place. Best effort — a directory that cannot be opened
    // must not turn a written transcript into a failure.
    if let Ok(handle) = fs::File::open(directory) {
        let _ = handle.sync_all();
    }
    Ok(())
}

/// A fresh temporary file in `directory`, created exclusively.
///
/// `create_new` rather than `create`: a `.part` name that already exists is
/// skipped instead of followed, so a symlink planted in the directory cannot
/// make this write land somewhere else.
fn create_temporary(directory: &Path) -> io::Result<(PathBuf, fs::File)> {
    loop {
        let candidate = directory.join(format!(
            ".transcript-{}-{}.part",
            std::process::id(),
            temporary_name()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// `payload.get("format") != 1`, with Python's `True == 1` honoured: the check
/// is about the version, not about the JSON type someone wrote.
fn is_format_one(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Number(number)) => number.as_f64() == Some(f64::from(FORMAT_VERSION)),
        Some(Value::Bool(flag)) => *flag,
        _ => false,
    }
}

/// `sidecar._clean_text`: control characters and DEL become spaces, whitespace
/// runs collapse, an empty text is an error, and 2 000 characters is the cut.
///
/// Note what this does NOT do, unlike the engine's `clean_text`: it keeps a line
/// that is only a tag. `[Musique]` in a document written by the server is text
/// the person can search for; only whisper's raw output drops it.
fn clean_segment_text(raw: Option<&Value>) -> Result<String, SidecarError> {
    let raw = raw.and_then(Value::as_str).ok_or(SidecarError::NoText)?;
    let mut replaced = String::with_capacity(raw.len());
    for character in raw.chars() {
        if (character as u32) < 0x20 || character == '\u{7f}' {
            replaced.push(' ');
        } else {
            replaced.push(character);
        }
    }
    let collapsed = replaced.split_whitespace().collect::<Vec<&str>>().join(" ");
    if collapsed.is_empty() {
        return Err(SidecarError::EmptyText);
    }
    if collapsed.chars().count() > MAX_TEXT {
        let cut: String = collapsed.chars().take(MAX_TEXT).collect();
        return Ok(cut.trim().to_owned());
    }
    Ok(collapsed)
}

/// `sidecar._ms`: a JSON number as whole non-negative milliseconds, half to
/// even (`round()`), with a bool rejected as the non-number Python says it is.
fn milliseconds(value: Option<&Value>, what: &'static str) -> Result<i64, SidecarError> {
    let number = value
        .and_then(Value::as_f64)
        .ok_or(SidecarError::NotANumber { what })?;
    if !number.is_finite() {
        return Err(SidecarError::NotANumber { what });
    }
    let rounded = number.round_ties_even() as i64;
    if rounded < 0 {
        return Err(SidecarError::Negative { what });
    }
    Ok(rounded)
}

/// `sidecar.normalize`'s `duration_ms`: `payload.get("duration_ms") or 0`, then
/// the millisecond rules. Falsy (`None`, `false`, `0`, `""`, `[]`, `{}`) is 0.
fn duration_ms(value: Option<&Value>) -> Result<i64, SidecarError> {
    match value {
        None => Ok(0),
        Some(value) if falsy(value) => Ok(0),
        Some(value) => milliseconds(Some(value), "duration_ms"),
    }
}

fn falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
    }
}

/// `str(payload.get("episode_id") or "")`: the id is text on the wire, and a
/// number is rendered as itself rather than dropped.
fn episode_id(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

/// A metadata field: a string, or `None`.
fn text_field(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        FORMAT_VERSION, Meta, build, dumps, json_path, load, pair_for, remove, srt_bytes, srt_time,
    };
    use crate::error::SidecarError;
    use crate::segments::Segment;

    fn meta() -> Meta {
        Meta {
            episode_id: "0".repeat(36),
            language: Some("fr".to_owned()),
            engine: Some("whisper.cpp".to_owned()),
            model: Some("ggml-small.bin".to_owned()),
            engine_version: Some("1.9.5".to_owned()),
            duration_ms: 3_600_000,
            created: Some("2026-11-14T10:00:00".to_owned()),
            updated: Some("2026-11-14T10:01:00".to_owned()),
        }
    }

    fn segments() -> Vec<Segment> {
        vec![
            Segment::new(0, 6000, "Bonjour à tous."),
            Segment::new(6000, 12_000, "Deuxième phrase."),
        ]
    }

    /// The exact bytes Python writes: compact, key order as in `normalize`, raw
    /// UTF-8, one trailing newline.
    #[test]
    fn the_document_is_pythons_bytes() {
        let document = build(meta(), &segments()).expect("a valid document");
        let text = String::from_utf8(dumps(&document)).expect("UTF-8");
        assert_eq!(
            text,
            format!(
                "{{\"format\":{FORMAT_VERSION},\"episode_id\":\"{}\",\"language\":\"fr\",\
                 \"engine\":\"whisper.cpp\",\"model\":\"ggml-small.bin\",\"engine_version\":\"1.9.5\",\
                 \"duration_ms\":3600000,\"created\":\"2026-11-14T10:00:00\",\
                 \"updated\":\"2026-11-14T10:01:00\",\"segments\":[\
                 {{\"start_ms\":0,\"end_ms\":6000,\"text\":\"Bonjour à tous.\"}},\
                 {{\"start_ms\":6000,\"end_ms\":12000,\"text\":\"Deuxième phrase.\"}}]}}\n",
                "0".repeat(36)
            )
        );
        assert!(text.ends_with("\n"));
        assert!(!text.contains(": "));
    }

    /// An hour of speech is about 100 kB of document, as the server's test says
    /// (`tests/test_transcribe.py#test_an_hour_of_text_is_about_two_hundred_kilobytes`);
    /// measured here: 100 695 bytes of JSON, 93 492 bytes of SRT for 600 lines of
    /// six seconds. The bounds are the server's, so the two ports cannot drift.
    #[test]
    fn an_hour_of_speech_is_about_a_hundred_kilobytes() {
        let line = "Ceci est une phrase française de podcast d'environ quatre-vingt-dix \
                    caractères, avec des accents et de la ponctuation.";
        let segments: Vec<Segment> = (0..600)
            .map(|i| Segment::new(i * 6000, (i + 1) * 6000, line))
            .collect();
        let document = build(meta(), &segments).expect("a valid document");
        let json = dumps(&document).len();
        let srt = srt_bytes(&document.segments).len();
        assert!((100_000..250_000).contains(&json), "{json} bytes");
        assert!(srt < 300_000, "{srt} bytes of SRT");
    }

    /// SRT cues: `HH:MM:SS,mmm`, numbered from 1, separated by a blank line, one
    /// trailing newline; an end at or before the start is widened to 1 ms.
    #[test]
    fn the_subtitles_are_numbered_cues() {
        assert_eq!(srt_time(0), "00:00:00,000");
        assert_eq!(srt_time(3_723_004), "01:02:03,004");
        assert_eq!(srt_time(-5), "00:00:00,000");
        assert_eq!(srt_bytes(&[]), b"");
        let one = srt_bytes(&[Segment::new(1000, 1000, "une")]);
        assert_eq!(
            String::from_utf8(one).expect("UTF-8"),
            "1\n00:00:01,000 --> 00:00:01,001\nune\n\n"
        );
    }

    /// A document with a tag-only line keeps it: the sidecar is the server's
    /// text, not whisper's raw output, and `[Musique]` there is searchable text.
    /// Control characters and DEL still become spaces, and 2 000 is the cut.
    #[test]
    fn a_documents_text_is_cleaned_but_its_tags_stay() {
        let raw = serde_json::json!({
            "format": 1,
            "episode_id": "0".repeat(36),
            "duration_ms": 12_000.0,
            "created": null,
            "segments": [
                {"start_ms": 0, "end_ms": 1000, "text": "[Musique]"},
                {"start_ms": 1000, "end_ms": 2000, "text": " a\u{1}\tb \u{7f}c "},
                {"start_ms": 2000, "end_ms": 3000, "text": "z".repeat(2500)},
            ],
        });
        let document = super::normalize(&raw).expect("a valid document");
        assert_eq!(document.duration_ms, 12_000);
        assert_eq!(document.created, None);
        assert_eq!(document.segments[0].text, "[Musique]");
        assert_eq!(document.segments[1].text, "a b c");
        assert_eq!(document.segments[2].text.chars().count(), 2000);
    }

    /// Every check `sidecar.normalize` makes is a named error, in its order.
    #[test]
    fn a_document_that_is_not_one_says_which_rule_it_broke() {
        let base = |value: serde_json::Value| super::normalize(&value).expect_err("refused");
        let cases: Vec<(serde_json::Value, SidecarError)> = vec![
            (serde_json::json!([1, 2]), SidecarError::NotAnObject),
            (
                serde_json::json!({"format": 2, "segments": []}),
                SidecarError::UnknownVersion,
            ),
            (serde_json::json!({"format": 1}), SidecarError::NoSegments),
            (
                serde_json::json!({"format": 1, "segments": [42]}),
                SidecarError::NotASegment,
            ),
            (
                serde_json::json!({"format": 1, "segments": [{"start_ms": "0", "end_ms": 1, "text": "x"}]}),
                SidecarError::NotANumber { what: "start_ms" },
            ),
            (
                serde_json::json!({"format": 1, "segments": [{"start_ms": -1, "end_ms": 1, "text": "x"}]}),
                SidecarError::Negative { what: "start_ms" },
            ),
            (
                serde_json::json!({"format": 1, "segments": [{"start_ms": 5, "end_ms": 5, "text": "x"}]}),
                SidecarError::EndBeforeStart,
            ),
            (
                serde_json::json!({"format": 1, "segments": [{"start_ms": 0, "end_ms": 5}]}),
                SidecarError::NoText,
            ),
            (
                serde_json::json!({"format": 1, "segments": [{"start_ms": 0, "end_ms": 5, "text": "  "}]}),
                SidecarError::EmptyText,
            ),
        ];
        for (value, expected) in cases {
            let error = base(value);
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&expected),
                "{error} for {expected}"
            );
        }
        // 100 001 segments: refused before any of them is read.
        let many = serde_json::json!({
            "format": 1,
            "segments": (0..100_001)
                .map(|i| serde_json::json!({"start_ms": i, "end_ms": i + 1, "text": "x"}))
                .collect::<Vec<_>>(),
        });
        assert!(matches!(
            super::normalize(&many),
            Err(SidecarError::TooManySegments)
        ));
    }

    /// A write then a read gives the same document back, and no `.part` file is
    /// left behind — the property the server's own sidecar test checks.
    #[test]
    fn a_document_survives_a_round_trip_and_leaves_no_temporary_file() {
        let directory = scratch();
        let audio = directory.join("Episode.mp3");
        let pair = pair_for(&audio);
        assert_eq!(pair.json, directory.join("Episode.transcript.json"));
        assert_eq!(pair.srt, directory.join("Episode.transcript.srt"));
        assert_eq!(
            json_path(Path::new("0".repeat(36).as_str())),
            PathBuf::from(format!("{}.transcript.json", "0".repeat(36)))
        );

        let document = build(meta(), &segments()).expect("a valid document");
        super::write(&pair, &document).expect("write");
        assert_eq!(load(&pair.json).expect("load"), document);
        let leftovers: Vec<_> = std::fs::read_dir(&directory)
            .expect("read the directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".part"))
            .collect();
        assert_eq!(leftovers, Vec::<String>::new());
        remove(&pair).expect("remove");
        assert!(!pair.json.exists());
        assert!(!pair.srt.exists());
        // Removing what is not there is not a failure: a retried job removes twice.
        remove(&pair).expect("remove twice");
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A write that cannot land removes its temporary file: a `.part` left next
    /// to the transcript would be a leak per failed chunk. The JSON lands first,
    /// so a reader never pairs a new SRT with an old document.
    #[test]
    fn a_failed_write_leaves_no_part_file() {
        let directory = scratch();
        let pair = pair_for(&directory.join("Episode.mp3"));
        let document = build(meta(), &segments()).expect("a valid document");
        // A non-empty directory where the SRT must be: the JSON is written, the
        // SRT's rename cannot happen.
        std::fs::create_dir_all(pair.srt.join("blocker")).expect("blocker");
        assert!(super::write(&pair, &document).is_err());
        assert!(pair.json.is_file());
        let leftovers: Vec<_> = std::fs::read_dir(&directory)
            .expect("read the directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".part"))
            .collect();
        assert_eq!(leftovers, Vec::<String>::new());
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A file too big is refused on its `stat`, before it is read.
    #[test]
    fn a_sidecar_over_the_cap_is_refused_without_being_read() {
        let directory = scratch();
        let path = directory.join("huge.transcript.json");
        std::fs::write(&path, vec![b' '; (super::MAX_SIDECAR_BYTES + 1) as usize]).expect("write");
        assert!(matches!(load(&path), Err(SidecarError::TooBig)));
        std::fs::write(&path, b"{nope").expect("write");
        assert!(matches!(load(&path), Err(SidecarError::InvalidJson(_))));
        assert!(matches!(
            load(&directory.join("absent.json")),
            Err(SidecarError::Unreadable(_))
        ));
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A scratch directory of this test's own (tests run in parallel).
    fn scratch() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nz-transcribe-sidecar-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("scratch directory");
        path
    }
}
