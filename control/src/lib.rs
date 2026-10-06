//! The wire contract of Switchboard's control port: what another process
//! may ask the app to do or tell it, one JSON line per request and one per
//! reply over a Unix socket in the data directory. Dispatch speaks this;
//! so does the app's `adapters::control`. It is deliberately not the
//! app's `AppAction`: adapter results and startup actions never travel.
//!
//! Every request carries an operation id, `op`. A command's reply is
//! terminal: `persisted`, `launched`, or `failed`. The app stores `op` on
//! every record a command makes and in an append-only operations log, so
//! `find` and `op.status` answer for a reply that was lost.

pub mod client;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use client::Client;

/// The socket's file name inside Switchboard's data directory.
pub const SOCKET_FILE: &str = "control.sock";

/// `SessionView::card` for an agent at work. The app's card label reads
/// this, so a client matching on it cannot drift from the app.
pub const CARD_WORKING: &str = "working";
/// `SessionView::card` for an agent alive at its prompt, nothing pending.
pub const CARD_IDLE: &str = "idle";

/// One request line. `body` is flattened beside `op`, so a line reads
/// `{"op":"...","kind":"session.new",...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub op: String,
    #[serde(flatten)]
    pub body: Body,
}

impl Request {
    #[must_use]
    pub fn new(op: impl Into<String>, body: Body) -> Self {
        Self {
            op: op.into(),
            body,
        }
    }

    /// Parse one line as it arrives on the socket.
    pub fn parse(line: &str) -> Result<Self, String> {
        parse_line(line)
    }

    /// One line, newline included, ready for the socket.
    #[must_use]
    pub fn to_line(&self) -> String {
        to_line(self)
    }
}

/// How recovery treats a request whose reply was lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Makes a record: found again with `find {op}`.
    Creation,
    /// Harmless to repeat.
    Idempotent,
    /// A repeat can spend money: never repeated without a human.
    NonReplayable,
    /// Changes nothing.
    Query,
}

