//! Capabilities the core needs from the outside world, as traits. Every
//! adapter ships a fake next to it (see `adapters::fakes`).

pub mod agent;
pub mod events;
pub mod host;
pub mod opener;
pub mod secrets;
pub mod store;
pub mod transcript;
