//! Dispatch as the app shows it: the runner's last status, the
//! artifacts, events and full tickets read for the ticket page, and the
//! console, a shell session of the app's own where `dispatch` commands
//! are typed. The core holds views from Dispatch's port and never its
//! records; a status is data that arrived, a decision answered is a call
//! the app runs, and the reply is one more action.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use super::action::{AppAction, AppCore, Clock, Effect, Out, View};
use super::model::{
    CardState, Launch, PageWindow, ProjectId, RecordId, Run, SessionKind, SessionRecord, Space,
    SpaceId,
};
use crate::ports::dispatch::{
    Body, DecisionView, EventView, ProjectView, Reply, Status, TicketView,
};
use crate::ports::host::Liveness;

/// An agent of a ticket that waits on the user for itself, with the
/// attempt it runs and why it waits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingAgent {
    pub session: RecordId,
    pub ticket: String,
    pub stage: String,
    pub context: String,
    pub reason: String,
}

/// What a project's supervisor session is doing, as its chip says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorState {
    /// No session, or one this window does not have.
    None,
    /// Its pane is starting or in a turn.
    Working,
    /// Its card waits on the owner.
    WaitingOnYou,
    /// Its pane runs and its turn is over.
    Idle,
    /// No pane: it ended or was never started here.
    Cold,
    /// Claude asks whether to trust the workspace.
    AsksTrust,
}

impl SupervisorState {
    /// The chip's word for the state.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Working => "working",
            Self::WaitingOnYou => "waiting on you",
            Self::Idle => "idle",
            Self::Cold => "cold",
            Self::AsksTrust => "asks to trust its folder",
        }
    }
}

/// A project's supervisor for the board: its session in this window,
/// its state, and whether Resume is offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorChip {
    /// The session's record in this window; `None` when the project has
    /// no session or this window does not hold it.
    pub session: Option<RecordId>,
    /// What the session is doing.
    pub state: SupervisorState,
    /// An agent with a conversation to resume and no pane running.
    pub resumable: bool,
    /// Why it waits on you, while it does: what the session asked, say.
    pub reason: Option<String>,
}

/// A column the ticket table can be ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TicketSort {
    /// Last written, newest first by default.
    #[default]
    Updated,
    /// When the ticket was taken.
    Created,
    /// The project's name.
    Project,
    /// The issue or pull request, by kind and number.
    Source,
    /// The title, ignoring case.
    Title,
    /// How far along its pipeline the ticket is.
    Stage,
    /// What the ticket waits on: your answers first, then running
    /// work, then held, parked and closed.
    Standing,
}

impl TicketSort {
    /// Every column, in the order the table shows them.
    pub const ALL: [Self; 7] = [
        Self::Source,
        Self::Title,
        Self::Project,
        Self::Stage,
        Self::Standing,
        Self::Updated,
        Self::Created,
    ];

    /// The column's header.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Updated => "Updated",
            Self::Created => "Created",
            Self::Project => "Project",
            Self::Source => "Ticket",
            Self::Title => "Title",
            Self::Stage => "Stage",
            Self::Standing => "Standing",
        }
    }

    /// Whether the column reads best newest or most urgent first, so
    /// the first click on its header sorts that way.
    #[must_use]
    pub fn descends_first(self) -> bool {
        matches!(self, Self::Updated | Self::Created)
    }
}

/// Which tickets the table lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TicketOnly {
    /// Every ticket, whatever its state.
    #[default]
    All,
    /// A decision pending, or an agent at a prompt of its own.
    Waiting,
    /// Running on its pipeline.
    Active,
    /// Stopped, or on its way to stopping or closing: parking, parked,
    /// or a close still under way.
    Parked,
    /// Closed for good.
    Closed,
}

impl TicketOnly {
    /// Every filter, in the order the picker lists them.
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Waiting,
        Self::Active,
        Self::Parked,
        Self::Closed,
    ];

    /// The filter's name in the picker.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Any state",
            Self::Waiting => "Waiting on you",
            Self::Active => "Active",
            Self::Parked => "Parked",
            Self::Closed => "Closed",
        }
    }
}

/// How the ticket table is narrowed and ordered: a project, a state,
/// words that must appear somewhere on the row, and the column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketListing {
    pub project: Option<String>,
    pub only: TicketOnly,
    pub text: String,
    pub sort: TicketSort,
    pub ascending: bool,
}

impl Default for TicketListing {
    fn default() -> Self {
        Self {
            project: None,
            only: TicketOnly::All,
            text: String::new(),
            sort: TicketSort::Updated,
            ascending: false,
        }
    }
}

impl TicketListing {
    /// A header click: the same column flips the direction, another
    /// column starts the way it reads best.
    pub fn sort_by(&mut self, sort: TicketSort) {
        if self.sort == sort {
            self.ascending = !self.ascending;
        } else {
            self.sort = sort;
            self.ascending = !sort.descends_first();
        }
    }
}

/// A ticket's events as read through the port so far. Transient: on
/// launch it starts empty and fills on the ticket page's first draw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TicketEvents {
    /// The cursor: the highest seq the runner has read for this ticket.
    pub last: u64,
    /// The ticket's events, oldest first, none withdrawn.
    pub events: Vec<EventView>,
    /// The ticket's `updated_ms` at the last ask; `None` before the
    /// first, or after a call that never reached the runner.
    pub asked_at: Option<u64>,
    /// A read is on its way; nothing more is asked until it answers.
    pub in_flight: bool,
    /// The runner does not serve events (an older build); the page reads
    /// the record instead until the runner reconnects.
    pub unavailable: bool,
}

/// A ticket read in full, with what only that reply carries (its paths,
/// its lanes' clones). Transient, like `TicketEvents`; kept when the
/// runner goes away, so the page still reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fetched {
    /// The last reply, kept until the next replaces it.
    pub view: Option<TicketView>,
    /// The ticket's `updated_ms` at the last ask, as on `TicketEvents`.
    pub asked_at: Option<u64>,
    /// A read is on its way; nothing more is asked until it answers.
    pub in_flight: bool,
}

/// One artifact's reads that have not brought its text yet. Transient,
/// like `TicketEvents`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactRead {
    /// The ticket's `updated_ms` at the last ask: a failed read is asked
    /// again on the page once the ticket changes, since an agent writes
    /// its notes after the runner hands out their path. `None` after a
    /// call that never reached the runner.
    pub asked_at: Option<u64>,
    /// A read is on its way; nothing more is asked until it answers.
    pub in_flight: bool,
    /// Why the last read failed, as the runner said.
    pub failed: Option<String>,
}

