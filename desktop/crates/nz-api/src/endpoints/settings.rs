//! The server's own surface: sources, settings, storage, archive rules, sync,
//! services, and the URLs a person downloads from.
//!
//! Réglages reads these (docs/agents/desktop.md, *Feature inventory*). The two
//! MULTIPART routes of `api.js` — `POST /api/upload` and the genre extractor's
//! model upload — are deliberately absent: `reqwest`'s `multipart` feature is
//! not pinned in `[workspace.dependencies]` (that file belongs to D0), and a lot
//! may not add a version of its own. They land with the lots that own their
//! screens (D14b for uploads, D16 for the extractor), together with the feature.

use reqwest::Method;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::error::ApiError;
use nz_model::{
    ArchiveRules, ArchiveStatus, CleanupPreview, DeezerStatus, ExportFormats, MigrateStatus,
    ServerSettings, Services, Sources, Storage, SyncStatus, UploadUsage,
};

impl Api {
    /// `GET /api/sources` — the sources of sound in their order, with why each
    /// one rests when it does.
    pub async fn sources(&self) -> Result<Sources, ApiError> {
        self.get_value("/sources").await
    }

    /// `PUT /api/sources` — change the order, enable/disable, or set the lead.
    pub async fn set_sources(&self, change: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::PUT, "/sources", change).await
    }

    /// `GET /api/settings` — the server's own settings (admin).
    pub async fn get_settings(&self) -> Result<ServerSettings, ApiError> {
        self.get_value("/settings").await
    }

    /// `POST /api/settings` — save them. An ARL travels here once and is stored
    /// server-side, never on this device.
    pub async fn set_settings(&self, fields: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::POST, "/settings", fields).await
    }

    /// `GET /api/deezer/status?force=` — is the provider's credential still
    /// good? `force` re-tests the login instead of reading the cached verdict.
    pub async fn deezer_status(&self, force: bool) -> Result<DeezerStatus, ApiError> {
        let path = if force {
            "/deezer/status?force=1"
        } else {
            "/deezer/status"
        };
        self.get_value(path).await
    }

    /// `GET /api/storage` — what the archive and the caches cost, and what is
    /// left on the disk.
    pub async fn storage(&self) -> Result<Storage, ApiError> {
        self.get_value("/storage").await
    }

    /// `POST /api/cache/flush` — empty the server's own caches.
    pub async fn flush_cache(&self) -> Result<(), ApiError> {
        self.post_action("/cache/flush").await
    }

    /// `GET /api/upload/usage` — this person's uploads against the quota.
    pub async fn upload_usage(&self) -> Result<UploadUsage, ApiError> {
        self.get_value("/upload/usage").await
    }

    /// `POST /api/sync` — ask the server to sync the provider account now (a
    /// background job; poll `sync_status`).
    pub async fn sync(&self) -> Result<Value, ApiError> {
        self.send::<(), Value>(crate::lanes::Lane::User, Method::POST, "/sync", None)
            .await?
            .ok_or_else(|| ApiError::Decode("/sync: une réponse était attendue".to_owned()))
    }

    /// `GET /api/sync/status` — is the sync still running?
    pub async fn sync_status(&self) -> Result<SyncStatus, ApiError> {
        self.get_value("/sync/status").await
    }

    /// `POST /api/archive/backfill` — archive everything of the person's that is
    /// not on disk yet (favorites, playlists, podcasts). Adds only: it never
    /// deletes an archived file.
    pub async fn archive_backfill(&self, scope: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "scope": scope });
        self.send_json(Method::POST, "/archive/backfill", &body)
            .await
    }

    /// `GET /api/archive/status` — the backfill's progress, and the CAUSE of
    /// each failure.
    pub async fn archive_status(&self) -> Result<ArchiveStatus, ApiError> {
        self.get_value("/archive/status").await
    }

    /// `GET /api/archive/rules` — which events archive, how much of an artist,
    /// and the cleanup policy.
    pub async fn archive_rules(&self) -> Result<ArchiveRules, ApiError> {
        self.get_value("/archive/rules").await
    }

    /// `POST /api/archive/rules` — save them.
    pub async fn set_archive_rules(&self, fields: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::POST, "/archive/rules", fields).await
    }

    /// `GET /api/archive/cleanup/preview` — what a cleanup would delete right
    /// now. Deletes nothing.
    pub async fn cleanup_preview(&self) -> Result<CleanupPreview, ApiError> {
        self.get_value("/archive/cleanup/preview").await
    }

    /// `POST /api/archive/cleanup` — run the cleanup, the ONLY thing that may
    /// delete archived audio.
    pub async fn run_cleanup(&self) -> Result<Value, ApiError> {
        self.send::<(), Value>(
            crate::lanes::Lane::User,
            Method::POST,
            "/archive/cleanup",
            None,
        )
        .await?
        .ok_or_else(|| ApiError::Decode("/archive/cleanup: une réponse était attendue".to_owned()))
    }

    /// `POST /api/archive/migrate` — bring the archived files to the current
    /// format (job + poll).
    pub async fn archive_migrate(&self, force: bool) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "force": force });
        self.send_json(Method::POST, "/archive/migrate", &body)
            .await
    }

    /// `GET /api/archive/migrate` — the migration's progress and failure causes.
    pub async fn archive_migrate_status(&self) -> Result<MigrateStatus, ApiError> {
        self.get_value("/archive/migrate").await
    }

    /// `GET /api/services` — the linked services and the export formats.
    pub async fn services(&self) -> Result<Services, ApiError> {
        self.get_value("/services").await
    }

    /// `PUT /api/services/spotify/app` — the operator's own Spotify app.
    pub async fn set_spotify_app(&self, client_id: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "client_id": client_id });
        self.send_json(Method::PUT, "/services/spotify/app", &body)
            .await
    }

    /// `GET /api/services/spotify/authorize` — the consent URL to open.
    pub async fn spotify_authorize(&self) -> Result<Value, ApiError> {
        self.get_value("/services/spotify/authorize").await
    }

    /// `POST /api/services/youtube/connect` — link YouTube Music with the
    /// browser's own request headers.
    pub async fn connect_youtube(&self, headers: &Value) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "headers": headers });
        self.send_json(Method::POST, "/services/youtube/connect", &body)
            .await
    }

    /// `DELETE /api/services/<id>` — unlink a service (it cannot touch the
    /// archive).
    pub async fn disconnect_service(&self, id: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/services/{}", encode(id)),
            &serde_json::json!({}),
        )
        .await
    }

    /// `POST /api/services/<service>/export` — match and create one playlist or
    /// album over there (a job).
    pub async fn export_to_service(
        &self,
        service: &str,
        kind: &str,
        id: &str,
        name: &str,
        is_public: bool,
    ) -> Result<Value, ApiError> {
        let path = format!("/services/{}/export", encode(service));
        let body = serde_json::json!({ "kind": kind, "id": id, "name": name, "public": is_public });
        self.send_json(Method::POST, &path, &body).await
    }

    /// `GET /api/services/export/<job>` — how the export is going.
    pub async fn service_export_job(&self, job: &str) -> Result<Value, ApiError> {
        self.get_value(&format!("/services/export/{}", encode(job)))
            .await
    }

    /// `GET /api/export/formats` — what the bulk export can produce.
    pub async fn export_formats(&self) -> Result<ExportFormats, ApiError> {
        self.get_value("/export/formats").await
    }
}
