//! Dispatch, as the app sees it: a port that answers with tickets as
//! views and takes the commands the `dispatch` command line has. The
//! app never reads Dispatch's records; a runner on another machine
//! looks the same through a forwarded socket.

use std::io;
use std::path::PathBuf;

pub use dispatch_control::{
    AttemptView, Body, DecisionView, LaneView, ProjectView, Reply, Status, TicketView,
    WorktreesView,
};

pub trait DispatchPort: Send {
    /// The `dispatch` executable to type into the console, as a path
    /// the shell can run.
    fn command(&self) -> PathBuf;
    /// Dispatch's data directory: where the socket is, and the console's
    /// working directory.
    fn data_dir(&self) -> PathBuf;
    /// One request, one reply. An error is no runner (or a runner gone
    /// mid-call); a `Reply::Failed` is an answer.
    fn call(&mut self, body: &Body) -> io::Result<Reply>;
}
