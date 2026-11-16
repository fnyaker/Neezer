//! One instance per session: the D-Bus name `fr.neezer.Desktop`.
//!
//! A second launch asks the running app to raise its window and exits, which is
//! what a launcher click expects. The name is claimed on the session bus before
//! the window opens — but on a short budget: no session bus (a TTY, a minimal
//! compositor, CI) or a slow bus must not delay the first frame, so the claim
//! is best-effort and the app runs single-window anyway.
//!
//! The method is a plain D-Bus call, not a signal: the running instance's
//! `ObjectServer` receives it because it owns the name, and there is no match
//! rule for a broadcast to get wrong.

use std::time::Duration;

use crate::state::AppEvent;
use crate::wiring::events::Sender;

/// The well-known name, also the object path's interface.
pub const NAME: &str = "fr.neezer.Desktop";
const PATH: &str = "/fr/neezer/Desktop";
const INTERFACE: &str = "fr.neezer.Desktop";

/// How long the background claim waits for the session bus. It runs after the
/// window is up, so this is a budget for a hung bus, not for startup.
pub const CLAIM_BUDGET: Duration = Duration::from_secs(2);

/// Whether this process owns the app name.
pub enum Claim {
    /// We are the app: hold this for the process's life.
    Primary(Instance),
    /// Somebody else is: they were asked to raise, and this process exits.
    Secondary,
    /// No usable session bus: run, without single-instance protection.
    Unavailable(String),
}

/// The connection that holds the name; dropping it releases it.
pub struct Instance {
    _connection: zbus::Connection,
}

struct RaiseTarget {
    events: Sender<AppEvent>,
}

#[zbus::interface(name = "fr.neezer.Desktop")]
impl RaiseTarget {
    /// The second launch's call: bring the window to the front.
    async fn raise(&self) {
        self.events.send(AppEvent::Raise);
    }
}

/// Ask for the name, once. The caller wraps this in [`CLAIM_BUDGET`].
pub async fn claim(events: Sender<AppEvent>) -> Claim {
    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(error) => return Claim::Unavailable(error.to_string()),
    };
    match connection.request_name(NAME).await {
        Ok(()) => register(connection, events).await,
        Err(zbus::Error::NameTaken) => {
            let raised = connection
                .call_method(Some(NAME), PATH, Some(INTERFACE), "Raise", &())
                .await;
            if raised.is_err() {
                // The owner died between the request and the call: take the
                // name instead of exiting into nothing.
                if connection.request_name(NAME).await.is_ok() {
                    return register(connection, events).await;
                }
            }
            Claim::Secondary
        }
        Err(error) => Claim::Unavailable(error.to_string()),
    }
}

async fn register(connection: zbus::Connection, events: Sender<AppEvent>) -> Claim {
    match connection
        .object_server()
        .at(PATH, RaiseTarget { events })
        .await
    {
        Ok(_) => Claim::Primary(Instance {
            _connection: connection,
        }),
        Err(error) => Claim::Unavailable(error.to_string()),
    }
}
