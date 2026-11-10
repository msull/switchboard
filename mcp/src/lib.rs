//! `switchboard-mcp`: an MCP server, run by an agent session as a
//! stdio child, that watches and drives paired Switchboard hosts over
//! their remote port, as a headset does: pinned TLS, a bearer token
//! from pairing, and the port's own routes and scope.
//!
//! - `pin`: the TLS client that trusts one key, copied from the app.
//! - `http`: one HTTPS exchange over it.
//! - `config`: the paired hosts, a private file in the config dir.
//! - `events`: one shared event subscription per host, for `wait`.
//! - `tools`: what an agent can ask, each mapped to the port.
//! - `server`: the stdio JSON-RPC loop.
//! - `pair`: the command line's pairing, listing and forgetting.

/// The paired hosts and their private file.
pub mod config;
/// One shared event subscription per host.
pub mod events;
/// One pinned HTTPS exchange.
pub mod http;
/// The command line's pairing, listing and forgetting.
pub mod pair;
/// The pinned TLS client.
pub mod pin;
/// The stdio JSON-RPC loop.
pub mod server;
/// What an agent can ask, each mapped to the port.
pub mod tools;

pub use server::Server;
