//! Capabilities the core needs from the outside world, as traits. Every
//! port has a fake in `adapters::fakes`.

pub mod agent;
pub mod artifacts;
pub mod control;
pub mod controller;
pub mod dispatch;
pub mod events;
pub mod host;
pub mod opener;
pub mod project_config;
pub mod round_files;
pub mod secrets;
pub mod store;
pub mod transcript;
