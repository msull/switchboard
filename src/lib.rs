//! Switchboard library crate. `src/main.rs` is a thin launcher and
//! `src/bin/switchboard-hook.rs` is the hook helper.
//!
//! Layering (see `docs/design.md`):
//! - `core`: deterministic. Records, card states, the reconcile, and the
//!   state machine. No egui, no threads, no I/O, no wall clock.
//! - `ports`: traits for every capability the app needs from outside
//!   the process, one module per port.
//! - `adapters`: real implementations, plus fakes for tests.
//! - `app`: owns core and adapters; runs effects; drains workers.
//! - `ui`: egui drawing and input -> actions.

// Tests assert emptiness with `assert!` throughout; the rest of the
// crate is held to the lint.
#![cfg_attr(test, allow(clippy::assert_is_empty))]

pub mod adapters;
pub mod app;
pub mod core;
pub mod ports;
pub mod script;
pub mod ui;

pub use app::SwitchboardApp;
