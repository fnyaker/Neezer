//! `mood_card`: the mood-mix tile a later lot expected to need.
//!
//! D8a's Accueil already draws the mixes itself (`screens/home.rs`: the
//! `MoodCard` tiles, `GET /api/moods` + `GET /api/moods/<id>`, the
//! "pas assez de titres mesurés" sentence and the queue it starts), so the file
//! this lot was allotted has no caller. A module with an unused builder is dead
//! code a `-D warnings` build refuses, and a second mood tile would drift from
//! the one on screen. It stays empty; the note in `docs/agents/desktop.md`
//! (D12) records why.
