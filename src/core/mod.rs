//! Deterministic application core: no egui, no threads, no I/O.

pub mod action;
mod control;
mod controller;
mod definitions;
pub mod dispatch;
pub mod env;
mod events;
pub mod file_refs;
pub mod grid;
pub mod model;
pub mod reconcile;
mod sessions;
mod workflow;

#[cfg(test)]
mod tests;

pub use action::{
    AppAction, AppCore, Clock, ENV_SETUP_LOCKED, ENV_SETUP_WINDOW, Effect, Notice, RULE_SCALE,
    View, clamp_scale,
};
pub use control::{ControlAction, ControlOutcome, GrantTarget, aws_view};
pub use controller::{MenuKind, RadialMenu, UiRequest};
pub use dispatch::{
    DISPATCH_POLL, DispatchState, RunnerStanding, SupervisorChip, SupervisorState, TicketListing,
    TicketOnly, TicketSort, WaitingAgent,
};
pub use env::{Resolved, ResolvedVar, SecretScope, SetsResolved, Source, resolve_sets, token_hash};
pub use model::*;
pub use reconcile::{RECORD_ID_ENV, RECORD_TOKEN_ENV};
pub use sessions::can_fork;
pub use workflow::{round_paths, round_status, snapshot_dir};
