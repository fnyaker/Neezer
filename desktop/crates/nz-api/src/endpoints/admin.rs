//! The administrator's surface: users and invitations, the analysis backfills,
//! the genre studio, similar/enrich/credits rebuilds, the automatch, the
//! replacement jobs and the explorer.
//!
//! Rule 2 is the shape of everything here: starting a long job returns at once,
//! and the screen polls its `*_status` route, which carries the progress AND the
//! cause of each failure. Nothing in this module blocks on a backfill.
//!
//! Two conventions, both the server's:
//! `{"user": …}` / `{"invite": …}` envelopes (a mutation answers the object it
//! changed) are returned as `serde_json::Value`, because `nz-model` typed the
//! LIST payloads, not those envelopes; and a start route answers the job handle
//! the poll route then takes.

use reqwest::Method;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::endpoints::{merge, query, with};
use crate::error::ApiError;
use nz_model::{
    AdminInvites, AdminUsers, AnalysisBackfill, AnalysisPlan, AnalysisTodo, Automatch,
    CreditsBackfill, EnrichStatus, Explorer, ExtractorTest, GenreCandidates, GenreConstruction,
    GenreEmbed, GenreLabelled, GenreModel, GenrePool, GenreReferences, GenreReferencesFor,
    GenreRelabel, GenreShareStatus, GenreStatus, GenreTrack, GenreTree, SimilarStatus,
};

impl Api {
    // -- people ------------------------------------------------------------

    /// `GET /api/admin/users?q=&after=&limit=` — the user list, paged.
    pub async fn admin_users(
        &self,
        q: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<AdminUsers, ApiError> {
        let path = format!(
            "/admin/users{}",
            query(&[
                ("q", (!q.is_empty()).then(|| q.to_owned())),
                ("after", after.filter(|a| !a.is_empty()).map(str::to_owned)),
                ("limit", Some(limit.to_string())),
            ])
        );
        self.get_value(&path).await
    }

    /// `GET /api/admin/users/<id>` — one account, with how much it uploaded and
    /// made.
    pub async fn admin_user(&self, id: &str) -> Result<Value, ApiError> {
        self.get_value(&format!("/admin/users/{}", encode(id)))
            .await
    }

    /// `PATCH /api/admin/users/<id>` — role, disabled, quota…
    pub async fn admin_user_patch(&self, id: &str, payload: &Value) -> Result<Value, ApiError> {
        let path = format!("/admin/users/{}", encode(id));
        self.send_json(Method::PATCH, &path, payload).await
    }

    /// `DELETE /api/admin/users/<id>` — delete an account, confirming its name.
    pub async fn admin_user_delete(&self, id: &str, name: &str) -> Result<Value, ApiError> {
        let path = format!("/admin/users/{}", encode(id));
        let body = serde_json::json!({ "confirm": name });
        self.send_json(Method::DELETE, &path, &body).await
    }

    /// `GET /api/admin/invites?state=&limit=&after_created=&after_id=` — the
    /// invitation list, paged by `(created, id)`.
    pub async fn admin_invites(
        &self,
        state: Option<&str>,
        limit: u32,
        after_created: Option<&str>,
        after_id: Option<&str>,
    ) -> Result<AdminInvites, ApiError> {
        let path = format!(
            "/admin/invites{}",
            query(&[
                ("state", state.map(str::to_owned)),
                ("limit", Some(limit.to_string())),
                ("after_created", after_created.map(str::to_owned)),
                ("after_id", after_id.map(str::to_owned)),
            ])
        );
        self.get_value(&path).await
    }

    /// `POST /api/admin/invites` — mint one; the token is in the answer ONCE.
    pub async fn admin_invite_create(&self, payload: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::POST, "/admin/invites", payload)
            .await
    }

    /// `POST /api/admin/invites/<id>/revoke`.
    pub async fn admin_invite_revoke(&self, id: &str) -> Result<Value, ApiError> {
        let path = format!("/admin/invites/{}/revoke", encode(id));
        self.send::<(), Value>(crate::lanes::Lane::User, Method::POST, &path, None)
            .await?
            .ok_or_else(|| ApiError::Decode(format!("{path}: une réponse était attendue")))
    }

    /// `POST /api/admin/invites/<id>/renew` — extend a pending invitation.
    pub async fn admin_invite_renew(&self, id: &str) -> Result<Value, ApiError> {
        let path = format!("/admin/invites/{}/renew", encode(id));
        self.send::<(), Value>(crate::lanes::Lane::User, Method::POST, &path, None)
            .await?
            .ok_or_else(|| ApiError::Decode(format!("{path}: une réponse était attendue")))
    }

