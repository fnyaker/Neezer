//! The MPRIS server: `org.mpris.MediaPlayer2.neezer` on the session bus, on its
//! own `LocalSet` thread.
//!
//! Owned by lot M1 (`docs/agents/desktop.md`). This is D0's skeleton, and its
//! test pins the wire vocabulary the bar and `playerctl` read: MPRIS spells
//! the playback state `Playing`/`Paused`/`Stopped`, exactly what
//! `mpris_server::PlaybackStatus::as_str` emits.

#[cfg(test)]
mod tests {
    use mpris_server::PlaybackStatus;

    #[test]
    fn the_playback_vocabulary_is_the_one_playerctl_reads() {
        assert_eq!(PlaybackStatus::Playing.as_str(), "Playing");
        assert_eq!(PlaybackStatus::Paused.as_str(), "Paused");
        assert_eq!(PlaybackStatus::Stopped.as_str(), "Stopped");
    }
}
