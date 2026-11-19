//! The HOST side on the phone: this device's player, WATCHED and published.
//!
//! The protocol, the fit, the decision and the publish loop are `nz-party`'s
//! (`host.rs`, SC6, extracted from desktop D15b) and this file never
//! second-guesses them. What it adds is the ONE thing a crate cannot know: how
//! to read THIS app's player. The desktop takes its numbers from an `AppState`
//! (`state.player`, `state.audio`, the engine's output clock); the phone has
//! `nz_core::player::Player`, whose hot reads are atomics (MB3b), so the tick
//! costs the person's screen nothing.
//!
//! Four rules, each one the SPA's (`lib/party/host.js`, `hostrules.js`):
//!
//! - **The player is left exactly as it is**: its queue, its crossfade, its
//!   trim. Hosting publishes a line; it never seeks, pauses or loads anything.
//! - **The line is a fit of what is HEARD**, `nz-party`'s `AnchorFit` — the
//!   phone's `Player::position()` is the engine's own clock (the same source
//!   `tap → son` is measured on), not a wall-clock guess.
//! - **A track's name never travels with another track's position**
//!   (`hostrules.js`): `playing` is the transport and `carries` is whether the
//!   ENGINE is emitting this track yet. On the phone that is the difference
//!   between `PlayState::Playing` and `PlayState::Buffering`, and a skip spends
//!   a moment in the second — which is what makes the guests say
//!   « Chargement chez … » instead of playing a frozen position.
//! - **The next track is announced** where it hands over, so a guest starts it
//!   on the beat: `at` is the crossfade's start (or the end of the track when
//!   the person turned the crossfade off), and `gap` is the latency this player
//!   has learnt at its own handovers (`nz-party`'s `Gaps`).

use nz_audio::{Normalization, PartyChannel};
use nz_model::{PartyNext, PartyTrack};
use nz_party::host::{Gaps, Snapshot, norm_wire};

use crate::lists::Row;
use crate::player::{NowPlaying, PlayState, Player, Snapshot as PlayerSnapshot};

/// How often the line is measured: a quarter of a second (`host.js#MEASURE_MS`).
pub const MEASURE_MS: u64 = 250;
/// …and every 40 ms while the line is young (`QUICK_MS`): the first reading
/// after a start is the worst there is.
pub const QUICK_MS: u64 = 40;

/// The track a publish carries, from what the phone's player knows.
///
/// `gain` is `None`: the phone's queue carries no ReplayGain measurement per
/// title (the server's loudness is asked for by the SPA's player, MB3b does not
/// port it), so a guest applies the level's own normalisation and no per-track
/// offset. That is the same result as a track whose gain the server never
/// measured, which is what `norm_gain(None, …)` already handles.
pub fn track_of(now: &NowPlaying) -> PartyTrack {
    PartyTrack {
        id: now.id.clone(),
        title: if now.title.is_empty() {
            now.key.clone()
        } else {
            now.title.clone()
        },
        artist: now.artists.clone(),
        album: now.album.clone(),
        duration: now.duration_s,
        podcast: now.is_episode,
        gain: None,
        cover: now.cover.clone(),
    }
}

/// The next handover, `nz-party`'s `next_plan` arithmetic over the numbers this
/// player exposes: the row after the playhead, the current track's duration, the
/// person's crossfade and the gaps this device has learnt.
///
/// The row after the playhead is the SPA's own rule too (`host.js#planNext` reads
/// the store's next entry, which is the queue's order even under shuffle).
pub fn next_plan(
    index: Option<u32>,
    rows: &[Row],
    duration: f64,
    fade: f64,
    gaps: &Gaps,
) -> Option<PartyNext> {
    let index = index? as usize;
    let next = rows.get(index + 1)?;
    if next.id.is_empty() {
        return None;
    }
    if duration <= 0.0 {
        return None;
    }
    let fade = fade.min(duration / 2.0).max(0.0);
    Some(PartyNext {
        track: Some(PartyTrack {
            id: next.id.clone(),
            title: if next.title.is_empty() {
                next.key.clone()
            } else {
                next.title.clone()
            },
            artist: next.artists.clone(),
            album: next.album.clone(),
            duration: f64::from(next.duration_s),
            podcast: false,
            gain: None,
            cover: next.cover.clone(),
        }),
        at: (duration - fade).max(0.0),
        // The next track's trimmed lead-in: MB3b's trim removes silence from the
        // file it decodes, and its measured bounds are private to the player, so
        // a guest starts at 0 and the seam crossfade hides the few milliseconds a
        // trim would have removed (the desktop's `PartyHost` makes the same call).
        start: 0.0,
        fade,
        gap: gaps.get(fade),
    })
}

