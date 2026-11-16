//! Background work: the subsystems that do I/O, and the shell's glue to them.
//!
//! D7 wires what is merged (`net`, `engine`, `viz`, `mpris`, `instance`); the
//! files below are declared now and filled by the lots that own them, so no
//! later lot edits this module. A stub is a real module with a doc comment, not
//! a placeholder that a parallel lot would have to add: wave 5 runs on disjoint
//! files, which is only true if the declaration list is already complete.

pub mod analysis;
pub mod engine;
pub mod events;
pub mod instance;
pub mod mpris;
pub mod net;
pub mod offline;
pub mod party_guest;
pub mod party_host;
pub mod remote_controller;
pub mod remote_host;
pub mod transcribe;
pub mod viz;
