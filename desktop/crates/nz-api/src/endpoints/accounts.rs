//! The person's own provider accounts (`/api/me/accounts*`): link one, import
//! what it holds, choose which one is the default.
//!
//! The ARL typed in Réglages → Sources is sent once, here, and never stored on
//! the device (`docs/agents/desktop.md`, *Auth and security*): it is a
//! full-account credential, and the server is where it lives.

use reqwest::Method;
use serde_json::Value;

use crate::client::{Api, encode};
use crate::error::ApiError;
use nz_model::MyAccounts;

impl Api {
    /// `GET /api/me/accounts` — the accounts this person may use, and whether
    /// another kind can be linked.
    pub async fn my_accounts(&self) -> Result<MyAccounts, ApiError> {
        self.get_value("/me/accounts").await
    }

    /// `POST /api/me/accounts/deezer` — link a Deezer account with its ARL.
    ///
    /// `mode` is `"browser"` when the ARL came from Deezer's own login page, and
    /// anything else (or `None`) for an ARL the person typed: the server only
    /// tells those two apart (`neezer/webui/myaccounts.py#link_deezer`:
    /// `mode = "browser" if data.get("mode") == "browser" else "arl"`). The
    /// doc used to name `"shared"`/`"personal"`, which the server never read.
    pub async fn link_deezer(&self, arl: &str, mode: Option<&str>) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "arl": arl, "mode": mode });
        self.send_json(Method::POST, "/me/accounts/deezer", &body)
            .await
    }

    /// `DELETE /api/me/accounts/<id>` — unlink an account (the archive stays).
    pub async fn unlink_account(&self, id: &str) -> Result<(), ApiError> {
        let path = format!("/me/accounts/{}", encode(id));
        self.send_none(Method::DELETE, &path, &serde_json::json!({}))
            .await
    }

    /// `PUT /api/me/accounts/<id>` — what this account imports automatically.
    ///
    /// The server requires `{"auto": {<kind>: bool}}` and answers `400 auto
    /// invalide` for anything else (`neezer/webui/myaccounts.py#my_accounts_options`
    /// checks `isinstance(auto, dict)`, then that every key is one of
    /// `libraryimport.KINDS`). A bare boolean was refused with that `400`, which
    /// is why this takes the kinds the account announced (`MyAccount::kinds`)
    /// rather than one flag. A kind left out keeps its saved value.
    pub async fn set_account_auto(
        &self,
        id: &str,
        kinds: &[String],
        on: bool,
    ) -> Result<Value, ApiError> {
        let path = format!("/me/accounts/{}", encode(id));
        let mut auto = serde_json::Map::new();
        for kind in kinds {
            auto.insert(kind.clone(), serde_json::Value::Bool(on));
        }
        let body = serde_json::json!({ "auto": auto });
        self.send_json(Method::PUT, &path, &body).await
    }

    /// `PUT /api/me/accounts/default` — which account leads.
    pub async fn set_default_account(&self, id: &str) -> Result<Value, ApiError> {
        let body = serde_json::json!({ "account": id });
        self.send_json(Method::PUT, "/me/accounts/default", &body)
            .await
    }

    /// `POST /api/me/accounts/<id>/import` — import the chosen kinds
    /// (`None`: the server's own defaults). Runs as a job (rule 2).
    pub async fn import_account(
        &self,
        id: &str,
        kinds: Option<&[String]>,
    ) -> Result<Value, ApiError> {
        let path = format!("/me/accounts/{}/import", encode(id));
        let body = match kinds {
            Some(kinds) => serde_json::json!({ "kinds": kinds }),
            None => serde_json::json!({}),
        };
        self.send_json(Method::POST, &path, &body).await
    }

    /// `GET /api/me/accounts/<id>/library/<kind>` — what this account holds of
    /// one kind. `fresh` asks the provider instead of the last import.
    pub async fn account_library(
        &self,
        id: &str,
        kind: &str,
        fresh: bool,
    ) -> Result<Value, ApiError> {
        let path = format!(
            "/me/accounts/{}/library/{}{}",
            encode(id),
            encode(kind),
            if fresh { "?fresh=1" } else { "" }
        );
        self.get_value(&path).await
    }

    /// `PUT /api/me/accounts/<id>/library/<kind>` — keep or drop individual
    /// items of the imported library.
    pub async fn set_account_items(
        &self,
        id: &str,
        kind: &str,
        refs: &[String],
        on: bool,
    ) -> Result<Value, ApiError> {
        let path = format!("/me/accounts/{}/library/{}", encode(id), encode(kind));
        let body = serde_json::json!({ "refs": refs, "on": on });
        self.send_json(Method::PUT, &path, &body).await
    }
}
