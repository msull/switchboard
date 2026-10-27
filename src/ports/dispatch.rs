//! Dispatch, as the app sees it: a port that answers with tickets as
//! views and takes the commands the `dispatch` command line has. The
//! app never reads Dispatch's records; a runner on another machine
//! looks the same through a forwarded socket.

use std::io;
use std::path::PathBuf;

pub use dispatch_control::{
    AttemptUsage, AttemptView, Body, BuildId, ChecksView, DecisionView, EventView, EventsView,
    EvidenceView, LaneView, PathsView, PlanRevision, PlanRoundView, ProjectView, PullRequestView,
    Reply, ReviewRoundView, RewriteView, ServiceView, SessionUsage, StageUsage, StaleWhy, Status,
    SupervisorView, TicketView, Tokens, UsageSource, UsageView, WorktreesView, nudged, stale,
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
    /// Whether a call may take real time (a socket to a runner that
    /// holds a lock while it works). The app then calls from a thread
    /// of its own; a port that answers at once is called in place.
    fn may_block(&self) -> bool {
        false
    }
    /// The `dispatch` at `command()` as it is on disk now, which a
    /// restart would run; compared with the build the runner started
    /// with. Only its time and length are read, so `version` is blank.
    /// `None` when it cannot be read, or for a port with no file.
    fn build_on_disk(&self) -> Option<BuildId> {
        None
    }
}