/// Every command and query the port accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Body {
    // --- commands: creations
    #[serde(rename = "project.add")]
    ProjectAdd {
        space: String,
        name: String,
        root: PathBuf,
    },
    #[serde(rename = "session.new")]
    SessionNew {
        project: String,
        name: String,
        /// Named `session_kind` on the wire because `kind` is the tag.
        session_kind: SessionKind,
        cwd: PathBuf,
        launch: Launch,
        /// An agent's first prompt, on its command line; never keys into
        /// a starting pane.
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        notes: String,
        /// Variables set on every spawn of the session, resumes
        /// included: paths and names, never secret values. Not written
        /// when empty, so an older app reads the same request.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
    /// A new Claude Code session whose conversation is a copy of
    /// `source`'s whole transcript, in the source's project and cwd,
    /// launched with `prompt` as its first turn. Refused for a source
    /// with no transcript.
    #[serde(rename = "session.clone")]
    SessionClone {
        source: String,
        name: String,
        prompt: String,
        #[serde(default)]
        notes: String,
    },
    #[serde(rename = "space.new")]
    SpaceNew { name: String },
    #[serde(rename = "set.new")]
    SetNew { space: String, name: String },
    #[serde(rename = "workflow.start")]
    WorkflowStart {
        source: String,
        plan: PathBuf,
        definition: String,
        /// Where the reviewer runs; the source session's directory when
        /// absent. Codex writes only inside its directory, so a review
        /// whose files live elsewhere names that place here.
        #[serde(default)]
        reviewer_cwd: Option<PathBuf>,
        /// Extra flags for the reviewer's command line, ahead of its
        /// first prompt: a model, or the allow rule a Claude Code
        /// reviewer needs to write its feedback where the caller put
        /// the review.
        #[serde(default)]
        reviewer_args: Vec<String>,
    },

    // --- commands: idempotent state
    #[serde(rename = "project.remove")]
    ProjectRemove { project: String },
    #[serde(rename = "session.kill")]
    SessionKill { session: String },
    #[serde(rename = "session.remove")]
    SessionRemove { session: String },
    #[serde(rename = "session.notes")]
    SessionNotes { session: String, text: String },
    #[serde(rename = "session.waiting")]
    SessionWaiting {
        session: String,
        on: bool,
        #[serde(default)]
        reason: String,
    },
    /// Answer Claude Code's folder trust question with yes; nothing is
    /// sent unless the pane was last seen showing it.
    #[serde(rename = "session.trust")]
    SessionTrust { session: String },
    /// The session record moves to another project's board; its pane
    /// and its cards stay.
    #[serde(rename = "session.move")]
    SessionMove { session: String, project: String },
    /// The project's directory moved (a worktree relocated); its
    /// sessions' own records keep the cwd they were launched with.
    #[serde(rename = "project.root")]
    ProjectRoot { project: String, root: PathBuf },
    #[serde(rename = "project.rename")]
    ProjectRename { project: String, name: String },
    #[serde(rename = "set.sync")]
    SetSync { set: String, items: Vec<Pin> },
    #[serde(rename = "workflow.definitions.install")]
    DefinitionInstall { definition: Definition },
    #[serde(rename = "workflow.pause")]
    WorkflowPause { run: String },
    #[serde(rename = "workflow.finalize")]
    WorkflowFinalize { run: String },
    #[serde(rename = "workflow.remove")]
    WorkflowRemove { run: String },
    /// Stop, start or restart the Dispatch runner the app runs, as the
    /// overview's buttons do. A restart keeps the runner's autostart on
    /// throughout, so an app that dies between the kill and the start
    /// still brings the runner back.
    #[serde(rename = "dispatch.runner")]
    DispatchRunner { action: RunnerVerb },

    // --- commands: non-replayable
    #[serde(rename = "session.send")]
    SessionSend { session: String, text: String },
    /// Resume an agent's conversation from its resume handle, with no
    /// terminal opened. A running pane is left alone, and a session that
    /// cannot be resumed is refused: it never launches fresh. Never
    /// repeated by recovery, since it starts a paid turn.
    #[serde(rename = "session.resume")]
    SessionResume { session: String },
    #[serde(rename = "workflow.continue")]
    WorkflowContinue { run: String },

    // --- queries
    #[serde(rename = "projects")]
    Projects {
        #[serde(default)]
        space: Option<String>,
    },
    #[serde(rename = "spaces")]
    Spaces,
    #[serde(rename = "sets")]
    Sets { space: String },
    #[serde(rename = "sessions")]
    Sessions { project: String },
    #[serde(rename = "session")]
    Session { session: String },
    #[serde(rename = "waiting")]
    Waiting,
    #[serde(rename = "workflow")]
    Workflow { run: String },
    #[serde(rename = "workflows")]
    Workflows { project: String },
    /// Every record the operation made. The field is `operation`, not
    /// `op`, which is this request's own id.
    #[serde(rename = "find")]
    Find { operation: String },
    #[serde(rename = "op.status")]
    OpStatus { operation: String },
    /// The last `lines` of a running session's pane (40 when absent),
    /// with the project's secret values replaced by their names.
    #[serde(rename = "session.screen")]
    SessionScreen {
        session: String,
        #[serde(default)]
        lines: Option<u32>,
    },
}

impl Body {
    #[must_use]
    pub fn class(&self) -> Class {
        match self {
            Self::ProjectAdd { .. }
            | Self::SessionNew { .. }
            | Self::SessionClone { .. }
            | Self::SpaceNew { .. }
            | Self::SetNew { .. }
            | Self::WorkflowStart { .. } => Class::Creation,
            Self::ProjectRemove { .. }
            | Self::SessionKill { .. }
            | Self::SessionRemove { .. }
            | Self::SessionNotes { .. }
            | Self::SessionWaiting { .. }
            | Self::SessionTrust { .. }
            | Self::SessionMove { .. }
            | Self::ProjectRename { .. }
            | Self::ProjectRoot { .. }
            | Self::SetSync { .. }
            | Self::DefinitionInstall { .. }
            | Self::WorkflowPause { .. }
            | Self::WorkflowFinalize { .. }
            | Self::WorkflowRemove { .. }
            | Self::DispatchRunner {
                action: RunnerVerb::Stop | RunnerVerb::Start,
            } => Class::Idempotent,
            // A repeat would stop a second runner.
            Self::DispatchRunner {
                action: RunnerVerb::Restart,
            }
            | Self::SessionSend { .. }
            | Self::SessionResume { .. }
            | Self::WorkflowContinue { .. } => Class::NonReplayable,
            Self::Projects { .. }
            | Self::Spaces
            | Self::Sets { .. }
            | Self::Sessions { .. }
            | Self::Session { .. }
            | Self::Waiting
            | Self::Workflow { .. }
            | Self::Workflows { .. }
            | Self::Find { .. }
            | Self::OpStatus { .. }
            | Self::SessionScreen { .. } => Class::Query,
        }
    }

