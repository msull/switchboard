//! Dispatch: tickets (a GitHub issue, a task line) driven through a
//! per-project pipeline of stages on top of Switchboard, which it talks to
//! only over the control port. Tickets are data, sessions are a cache:
//! every request to Switchboard is written to the ticket's ledger before
//! it is sent, so a restart can find what was already made.
//!
//! Layering:
//! - `pipeline`: the TOML file, parsed and validated; pure.
//! - `ticket`: the records (ticket, attempt, decision, ledger); pure.
//! - `history`: which commits a code review's fix rounds fold into, as
//!   plans; pure.
//! - `store`: the data directory, atomic writes, the writer lock; the
//!   one ticket write, which logs its events first.
//! - `events`: what a ticket write changed, as events (pure), and the
//!   log file they are appended to, read and waited on.
//! - `health`: the runner's call counters and `runner.json`, beside
//!   `store`, and the check of both sockets.
//! - `report`: how a ticket went, from its record and round files;
//!   pure.
//! - `port`, `git`, `github`, `bitbucket`: the outside world behind
//!   traits with fakes.
//! - `template`: prompt fields filled from a map; pure.
//! - `scheduler`: one step of one ticket; decides from a probe of the
//!   world, then acts through the traits.
//! - `review`: the code review stage's rounds, stepped by the scheduler.
//! - `recover`: the ledger reconciled against Switchboard at start.
//! - `view`: the queue's working set, redrawn whole.
//! - `serve`: Dispatch's own port, tickets as views and the commands.

// Tests assert emptiness with `assert!` throughout; the rest of the
// crate is held to the lint.
#![cfg_attr(test, allow(clippy::assert_is_empty))]

pub mod bitbucket;
mod confine;
pub mod events;
pub mod git;
pub mod github;
pub mod health;
pub mod history;
pub mod pipeline;
pub mod port;
pub mod recover;
pub mod report;
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

/// The command line's usage, printed on a usage error (exit 64).
pub const USAGE: &str = "usage:
  dispatch take <project> <issue-number>   make a ticket from an issue and queue it
  dispatch take <project> pr <lane>/<n>... a ticket from someone's pull requests, one per lane,
                                           on <project>.pr.toml; the lane may be left off with one lane
  dispatch run [--once]                    drive every ticket (once, or until stopped)
  dispatch decide <ticket> <decision> <answer> [--note <text>]
  dispatch decisions                       what waits on you
  dispatch status                          every ticket, its stage and state
  dispatch queue <project> [<ticket>...]   show, or reorder, a project's queue
  dispatch resume <ticket>                 a parked ticket back to active
  dispatch close <ticket> [--reason <text>]  a ticket closed, its trees removed (its branches are kept; close lists them)
  dispatch worktrees [<path>] [--migrate]  where tickets' trees go (default ~/.dispatch/worktrees);
                                           with a path, set it; --migrate moves idle tickets' trees there

Supervising (see docs/dispatch-agent-guide.md):
  dispatch events [--since <seq>] [--follow] [--ticket <id>]... [--project <name>] [--json]
                                           what happened, from the event log
  dispatch wait <ticket> [--for decision|stage|pr|closed|any] [--since <seq>] [--timeout <secs>] [--json]
                                           block until it happens: exit 0 matched,
                                           2 timed out, 3 the ticket parked or closed
  dispatch show <ticket> [--json]          one ticket: stage, lanes, attempts, decisions, files
  dispatch report <ticket> [--json]        how a ticket went
  dispatch report --project <name> [--since YYYY-MM-DD] [--json]
  dispatch tail <ticket> [--lines N]       what its running agents' panes show
  dispatch health [--timeout <secs>] [--stale <secs>] [--json]
                                           is the runner alive and getting on (exit 1 if not)

Data: $DISPATCH_DATA_DIR (default ~/Library/Application Support/Dispatch).
Switchboard: $SWITCHBOARD_DATA_DIR/control.sock (default Switchboard's).
While `run` is up it serves the same commands on <data>/dispatch.sock.";

#[cfg(test)]
mod tests {
    /// The agent guide is the CLI's contract: every verb the usage
    /// lists is in it.
    #[test]
    fn every_verb_in_the_usage_is_in_the_agent_guide() {
        let guide = include_str!("../../docs/dispatch-agent-guide.md");
        let verbs: Vec<&str> = super::USAGE
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("dispatch "))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        assert!(verbs.len() >= 16, "{verbs:?}");
        for verb in verbs {
            assert!(
                guide.contains(&format!("dispatch {verb}")),
                "the guide never names `dispatch {verb}`"
            );
        }
    }
}
