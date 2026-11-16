//! The server's JSON as Rust types — serde only, tolerant by construction.
//!
//! Owned by lot D1 (`docs/agents/desktop.md`): every payload the app reads gets
//! a struct here, and every optional field is `#[serde(default)]` so an older
//! or a newer server still parses (rule 7). This is D0's skeleton: the crate,
//! its manifest and the one rule the rest of the types are built on.

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
}