impl ArtifactRead {
    /// The last read failed because the file is not there: the runner
    /// reports the OS's error, and `ENOENT` is 2 on every platform it
    /// runs on.
    #[must_use]
    pub fn missing(&self) -> bool {
        self.failed
            .as_deref()
            .is_some_and(|r| r.ends_with("(os error 2)"))
    }
}

/// One line of a ticket's timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineRow {
    /// When it happened, in milliseconds since the epoch.
    pub at_ms: u64,
    /// The event log's word: `attempt-started`, `answered`, ...
    pub kind: String,
    /// The log's line, or one made from the record in its words.
    pub text: String,
    /// The attempt the row is about, as stage and number.
    pub attempt: Option<(String, u32)>,
    /// The decision the row is about, by id.
    pub decision: Option<String>,
    /// How long the attempt ran, on an `attempt-ended` row.
    pub duration_ms: Option<u64>,
    /// The attempt whose card is drawn under this row: each attempt's
    /// newest end row, or its newest start row while it runs.
    pub details: Option<(String, u32)>,
}

/// Consecutive timeline rows of one stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineGroup {
    /// The stage the rows happened in; empty for the whole ticket.
    pub stage: String,
    /// Newest first.
    pub rows: Vec<TimelineRow>,
}

/// A ticket's timeline as the page draws it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timeline {
    /// Newest first, a new group each time the stage changes.
    pub groups: Vec<TimelineGroup>,
    /// Attempts no row carries, newest first, as stage and number: drawn
    /// after the groups as earlier attempts.
    pub earlier: Vec<(String, u32)>,
}

/// The space and project the console lives in.
pub(crate) const CONSOLE_SPACE: &str = "Dispatch";
/// The console session's name.
pub(crate) const CONSOLE_NAME: &str = "console";
/// The runner service's name, beside the console.
pub(crate) const RUNNER_NAME: &str = "runner";

/// How often the app asks Dispatch for its status.
pub const DISPATCH_POLL: Duration = Duration::from_secs(2);

/// How long after a Stop the old runner is taken to have let go of
/// `runner.lock`: one status poll plus a margin, so a no-runner answer
/// this late is to a request sent after the kill.
pub(crate) const RUNNER_LET_GO: Duration = DISPATCH_POLL.saturating_add(Duration::from_secs(1));

/// A Stop of the runner the app started, tracked until the port has
/// been silent for [`RUNNER_LET_GO`], since the killed runner can hold
/// `runner.lock` for a moment and a runner launched then would exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RunnerStop {
    /// No Stop under way.
    #[default]
    None,
    /// Killed at `since` (the clock's monotonic time).
    Stopping { since: Duration },
    /// Killed at `since`, and a Start waits for it to let go.
    StartWhenStopped { since: Duration },
}

impl RunnerStop {
    /// When the Stop was clicked, while one is under way.
    fn since(self) -> Option<Duration> {
        match self {
            Self::None => None,
            Self::Stopping { since } | Self::StartWhenStopped { since } => Some(since),
        }
    }
}

/// Where the runner stands, as the overview's row reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerStanding {
    /// The app's runner pane runs and the port answers; `pid` is `None`
    /// for a pane just spawned, which has none until the next host poll.
    Up { pid: Option<u32> },
    /// The app's runner pane runs but the port has not answered yet.
    Starting,
    /// A Start waits for the stopped runner to let go.
    StartQueued,
    /// A Stop is letting go.
    Stopping,
    /// A runner the app did not start answers the port.
    Outside,
    /// No runner answers; the page keeps the last status it had.
    Gone,
    /// No runner has answered since the app started.
    Stopped,
}

impl RunnerStanding {
    /// Whether a runner answers the port or the app's pane runs.
    #[must_use]
    pub fn up(self) -> bool {
        matches!(self, Self::Up { .. } | Self::Starting | Self::Outside)
    }

    /// Whether the toggle offers Stop rather than Start.
    #[must_use]
    pub fn stoppable(self) -> bool {
        matches!(self, Self::Up { .. } | Self::Starting | Self::StartQueued)
    }
}

/// The runner's launch: `env` sets the log level and both data
/// directories, then execs `dispatch run`, so the pane's pid is the
/// runner's. Both directories are explicit because the tmux server
/// keeps the environment it started with, not this app's.
#[must_use]
pub fn runner_launch(state: &DispatchState) -> Launch {
    Launch::Argv(vec![
        "/usr/bin/env".into(),
        "RUST_LOG=info".into(),
        format!("DISPATCH_DATA_DIR={}", state.data_dir.display()),
        format!(
            "SWITCHBOARD_DATA_DIR={}",
            state.switchboard_data_dir.display()
        ),
        state.command.display().to_string(),
        "run".into(),
    ])
}

/// Why Start is refused when no `dispatch` sits beside the app.
pub(crate) const RUNNER_NO_COMMAND: &str =
    "no `dispatch` beside this app: build it with `cargo build -p dispatch`, or run the bundle";
/// Why Start is refused while a runner the app did not start answers.
pub(crate) const RUNNER_OUTSIDE: &str = "a runner outside the app is already up; stop it first";
/// Why a Stop over the control port is refused: the app cannot stop a
/// runner it did not start.
pub(crate) const RUNNER_STOP_OUTSIDE: &str =
    "a runner outside the app is up; stop it where it runs";
/// Why a Start queued behind a Stop is dropped while a runner still
/// answers: the port cannot tell the killed runner, slow to exit, from
/// one the app did not start.
pub(crate) const RUNNER_STILL_ANSWERS: &str = "a runner still answers after Stop: the old one slow to exit, or one outside the app; Start again once it is gone";

#[derive(Debug, Default)]
pub struct DispatchState {
    /// The last status answered; kept when the runner goes away, so the
    /// page still reads, marked stale.
    pub status: Status,
    /// A runner answered the last poll.
    pub connected: bool,
    /// A status has arrived at least once.
    pub seen: bool,
    /// Artifact text by path, as read through the port.
    pub artifacts: HashMap<PathBuf, String>,
    /// Events by ticket id, as read through the port.
    pub events: HashMap<String, TicketEvents>,
    /// Single-ticket replies by ticket id.
    pub details: HashMap<String, Fetched>,
    /// Artifact reads with no text yet, by path.
    pub artifact_reads: HashMap<PathBuf, ArtifactRead>,
    /// The `dispatch` executable the console types.
    pub command: PathBuf,
    /// Dispatch's data directory: the console's and the runner's
    /// working directory.
    pub data_dir: PathBuf,
    /// This app's data directory, which the runner finds the app's
    /// control socket through.
    pub switchboard_data_dir: PathBuf,
    /// A Stop of the runner service still letting go. Never saved.
    pub(crate) runner_stop: RunnerStop,
}

