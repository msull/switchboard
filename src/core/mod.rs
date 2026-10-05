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

pub use action::{AppAction, AppCore, Clock, Effect, Notice, View};
pub use control::{ControlAction, ControlOutcome};
pub use controller::{MenuKind, RadialMenu, UiRequest};
pub use dispatch::{
    DispatchState, SupervisorChip, SupervisorState, TicketListing, TicketOnly, TicketSort,
    WaitingAgent,
};
pub use env::{Resolved, ResolvedVar, SecretScope, Source};
pub use model::*;
pub use reconcile::RECORD_ID_ENV;
pub use sessions::can_fork;
pub use workflow::{round_paths, round_status, snapshot_dir};
