//! The ids the server speaks, as newtypes over `String`.
//!
//! A track is NEVER addressed by an integer here. The server's `deezer_id` is a
//! Deezer number for a catalogue row, and the row's own UUID for an upload, a
//! YouTube fallback row or a podcast episode; `GET /api/stream/<id>` accepts
//! both. Parsing that into `u64` would work on the bench and fail on the first
//! uploaded file, so every id is a string and only its newtype says what it
//! names (docs/agents/desktop.md, D1 *Types*).

/// Declares one string id: transparent serde, `Display`, `Deref<Target = str>`,
/// and the `From` conversions the endpoints' signatures read best with.
macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash,
            serde::Serialize, serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wraps anything string-like the caller already holds.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// The id as the server spells it.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// The owned id, for a struct that wants to keep it.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;

            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&String> for $name {
            fn from(value: &String) -> Self {
                Self(value.clone())
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }
    };
}

string_id! {
    /// A track's UNIVERSAL id: the `deezer_id` field. A Deezer number as a string
    /// (`"3000000"`) or a UUID (`"89e80170-…"` for an upload, a YouTube fallback
    /// row or a podcast episode). This is what every `/api` route taking a track
    /// id accepts, and what `/api/stream/<id>` wants.
    TrackId
}

string_id! {
    /// A track's ROW id: the `id` field, always a UUID. Absent from the payloads
    /// that name catalogue rows only (a release's preview tracks), so the field
    /// holding it is optional.
    TrackRowId
}

string_id! {
    /// An album's row id (`id`), a UUID. `/api/cover/<id>` takes it.
    AlbumRowId
}

string_id! {
    /// An album's `deezer_id`, a Deezer number as a string. Absent (`""`) on a
    /// YouTube-sourced row.
    AlbumId
}

string_id! {
    /// An artist's `deezer_id`, a Deezer number as a string, or the row UUID for
    /// an artist the library only knows from an upload. `""` when there is no
    /// page to open (a YouTube label).
    ArtistId
}

string_id! {
    /// A playlist's row id (`id`), the UUID `/api/playlist/<id>` takes.
    PlaylistId
}

string_id! {
    /// A podcast episode's id, a UUID. `/api/podcast/episode/<id>/…` takes it.
    EpisodeId
}

string_id! {
    /// A podcast show's id, a UUID. `/api/podcast/<id>` takes it.
    ChannelId
}

string_id! {
    /// A user's id, a UUID (`/api/admin/users/<id>`).
    UserId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_a_string_and_keeps_a_uuid_intact() {
        let uuid = "89e80170-adc3-5d61-9266-df93e520230f";
        let id: TrackId = uuid.into();
        assert_eq!(id.as_str(), uuid);
        assert_eq!(&*id, uuid);
        assert_eq!(id, uuid);
        assert_eq!(id.to_string(), uuid);
        // And it travels through JSON as the bare string it was.
        let text = serde_json::to_string(&id).unwrap();
        assert_eq!(text, format!("\"{uuid}\""));
        assert_eq!(serde_json::from_str::<TrackId>(&text).unwrap(), id);
    }

    #[test]
    fn a_numeric_deezer_id_never_becomes_a_number() {
        let id: TrackId = "3000000".into();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"3000000\"");
        // A JSON number is NOT an id: the server always sends a string, and
        // accepting a number here would hide a payload change.
        assert!(serde_json::from_str::<TrackId>("3000000").is_err());
    }
}
