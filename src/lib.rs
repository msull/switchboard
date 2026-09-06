//! Switchboard library crate. `src/main.rs` is a thin launcher and
//! `src/bin/switchboard-hook.rs` is the hook helper.
//!
//! Layering (see `docs/design.md`):
//! - `core`: deterministic. Records, card states, the reconcile, and the
//!   state machine. No egui, no threads, no I/O, no wall clock.
//! - `ports`: traits for the outside world (store, process host, agent
//!   launchers, session events, opener).
//! - `adapters`: real implementations, plus fakes for tests.
//! - `app`: owns core and adapters; runs effects; drains workers.
//! - `ui`: egui drawing and input -> actions.

pub mod adapters;
pub mod app;
pub mod core;
pub mod ports;
pub mod script;
pub mod ui;

pub use app::SwitchboardApp;