/// `#104` for an issue, `PR #3` for a pull request, nothing for a
/// ticket without a number.
#[must_use]
pub fn ticket_source(t: &TicketView) -> String {
    match t.number {
        Some(n) if t.kind == "pull-request" => format!("PR #{n}"),
        Some(n) => format!("#{n}"),
        None => String::new(),
    }
}

/// The current stage and where it falls: `review-code 6/8`, or `done`
/// past the end.
#[must_use]
pub fn ticket_stage(t: &TicketView) -> String {
    match t.stages.get(t.stage) {
        Some(name) => format!("{name} {}/{}", t.stage + 1, t.stages.len()),
        None => "done".to_owned(),
    }
}

/// Whether the ticket's last attempt is starting or running: the
/// standing names it, and the Standing sort puts it after the answers.
fn last_attempt_open(t: &TicketView) -> bool {
    t.attempts
        .last()
        .is_some_and(|a| matches!(a.state.as_str(), "starting" | "running"))
}

/// Whether the page offers a close (or a retry of a kept tree's
/// removal) for this ticket. The runner's flags never both hold: a
/// retry is only for a closed ticket, a close only for one that is not.
#[must_use]
pub fn close_offered(t: &TicketView) -> bool {
    t.closable || t.trees_retryable
}

/// How long an attempt ran, once it ended.
fn attempt_duration(t: &TicketView, stage: &str, n: u32) -> Option<u64> {
    let a = t.attempts.iter().find(|a| a.stage == stage && a.n == n)?;
    Some(a.ended_ms?.saturating_sub(a.started_ms))
}

/// The timeline's rows from the record, for a runner that serves no
/// events: attempts starting and ending, decisions made and answered,
/// restarts. Oldest first, as the log would have them.
fn record_rows(t: &TicketView) -> Vec<(String, TimelineRow)> {
    let mut rows = Vec::new();
    let row = |at_ms: u64, kind: &str, text: String| TimelineRow {
        at_ms,
        kind: kind.to_owned(),
        text,
        attempt: None,
        decision: None,
        duration_ms: None,
        details: None,
    };
    for a in &t.attempts {
        let attempt = Some((a.stage.clone(), a.n));
        rows.push((
            a.stage.clone(),
            TimelineRow {
                attempt: attempt.clone(),
                ..row(
                    a.started_ms,
                    "attempt-started",
                    format!("{}/{} ({}) started", a.stage, a.n, a.context),
                )
            },
        ));
        if let Some(ended) = a.ended_ms {
            let reason = a
                .reason
                .as_ref()
                .map_or(String::new(), |r| format!(": {r}"));
            rows.push((
                a.stage.clone(),
                TimelineRow {
                    attempt,
                    duration_ms: Some(ended.saturating_sub(a.started_ms)),
                    ..row(
                        ended,
                        "attempt-ended",
                        format!("{}/{} {}{reason}", a.stage, a.n, a.state),
                    )
                },
            ));
        }
    }
    for d in &t.decisions {
        let decision = Some(d.id.clone());
        rows.push((
            d.stage.clone(),
            TimelineRow {
                decision: decision.clone(),
                ..row(d.made_ms, "decision", d.question.clone())
            },
        ));
        if let (Some(at), Some(answer)) = (d.answered_ms, &d.answer) {
            let by = d.answered_by.as_deref().unwrap_or("someone");
            rows.push((
                d.stage.clone(),
                TimelineRow {
                    decision,
                    ..row(at, "answered", format!("{} by {by}: {answer}", d.name))
                },
            ));
        }
    }
    for r in &t.restarts {
        rows.push((
            r.to.clone(),
            row(
                r.at_ms,
                "restarted",
                format!("restarted at {} from {}", r.to, r.from),
            ),
        ));
    }
    rows.sort_by_key(|(_, r)| r.at_ms);
    rows
}

/// Rows newest first, a new group each time the stage changes.
fn grouped(rows: impl Iterator<Item = (String, TimelineRow)>) -> Vec<TimelineGroup> {
    let mut groups: Vec<TimelineGroup> = Vec::new();
    for (stage, row) in rows {
        match groups.last_mut() {
            Some(g) if g.stage == stage => g.rows.push(row),
            _ => groups.push(TimelineGroup {
                stage,
                rows: vec![row],
            }),
        }
    }
    groups
}

/// What makes a timeline row the same happening in the log and in the
/// record: a decision's rows by decision, an attempt's by attempt.
type RowKey<'a> = (&'a str, Option<&'a (String, u32)>, Option<&'a String>);

fn row_key<'a>(
    kind: &'a str,
    attempt: Option<&'a (String, u32)>,
    decision: Option<&'a String>,
) -> RowKey<'a> {
    match decision {
        Some(_) => (kind, None, decision),
        None => (kind, attempt, None),
    }
}

/// Marks the row each attempt's card goes under, in groups newest
/// first: its newest end row, else (it runs) its newest start row.
/// Returns the attempts no row carries, newest first.
fn place_details(t: &TicketView, groups: &mut [TimelineGroup]) -> Vec<(String, u32)> {
    let ended: HashSet<(String, u32)> = groups
        .iter()
        .flat_map(|g| &g.rows)
        .filter(|r| r.kind == "attempt-ended")
        .filter_map(|r| r.attempt.clone())
        .collect();
    let mut placed: HashSet<(String, u32)> = HashSet::new();
    for row in groups.iter_mut().flat_map(|g| &mut g.rows) {
        let Some(key) = &row.attempt else {
            continue;
        };
        let carries = match row.kind.as_str() {
            "attempt-ended" => true,
            "attempt-started" => !ended.contains(key),
            _ => false,
        };
        if carries
            && t.attempts
                .iter()
                .any(|a| (&a.stage, a.n) == (&key.0, key.1))
            && placed.insert(key.clone())
        {
            row.details = Some(key.clone());
        }
    }
    t.attempts
        .iter()
        .rev()
        .map(|a| (a.stage.clone(), a.n))
        .filter(|key| !placed.contains(key))
        .collect()
}

/// Whether the ticket is parked, which is when Resume is offered.
#[must_use]
pub fn parked(t: &TicketView) -> bool {
    t.state == "parked"
}

/// The pending `finalize` of a plan review that offers `revise`, which
/// is when the ticket page takes the owner's objection above the plan.
/// The stage's newest workflow attempt must review the `plan`: a
/// workflow stage over another subject is asked the same `finalize`,
/// and its ticket has no plan to pin the box over.
#[must_use]
pub fn revisable(t: &TicketView) -> Option<&DecisionView> {
    t.decisions.iter().find(|d| {
        d.state == "pending"
            && d.name == "finalize"
            && d.options.iter().any(|o| o == "revise")
            && t.attempts
                .iter()
                .filter(|a| a.kind == "workflow" && a.stage == d.stage)
                .max_by_key(|a| a.n)
                .is_some_and(|a| a.artifacts.iter().any(|(name, _)| name == "plan"))
    })
}