    // -- analysis (tempo, descriptors, embeddings) -------------------------

    /// `POST /api/analysis/backfill` — (re)measure the library, with a chosen
    /// parallelism. A job: poll [`Api::analysis_backfill_status`].
    pub async fn analysis_backfill(&self, force: bool, workers: u32) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "force": force, "workers": workers });
        self.send_json(Method::POST, "/analysis/backfill", &body)
            .await
    }

    /// `GET /api/analysis/backfill` — the job's state, stage and counters.
    pub async fn analysis_backfill_status(&self) -> Result<AnalysisBackfill, ApiError> {
        self.get_value("/analysis/backfill").await
    }

    /// `POST /api/analysis/backfill/stop` — stop the running backfill.
    pub async fn analysis_backfill_stop(&self) -> Result<(), ApiError> {
        self.post_action("/analysis/backfill/stop").await
    }

    /// `GET /api/analysis/plan` — what is out of date and at which step, plus
    /// the nightly window.
    pub async fn analysis_plan(&self) -> Result<AnalysisPlan, ApiError> {
        self.get_value("/analysis/plan").await
    }

    /// `PUT /api/analysis/schedule` — the nightly window's settings.
    pub async fn analysis_schedule(&self, settings: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::PUT, "/analysis/schedule", settings)
            .await
    }

    /// `GET /api/analysis/todo?limit=20[&embed=1][&after=]` — what a CLIENT
    /// machine should measure next (the studio's own worker loop).
    pub async fn analysis_todo(
        &self,
        after: Option<&str>,
        embed: bool,
    ) -> Result<AnalysisTodo, ApiError> {
        let path = format!(
            "/analysis/todo{}",
            query(&[
                ("limit", Some("20".to_owned())),
                ("embed", embed.then(|| "1".to_owned())),
                ("after", after.map(str::to_owned)),
            ])
        );
        self.get_value(&path).await
    }

    /// `POST /api/analysis/<id>/embedding` — a client-measured vector.
    pub async fn analysis_embedding(&self, id: &str, vector: &Value) -> Result<(), ApiError> {
        let path = format!("/analysis/{}/embedding", encode(id));
        self.send_none(Method::POST, &path, vector).await
    }

    /// `POST /api/analysis/<id>/measures` — a client-measured descriptor set.
    pub async fn analysis_measures(&self, id: &str, measures: &Value) -> Result<(), ApiError> {
        let path = format!("/analysis/{}/measures", encode(id));
        self.send_none(Method::POST, &path, measures).await
    }

    /// `PUT /api/analysis/where` — where the analysis runs (server/client/both).
    pub async fn analysis_where(&self, place: &str) -> Result<(), ApiError> {
        self.send_none(
            Method::PUT,
            "/analysis/where",
            &serde_json::json!({ "where": place }),
        )
        .await
    }

    /// `GET /api/explorer` — the background explorer's state (or `PUT` its
    /// switches, when `settings` is given).
    pub async fn explorer(&self, settings: Option<&Value>) -> Result<Explorer, ApiError> {
        match settings {
            Some(settings) => self.send_json(Method::PUT, "/explorer", settings).await,
            None => self.get_value("/explorer").await,
        }
    }

    // -- the library-wide rebuilds ----------------------------------------

    /// `POST /api/similar/rebuild` — recompute "what sounds alike" (a job).
    pub async fn similar_rebuild(&self, force: bool) -> Result<Value, ApiError> {
        let path = if force {
            "/similar/rebuild?force=1"
        } else {
            "/similar/rebuild"
        };
        self.send::<(), Value>(crate::lanes::Lane::User, Method::POST, path, None)
            .await?
            .ok_or_else(|| ApiError::Decode(format!("{path}: une réponse était attendue")))
    }

    /// `GET /api/similar/rebuild` — its progress.
    pub async fn similar_status(&self) -> Result<SimilarStatus, ApiError> {
        self.get_value("/similar/rebuild").await
    }

    /// `POST /api/enrich/backfill` — MusicBrainz facts for the archived library.
    pub async fn enrich_backfill(&self, force: bool) -> Result<Value, ApiError> {
        let path = if force {
            "/enrich/backfill?force=1"
        } else {
            "/enrich/backfill"
        };
        self.send::<(), Value>(crate::lanes::Lane::User, Method::POST, path, None)
            .await?
            .ok_or_else(|| ApiError::Decode(format!("{path}: une réponse était attendue")))
    }

    /// `GET /api/enrich/backfill` — coverage and progress.
    pub async fn enrich_status(&self) -> Result<EnrichStatus, ApiError> {
        self.get_value("/enrich/backfill").await
    }

    /// `POST /api/credits/backfill` — complete the credits of the library.
    pub async fn credits_backfill(&self, force: bool) -> Result<Value, ApiError> {
        let path = if force {
            "/credits/backfill?force=1"
        } else {
            "/credits/backfill"
        };
        self.send::<(), Value>(crate::lanes::Lane::User, Method::POST, path, None)
            .await?
            .ok_or_else(|| ApiError::Decode(format!("{path}: une réponse était attendue")))
    }

    /// `GET /api/credits/backfill` — its counters.
    pub async fn credits_backfill_status(&self) -> Result<CreditsBackfill, ApiError> {
        self.get_value("/credits/backfill").await
    }

    // -- availability ------------------------------------------------------

    /// `GET /api/automatch` — the audio auto-match's switch, its last run and
    /// what it did.
    pub async fn automatch(&self) -> Result<Automatch, ApiError> {
        self.get_value("/automatch").await
    }

    /// `PUT /api/automatch/settings` — turn it on or off.
    pub async fn automatch_settings(&self, enabled: bool) -> Result<(), ApiError> {
        self.send_none(
            Method::PUT,
            "/automatch/settings",
            &serde_json::json!({ "enabled": enabled }),
        )
        .await
    }

    /// `POST /api/automatch/run` — a library-wide run (a job).
    pub async fn automatch_run(&self) -> Result<Value, ApiError> {
        self.send::<(), Value>(
            crate::lanes::Lane::User,
            Method::POST,
            "/automatch/run",
            None,
        )
        .await?
        .ok_or_else(|| ApiError::Decode("/automatch/run: une réponse était attendue".to_owned()))
    }

    /// `POST /api/automatch/cancel`.
    pub async fn automatch_cancel(&self) -> Result<(), ApiError> {
        self.post_action("/automatch/cancel").await
    }

    // -- the genre studio --------------------------------------------------

    /// `GET /api/genre/status` — the vocabulary, its counts, the trained head
    /// and the extractor.
    pub async fn genre_status(&self) -> Result<GenreStatus, ApiError> {
        self.get_value("/genre/status").await
    }

    /// `POST /api/genre/tags` — a new tag in the person's own vocabulary.
    pub async fn genre_tag_create(
        &self,
        name: &str,
        color: Option<&str>,
        archetype: Option<&str>,
    ) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "name": name, "color": color, "archetype": archetype });
        self.send_json(Method::POST, "/genre/tags", &body).await
    }

    /// `PATCH /api/genre/tags/<id>` — rename, recolour, change the archetype.
    pub async fn genre_tag_edit(&self, id: i64, patch: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::PATCH, &format!("/genre/tags/{id}"), patch)
            .await
    }

    /// `DELETE /api/genre/tags/<id>`.
    pub async fn genre_tag_delete(&self, id: i64) -> Result<(), ApiError> {
        self.send_none(
            Method::DELETE,
            &format!("/genre/tags/{id}"),
            &serde_json::json!({}),
        )
        .await
    }

    /// `POST /api/genre/tags/defaults` — (re)add the engine's own genres as
    /// tags; only ever adds what is missing.
    pub async fn genre_defaults(&self) -> Result<Value, ApiError> {
        self.send::<(), Value>(
            crate::lanes::Lane::User,
            Method::POST,
            "/genre/tags/defaults",
            None,
        )
        .await?
        .ok_or_else(|| {
            ApiError::Decode("/genre/tags/defaults: une réponse était attendue".to_owned())
        })
    }

    /// `POST /api/genre/label` — label one track with one tag.
    pub async fn genre_label(&self, track: &str, tag: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "track": track, "tag": tag });
        self.send_json(Method::POST, "/genre/label", &body).await
    }

    /// `GET /api/genre/candidates?limit=&sort=&last=` — what to label next.
    pub async fn genre_candidates(
        &self,
        limit: u32,
        sort: Option<&str>,
        last: Option<&str>,
    ) -> Result<GenreCandidates, ApiError> {
        let path = format!(
            "/genre/candidates{}",
            query(&[
                ("limit", Some(limit.to_string())),
                ("sort", sort.map(str::to_owned)),
                ("last", last.map(str::to_owned)),
            ])
        );
        self.get_value(&path).await
    }

    /// `GET /api/genre/labelled` — everything labelled so far.
    pub async fn genre_labelled(&self) -> Result<GenreLabelled, ApiError> {
        self.get_value("/genre/labelled").await
    }

    /// `GET /api/genre/tree` — the genre hierarchy, its version and whether it
    /// can be undone.
    pub async fn genre_tree(&self) -> Result<GenreTree, ApiError> {
        self.get_value("/genre/tree").await
    }

    /// `POST|DELETE /api/genre/tree/edge` — add a link, or cut one.
    pub async fn genre_tree_edge(
        &self,
        child: &str,
        parent: &str,
        weight: f64,
        method: Method,
    ) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "child": child, "parent": parent, "weight": weight });
        self.send_json(method, "/genre/tree/edge", &body).await
    }

    /// `DELETE /api/genre/tree/edge` — the SPA's `genreTreeCut`.
    pub async fn genre_tree_cut(&self, child: &str, parent: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "child": child, "parent": parent });
        self.send_json(Method::DELETE, "/genre/tree/edge", &body)
            .await
    }

    /// `POST /api/genre/tree/move` — reparent a node, optionally from a known
    /// old parent.
    pub async fn genre_tree_move(
        &self,
        child: &str,
        parent: &str,
        old_parent: Option<&str>,
        weight: f64,
    ) -> Result<Value, ApiError> {
        let body = serde_json::json!({
            "child": child,
            "parent": parent,
            "old_parent": old_parent,
            "weight": weight,
        });
        self.send_json(Method::POST, "/genre/tree/move", &body)
            .await
    }

    /// `POST /api/genre/tree/undo` — undo the last tree edit.
    pub async fn genre_tree_undo(&self) -> Result<Value, ApiError> {
        self.send::<(), Value>(
            crate::lanes::Lane::User,
            Method::POST,
            "/genre/tree/undo",
            None,
        )
        .await?
        .ok_or_else(|| ApiError::Decode("/genre/tree/undo: une réponse était attendue".to_owned()))
    }

    /// `POST /api/genre/tree/reset` — back to the engine's own tree.
    pub async fn genre_tree_reset(&self) -> Result<Value, ApiError> {
        self.send::<(), Value>(
            crate::lanes::Lane::User,
            Method::POST,
            "/genre/tree/reset",
            None,
        )
        .await?
        .ok_or_else(|| ApiError::Decode("/genre/tree/reset: une réponse était attendue".to_owned()))
    }

    /// `GET /api/genre/pool?limit=` — untagged tracks with a vector, for
    /// self-training.
    pub async fn genre_pool(&self, limit: u32) -> Result<GenrePool, ApiError> {
        self.get_value(&format!(
            "/genre/pool{}",
            query(&[("limit", Some(limit.to_string()))])
        ))
        .await
    }

    /// `GET /api/genre/track/<id>` — one track's tag and the ranked vocabulary.
    pub async fn genre_track(&self, id: &str) -> Result<GenreTrack, ApiError> {
        self.get_value(&format!("/genre/track/{}", encode(id)))
            .await
    }

    /// `POST /api/genre/embeddings` — vectors for a run of tracks.
    pub async fn genre_embeddings(&self, ids: &[String]) -> Result<Value, ApiError> {
        self.send_json(
            Method::POST,
            "/genre/embeddings",
            &serde_json::json!({ "ids": ids }),
        )
        .await
    }

    /// `GET /api/genre/model` — is a head trained and stored?
    pub async fn genre_model(&self) -> Result<GenreModel, ApiError> {
        self.get_value("/genre/model").await
    }

    /// `PUT /api/genre/model` — store the head the browser just trained.
    pub async fn genre_model_put(&self, payload: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::PUT, "/genre/model", payload).await
    }

    /// `DELETE /api/genre/model`.
    pub async fn genre_model_delete(&self) -> Result<(), ApiError> {
        self.send_none(Method::DELETE, "/genre/model", &serde_json::json!({}))
            .await
    }

    /// `POST /api/genre/share/preview` — what a shared model file would do.
    pub async fn genre_share_preview(&self, file: &Value) -> Result<Value, ApiError> {
        self.send_json(Method::POST, "/genre/share/preview", file)
            .await
    }

    /// `POST /api/genre/share/import` — import the model and apply it.
    pub async fn genre_share_import(
        &self,
        file: &Value,
        options: &Value,
    ) -> Result<Value, ApiError> {
        let body = merge(file.clone(), options);
        self.send_json(Method::POST, "/genre/share/import", &body)
            .await
    }

    /// `GET /api/genre/share/import` — the import job's progress.
    pub async fn genre_share_status(&self) -> Result<GenreShareStatus, ApiError> {
        self.get_value("/genre/share/import").await
    }

    /// `GET /api/genre/relabel` — the stored verdicts re-decided under the
    /// active head (no re-measure).
    pub async fn genre_relabel(&self) -> Result<GenreRelabel, ApiError> {
        self.get_value("/genre/relabel").await
    }

    /// `POST /api/genre/relabel` — start it.
    pub async fn genre_relabel_start(&self) -> Result<Value, ApiError> {
        self.send::<(), Value>(
            crate::lanes::Lane::User,
            Method::POST,
            "/genre/relabel",
            None,
        )
        .await?
        .ok_or_else(|| ApiError::Decode("/genre/relabel: une réponse était attendue".to_owned()))
    }

    /// `GET /api/genre/construction` — how much of the library is measured,
    /// labelled, and what is missing.
    pub async fn genre_construction(&self) -> Result<GenreConstruction, ApiError> {
        self.get_value("/genre/construction").await
    }

    /// `GET /api/genre/references` — the genres with well-known recordings.
    pub async fn genre_reference_genres(&self) -> Result<GenreReferences, ApiError> {
        self.get_value("/genre/references").await
    }

    /// `GET /api/genre/references?genre=` — the recordings looked up for one
    /// genre.
    pub async fn genre_references(&self, genre: &str) -> Result<GenreReferencesFor, ApiError> {
        self.get_value(&format!(
            "/genre/references{}",
            query(&[("genre", Some(genre.to_owned()))])
        ))
        .await
    }

    /// `POST /api/genre/references/import` — take one provider recording into
    /// the library as an example of a genre.
    pub async fn genre_reference_import(
        &self,
        deezer_id: &str,
        tag: &str,
    ) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "deezer_id": deezer_id, "tag": tag });
        self.send_json(Method::POST, "/genre/references/import", &body)
            .await
    }

    /// `POST /api/genre/bulk/preview` — what tagging a whole album/playlist/
    /// artist would do. `scope` is the SPA's own `{...scope}` object.
    pub async fn genre_bulk_preview(&self, scope: &Value, tag: &str) -> Result<Value, ApiError> {
        let body = with(scope.clone(), "tag", Value::from(tag));
        self.send_json(Method::POST, "/genre/bulk/preview", &body)
            .await
    }

    /// `POST /api/genre/bulk` — tag a whole run of tracks at once.
    pub async fn genre_bulk(&self, tracks: &[String], tag: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "tracks": tracks, "tag": tag });
        self.send_json(Method::POST, "/genre/bulk", &body).await
    }

    /// `DELETE /api/genre/extractor` — remove the operator's own ONNX extractor.
    pub async fn genre_extractor_delete(&self) -> Result<(), ApiError> {
        self.send_none(Method::DELETE, "/genre/extractor", &serde_json::json!({}))
            .await
    }

    /// `POST /api/genre/extractor/test` — the built-in sanity check on real
    /// tracks (a job; poll [`Api::genre_extractor_test_status`]).
    pub async fn genre_extractor_test(&self) -> Result<(), ApiError> {
        self.post_action("/genre/extractor/test").await
    }

    /// `GET /api/genre/extractor/test` — the check's progress and verdict.
    pub async fn genre_extractor_test_status(&self) -> Result<ExtractorTest, ApiError> {
        self.get_value("/genre/extractor/test").await
    }

    /// `POST /api/genre/embed[?force=1]` — measure every archived track that has
    /// no vector yet (the slow one; a job).
    pub async fn genre_embed(&self, force: bool) -> Result<Value, ApiError> {
        let path = if force {
            "/genre/embed?force=1"
        } else {
            "/genre/embed"
        };
        self.send::<(), Value>(crate::lanes::Lane::User, Method::POST, path, None)
            .await?
            .ok_or_else(|| ApiError::Decode(format!("{path}: une réponse était attendue")))
    }

    /// `GET /api/genre/embed` — its progress.
    pub async fn genre_embed_status(&self) -> Result<GenreEmbed, ApiError> {
        self.get_value("/genre/embed").await
    }
}
