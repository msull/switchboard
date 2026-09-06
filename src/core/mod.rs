//! Deterministic application core: no egui, no threads, no I/O.

pub mod action;
pub mod model;

pub use action::{AppAction, AppCore, Clock, Effect, Notice, View};
pub use model::*;
