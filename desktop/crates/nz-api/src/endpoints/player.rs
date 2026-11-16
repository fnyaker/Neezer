//! What the player tells the server: the listens that feed the person's own
//! statistics, Flow and recap.
//!
//! `POST /api/listen` counts the play on THIS server only; nothing goes to
//! Deezer. It is fire-and-forget: a lost listen is noise, and a failed one must
//! never surface as an error over the music.

use crate::client::Api;
use crate::error::ApiError;
use nz_model::ListenPayload;

impl Api {
    /// `POST /api/listen` — sent on every track change with the seconds
    /// actually PLAYED (pauses excluded, `Player.svelte#flushListen`).
    ///
    /// Skipped entirely while the link is known dead: a doomed POST per track
    /// change is just noise (`api.reportListen`). The caller ignores the error,
    /// which is why this returns `()` on success and never blocks anything.
    pub async fn report_listen(&self, payload: &ListenPayload) -> Result<(), ApiError> {
        if !self.is_online() {
            return Ok(());
        }
        self.send_none(reqwest::Method::POST, "/listen", payload)
            .await
    }
}
