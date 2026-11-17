//! Shared widgets, one module per thing a screen draws.
//!
//! D7 declares every widget a later lot names, empty on purpose: wave 5 runs
//! the screen lots in parallel on disjoint files, which requires that none of
//! them needs to edit this list. A widget's lot fills its file.

pub mod bars;
pub mod card;
pub mod chapters;
pub mod context_menu;
pub mod cover;
pub mod discovery_slider;
pub mod download_badge;
pub mod download_control;
pub mod episode_panel;
pub mod flow;
pub mod folder_tree;
pub mod genre_bulk_sheet;
pub mod genre_explorer;
pub mod genre_extractor;
pub mod genre_label;
pub mod genre_tag_sheet;
pub mod genre_tree;
pub mod lyrics;
pub mod markers;
pub mod mood_card;
pub mod page_scroll;
pub mod playlist_picker;
pub mod quality_menu;
pub mod recap_story;
pub mod scope;
pub mod sheets;
pub mod sleep_timer;
pub mod speed;
pub mod track_list;
pub mod transcript_panel;
