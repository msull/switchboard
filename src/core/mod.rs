//! Deterministic application core: no egui, no threads, no I/O.

pub mod action;
mod control;
mod controller;
mod definitions;
pub mod dispatch;
pub mod env;
mod events;
pub mod grid;
pub mod model;
pub mod reconcile;
mod sessions;
mod workflow;

#[cfg(test)]
mod tests;

pub use action::{
    AppAction, AppCore, Clock, ConfigStatus, Effect, Notice, TRUST_YES_KEYS, UNDO_WINDOW, View,
};
pub use control::{ControlAction, ControlOutcome};
pub use controller::{DOUBLE_PRESS, DWELL, MenuKind, RadialMenu, UiRequest};
pub use definitions::entry_hash;
pub use dispatch::{
    CONSOLE_NAME, CONSOLE_SPACE, DispatchState, TicketListing, TicketOnly, TicketSort, WaitingAgent,
};
pub use env::{Resolved, ResolvedVar, SecretScope, Source};
pub use model::*;
pub use reconcile::{RECORD_ID_ENV, spawn_spec};
pub use workflow::{SETTLE_PROBES, STALL_AFTER, round_paths, snapshot_dir};