    #[must_use]
    pub fn is_command(&self) -> bool {
        self.class() != Class::Query
    }

    /// The `kind` string of the line, for logs.
    #[must_use]
    pub fn kind(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("kind")?.as_str().map(str::to_owned))
            .unwrap_or_default()
    }
}

/// What `dispatch.runner` asks of the Dispatch runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerVerb {
    Stop,
    Start,
    Restart,
}

impl RunnerVerb {
    /// The word on the command line and in the request's `action`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Start => "start",
            Self::Restart => "restart",
        }
    }
}

/// What a session runs as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionKind {
    Claude,
    Codex,
    Shell,
    Command,
    Service,
}

/// A reviewer's agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    Claude,
    Codex,
}

/// What to run in the pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Launch {
    /// The login shell (and for an agent, the agent's own command).
    Shell,
    /// A command; for an agent, extra flags for its composed command,
    /// such as `["--model", "haiku"]`.
    Argv(Vec<String>),
    Command {
        command: String,
        shell: String,
    },
}

/// A complete workflow definition, as the app's Definitions dialog holds
/// one. Templates take `{plan}`, `{feedback}`, `{response}`, `{round}`,
/// `{cap}`, `{no_feedback}`, and `{text}` in `respond_to_user`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub name: String,
    pub reviewer: AgentKind,
    pub review_first: String,
    pub review_round: String,
    pub respond: String,
    pub respond_to_user: String,
    pub handoff: String,
    pub no_feedback: String,
    #[serde(default)]
    pub cap: Option<u32>,
}

/// One card of a working set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub target: PinTarget,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PinTarget {
    Session { session: String },
    File { project: String, path: PathBuf },
}

/// Grid units, not pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// What kind of record an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecordKind {
    Project,
    Session,
    Space,
    Set,
    Run,
}

/// One record a command made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Made {
    pub kind: RecordKind,
    pub id: String,
}

/// Whether the pane exists and what it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Liveness {
    Running,
    Exited { code: Option<i32> },
    Missing,
}

/// One session as the port reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionView {
    pub id: String,
    pub project: String,
    pub name: String,
    pub kind: SessionKind,
    pub cwd: PathBuf,
    pub notes: String,
    pub liveness: Liveness,
    /// The card's word: `CARD_WORKING`, `CARD_IDLE`, "waiting on you", ...
    pub card: String,
    pub last_exit: Option<i32>,
    /// When the agent last reported that it finished a turn (Claude
    /// Code's Stop hook), in milliseconds since the epoch.
    pub last_stop_at_ms: Option<u64>,
    /// How long the pane has printed nothing, when it is running.
    pub quiet_secs: Option<u64>,
    pub waiting: bool,
    pub waiting_reason: Option<String>,
    /// The pane shows Claude Code's own folder trust question, which
    /// comes before any hook and which `session.trust` answers.
    #[serde(default)]
    pub trust_question: bool,
    /// The provider's session id, once the agent has one.
    pub resume_id: Option<String>,
    pub op: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub space: String,
    pub op: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceView {
    pub id: String,
    pub name: String,
    pub op: Option<String>,
    /// The global space: a view of every space rather than a record. It
    /// holds sets (`set.new`, `sets`) but no projects, and cannot be
    /// renamed or removed. Its id is fixed.
    #[serde(default)]
    pub view: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetView {
    pub id: String,
    pub name: String,
    pub space: String,
    pub items: Vec<Pin>,
    pub op: Option<String>,
    /// What chooses the cards, for a set the user did not arrange by
    /// hand; `items` is then the members as they are laid out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<SetRule>,
}

/// What chooses a rule set's cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetRule {
    /// Every session active within the last `hours`.
    Recent { hours: u32 },
}

/// Where a review run stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum RunState {
    Starting,
    AwaitingFeedback,
    AwaitingResponse,
    Converged,
    AtCap,
    Paused {
        reason: String,
        /// The awaited agent let the round down, rather than a user's
        /// Pause. A field rather than a variant so an older reader,
        /// which has no catch-all, still reads the reply.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        failed: bool,
    },
    Finalized,
    HandedOff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunView {
    pub id: String,
    pub project: String,
    pub definition: String,
    #[serde(flatten)]
    pub state: RunState,
    pub round: u32,
    pub cap: u32,
    pub source: String,
    pub plan: PathBuf,
    pub reviewer: String,
    pub planner: Option<String>,
    pub op: Option<String>,
}