impl AppCore {
    /// Send `body` to the runner, or say it is not running.
    fn dispatch_call(&mut self, out: &mut Out, body: Body) {
        if !self.dispatch.connected {
            self.error("Dispatch is not running; start it from the console");
            return;
        }
        out.push(Effect::DispatchCall(body));
    }

    #[must_use]
    pub fn dispatch_state(&self) -> &DispatchState {
        &self.dispatch
    }

    #[must_use]
    pub fn ticket(&self, id: &str) -> Option<&TicketView> {
        self.dispatch.status.tickets.iter().find(|t| t.id == id)
    }

    /// Whether the ticket page should ask for ticket `id`'s events,
    /// given its `updated_ms` in the last status: connected, not already
    /// asking, the runner serves them, and not asked at this
    /// `updated_ms`. The page dispatches a read only when this says so.
    #[must_use]
    pub fn events_read_due(&self, id: &str, updated_ms: u64) -> bool {
        self.dispatch.connected
            && self
                .dispatch
                .events
                .get(id)
                .is_none_or(|e| !e.in_flight && !e.unavailable && e.asked_at != Some(updated_ms))
    }

    /// Whether the ticket page should ask for ticket `id` in full, as
    /// `events_read_due` decides for its events.
    #[must_use]
    pub fn ticket_read_due(&self, id: &str, updated_ms: u64) -> bool {
        self.dispatch.connected
            && self
                .dispatch
                .details
                .get(id)
                .is_none_or(|f| !f.in_flight && f.asked_at != Some(updated_ms))
    }

    /// Whether the ticket page should ask for the artifact at `path` of
    /// ticket `t`: connected, no text yet, not already asking, and not
    /// asked at the ticket's current `updated_ms`. Never for a secret
    /// artifact, which the runner refuses to read.
    #[must_use]
    pub fn artifact_read_due(&self, t: &TicketView, path: &std::path::Path) -> bool {
        !t.attempts.iter().any(|a| a.secret_at(path))
            && self.dispatch.connected
            && !self.dispatch.artifacts.contains_key(path)
            && self
                .dispatch
                .artifact_reads
                .get(path)
                .is_none_or(|r| !r.in_flight && r.asked_at != Some(t.updated_ms))
    }

    /// The reads of the artifact at `path` that brought no text yet.
    #[must_use]
    pub fn artifact_read(&self, path: &std::path::Path) -> Option<&ArtifactRead> {
        self.dispatch.artifact_reads.get(path)
    }

    /// The last single-ticket reply for `id`, if one came.
    #[must_use]
    pub fn ticket_details(&self, id: &str) -> Option<&TicketView> {
        self.dispatch.details.get(id)?.view.as_ref()
    }

    /// Ticket `t`'s timeline, newest first in groups by stage: its
    /// events as read through the port, else (a runner that serves
    /// none, or none read yet) rows made from the record. The log
    /// begins when the runner that writes it was installed, so the
    /// record's rows from before its first event are kept, less those
    /// the log has too. Each attempt's card goes under one row; those
    /// no row carries are the timeline's earlier attempts.
    #[must_use]
    pub fn ticket_timeline(&self, t: &TicketView) -> Timeline {
        let events = self
            .dispatch
            .events
            .get(&t.id)
            .map_or(&[][..], |e| e.events.as_slice());
        let rows = match events.first() {
            None => record_rows(t),
            Some(first) => {
                let logged: HashSet<RowKey<'_>> = events
                    .iter()
                    .map(|e| row_key(&e.kind, e.attempt.as_ref(), e.decision.as_ref()))
                    .collect();
                let mut rows: Vec<(String, TimelineRow)> = record_rows(t)
                    .into_iter()
                    .filter(|(_, r)| {
                        r.at_ms < first.at_ms
                            && !logged.contains(&row_key(
                                &r.kind,
                                r.attempt.as_ref(),
                                r.decision.as_ref(),
                            ))
                    })
                    .collect();
                rows.extend(events.iter().map(|e| {
                    let duration_ms = e
                        .attempt
                        .as_ref()
                        .filter(|_| e.kind == "attempt-ended")
                        .and_then(|(stage, n)| attempt_duration(t, stage, *n));
                    (
                        e.stage.clone(),
                        TimelineRow {
                            at_ms: e.at_ms,
                            kind: e.kind.clone(),
                            text: e.text.clone(),
                            attempt: e.attempt.clone(),
                            decision: e.decision.clone(),
                            duration_ms,
                            details: None,
                        },
                    )
                }));
                rows
            }
        };
        let mut groups = grouped(rows.into_iter().rev());
        let earlier = place_details(t, &mut groups);
        Timeline { groups, earlier }
    }

    /// Every pending decision across every ticket, oldest question
    /// first.
    #[must_use]
    pub fn pending_decisions(&self) -> Vec<&DecisionView> {
        // Oldest question first, and the same order every poll: the
        // runner lists tickets by their last write, which moves every
        // time a watch or a check saves, and a list the user is about to
        // click in must not follow that.
        let mut pending: Vec<&DecisionView> = self
            .dispatch
            .status
            .tickets
            .iter()
            .flat_map(|t| t.decisions.iter().filter(|d| d.state == "pending"))
            .collect();
        pending.sort_by(|a, b| (a.made_ms, &a.ticket, &a.id).cmp(&(b.made_ms, &b.ticket, &b.id)));
        pending
    }

    /// The agents of `t` that wait on the user for themselves, as the
    /// window sees their panes: a prompt of Claude's own, a permission,
    /// a question. Dispatch's decisions are listed separately, and a
    /// session waiting only under Dispatch's mark is not an agent
    /// waiting.
    #[must_use]
    pub fn waiting_agents_of(&self, t: &TicketView) -> Vec<WaitingAgent> {
        t.attempts
            .iter()
            .filter(|a| matches!(a.state.as_str(), "starting" | "running"))
            .filter_map(|a| {
                let session = RecordId(uuid::Uuid::parse_str(a.session.as_deref()?).ok()?);
                if !self.counts_as_waiting(session) {
                    return None;
                }
                let reason = self
                    .waiting_reason(session)
                    .unwrap_or_else(|| "waiting on you".to_owned());
                Some(WaitingAgent {
                    session,
                    ticket: t.id.clone(),
                    stage: a.stage.clone(),
                    context: a.context.clone(),
                    reason,
                })
            })
            .collect()
    }

