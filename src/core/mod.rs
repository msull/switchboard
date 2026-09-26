//! Deterministic application core: no egui, no threads, no I/O.

pub mod action;
mod controller;
mod definitions;
pub mod env;
mod events;
pub mod grid;
pub mod model;
pub mod reconcile;
mod sessions;
mod workflow;

#[cfg(test)]
mod tests;

pub use action::{AppAction, AppCore, Clock, ConfigStatus, Effect, Notice, UNDO_WINDOW, View};
pub use controller::{RadialMenu, UiRequest};
pub use definitions::entry_hash;
pub use env::{Resolved, ResolvedVar, SecretScope, Source};
pub use model::*;
pub use reconcile::{RECORD_ID_ENV, spawn_spec};
pub use workflow::{SETTLE_PROBES, STALL_AFTER, round_paths, snapshot_dir};
