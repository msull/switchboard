//! Deterministic application core: no egui, no threads, no I/O.

pub mod action;
mod events;
pub mod model;
pub mod reconcile;
mod sessions;

#[cfg(test)]
mod tests;

pub use action::{AppAction, AppCore, Clock, Effect, Notice, View};
pub use model::*;
pub use reconcile::{RECORD_ID_ENV, spawn_spec};
