//! Every action the shell can dispatch, in one place so no later lot edits
//! `mod.rs` or `keymap.rs` to add one.
//!
//! An action is a *verb of the application*, not of a screen: the keyboard
//! binding, the mouse click and a remote command all end up in the same
//! handler, which is what keeps the two input paths from drifting apart
//! (`docs/agents/desktop.md`, *Keyboard AND mouse*). Actions a later lot owns
//! are declared here from the start and do nothing until that screen handles
//! them; the alternative — a lot adding a variant later — would mean every lot
//! touching this file and the keymap, which the plan forbids after D7.

use gpui_kit as gpui;

gpui::actions!(
    neezer,
    [
        // --- application ---------------------------------------------------
        Quit,
        Logout,
        Retry,
        DismissToast,
        ShowShortcuts,
        CloseTop,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        // --- navigation ----------------------------------------------------
        Back,
        Forward,
        GoHome,
        GoSearch,
        GoLibrary,
        GoPodcasts,
        GoReleases,
        GoNowPlaying,
        GoQueue,
        GoSettings,
        // --- transport -----------------------------------------------------
        PlayPause,
        NextTrack,
        PreviousTrack,
        SeekForward,
        SeekBackward,
        VolumeUp,
        VolumeDown,
        Mute,
        ToggleShuffle,
        CycleRepeat,
        ToggleQueue,
        ToggleFullscreen,
        ToggleViz,
        // --- lists and rows -------------------------------------------------
        ListNext,
        ListPrevious,
        ListFirst,
        ListLast,
        ListPageUp,
        ListPageDown,
        PlayFromHere,
        PlayNextHere,
        AddHereToQueue,
        RowContextMenu,
        LikeHere,
        DislikeHere,
        SelectAll,
        RemoveFromQueue,
        ClearQueue,
        MoveRowUp,
        MoveRowDown,
        // --- track and playlist sheets --------------------------------------
        AddToPlaylist,
        StartRadio,
        ShowCredits,
        ReplaceTrack,
        DeleteTrack,
        DownloadSelection,
        RemoveDownload,
        NewPlaylist,
        ImportPlaylist,
        SharePlaylist,
        ExportPlaylist,
        // --- player sheets and settings -------------------------------------
        SleepTimer,
        QualityMenu,
        SpeedUp,
        SpeedDown,
        SaveSettings,
        Refresh,
        // --- remote, party, transcription -----------------------------------
        PartyStart,
        RemoteClaim,
        StartTranscription,
        StopTranscription,
        // --- login ----------------------------------------------------------
        SubmitLogin,
        FocusNextField,
        FocusPreviousField,
    ]
);

/// `Seek` is the one action that carries a value (a relative position asked for
/// by the progress bar). It is not a keymap binding: the zones of the bar set
/// [`crate::state::AppState::seek_target`] and dispatch it, so a click and
/// `Shift+→` go through the same `PlayerState::apply` path.
#[derive(Clone, Copy, Debug, PartialEq, gpui::Action)]
#[action(namespace = neezer, no_json)]
pub struct SeekFraction(pub f64);
