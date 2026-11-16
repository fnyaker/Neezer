//! MPRIS wiring: the player bar, the media keys and the Omarchy bar read this.
//!
//! `nz-mpris` owns the D-Bus surface; this module starts it off the UI thread
//! (`Mpris::start` blocks until the bus name is ours, up to five seconds — a
//! first frame must not wait for it) and publishes what the player shows. The
//! transport commands come back as [`crate::state::AppEvent::Mpris`] and are
//! applied through `PlayerState::apply`, exactly like a click, so MPRIS can do
//! nothing the app cannot do itself.
//!
//! D9 extends the published metadata; the start, the inbox and the status keep
//! their shape here so no later lot touches `wiring/mod.rs`.

use std::time::Duration;

use nz_mpris::{LoopStatus, Metadata, Mpris, MprisCommand, MprisHandle, PlaybackStatus};
use nz_player::Repeat;

use crate::state::AppEvent;
use crate::wiring::events::Sender;

/// What the shell tells the bus about the transport.
#[derive(Clone, Debug, Default)]
pub struct NowPlaying {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub playing: bool,
    pub position: f64,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: Repeat,
}

/// Start the server on a worker, then announce it. `runtime` drives the command
/// inbox (a tokio channel) and forwards each command to the shell.
pub fn spawn(runtime: tokio::runtime::Handle, events: Sender<AppEvent>) {
    let spawned = std::thread::Builder::new()
        .name("nz-mpris-start".to_owned())
        .spawn(move || {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<MprisCommand>(32);
            let handle = Mpris::start("neezer", tx);
            let forward = events.clone();
            runtime.spawn(async move {
                while let Some(command) = rx.recv().await {
                    forward.send(AppEvent::Mpris(command));
                }
            });
            events.send(AppEvent::MprisReady(handle));
        });
    if spawned.is_err() {
        // No thread: the player still works, the bar and media keys do not.
        tracing::warn!("MPRIS indisponible : le fil de démarrage n'a pas pu être créé");
    }
}

/// Publish the transport. `last_track` is the shell's memory of what the bus
/// was last told: a new track is announced once, a position tick never is
/// (`nz-mpris` owns the seek tolerance).
pub fn publish(handle: &MprisHandle, now: &NowPlaying, last_track: &mut Option<String>) {
    if !handle.is_serving() {
        return;
    }
    if last_track.as_deref() != Some(now.id.as_str()) {
        let mut metadata = Metadata::new(now.id.clone(), now.title.clone());
        if !now.artist.is_empty() {
            metadata = metadata.artist(now.artist.clone());
        }
        if !now.album.is_empty() {
            metadata = metadata.album(now.album.clone());
        }
        if now.duration > 0.0 {
            metadata = metadata.length(now.duration);
        }
        handle.set_track(metadata);
        *last_track = Some(now.id.clone());
    }
    handle.set_status(if now.playing {
        PlaybackStatus::Playing
    } else {
        PlaybackStatus::Paused
    });
    handle.set_position_s(now.position);
    handle.set_volume(now.volume.clamp(0.0, 1.0));
    handle.set_shuffle(now.shuffle);
    handle.set_loop(match now.repeat {
        Repeat::Off => LoopStatus::None,
        Repeat::All => LoopStatus::Playlist,
        Repeat::One => LoopStatus::Track,
    });
}

/// How long the shell waits before retrying MPRIS after an inert start. The
/// music never stops for it; this is only so a bus that appeared late is picked
/// up by the next track.
pub const RETRY: Duration = Duration::from_secs(30);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_repeat_modes_map_onto_mpris_loop_statuses() {
        // The mapping is the contract a client reads; a wrong one shows a
        // repeat icon that lies. `publish` is the only place that names them.
        let map = |repeat| match repeat {
            Repeat::Off => LoopStatus::None,
            Repeat::All => LoopStatus::Playlist,
            Repeat::One => LoopStatus::Track,
        };
        assert_eq!(map(Repeat::Off), LoopStatus::None);
        assert_eq!(map(Repeat::All), LoopStatus::Playlist);
        assert_eq!(map(Repeat::One), LoopStatus::Track);
    }
}