/// One record an operation made, as `find` reports it: still present, or
/// logged and since removed in the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub kind: RecordKind,
    pub id: String,
    pub removed: bool,
    #[serde(default)]
    pub session: Option<SessionView>,
    #[serde(default)]
    pub run: Option<RunView>,
}

/// What `op.status` says about an operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum OpStatus {
    /// Never seen: nothing ran.
    Unknown,
    /// Received, and an effect of it is still running.
    InProgress,
    /// The app shut down with an effect of it running.
    Interrupted,
    /// The terminal reply, again.
    Done { reply: Box<Reply> },
}

/// One reply line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "reply")]
pub enum Reply {
    /// The records were made and saved; nothing was launched.
    Persisted {
        made: Vec<Made>,
    },
    /// The records were saved and the launch effect ran.
    Launched {
        made: Vec<Made>,
    },
    Failed {
        reason: String,
    },
    Found {
        records: Vec<Found>,
    },
    OpStatus {
        status: OpStatus,
    },
    Projects {
        projects: Vec<ProjectView>,
    },
    Spaces {
        spaces: Vec<SpaceView>,
    },
    Sets {
        sets: Vec<SetView>,
    },
    Sessions {
        sessions: Vec<SessionView>,
    },
    Session {
        session: SessionView,
    },
    Waiting {
        sessions: Vec<SessionView>,
    },
    Workflow {
        run: RunView,
    },
    Workflows {
        runs: Vec<RunView>,
    },
    /// A pane's text, as `session.screen` asked for it.
    Screen {
        text: String,
    },
}

impl Reply {
    #[must_use]
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed {
            reason: reason.into(),
        }
    }

    /// Parse one line as it arrives on the socket.
    pub fn parse(line: &str) -> Result<Self, String> {
        parse_line(line)
    }

    /// One line, newline included, ready for the socket.
    #[must_use]
    pub fn to_line(&self) -> String {
        to_line(self)
    }

    /// The ids a creation made, if this is a creation's reply.
    #[must_use]
    pub fn made(&self) -> &[Made] {
        match self {
            Self::Persisted { made } | Self::Launched { made } => made,
            _ => &[],
        }
    }
}

/// One socket line as a request or reply.
fn parse_line<T: serde::de::DeserializeOwned>(line: &str) -> Result<T, String> {
    serde_json::from_str(line).map_err(|e| e.to_string())
}

