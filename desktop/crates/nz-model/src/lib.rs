//! The server's JSON as Rust types — serde only, tolerant by construction.
//!
//! Owned by lot D1 (`docs/agents/desktop.md`): every payload the app reads has a
//! struct here, and every optional field is `#[serde(default)]` so an older or a
//! newer server still parses (rule 7). No I/O, no network, no database: the HTTP
//! client (`nz-api`, D2) only ever hands these types strings to deserialize.
//!
//! The types are proven against the REAL server's answers, not against hand
//! written samples: `desktop/tools/dump_fixtures.py` starts `tools/perf_api.py`,
//! reads one body per read route of `webapp/src/lib/api.js` into
//! `tests/fixtures/*.json`, and `tests/fixtures.rs` deserializes every file with
//! the type named in its table — including once with an extra unknown field and
//! once with every field the model does not truly need removed.
//!
//! Two rules that the payloads themselves impose:
//!
//! - **Ids are strings, never numbers.** A track is addressed by its `deezer_id`
//!   (a Deezer number, or a UUID for uploads, YouTube rows and podcast
//!   episodes); parsing that into an integer would break uploads. See [`ids`].
//! - **Remote URLs stay relative.** `cover` and `picture` are paths like
//!   `/api/cover/<id>`; the base URL is the client's business.

pub mod admin;
pub mod album;
pub mod artist;
pub mod discovery;
pub mod ids;
pub mod lan;
pub mod library;
pub mod party;
pub mod player;
pub mod playlist;
pub mod podcast;
pub mod remote;
pub mod search;
pub mod settings;
pub mod track;
pub mod transcript;

pub use admin::*;
pub use album::*;
pub use artist::*;
pub use discovery::*;
pub use ids::*;
pub use lan::*;
pub use library::*;
pub use party::*;
pub use player::*;
pub use playlist::*;
pub use podcast::*;
pub use remote::*;
pub use search::*;
pub use settings::*;
pub use track::*;
pub use transcript::*;

/// Deserializers for the fields whose JSON type changes with the row's origin.
///
/// One measured case: an album's `year` is a four-character STRING when the card
/// comes from a browse page (`_album_api`, `release_date[:4]`) and an INTEGER
/// when it comes from the database (`_db_album_card`, `MIN(track.year)`). A
/// desktop that typed it `String` would fail to parse half the grids.
pub(crate) mod loose {
    use serde::{Deserialize, Deserializer};

    /// A string, a number or a boolean as `Some(String)`; `null` as `None`.
    pub(crate) fn opt_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<serde_json::Value>::deserialize(deserializer)?;
        Ok(match value {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => Some(s),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            Some(serde_json::Value::Bool(b)) => Some(b.to_string()),
            Some(other) => {
                return Err(serde::de::Error::custom(format!(
                    "a string or a number was expected, got {other}"
                )));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    /// The D1 rule, in miniature: an unknown field is ignored and a missing
    /// optional field falls back to its default instead of failing the request.
    #[derive(Debug, Deserialize, PartialEq)]
    struct Tolerant {
        id: String,
        #[serde(default)]
        title: Option<String>,
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_optionals_default() {
        let json = r#"{"id":"3135556","title":null,"future_key":{"nested":[1,2]}}"#;
        let parsed: Tolerant = serde_json::from_str(json).expect("tolerant parse");
        assert_eq!(
            parsed,
            Tolerant {
                id: "3135556".to_owned(),
                title: None,
            }
        );
    }

    /// `loose::opt_string` accepts the three shapes the server really sends for
    /// an album's `year` (string, integer, null) — see its doc comment.
    #[derive(Debug, Deserialize)]
    struct Year {
        #[serde(default, deserialize_with = "super::loose::opt_string")]
        year: Option<String>,
    }

    #[test]
    fn a_year_parses_from_a_string_a_number_or_null() {
        for (json, want) in [
            (r#"{"year":"2026"}"#, Some("2026".to_owned())),
            (r#"{"year":2026}"#, Some("2026".to_owned())),
            (r#"{"year":null}"#, None),
            (r#"{}"#, None),
        ] {
            let parsed: Year = serde_json::from_str(json).expect("year parses");
            assert_eq!(parsed.year, want, "{json}");
        }
    }
}