    /// A project's supervisor as the board's chip shows it: its session
    /// in this window, what it is doing, and whether Resume may be
    /// offered. `None` when the project has no `[supervisor]` table.
    #[must_use]
    pub fn supervisor_chip(&self, project: &ProjectView) -> Option<SupervisorChip> {
        let view = project.supervisor.as_ref()?;
        let session = view
            .session
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(RecordId)
            .filter(|id| self.session(*id).is_some());
        let Some(id) = session else {
            return Some(SupervisorChip {
                session: None,
                state: SupervisorState::None,
                resumable: false,
                reason: None,
            });
        };
        let state = if self.at_trust_prompt(id) {
            SupervisorState::AsksTrust
        } else {
            match self.card_state(id) {
                CardState::WaitingOnYou => SupervisorState::WaitingOnYou,
                CardState::Working | CardState::Starting => SupervisorState::Working,
                CardState::Idle => SupervisorState::Idle,
                CardState::NotRunning | CardState::NotResumable | CardState::Exited(_) => {
                    SupervisorState::Cold
                }
            }
        };
        // Offered only where the port's resume would not refuse it, since
        // a fresh launch would skip the seed, the hand-off and the record
        // of what it replaced, at a cost.
        let resumable =
            !self.is_running(id) && self.session(id).is_some_and(SessionRecord::resumable);
        let reason = (state == SupervisorState::WaitingOnYou)
            .then(|| self.waiting_reason(id))
            .flatten();
        Some(SupervisorChip {
            session: Some(id),
            state,
            resumable,
            reason,
        })
    }

    /// `investigate running`, `parked: <reason>`, `2 waiting on you`, an
    /// agent at a prompt of its own, or why a ticket with nothing open
    /// is not moving: a resource another ticket holds, or its project at
    /// a limit.
    #[must_use]
    pub fn ticket_standing(&self, t: &TicketView) -> String {
        let waiting = t.decisions.iter().filter(|d| d.state == "pending").count();
        if waiting > 0 {
            return format!("{waiting} waiting on you");
        }
        if let Some(a) = self.waiting_agents_of(t).first() {
            return format!(
                "agent waiting on you: {} ({}) {}",
                a.stage, a.context, a.reason
            );
        }
        match t.state.as_str() {
            "active" => {
                // A resource another ticket holds stops this one before
                // any limit of its project does.
                if let Some(what) = t.waiting_for.as_ref().filter(|_| !last_attempt_open(t)) {
                    return format!("waiting for {what}");
                }
                let held = if last_attempt_open(t) {
                    None
                } else {
                    self.dispatch
                        .status
                        .projects
                        .iter()
                        .find(|p| p.name == t.project)
                        .and_then(crate::ports::dispatch::ProjectView::held)
                };
                match (held, t.attempts.last()) {
                    (Some(why), _) => format!("held: {why}"),
                    (None, Some(a)) => format!("{} {}", a.stage, a.state),
                    (None, None) => "queued".to_owned(),
                }
            }
            other => match &t.reason {
                Some(reason) => format!("{other}: {reason}"),
                None => other.to_owned(),
            },
        }
    }

    /// Whether the ticket waits on the user: a pending decision or an
    /// agent at a prompt of its own.
    #[must_use]
    pub fn ticket_waits(&self, t: &TicketView) -> bool {
        t.decisions.iter().any(|d| d.state == "pending") || !self.waiting_agents_of(t).is_empty()
    }

    /// Whether the ticket's standing is drawn as needing a look: it
    /// waits on the user, or it is not simply running. The table and
    /// the ticket's header colour it by this alone.
    #[must_use]
    pub fn ticket_urgent(&self, t: &TicketView) -> bool {
        self.ticket_waits(t) || t.state != "active"
    }

    /// The ticket whose page is on screen: the Dispatch window's own
    /// page while that window is open (`window_ticket`, which the
    /// window's navigation keeps), the main view's otherwise.
    #[must_use]
    pub fn ticket_on_screen<'a>(&'a self, window_ticket: Option<&'a str>) -> Option<&'a str> {
        if self.settings.dispatch_window.is_some() {
            return window_ticket;
        }
        match self.view_stack.last() {
            Some(View::Ticket(id)) => Some(id),
            _ => None,
        }
    }

    /// Whether a close confirmation for ticket `id` may stay open: its
    /// page is on screen and the runner still offers the close. Nothing
    /// would draw it otherwise, yet it would hold the keyboard and come
    /// back by itself the next time the ticket is opened; and once a
    /// close is under way, the runner no longer offers it.
    #[must_use]
    pub fn close_dialog_stands(&self, id: &str, window_ticket: Option<&str>) -> bool {
        self.ticket_on_screen(window_ticket) == Some(id)
            && self.ticket(id).is_some_and(close_offered)
    }

    /// The tickets the table shows, narrowed and ordered as `listing`
    /// says. The order is total (ids break ties), so it holds still
    /// from one poll to the next.
    #[must_use]
    pub fn tickets_listed(&self, listing: &TicketListing) -> Vec<&TicketView> {
        let words: Vec<String> = listing
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut rows: Vec<(&TicketView, String)> = self
            .dispatch
            .status
            .tickets
            .iter()
            .filter(|t| listing.project.as_deref().is_none_or(|p| p == t.project))
            .filter(|t| match listing.only {
                TicketOnly::All => true,
                TicketOnly::Waiting => self.ticket_waits(t),
                TicketOnly::Active => t.state == "active",
                TicketOnly::Parked => {
                    matches!(t.state.as_str(), "parked" | "parking" | "closing")
                }
                TicketOnly::Closed => t.state == "closed",
            })
            .map(|t| (t, self.ticket_standing(t)))
            .filter(|(t, standing)| {
                if words.is_empty() {
                    return true;
                }
                let haystack = format!(
                    "{} {} {} {} {} {}",
                    ticket_source(t),
                    t.title,
                    t.project,
                    ticket_stage(t),
                    standing,
                    t.labels.join(" ")
                )
                .to_lowercase();
                words.iter().all(|w| haystack.contains(w.as_str()))
            })
            .collect();
        let rank = |t: &TicketView, standing: &str| -> (u8, String) {
            let r = if self.ticket_waits(t) {
                0
            } else if t.state == "active" && last_attempt_open(t) {
                1
            } else if t.state == "active" {
                2
            } else if t.state == "closed" {
                4
            } else {
                3
            };
            (r, standing.to_owned())
        };
        rows.sort_by(|(a, sa), (b, sb)| {
            let order = match listing.sort {
                TicketSort::Updated => a.updated_ms.cmp(&b.updated_ms),
                TicketSort::Created => a.created_ms.cmp(&b.created_ms),
                TicketSort::Project => a.project.cmp(&b.project),
                TicketSort::Source => (&a.kind, a.number).cmp(&(&b.kind, b.number)),
                TicketSort::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                TicketSort::Stage => (a.stage, &a.project).cmp(&(b.stage, &b.project)),
                TicketSort::Standing => rank(a, sa).cmp(&rank(b, sb)),
            };
            let order = if listing.ascending {
                order
            } else {
                order.reverse()
            };
            order.then_with(|| a.id.cmp(&b.id))
        });
        rows.into_iter().map(|(t, _)| t).collect()
    }