/// A request or reply as one socket line, newline included.
fn to_line<T: Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).unwrap_or_default();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_request(body: Body) {
        let req = Request::new("op-1", body);
        let line = req.to_line();
        assert!(line.ends_with('\n'));
        assert_eq!(Request::parse(line.trim_end()).unwrap(), req);
    }

    fn round_trip_reply(reply: &Reply) {
        let line = reply.to_line();
        assert_eq!(&Reply::parse(line.trim_end()).unwrap(), reply);
    }

    fn session() -> SessionView {
        SessionView {
            id: "s".into(),
            project: "p".into(),
            name: "investigator".into(),
            kind: SessionKind::Claude,
            cwd: "/tmp".into(),
            notes: String::new(),
            liveness: Liveness::Exited { code: Some(0) },
            card: "exited (0)".into(),
            last_exit: Some(0),
            last_stop_at_ms: Some(1),
            quiet_secs: None,
            waiting: false,
            waiting_reason: None,
            trust_question: false,
            resume_id: Some("uuid".into()),
            op: Some("op-1".into()),
        }
    }

    fn run() -> RunView {
        RunView {
            id: "r".into(),
            project: "p".into(),
            definition: "Dispatch: reviewer@abc".into(),
            state: RunState::Paused {
                reason: "by you".into(),
                failed: false,
            },
            round: 2,
            cap: 4,
            source: "s".into(),
            plan: "/plan.md".into(),
            reviewer: "rv".into(),
            planner: None,
            op: None,
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one case per variant
    fn every_request_round_trips_and_names_its_kind() {
        let bodies = vec![
            Body::ProjectAdd {
                space: "sp".into(),
                name: "#1".into(),
                root: "/r".into(),
            },
            Body::SessionNew {
                project: "p".into(),
                name: "n".into(),
                session_kind: SessionKind::Claude,
                cwd: "/r".into(),
                launch: Launch::Shell,
                prompt: Some("go".into()),
                notes: "ticket".into(),
                env: BTreeMap::from([("DISPATCH_INPUT_PLAN".into(), "/plan.md".into())]),
            },
            Body::SessionClone {
                source: "s1".into(),
                name: "rebaser".into(),
                prompt: "rebase".into(),
                notes: "ticket".into(),
            },
            Body::SessionNew {
                project: "p".into(),
                name: "n".into(),
                session_kind: SessionKind::Command,
                cwd: "/r".into(),
                launch: Launch::Command {
                    command: "ls".into(),
                    shell: "/bin/zsh".into(),
                },
                prompt: None,
                notes: String::new(),
                env: BTreeMap::new(),
            },
            Body::SpaceNew { name: "D".into() },
            Body::SetNew {
                space: "sp".into(),
                name: "queue".into(),
            },
            Body::WorkflowStart {
                source: "s".into(),
                plan: "/p.md".into(),
                definition: "d".into(),
                reviewer_cwd: Some("/att".into()),
                reviewer_args: vec!["--model".into(), "haiku".into()],
            },
            Body::ProjectRemove {
                project: "p".into(),
            },
            Body::SessionKill {
                session: "s".into(),
            },
            Body::SessionRemove {
                session: "s".into(),
            },
            Body::SessionNotes {
                session: "s".into(),
                text: "t".into(),
            },
            Body::SessionMove {
                session: "s".into(),
                project: "p".into(),
            },
            Body::ProjectRename {
                project: "p".into(),
                name: "n".into(),
            },
            Body::ProjectRoot {
                project: "p".into(),
                root: "/r2".into(),
            },
            Body::SessionWaiting {
                session: "s".into(),
                on: true,
                reason: "finalize?".into(),
            },
            Body::SessionTrust {
                session: "s".into(),
            },
            Body::SetSync {
                set: "set".into(),
                items: vec![Pin {
                    target: PinTarget::Session {
                        session: "s".into(),
                    },
                    rect: Rect {
                        x: 0,
                        y: 0,
                        w: 10,
                        h: 8,
                    },
                }],
            },
            Body::DefinitionInstall {
                definition: Definition {
                    name: "d".into(),
                    reviewer: AgentKind::Codex,
                    review_first: "a".into(),
                    review_round: "b".into(),
                    respond: "c".into(),
                    respond_to_user: "d".into(),
                    handoff: "e".into(),
                    no_feedback: "f".into(),
                    cap: Some(3),
                },
            },
            Body::WorkflowPause { run: "r".into() },
            Body::WorkflowFinalize { run: "r".into() },
            Body::WorkflowRemove { run: "r".into() },
            Body::SessionSend {
                session: "s".into(),
                text: "hi".into(),
            },
            Body::WorkflowContinue { run: "r".into() },
            Body::Projects { space: None },
            Body::Spaces,
            Body::Sets { space: "sp".into() },
            Body::Sessions {
                project: "p".into(),
            },
            Body::Session {
                session: "s".into(),
            },
            Body::Waiting,
            Body::Workflow { run: "r".into() },
            Body::Workflows {
                project: "p".into(),
            },
            Body::Find {
                operation: "op".into(),
            },
            Body::OpStatus {
                operation: "op".into(),
            },
            Body::SessionScreen {
                session: "s".into(),
                lines: Some(20),
            },
        ];
        for body in bodies {
            assert!(!body.kind().is_empty(), "{body:?}");
            round_trip_request(body);
        }
        assert_eq!(Body::Spaces.kind(), "spaces");
        assert_eq!(
            Body::Find {
                operation: "x".into()
            }
            .kind(),
            "find"
        );
        assert_eq!(Body::Spaces.class(), Class::Query);
        assert_eq!(
            Body::SessionScreen {
                session: "s".into(),
                lines: None
            }
            .class(),
            Class::Query
        );
        assert_eq!(
            Request::parse(r#"{"op":"q","kind":"session.screen","session":"s"}"#)
                .unwrap()
                .body,
            Body::SessionScreen {
                session: "s".into(),
                lines: None
            }
        );
        assert_eq!(
            Body::SessionSend {
                session: "s".into(),
                text: String::new()
            }
            .class(),
            Class::NonReplayable
        );
        // A resume starts a paid turn: recovery never repeats it.
        assert_eq!(
            Body::SessionResume {
                session: "s".into()
            }
            .class(),
            Class::NonReplayable
        );
    }

    #[test]
    fn a_runner_line_round_trips_for_each_verb_and_only_a_restart_is_non_replayable() {
        for (verb, class) in [
            (RunnerVerb::Stop, Class::Idempotent),
            (RunnerVerb::Start, Class::Idempotent),
            (RunnerVerb::Restart, Class::NonReplayable),
        ] {
            let body = Body::DispatchRunner { action: verb };
            assert_eq!(body.kind(), "dispatch.runner");
            assert_eq!(body.class(), class);
            round_trip_request(body);
        }
        let req =
            Request::parse(r#"{"op":"r","kind":"dispatch.runner","action":"restart"}"#).unwrap();
        assert_eq!(
            req.body,
            Body::DispatchRunner {
                action: RunnerVerb::Restart
            }
        );
    }

    #[test]
    fn a_resume_line_reads_as_the_doc_writes_it() {
        let req = Request::parse(r#"{"op":"r","kind":"session.resume","session":"s1"}"#).unwrap();
        assert_eq!(
            req.body,
            Body::SessionResume {
                session: "s1".into()
            }
        );
        assert_eq!(req.body.kind(), "session.resume");
        round_trip_request(req.body);
    }

    #[test]
    fn a_request_line_reads_as_the_doc_writes_it() {
        let req = Request::parse(r#"{"op":"a","kind":"session.kill","session":"s1"}"#).unwrap();
        assert_eq!(req.op, "a");
        assert_eq!(
            req.body,
            Body::SessionKill {
                session: "s1".into()
            }
        );
        assert!(Request::parse("not json").is_err());
        assert!(Request::parse(r#"{"op":"a","kind":"nothing"}"#).is_err());
    }

    #[test]
    fn every_reply_round_trips() {
        let made = vec![Made {
            kind: RecordKind::Session,
            id: "s".into(),
        }];
        let replies = vec![
            Reply::Persisted { made: made.clone() },
            Reply::Launched { made: made.clone() },
            Reply::failed("no"),
            Reply::Found {
                records: vec![Found {
                    kind: RecordKind::Session,
                    id: "s".into(),
                    removed: true,
                    session: None,
                    run: Some(run()),
                }],
            },
            Reply::OpStatus {
                status: OpStatus::Unknown,
            },
            Reply::OpStatus {
                status: OpStatus::Interrupted,
            },
            Reply::OpStatus {
                status: OpStatus::Done {
                    reply: Box::new(Reply::Launched { made }),
                },
            },
            Reply::Projects {
                projects: vec![ProjectView {
                    id: "p".into(),
                    name: "#1".into(),
                    root: "/r".into(),
                    space: "sp".into(),
                    op: None,
                }],
            },
            Reply::Spaces {
                spaces: vec![SpaceView {
                    id: "sp".into(),
                    name: "D".into(),
                    op: Some("o".into()),
                    view: false,
                }],
            },
            Reply::Sets {
                sets: vec![SetView {
                    id: "set".into(),
                    name: "q".into(),
                    space: "sp".into(),
                    items: vec![],
                    op: None,
                    rule: Some(SetRule::Recent { hours: 24 }),
                }],
            },
            Reply::Sessions {
                sessions: vec![session()],
            },
            Reply::Session { session: session() },
            Reply::Waiting {
                sessions: vec![session()],
            },
            Reply::Workflow { run: run() },
            Reply::Workflows { runs: vec![run()] },
            Reply::Screen {
                text: "$ ls\nsrc".into(),
            },
        ];
        for reply in &replies {
            round_trip_reply(reply);
        }
    }

    #[test]
    fn a_paused_run_reads_without_the_failed_flag_and_writes_it_only_when_set() {
        let paused: RunState = serde_json::from_str(r#"{"state":"paused","reason":"x"}"#).unwrap();
        assert_eq!(
            paused,
            RunState::Paused {
                reason: "x".into(),
                failed: false
            }
        );
        assert_eq!(
            serde_json::to_string(&paused).unwrap(),
            r#"{"state":"paused","reason":"x"}"#
        );
        let failed = RunState::Paused {
            reason: "x".into(),
            failed: true,
        };
        let text = serde_json::to_string(&failed).unwrap();
        assert_eq!(text, r#"{"state":"paused","reason":"x","failed":true}"#);
        assert_eq!(serde_json::from_str::<RunState>(&text).unwrap(), failed);
    }
}
