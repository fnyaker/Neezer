//! Typed settings: the SPA's `localStorage`, one row each.
//!
//! The desktop has no web storage, and the settings a person changes must
//! survive a restart the same way: palette, zoom, eco mode, the play-cache cap,
//! the queue's own small documents. Values are stored as their JSON text, so a
//! setting can be a bool, a number, a list of presets or a struct, and a value
//! written by an OLDER build that no longer parses reads as "not set" instead of
//! failing the boot (rule 7).

use rusqlite::params;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::debug;

use crate::{Cache, Result};

/// The typed key/value store.
pub struct Kv<'a> {
    cache: &'a Cache,
}

impl<'a> Kv<'a> {
    pub(crate) fn new(cache: &'a Cache) -> Self {
        Self { cache }
    }

    /// The value under `key`, or `None` when it was never set — or when what is
    /// stored no longer parses as `T` (an upgrade's older spelling).
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let body = match self.cache.read(|conn| {
            self.cache.query_optional(
                conn,
                "SELECT value FROM kv WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
        }) {
            Ok(body) => body?,
            Err(error) => {
                debug!(%error, key, "setting read failed");
                return None;
            }
        };
        match serde_json::from_str(&body) {
            Ok(value) => Some(value),
            Err(error) => {
                debug!(%error, key, "stored setting does not parse; treating it as unset");
                None
            }
        }
    }

    /// Stores a value (one statement: an upsert on the key).
    pub fn set<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let body = serde_json::to_string(value)?;
        self.cache.write(|cache, conn| {
            cache.exec(
                conn,
                "INSERT INTO kv(key, value) VALUES(?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, body],
            )?;
            Ok(())
        })
    }

    /// Forgets a setting. Returns whether it was there.
    pub fn delete(&self, key: &str) -> Result<bool> {
        let removed = self
            .cache
            .write(|cache, conn| cache.exec(conn, "DELETE FROM kv WHERE key = ?1", params![key]))?;
        Ok(removed > 0)
    }

    /// Whether a value exists, without parsing it.
    pub fn contains(&self, key: &str) -> bool {
        self.cache
            .read(|conn| {
                self.cache.query(
                    conn,
                    "SELECT COUNT(*) FROM kv WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, i64>(0),
                )
            })
            .map(|count| count > 0)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use crate::Result;
    use crate::testing::TempDir;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Playback {
        crossfade_seconds: f32,
        eq_enabled: bool,
        bands: Vec<i8>,
    }

    #[test]
    fn a_struct_round_trips() -> Result<()> {
        let dir = TempDir::new("kv-struct");
        let cache = dir.open();
        let value = Playback {
            crossfade_seconds: 6.0,
            eq_enabled: true,
            bands: vec![-2, 0, 3],
        };

        cache.kv().set("lecture", &value)?;
        assert_eq!(cache.kv().get::<Playback>("lecture"), Some(value));
        assert!(cache.kv().contains("lecture"));
        assert_eq!(cache.kv().get::<Playback>("absent"), None);
        Ok(())
    }

    #[test]
    fn scalars_and_overwrites_behave_like_local_storage() -> Result<()> {
        let dir = TempDir::new("kv-scalars");
        let cache = dir.open();

        assert_eq!(cache.kv().get::<i64>("viz.fps"), None);
        cache.kv().set("viz.fps", &60)?;
        cache.kv().set("viz.fps", &30)?;
        assert_eq!(cache.kv().get::<i64>("viz.fps"), Some(30));

        // One statement per write, so a settings screen can save a dozen toggles
        // without a transaction dance.
        let before = cache.statements();
        cache.kv().set("viz.eco", &true)?;
        assert_eq!(cache.statements() - before, 1);
        Ok(())
    }

    #[test]
    fn a_value_of_another_shape_reads_as_unset() -> Result<()> {
        let dir = TempDir::new("kv-shape");
        let cache = dir.open();
        cache.kv().set("cache.limit", &"2 Go")?;

        assert_eq!(cache.kv().get::<i64>("cache.limit"), None);
        assert!(cache.kv().contains("cache.limit"), "the row is still there");
        Ok(())
    }

    #[test]
    fn delete_reports_what_it_removed() -> Result<()> {
        let dir = TempDir::new("kv-delete");
        let cache = dir.open();

        cache.kv().set("a", &1i64)?;
        assert!(cache.kv().delete("a")?);
        assert!(!cache.kv().delete("a")?);
        assert!(!cache.kv().contains("a"));
        Ok(())
    }
}