    /// Every agent of every ticket that waits on the user for itself.
    #[must_use]
    pub fn waiting_agents(&self) -> Vec<WaitingAgent> {
        self.dispatch
            .status
            .tickets
            .iter()
            .flat_map(|t| self.waiting_agents_of(t))
            .collect()
    }

    /// `pending_decisions`, narrowed to the tickets of one project
    /// (`None` keeps every project).
    #[must_use]
    pub fn pending_decisions_in(&self, project: Option<&str>) -> Vec<&DecisionView> {
        let mut pending = self.pending_decisions();
        pending.retain(|d| self.ticket_in(&d.ticket, project));
        pending
    }

    /// `waiting_agents`, narrowed to the tickets of one project (`None`
    /// keeps every project).
    #[must_use]
    pub fn waiting_agents_in(&self, project: Option<&str>) -> Vec<WaitingAgent> {
        let mut agents = self.waiting_agents();
        agents.retain(|a| self.ticket_in(&a.ticket, project));
        agents
    }

    fn ticket_in(&self, ticket: &str, project: Option<&str>) -> bool {
        self.ticket(ticket)
            .is_some_and(|t| project.is_none_or(|p| p == t.project))
    }

    /// The ticket one of whose attempts ran in `session`.
    #[must_use]
    pub fn ticket_of_session(&self, session: RecordId) -> Option<&TicketView> {
        let id = session.0.to_string();
        self.dispatch
            .status
            .tickets
            .iter()
            .find(|t| t.attempts.iter().any(|a| a.session.as_deref() == Some(&id)))
    }

    /// The console session, if it still exists.
    #[must_use]
    pub fn console(&self) -> Option<RecordId> {
        self.settings
            .dispatch_console
            .filter(|id| self.session(*id).is_some())
    }

    /// The runner service, if it still exists.
    #[must_use]
    pub fn runner(&self) -> Option<RecordId> {
        self.settings
            .dispatch_runner
            .filter(|id| self.session(*id).is_some())
    }

    /// Where the runner stands: the app's pane first, then a Stop
    /// under way, then what the port says.
    #[must_use]
    pub fn runner_standing(&self) -> RunnerStanding {
        let pid = self
            .runner()
            .and_then(|id| self.host_status(id))
            .and_then(|h| match h.liveness {
                Liveness::Running { pid, .. } => Some(pid),
                Liveness::Exited { .. } | Liveness::Missing => None,
            });
        let state = &self.dispatch;
        match (pid, state.runner_stop) {
            (Some(pid), _) if state.connected => RunnerStanding::Up {
                pid: Some(pid).filter(|pid| *pid != 0),
            },
            (Some(_), _) => RunnerStanding::Starting,
            (None, RunnerStop::StartWhenStopped { .. }) => RunnerStanding::StartQueued,
            (None, RunnerStop::Stopping { .. }) => RunnerStanding::Stopping,
            (None, RunnerStop::None) if state.connected => RunnerStanding::Outside,
            (None, RunnerStop::None) if state.seen => RunnerStanding::Gone,
            (None, RunnerStop::None) => RunnerStanding::Stopped,
        }
    }

