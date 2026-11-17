//! The session: login, restore, logout, and the event the shell listens to.
//! Owned by **MB2**.
//!
//! The contract is `lib/api.js`'s and the desktop's (`docs/agents/desktop.md`,
//! *Auth*): `POST /api/login` sets a signed cookie that lives in
//! `filesDir/neezer/session.json` (app-private, one entry per origin),
//! `GET /api/me` restores it after a restart, and a `401` means the session
//! ended — the cookie is dropped and the shell is told, once, through
//! [`SessionSink`]. Nothing here logs a password or a cookie.
//!
//! Three decisions:
//!
//! - **A wrong password is not an expired session.** `login`'s `401` is the
//!   credentials being refused (`Invalid`, shown on the form), while a `401` on
//!   a call made with a session this launch already established is the session
//!   being gone (`Session` + `SessionEnded`, the shell goes back to the login
//!   screen). Reading them the same way is how a person is told "session
//!   expirée" right after mistyping their password.
//! - **A launch never claims a session it never had.** `restore()` on a cookie
//!   the server no longer knows answers `None` (nobody to restore, the setup
//!   screen shows the login) instead of a scary sentence.
//! - **Logout always succeeds locally.** Offline, the server cannot be told, so
//!   the stored cookie is deleted and the client rebuilt: the next launch must
//!   not sign the person back in (rule 2's "a button that seems to do nothing is
//!   a bug", read the other way).

use std::sync::Arc;

use nz_api::Lane;
use nz_model::Me;

use super::{Core, CoreError, KEY_VERSION, SessionEvent, TAG_SESSION};

impl Core {
    /// `POST /api/login` — the session cookie is set and the answer is
    /// `/api/me`'s own body, so no second request is needed to know who this is.
    pub async fn login(&self, username: &str, password: &str) -> Result<Me, CoreError> {
        let api = self.api()?;
        let sent = credentials_len(username, password);
        let answer = api.login(username, password).await;
        self.meter().record(TAG_SESSION, 0, sent);
        match answer {
            Ok(me) => {
                self.set_me(Some(me.clone()));
                Ok(me)
            }
            // The server refused the credentials: a form error, never
            // "session expirée" (see the module note).
            Err(nz_api::ApiError::Unauthorized) => Err(CoreError::Invalid {
                detail: "Nom d'utilisateur ou mot de passe refusé".to_owned(),
            }),
            Err(error) => Err(self.classify(error)),
        }
    }

    /// `GET /api/me` with the stored cookie. `None` means "nobody to restore":
    /// no server configured, no cookie, or a link that is down — in each case
    /// without inventing a person and without opening a socket.
    pub async fn restore(&self) -> Result<Option<Me>, CoreError> {
        if self.server().is_none() {
            return Ok(None);
        }
        // Offline: one look at the jar, zero sockets. MB4's first screen decides
        // what to show; the SPA does the same (its `user` store is in memory).
        if !self.is_online() {
            return Ok(None);
        }
        let api = self.api()?;
        if !api.has_session() {
            self.set_me(None);
            return Ok(None);
        }
        let had_person = self.me().is_some();
        let answer = api.bytes(Lane::User, "/me").await;
        match answer {
            Ok(body) => {
                self.meter().record(TAG_SESSION, body.len() as u64, 0);
                let me: Me =
                    serde_json::from_slice(&body).map_err(|error| CoreError::Internal {
                        detail: format!("profil illisible : {error}"),
                    })?;
                api.set_owner(Some(me.user.id.as_str().to_owned()));
                self.set_me(Some(me.clone()));
                Ok(Some(me))
            }
            // A cookie the server no longer knows: nz-api already cleared it
            // (`classify`'s 401). On a launch that is "nobody to restore"; with
            // a person this core already had, it is the session ending.
            Err(nz_api::ApiError::Unauthorized) if !had_person => {
                self.meter().record(TAG_SESSION, 0, 0);
                self.set_me(None);
                Ok(None)
            }
            Err(error) => {
                self.meter().record(TAG_SESSION, 0, 0);
                Err(self.classify(error))
            }
        }
    }

    /// `POST /api/logout`, then the cookie is gone whatever the link did.
    pub async fn logout(&self) -> Result<(), CoreError> {
        let had_session = self
            .api_snapshot()
            .map(|api| api.has_session())
            .unwrap_or(false);
        let mut server_told = false;
        if self.is_online()
            && let Ok(api) = self.api()
        {
            match api.logout().await {
                Ok(()) => {
                    self.meter().record(TAG_SESSION, 0, 0);
                    server_told = true;
                }
                // A dead link is not a reason to keep the person signed in on
                // this device: fall through to the local clear below.
                Err(error) => {
                    self.meter().record(TAG_SESSION, 0, 0);
                    if !matches!(error, nz_api::ApiError::Offline) {
                        tracing::debug!(%error, "logout: the server could not be told");
                    }
                }
            }
        }
        if !server_told {
            self.forget_session_locally();
        }
        self.set_me(None);
        if had_session {
            tracing::info!("logged out");
        }
        Ok(())
    }

