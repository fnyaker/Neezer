//! Réglages: the person's own settings, read and written through the core's
//! store (the SPA's `Meta` rule: a value set in the UI overrides the file).
//! Owned by **MB16**.
//!
//! The note names what this API must reach (docs/agents/mobile.md, "Screen lots,
//! wave 3"): the server settings `/settings` the person may change, the sources'
//! order, my accounts, the Deezer ARL (MB4 saves it), YouTube connect, access
//! grants, groups, export services, the diagnostic log, and the app/server
//! versions; the local settings live in the `kv` table. MB0 declared the shape a
//! screen compiles against and refused the writes.

use crate::errors::NzError;

/// One setting, as the screen names it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct SettingValue {
    /// The dotted key, the SPA's own (`playback.quality`, `appearance.theme`).
    pub key: String,
    /// A JSON scalar or object, exactly as stored.
    pub value: String,
}

/// The two versions Aide shows side by side.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    /// The app's `versionName` (`0.1.0` for MB0's skeleton).
    pub app: String,
    /// What the server's `/api/version` answered, empty when unknown.
    pub server: String,
}

/// The settings store. MB16 fills it; MB0 refuses the writes.
#[derive(uniffi::Object)]
pub struct SettingsApi;

#[uniffi::export(async_runtime = "tokio")]
impl SettingsApi {
    /// Reads one key from the local `kv` table — synchronous and disk-only, so
    /// the settings screen paints at once.
    pub fn get(&self, key: String) -> Option<String> {
        let _ = key;
        None
    }

    /// Every local key, for the screens that render a section.
    pub fn all(&self) -> Vec<SettingValue> {
        Vec::new()
    }

    /// Writes one LOCAL key optimistically (theme, accent, animations, listen,
    /// library, discovery): the next screen already reads it, and a failure
    /// rolls back.
    pub async fn set(&self, key: String, value: String) -> Result<(), NzError> {
        let _ = (key, value);
        Err(NzError::Unsupported {
            feature: "mb16:set".into(),
        })
    }

    /// The server's `/api/settings` as the person may change it: the sync ALSO
    /// returns what the server holds, so the UI can show a value it did not set
    /// (rule 6 — the UI overrides the file, and the file's value is shown).
    pub async fn server_settings(&self) -> Result<Vec<SettingValue>, NzError> {
        Err(NzError::Unsupported {
            feature: "mb16:server_settings".into(),
        })
    }

    pub fn versions(&self) -> Versions {
        Versions {
            app: env!("CARGO_PKG_VERSION").to_string(),
            server: String::new(),
        }
    }
}