/// The host's normalisation level, as the wire spells it.
pub fn norm_of(normalization: Normalization) -> &'static str {
    norm_wire(normalization)
}

/// One tick's numbers, or `None` when nothing is loaded at all (the room is then
/// told once that nothing plays, and nothing more).
///
/// Everything here is a hot read: `Player::snapshot()` takes the machine's lock
/// once, and `position()`, `audio()` and the queue are atomics or cheap copies —
/// four times a second on a screen that is drawing anyway.
pub fn snapshot(player: &Player, channel: &PartyChannel, gaps: &Gaps) -> Option<Snapshot> {
    let state: PlayerSnapshot = player.snapshot();
    let current = state.current.as_ref()?;
    let position = player.position();
    let audio = player.audio();
    let rows = player.queue_rows();
    let duration = if current.duration_s > 0.0 {
        current.duration_s
    } else {
        position.duration_s
    };
    let fade = f64::from(audio.crossfade_s);
    Some(Snapshot {
        id: current.id.clone(),
        track: track_of(current),
        // The engine is emitting THIS track — not the previous one it still
        // holds between a skip and the new source being attached.
        carries: matches!(state.state, PlayState::Playing),
        playing: matches!(state.state, PlayState::Playing | PlayState::Buffering),
        heard: position.position_s,
        fade,
        duration,
        norm: norm_of(audio.normalization.audio()),
        // A device-mapping step invalidates the fit (`host.js` restarts its line
        // when its output clock moves).
        generation: channel.output_clock().generation,
        next: next_plan(state.index, &rows, duration, fade, gaps),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, title: &str, duration: u32) -> Row {
        Row {
            key: format!("deezer:{id}"),
            id: id.to_owned(),
            title: title.to_owned(),
            artists: "Kaya Project".to_owned(),
            album: "Songs from the Inside".to_owned(),
            album_id: None,
            duration_s: duration,
            cover: None,
            explicit: false,
            liked: false,
            disliked: false,
            unavailable: false,
            downloaded: false,
            source: "deezer".to_owned(),
            position: 0,
            played_at: 0,
            added_at: 0,
        }
    }

    /// The announcement is `planNext`'s: the handover starts the crossfade
    /// before the end, a cut lands on the end, and no next row means no plan.
    #[test]
    fn the_next_handover_starts_where_the_crossfade_does() {
        let rows = vec![row("1", "Un", 300), row("2", "Deux", 200)];
        let gaps = Gaps::default();
        let cut = next_plan(Some(0), &rows, 300.0, 0.0, &gaps).expect("a cut plan");
        assert_eq!(cut.at, 300.0);
        assert_eq!(cut.fade, 0.0);
        assert_eq!(cut.gap, gaps.get(0.0));
        assert_eq!(cut.track.as_ref().map(|t| t.id.as_str()), Some("2"));
        assert_eq!(cut.track.as_ref().map(|t| t.title.as_str()), Some("Deux"));

        let faded = next_plan(Some(0), &rows, 300.0, 4.0, &gaps).expect("a crossfade plan");
        assert_eq!(faded.at, 296.0);
        assert_eq!(faded.fade, 4.0);
        assert_eq!(faded.gap, gaps.get(4.0));

        // The last row has nothing after it, and no playhead has no plan.
        assert!(next_plan(Some(1), &rows, 200.0, 0.0, &gaps).is_none());
        assert!(next_plan(None, &rows, 200.0, 0.0, &gaps).is_none());
        // A row the server has no id for is not something a guest can stream.
        let mut anonymous = rows.clone();
        anonymous[1].id.clear();
        assert!(next_plan(Some(0), &anonymous, 300.0, 0.0, &gaps).is_none());
    }

    /// A crossfade longer than half the track is capped, so a 2 s track cannot
    /// announce a handover before its own start (the SPA's `min(duration/2)`).
    #[test]
    fn a_crossfade_never_eats_more_than_half_the_track() {
        let rows = vec![row("1", "Court", 2), row("2", "Après", 100)];
        let plan = next_plan(Some(0), &rows, 2.0, 8.0, &Gaps::default()).expect("a plan");
        assert_eq!(plan.fade, 1.0);
        assert_eq!(plan.at, 1.0);
    }

    /// The wire's four levels (`norm_wire`, `_NORM_LEVELS` on the server).
    #[test]
    fn the_normalisation_level_is_the_wires_word() {
        assert_eq!(norm_of(Normalization::Off), "off");
        assert_eq!(norm_of(Normalization::Low), "low");
        assert_eq!(norm_of(Normalization::Medium), "medium");
        assert_eq!(norm_of(Normalization::High), "high");
    }

    /// **The phone's player, read the way the host publishes it.** A real
    /// `Player` over a throwaway core and a queue of two tracks: the snapshot the
    /// tick hands `nz-party` carries the playing row, its duration, the person's
    /// own normalisation level, the output clock's generation and the handover it
    /// will announce — and it never needs the network to say so (this core has no
    /// server, so the stream fails fast and the room is told "loading", which is
    /// exactly what `carries` is for).
    #[test]
    fn the_host_reads_the_players_own_numbers() {
        use std::sync::Arc;

        use crate::player::{Command, PlayState, Player, TrackRef};

        let dir = crate::party::tests::TempCore::new("party-host-snapshot");
        let core = Arc::new(dir.take().0);
        let player = Player::open(Arc::clone(&core)).expect("the player");
        player.command(
            Command::PlayQueue {
                tracks: vec![
                    TrackRef {
                        key: "deezer:3000001".to_owned(),
                        id: "3000001".to_owned(),
                        title: "Balancing the Dark".to_owned(),
                        artists: "Kaya Project".to_owned(),
                        album: "Songs from the Inside".to_owned(),
                        duration_s: 312,
                        cover: Some("/api/cover/album-1".to_owned()),
                        ..TrackRef::default()
                    },
                    TrackRef {
                        key: "deezer:3000002".to_owned(),
                        id: "3000002".to_owned(),
                        title: "Suivant".to_owned(),
                        artists: "Kaya Project".to_owned(),
                        duration_s: 200,
                        ..TrackRef::default()
                    },
                ],
                start: 0,
                context: None,
            },
            0,
        );
        let gaps = Gaps::default();
        let channel = player.party();
        let line = snapshot(&player, &channel, &gaps).expect("a playing row");
        assert_eq!(line.id, "3000001");
        assert_eq!(line.track.title, "Balancing the Dark");
        assert_eq!(line.track.artist, "Kaya Project");
        assert_eq!(line.duration, 312.0);
        // The person's own level travels with the line, so every device applies
        // the same one (the SPA's `norm`).
        assert_eq!(line.norm, "off");
        // `carries` never outruns `playing`: the engine cannot hold a track the
        // transport is not playing.
        assert!(!line.carries || line.playing);
        // With no output rendering, the track is loading — the state a guest
        // hears as « Chargement chez … », never a frozen position.
        assert_eq!(
            matches!(player.snapshot().state, PlayState::Buffering),
            line.playing && !line.carries,
            "the phone's Buffering IS playing-without-carries"
        );
        let next = line.next.expect("the handover");
        assert_eq!(
            next.track.as_ref().map(|track| track.id.as_str()),
            Some("3000002")
        );
        assert_eq!(next.at, 312.0, "a cut lands on the end of the track");
        assert_eq!(next.gap, gaps.get(0.0));
        player.shutdown();
        // An empty queue has nothing to publish at all (a fresh store: the queue
        // above is persisted, and a restarted player restores it).
        let fresh = crate::party::tests::TempCore::new("party-host-empty");
        let empty_core = Arc::new(fresh.take().0);
        let empty = Player::open(Arc::clone(&empty_core)).expect("an empty player");
        assert!(snapshot(&empty, &empty.party(), &gaps).is_none());
        empty.shutdown();
    }

    /// The published track is the player's own row: a title that is missing is
    /// the key (honest and visible), never an empty name on every phone.
    #[test]
    fn a_track_without_a_title_publishes_its_key() {
        let now = NowPlaying {
            key: "local:42".to_owned(),
            id: "42".to_owned(),
            title: String::new(),
            artists: "Anonyme".to_owned(),
            duration_s: 180.0,
            ..NowPlaying::default()
        };
        let track = track_of(&now);
        assert_eq!(track.title, "local:42");
        assert_eq!(track.id, "42");
        assert_eq!(track.duration, 180.0);
        assert!(track.gain.is_none());
    }
}