    /// The current client, when there is one, without the offline gate: logout
    /// and restore ask the jar even on a dead link.
    fn api_snapshot(&self) -> Option<Arc<nz_api::Api>> {
        self.api
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Stores the person and tells the shell, only when it really changed:
    /// every `/me` answers the same body, and a push per request would be the
    /// old app's tick again.
    fn set_me(&self, me: Option<Me>) {
        let changed = {
            let mut slot = self.me.write().unwrap_or_else(|err| err.into_inner());
            let same_person = slot.as_ref().map(|current| current.user.id.clone())
                == me.as_ref().map(|next| next.user.id.clone())
                && slot.as_ref().map(|current| current.user.name.as_str())
                    == me.as_ref().map(|next| next.user.name.as_str())
                && slot.is_some() == me.is_some();
            *slot = me.clone();
            !same_person
        };
        if changed {
            self.emit(SessionEvent::Me(me));
        }
    }

    /// Reads a failure the way the shell must see it, and fires the one event a
    /// dead session owes: `SessionEnded`, so the app goes back to the login
    /// screen instead of failing every screen one by one.
    fn classify(&self, error: nz_api::ApiError) -> CoreError {
        let error = CoreError::from(error);
        if error == CoreError::Session {
            self.emit(SessionEvent::Ended(error.clone()));
            self.set_me(None);
        }
        error
    }

    /// Drops the stored cookie and rebuilds the client — what an offline logout
    /// owes the person (see the module note).
    fn forget_session_locally(&self) {
        let path = self.data_path("session.json");
        match std::fs::remove_file(&path) {
            Ok(()) => tracing::debug!("session file removed"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(%error, "the session file could not be removed"),
        }
        self.replace_jar();
    }

    /// A fresh jar over the (now absent) file, and a client that uses it.
    fn replace_jar(&self) {
        let jar = Arc::new(nz_api::SessionJar::at(self.data_path("session.json")));
        if let Some(origin) = self.origin()
            && let Ok(api) = self.build_api_with(&origin, jar.clone())
        {
            *self.api.write().unwrap_or_else(|err| err.into_inner()) = Some(api);
        }
        self.set_jar(jar);
    }

    /// The version the server reported at the last `set_server`, shown in Aide.
    pub fn server_version(&self) -> String {
        self.cache()
            .kv()
            .get::<String>(KEY_VERSION)
            .unwrap_or_default()
    }
}

/// The length of the JSON body `POST /api/login` sends, in bytes — the request
/// size the counter attributes to the session tag. The body itself is built and
/// sent by `nz-api`; this never leaves this process and is never logged.
fn credentials_len(username: &str, password: &str) -> u64 {
    #[derive(serde::Serialize)]
    struct Credentials<'a> {
        username: &'a str,
        password: &'a str,
    }
    serde_json::to_vec(&Credentials { username, password })
        .map(|body| body.len() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::testing::{Bench, TempDir, sink::Recorder};

    /// Login, then the person is the answer, once: the shell hears one `Me`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn login_sets_the_session_and_tells_the_shell_once() {
        let bench = Bench::start(false);
        let dir = TempDir::new("login");
        let core = dir.core("test");
        core.set_server(&bench.base()).await.expect("set_server");
        let (recorder, events) = Recorder::new();
        core.subscribe_session(Arc::new(recorder));

        let me = core.login("bench", "Bench1").await.expect("login");
        assert_eq!(me.user.name, "bench");
        assert!(me.user.admin, "the bench account is an admin");
        match recv(&events).expect("one Me event") {
            SessionEvent::Me(Some(seen)) => assert_eq!(seen.user.id, me.user.id),
            other => panic!("expected Me(Some), got {other:?}"),
        }
        assert_eq!(core.me().map(|me| me.user.name), Some("bench".to_owned()));

        // The counters: two session requests (the probe and the login), and the
        // login's body counted on the way out.
        let stats = core.stats();
        let session = stats
            .iter()
            .find(|s| s.tag == TAG_SESSION)
            .expect("session row");
        assert_eq!(session.requests, 2, "{stats:?}");
        assert_eq!(
            session.bytes_in,
            bench.version_body_len(),
            "the version probe's own bytes"
        );
        assert_eq!(session.bytes_out, credentials_len("bench", "Bench1"));
    }

    /// The session survives a new core on the same directories: that is what
    /// "restore" means (rule 7 for the device's own data).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restore_uses_the_saved_cookie_and_origin() {
        let bench = Bench::start(false);
        let dir = TempDir::new("restore");
        {
            let core = dir.core("test");
            core.set_server(&bench.base()).await.expect("set_server");
            core.login("bench", "Bench1").await.expect("login");
        }
        let again = dir.core("test");
        assert_eq!(
            again.server().map(|server| server.origin.as_str()),
            Some(bench.base()),
            "the origin is remembered before anything is asked"
        );
        let me = again.restore().await.expect("restore").expect("somebody");
        assert_eq!(me.user.name, "bench");
        assert!(
            again
                .stats()
                .iter()
                .any(|s| s.tag == TAG_SESSION && s.requests > 0)
        );
    }

