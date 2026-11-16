//! A fake player on the real session bus, for the things a test cannot see:
//! the Omarchy bar's widget, `playerctl`, `busctl`, a phone's MPRIS remote.
//!
//! ```sh
//! cargo run -p nz-mpris --example fake_player        # in one terminal
//! playerctl -p neezer metadata                       # in another
//! playerctl -p neezer play-pause
//! playerctl -p neezer position
//! busctl --user introspect org.mpris.MediaPlayer2.neezer /org/mpris/MediaPlayer2
//! ```
//!
//! It publishes a track that changes on `Next`, ticks its position at the app's
//! 4 Hz, applies every command it receives, prints it, and stops on `Quit` or
//! `Ctrl+C`. The position and state are fake: what is real is the MPRIS surface
//! [`nz_mpris`] publishes, which is what D9's wiring will feed.

use std::{thread, time::Duration};

use nz_mpris::{Metadata, Mpris, MprisCommand, PlaybackStatus};
use tokio::sync::mpsc;

/// (track id, title, artist, album, length in seconds)
const TRACKS: [(&str, &str, &str, &str, f64); 3] = [
    (
        "3135556",
        "Balancing the Dark",
        "Kaya Project",
        "Songs from the Inside",
        312.0,
    ),
    (
        "9101112",
        "Wandering Eye",
        "Fat Freddy's Drop",
        "Based on a True Story",
        428.0,
    ),
    (
        "7f3c8a1e-1111-2222-3333-444455556666",
        "Épisode 12 : le mix du dimanche",
        "Neezer",
        "Podcasts",
        2_940.0,
    ),
];

fn publish(mpris: &nz_mpris::MprisHandle, index: usize, position: f64) {
    let (id, title, artist, album, length) = TRACKS[index % TRACKS.len()];
    mpris.set_track(
        Metadata::new(id, title)
            .artist(artist)
            .album(album)
            .length(length),
    );
    mpris.set_position_s(position);
}

fn main() {
    let (commands, mut received) = mpsc::channel(32);
    let mpris = Mpris::start("neezer", commands);

    let mut index = 0usize;
    let mut position = 0.0f64;
    let mut playing = true;

    publish(&mpris, index, position);
    mpris.set_status(PlaybackStatus::Playing);
    println!("org.mpris.MediaPlayer2.neezer is up; Ctrl+C stops it.");

    loop {
        thread::sleep(Duration::from_millis(250));
        if playing {
            position += 0.25;
            mpris.set_position_s(position);
        }

        while let Ok(command) = received.try_recv() {
            println!("{command:?}");
            match command {
                MprisCommand::PlayPause => {
                    playing = !playing;
                    mpris.set_status(status_of(playing));
                }
                MprisCommand::Play => {
                    playing = true;
                    mpris.set_status(PlaybackStatus::Playing);
                }
                MprisCommand::Pause => {
                    playing = false;
                    mpris.set_status(PlaybackStatus::Paused);
                }
                MprisCommand::Next => {
                    index += 1;
                    position = 0.0;
                    publish(&mpris, index, position);
                }
                MprisCommand::Previous => {
                    index = index.saturating_sub(1);
                    position = 0.0;
                    publish(&mpris, index, position);
                }
                MprisCommand::Seek(offset_us) => {
                    position = (position + offset_us as f64 / 1_000_000.0).max(0.0);
                    mpris.set_position_s(position);
                }
                MprisCommand::SetPosition(_, position_us) => {
                    position = (position_us as f64 / 1_000_000.0).max(0.0);
                    mpris.set_position_s(position);
                }
                MprisCommand::Volume(volume) => mpris.set_volume(volume),
                MprisCommand::Shuffle(shuffle) => mpris.set_shuffle(shuffle),
                MprisCommand::Loop(loop_status) => mpris.set_loop(loop_status),
                MprisCommand::Raise => println!("(the app would raise its window)"),
                MprisCommand::Quit => {
                    println!("quit asked: stopping");
                    return;
                }
            }
        }

        // Keep the loop honest if a client loops the track.
        let length = TRACKS[index % TRACKS.len()].4;
        if position > length {
            position = 0.0;
            index += 1;
            publish(&mpris, index, position);
        }
    }
}

fn status_of(playing: bool) -> PlaybackStatus {
    if playing {
        PlaybackStatus::Playing
    } else {
        PlaybackStatus::Paused
    }
}
