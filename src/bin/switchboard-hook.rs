//! Hook helper called by Claude Code. Reads the hook JSON from stdin,
//! appends one line to the durable event log, pokes the app's socket as a
//! wake-up, and always exits 0. std only: it must start in well under a
//! millisecond and never break the agent. Implemented by the hooks work
//! item.

fn main() {
    std::process::exit(0);
}