    /// A wrong password is a form error, never "session expirée", and it fires
    /// no `SessionEnded`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refused_password_is_not_an_ended_session() {
        let bench = Bench::start(false);
        let dir = TempDir::new("bad-password");
        let core = dir.core("test");
        core.set_server(&bench.base()).await.expect("set_server");
        let (recorder, events) = Recorder::new();
        core.subscribe_session(Arc::new(recorder));

        let error = core.login("bench", "wrong").await.expect_err("refused");
        assert!(matches!(error, CoreError::Invalid { .. }), "{error:?}");
        assert!(events.try_recv().is_err(), "no event for a form error");
    }

    /// Logout clears the cookie, online or not: the next restore finds nobody.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn logout_clears_the_jar_even_offline() {
        let bench = Bench::start(false);
        let dir = TempDir::new("logout");
        let core = dir.core("test");
        core.set_server(&bench.base()).await.expect("set_server");
        core.login("bench", "Bench1").await.expect("login");
        assert!(core.api().unwrap().has_session());

        core.logout().await.expect("logout");
        assert!(!core.api().unwrap().has_session(), "the cookie is gone");
        assert!(core.me().is_none());
        assert!(core.restore().await.expect("restore").is_none());

        // And a restart does not sign the person back in.
        let again = dir.core("test");
        assert!(again.restore().await.expect("restore").is_none());

        // The same, with the link down: the LOCAL cookie must still go.
        core.set_network(crate::core::NetworkState::None);
        core.login("bench", "Bench1")
            .await
            .expect_err("login is offline");
        core.set_network(crate::core::NetworkState::Wifi { metered: false });
        core.login("bench", "Bench1").await.expect("login again");
        core.set_network(crate::core::NetworkState::None);
        core.logout().await.expect("offline logout");
        assert!(
            !core.api_snapshot().expect("a client").has_session(),
            "an offline logout still forgets the cookie"
        );
        let restored = {
            core.set_network(crate::core::NetworkState::Wifi { metered: false });
            core.restore().await.expect("restore")
        };
        assert!(restored.is_none(), "offline logout signed nobody back in");
    }

    /// A `401` on a session this core already established fires
    /// `SessionEnded` once, and the shell can go back to the login screen.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_401_on_a_live_session_ends_it_once() {
        // perf_api's session is a signed cookie: only the controlled server can
        // refuse one that was valid a moment ago.
        let bench = Bench::fake();
        let dir = TempDir::new("session-ended");
        let core = dir.core("test");
        core.set_server(&bench.base()).await.expect("set_server");
        core.login("bench", "Bench1").await.expect("login");
        let (recorder, events) = Recorder::new();
        core.subscribe_session(Arc::new(recorder));

        bench.invalidate_session();

        let error = core.restore().await.expect_err("the session ended");
        assert_eq!(error, CoreError::Session);
        assert!(core.me().is_none(), "nobody is signed in any more");
        // The cause first, then the state: the shell shows the sentence and
        // drops the person's chip in the same frame.
        match recv(&events).expect("the cause") {
            SessionEvent::Ended(CoreError::Session) => {}
            other => panic!("expected Ended(Session), got {other:?}"),
        }
        match recv(&events).expect("the state") {
            SessionEvent::Me(None) => {}
            other => panic!("expected Me(None), got {other:?}"),
        }
        assert!(events.try_recv().is_err(), "and nothing more");
    }

    /// A launch with a cookie the server no longer knows answers `None` (the
    /// setup screen), not "session expirée".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_launch_with_a_dead_cookie_answers_nobody() {
        let bench = Bench::fake();
        let dir = TempDir::new("dead-cookie");
        {
            let core = dir.core("test");
            core.set_server(&bench.base()).await.expect("set_server");
            core.login("bench", "Bench1").await.expect("login");
        }
        bench.invalidate_session();
        let again = dir.core("test");
        let (recorder, events) = Recorder::new();
        again.subscribe_session(Arc::new(recorder));
        assert!(again.restore().await.expect("restore answers").is_none());
        // Nobody was signed in on this launch, so nothing CHANGED: no `Me`
        // chip and no scary "session expirée" — the setup screen just asks.
        assert!(
            events.try_recv().is_err(),
            "a launch with a dead cookie pushes nothing"
        );
    }

    /// A bounded wait, so a wrong expectation fails the test instead of
    /// hanging the suite (a missing event is the failure, not the delay).
    fn recv(
        events: &std::sync::mpsc::Receiver<SessionEvent>,
    ) -> Result<SessionEvent, std::sync::mpsc::RecvTimeoutError> {
        events.recv_timeout(std::time::Duration::from_secs(30))
    }
}
