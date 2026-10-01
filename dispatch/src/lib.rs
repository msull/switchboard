//! Dispatch: tickets (a GitHub issue, a task line) driven through a
//! per-project pipeline of stages on top of Switchboard, which it talks to
//! only over the control port. Tickets are data, sessions are a cache:
//! every request to Switchboard is written to the ticket's ledger before
//! it is sent, so a restart can find what was already made.
//!
//! Layering:
//! - `pipeline`: the TOML file, parsed and validated; pure.
//! - `ticket`: the records (ticket, attempt, decision, ledger); pure.
//! - `store`: the data directory, atomic writes, the writer lock.
//! - `port`, `git`, `github`, `bitbucket`: the outside world behind
//!   traits with fakes.
//! - `scheduler`: one step of one ticket; decides from a probe of the
//!   world, then acts through the traits.
//! - `recover`: the ledger reconciled against Switchboard at start.
//! - `view`: the queue's working set, redrawn whole.
//! - `serve`: Dispatch's own port, tickets as views and the commands.

pub mod bitbucket;
pub mod git;
pub mod github;
pub mod pipeline;
pub mod port;
pub mod recover;
pub mod review;
pub mod scheduler;
pub mod serve;
pub mod store;
pub mod template;
pub mod ticket;
pub mod view;

/// Milliseconds since the epoch, the one time format the records use.
#[must_use]
pub fn epoch_ms(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