    /// Why a Start would be refused now, if it would.
    #[must_use]
    pub fn runner_refusal(&self) -> Option<&'static str> {
        if !self.dispatch.command.is_absolute() {
            Some(RUNNER_NO_COMMAND)
        } else if self.runner_standing() == RunnerStanding::Outside {
            Some(RUNNER_OUTSIDE)
        } else {
            None
        }
    }

    pub(super) fn dispatch_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::DispatchConfigured {
                command,
                data_dir,
                switchboard_data_dir,
            } => {
                self.dispatch.command = command;
                self.dispatch.data_dir = data_dir;
                self.dispatch.switchboard_data_dir = switchboard_data_dir;
                self.refresh_runner(out);
            }
            AppAction::DispatchRunnerStart => self.runner_start(now, out),
            AppAction::DispatchRunnerStop => self.runner_stop(now, out),
            AppAction::DispatchStatus(status) => self.dispatch_status(status, now, out),
            AppAction::ShowDispatch => self.show(View::Dispatch, now, out),
            AppAction::ShowTicket(id) => {
                if self.ticket(&id).is_some() {
                    self.show(View::Ticket(id), now, out);
                } else {
                    self.error(format!("no ticket {id} in the last status"));
                }
            }
            AppAction::DispatchDecide {
                ticket,
                decision,
                answer,
                note,
            } => {
                self.dispatch_call(
                    out,
                    Body::Decide {
                        ticket,
                        decision,
                        answer,
                        note,
                    },
                );
            }
            AppAction::DispatchResume(ticket) => {
                self.dispatch_call(out, Body::Resume { ticket });
            }
            AppAction::DispatchSupervisorFresh(project) => {
                self.dispatch_call(out, Body::SupervisorFresh { project });
            }
            AppAction::DispatchClose(ticket) => {
                self.dispatch_call(
                    out,
                    Body::Close {
                        ticket,
                        reason: None,
                    },
                );
            }
            AppAction::DispatchWorktrees { path, migrate } => {
                self.dispatch_call(out, Body::Worktrees { path, migrate });
            }
            AppAction::DispatchReadArtifact { ticket, path } => {
                self.read_artifact(ticket, path, out);
            }
            AppAction::DispatchReadEvents { ticket, updated_ms } => {
                self.read_events(ticket, updated_ms, out);
            }
            AppAction::DispatchReadTicket { id, updated_ms } => {
                self.read_ticket(id, updated_ms, out);
            }
            AppAction::DispatchReplied { body, result } => {
                self.dispatch_replied(body, result, now);
            }
            AppAction::OpenDispatchConsole => {
                self.ensure_console(now, out);
            }
            AppAction::DispatchConsole(line) => {
                let line = line.trim().to_owned();
                if line.is_empty() {
                    return;
                }
                let Some(id) = self.ensure_console(now, out) else {
                    return;
                };
                let command = self.dispatch.command.display().to_string();
                let text = match line.strip_prefix('!') {
                    Some(shell) => shell.trim().to_owned(),
                    None => format!("'{}' {line}", command.replace('\'', "'\\''")),
                };
                self.session_action(AppAction::SendInput { id, text }, now, out);
            }
            AppAction::PopOutDispatch
            | AppAction::CloseDispatchWindow
            | AppAction::DispatchWindowMoved(_) => self.dispatch_window_action(action, out),
            _ => {}
        }
    }

    /// A status poll's answer; `None` is no runner.
    fn dispatch_status(&mut self, status: Option<Status>, now: Clock, out: &mut Out) {
        self.runner_let_go(status.is_some(), now, out);
        if status.is_some() && !self.dispatch.connected {
            // A runner back, perhaps a newer build: ask again
            // for what the last one would not serve.
            for e in self.dispatch.events.values_mut() {
                if e.unavailable {
                    e.unavailable = false;
                    e.asked_at = None;
                }
            }
        }
        self.dispatch.connected = status.is_some();
        if let Some(status) = status {
            self.dispatch.status = status;
            self.dispatch.seen = true;
        }
    }

    /// A read of an artifact's text, asked unless it is already read or
    /// on its way; a page's click asks again after a failure.
    fn read_artifact(&mut self, ticket: String, path: PathBuf, out: &mut Out) {
        if self.dispatch.artifacts.contains_key(&path) {
            return;
        }
        let updated_ms = self.ticket(&ticket).map(|t| t.updated_ms);
        let r = self
            .dispatch
            .artifact_reads
            .entry(path.clone())
            .or_default();
        if r.in_flight {
            return;
        }
        r.asked_at = updated_ms;
        r.in_flight = true;
        out.push(Effect::DispatchCall(Body::Artifact { ticket, path }));
    }

    /// A ticket page's read of a ticket's events, sent only when
    /// `events_read_due` says it is due, and marked in flight until its
    /// reply.
    fn read_events(&mut self, ticket: String, updated_ms: u64, out: &mut Out) {
        if !self.events_read_due(&ticket, updated_ms) {
            return;
        }
        let e = self.dispatch.events.entry(ticket.clone()).or_default();
        e.asked_at = Some(updated_ms);
        e.in_flight = true;
        let since = e.last;
        out.push(Effect::DispatchCall(Body::Events { ticket, since }));
    }

    /// A ticket page's read of the ticket in full, as `read_events`.
    fn read_ticket(&mut self, id: String, updated_ms: u64, out: &mut Out) {
        if !self.ticket_read_due(&id, updated_ms) {
            return;
        }
        let f = self.dispatch.details.entry(id.clone()).or_default();
        f.asked_at = Some(updated_ms);
        f.in_flight = true;
        out.push(Effect::DispatchCall(Body::Ticket { id }));
    }

    /// What a call to Dispatch's port came back with: an error marks
    /// the runner gone, a failure is a notice, and an answer lands on
    /// the status it belongs to.
    fn dispatch_replied(&mut self, body: Body, result: Result<Reply, String>, now: Clock) {
        let failed = result.is_err();
        match &body {
            Body::Events { ticket, .. } => {
                if let Some(e) = self.dispatch.events.get_mut(ticket) {
                    e.in_flight = false;
                    if failed {
                        e.asked_at = None;
                    }
                }
            }
            Body::Ticket { id } => {
                if let Some(f) = self.dispatch.details.get_mut(id) {
                    f.in_flight = false;
                    if failed {
                        f.asked_at = None;
                    }
                }
            }
            Body::Artifact { path, .. } => {
                if let Some(r) = self.dispatch.artifact_reads.get_mut(path) {
                    r.in_flight = false;
                    if failed {
                        r.asked_at = None;
                    }
                }
            }
            _ => {}
        }
        match (body, result) {
            (Body::Events { ticket, .. }, Ok(Reply::Events(v))) => {
                let e = self.dispatch.events.entry(ticket).or_default();
                e.events.retain(|x| !v.withdrawn.contains(&x.seq));
                e.events.extend(v.events);
                e.last = v.last;
            }
            // A runner older than the events read answers it as a bad
            // request; the page falls back to the record, quietly.
            (Body::Events { ticket, .. }, Ok(Reply::Failed { .. })) => {
                self.dispatch.events.entry(ticket).or_default().unavailable = true;
            }
            (Body::Ticket { id }, Ok(Reply::Ticket(t))) => {
                self.dispatch.details.entry(id).or_default().view = Some(t);
            }
            // The page says why under the artifact's name; a notes file
            // not written yet is no reason for a notice.
            (Body::Artifact { path, .. }, Ok(Reply::Failed { reason })) => {
                self.dispatch.artifact_reads.entry(path).or_default().failed = Some(reason);
            }
            (_, Err(e)) => {
                self.dispatch.connected = false;
                self.error(format!("Dispatch did not answer: {e}"));
            }
            (_, Ok(Reply::Failed { reason })) => self.error(format!("Dispatch: {reason}")),
            (Body::Artifact { path, .. }, Ok(Reply::Artifact { text })) => {
                self.dispatch.artifact_reads.remove(&path);
                self.dispatch.artifacts.insert(path, text);
            }
            (Body::Worktrees { .. }, Ok(Reply::Worktrees(v))) => {
                self.dispatch.status.worktrees.clone_from(&v.root);
                let moved = if v.moved.is_empty() {
                    String::new()
                } else {
                    format!("; moved {} ticket(s)", v.moved.len())
                };
                let mut left = String::new();
                for (id, why) in &v.skipped {
                    let _ = write!(left, "; left {id}: {why}");
                }
                self.info(
                    format!("Dispatch worktrees: {}{moved}{left}", v.root.display()),
                    now,
                );
            }
            (Body::Resume { .. } | Body::Close { .. }, Ok(Reply::Ticket(t))) => {
                if let Some(slot) = self
                    .dispatch
                    .status
                    .tickets
                    .iter_mut()
                    .find(|x| x.id == t.id)
                {
                    *slot = t;
                }
            }
            (Body::Decide { ticket, .. }, Ok(Reply::Decided(d))) => {
                // The next status carries it too; this keeps the
                // buttons from being pressed twice in between.
                if let Some(t) = self
                    .dispatch
                    .status
                    .tickets
                    .iter_mut()
                    .find(|t| t.id == ticket)
                    && let Some(slot) = t.decisions.iter_mut().find(|x| x.id == d.id)
                {
                    *slot = d;
                }
            }
            _ => {}
        }
    }

    /// The Dispatch page's own window: opened once (the main window
    /// goes back to what was under the page), closed, or moved.
    fn dispatch_window_action(&mut self, action: AppAction, out: &mut Out) {
        let open = self.settings.dispatch_window.is_some();
        match action {
            AppAction::PopOutDispatch => {
                if !open {
                    self.update_settings(out, |s| s.dispatch_window = Some(PageWindow::default()));
                }
                while matches!(self.view(), View::Dispatch | View::Ticket(_)) {
                    self.view_stack.pop();
                }
            }
            AppAction::CloseDispatchWindow if open => {
                self.update_settings(out, |s| s.dispatch_window = None);
            }
            AppAction::DispatchWindowMoved(frame) if open => {
                self.update_settings(out, |s| {
                    if let Some(w) = &mut s.dispatch_window {
                        w.frame = Some(frame);
                    }
                });
            }
            _ => {}
        }
    }

    /// The console session: found, or made in the `Dispatch` project
    /// with Dispatch's data directory as its cwd. The one made is
    /// remembered in the settings.
    fn ensure_console(&mut self, now: Clock, out: &mut Out) -> Option<RecordId> {
        if let Some(id) = self.console() {
            return Some(id);
        }
        let project = self.ensure_dispatch_project(now, out);
        let id = self.add_record(
            project,
            CONSOLE_NAME.into(),
            SessionKind::Shell,
            self.dispatch.data_dir.clone(),
            Launch::Shell,
            now,
            out,
        )?;
        self.update_settings(out, |s| s.dispatch_console = Some(id));
        self.launch_fresh(id, now, out);
        Some(id)
    }

    /// The `Dispatch` space and project the console and the runner live
    /// in: found, or made with Dispatch's data directory as the root.
    fn ensure_dispatch_project(&mut self, now: Clock, out: &mut Out) -> ProjectId {
        let found = self
            .views
            .spaces
            .iter()
            .find(|s| s.name == CONSOLE_SPACE)
            .map(|s| s.id);
        let space = found.unwrap_or_else(SpaceId::new);
        if found.is_none() {
            self.update_views(out, |v| {
                v.spaces.push(Space {
                    id: space,
                    name: CONSOLE_SPACE.into(),
                    op: None,
                });
            });
        }
        let root = self.dispatch.data_dir.clone();
        match self
            .workspaces
            .iter()
            .find(|w| w.project.space == space && w.project.name == CONSOLE_SPACE)
            .map(|w| w.project.id)
        {
            Some(id) => id,
            None => self.add_project_record(CONSOLE_SPACE.into(), root, space, now, out),
        }
    }

    /// The runner service: found, or made cold in the `Dispatch`
    /// project and remembered in the settings.
    fn ensure_runner(&mut self, now: Clock, out: &mut Out) -> Option<RecordId> {
        if let Some(id) = self.runner() {
            return Some(id);
        }
        let project = self.ensure_dispatch_project(now, out);
        let id = self.add_record(
            project,
            RUNNER_NAME.into(),
            SessionKind::Service,
            self.dispatch.data_dir.clone(),
            runner_launch(&self.dispatch),
            now,
            out,
        )?;
        self.update_settings(out, |s| s.dispatch_runner = Some(id));
        Some(id)
    }

    /// The runner record's launch and cwd brought up to the executable
    /// and directories the app knows now, since the executable's path
    /// and either data directory can differ between runs (the bundle
    /// against `cargo run`, a changed `DISPATCH_DATA_DIR`). A bare
    /// `dispatch` is never written: nothing says which one would run,
    /// and the startup reconcile would launch it.
    fn refresh_runner(&mut self, out: &mut Out) {
        let Some(id) = self.runner() else {
            return;
        };
        if !self.dispatch.command.is_absolute() {
            return;
        }
        let launch = runner_launch(&self.dispatch);
        let cwd = self.dispatch.data_dir.clone();
        let stale = self
            .session(id)
            .is_some_and(|s| s.launch != launch || s.cwd != cwd);
        if stale {
            self.edit_session(id, out, |s| {
                s.launch = launch;
                s.cwd = cwd;
            });
        }
    }

    /// Start: the runner made if need be, marked to come back on the
    /// next app start, and launched unless its pane runs, an old one
    /// is still letting go, or a runner the app did not start answers.
    pub(super) fn runner_start(&mut self, now: Clock, out: &mut Out) {
        if let Some(why) = self.runner_refusal() {
            self.error(why);
            return;
        }
        let Some(id) = self.ensure_runner(now, out) else {
            return;
        };
        self.refresh_runner(out);
        self.edit_session(id, out, |s| s.autostart = true);
        if self.is_running(id) {
            return;
        }
        match self.dispatch.runner_stop {
            RunnerStop::Stopping { since } => {
                self.dispatch.runner_stop = RunnerStop::StartWhenStopped { since };
            }
            RunnerStop::StartWhenStopped { .. } => {}
            RunnerStop::None => self.return_to_session(id, now, out),
        }
    }

    /// Stop: no relaunch on the next app start, the open run closed as
    /// killed, and the pane killed and forgotten so the page sees it
    /// gone at once. A pane killed is tracked until it lets go.
    pub(super) fn runner_stop(&mut self, now: Clock, out: &mut Out) {
        let Some(id) = self.runner() else {
            return;
        };
        self.edit_session(id, out, |s| s.autostart = false);
        self.runner_kill(now, out);
    }

    /// The kill half of Stop, with the record's autostart left as it
    /// is: a restart kills this way, so an app that dies before the new
    /// runner starts still brings it back on its next start.
    pub(super) fn runner_kill(&mut self, now: Clock, out: &mut Out) {
        let Some(id) = self.runner() else {
            return;
        };
        if self
            .session(id)
            .and_then(|s| s.last_run())
            .is_some_and(Run::open)
        {
            self.close_run(id, None, now, out);
        }
        if self.host_status(id).is_some() {
            self.dispatch.runner_stop = RunnerStop::Stopping { since: now.mono };
        } else if let RunnerStop::StartWhenStopped { since } = self.dispatch.runner_stop {
            // The Start queued behind an earlier Stop is withdrawn.
            self.dispatch.runner_stop = RunnerStop::Stopping { since };
        }
        self.kill_and_forget(id, out);
    }

    /// A status answer `RUNNER_LET_GO` after a Stop ends it. Silent,
    /// the old runner has let go and a queued Start launches. Answered,
    /// the killed runner is slow to exit or another holds the port, and
    /// the two look alike, so a queued Start is dropped and stops coming
    /// back, with a notice that a second Start is the fix.
    fn runner_let_go(&mut self, answered: bool, now: Clock, out: &mut Out) {
        let stop = self.dispatch.runner_stop;
        let Some(since) = stop.since() else {
            return;
        };
        if now.mono < since + RUNNER_LET_GO {
            return;
        }
        self.dispatch.runner_stop = RunnerStop::None;
        let Some(id) = self.runner() else {
            return;
        };
        if !matches!(stop, RunnerStop::StartWhenStopped { .. }) || self.is_running(id) {
            return;
        }
        if answered {
            self.edit_session(id, out, |s| s.autostart = false);
            self.error(RUNNER_STILL_ANSWERS);
        } else {
            self.return_to_session(id, now, out);
        }
    }
}
