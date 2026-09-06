//! Deterministic application core: no egui, no threads, no I/O.

pub mod action;

pub use action::{AppAction, AppCore, Clock, Effect, Toast};
