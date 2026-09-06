//! Library crate. The app lives here so integration tests in `tests/` can
//! drive it; `src/main.rs` is a thin launcher.
//!
//! Layering:
//! - `core`: deterministic state machine. No egui, no threads, no I/O.
//!   Everything enters through `AppCore::dispatch` and leaves as `Effect`s.
//! - `ports`: traits describing what the core needs from the outside world.
//! - `adapters`: real implementations of the ports, plus fakes for tests.
//! - `app`: owns the core and the adapters; runs effects, feeds results back.
//! - `ui`: egui drawing and input -> actions. Nothing else lives here.

pub mod adapters;
pub mod app;
pub mod core;
pub mod ports;
pub mod ui;

pub use app::SwitchboardApp;
