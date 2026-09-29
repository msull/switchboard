//! The records: a ticket with its lanes, attempts, decisions and ledger,
//! and the per-project state (the workspace and set in Switchboard, the
//! queue). Plain data; the scheduler changes it, the store writes it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use switchboard_control::{Body, Reply};

/// Where a ticket came from, as it was when taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSnapshot {
    /// `github`, `task-file` or `manual`.
    pub kind: String,
    /// `<repo>#<number>` for GitHub; a hash of the line for a task file.
    /// A ticket is taken once per identity.
    pub identity: String,
    pub number: Option<u64>,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub labels: Vec<String>,
    pub taken_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneRecord {
    pub name: String,
    pub worktree: PathBuf,
    pub branch: String,
    /// The Switchboard project for this lane, once made.
    pub project: Option<String>,
    /// Whether the issue's work runs here: every lane's tree is cut, the
    /// `lanes` decision says which ones the stages use.
    #[serde(default = "yes")]
    pub chosen: bool,
    /// The lane's setup ran, once, before its first agent.
    #[serde(default)]
    pub setup_done: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptKind {
    Agent,
    Workflow,
    GateOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum AttemptState {
    /// Requests are going out; nothing observed yet.
    Starting,
    Running,
    Complete,
    Failed {
        reason: String,
    },
    /// Stopped by Dispatch (the ticket parked, or a rerun replaced it);
    /// no decision follows.
    Cancelled {
        reason: String,
    },
}

/// How an artifact has looked on the last polls; settled after
/// `SETTLE_POLLS` in a row unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settle {
    pub mtime_ms: u64,
    pub len: u64,
    pub polls: u32,
}

/// Polls an artifact must look the same for before it counts as written.
pub const SETTLE_POLLS: u32 = 3;

/// One run of one stage in one context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub stage: String,
    pub n: u32,
    /// `root`, a lane's name, or `joined`.
    pub context: String,
    pub kind: AttemptKind,
    #[serde(flatten)]
    pub state: AttemptState,
    /// The Switchboard project the attempt ran in.
    pub project: Option<String>,
    pub session: Option<String>,
    pub run: Option<String>,
    /// Artifact name to its path under the ticket directory.
    pub artifacts: BTreeMap<String, PathBuf>,
    pub settle: BTreeMap<String, Settle>,
    /// The Stop the agent reported, once seen.
    pub stop_at_ms: Option<u64>,
    /// Polls since the stop with an artifact still missing; a few are
    /// allowed, then the attempt failed.
    #[serde(default)]
    pub polls_since_stop: u32,
    /// The head commit of the context's tree when the attempt completed.
    pub head: Option<String>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

impl Attempt {
    #[must_use]
    pub fn is_open(&self) -> bool {
        matches!(self.state, AttemptState::Starting | AttemptState::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionKind {
    /// "May Dispatch do this": the answer authorises the next attempt.
    Permission,
    /// "You did this": never answered automatically.
    Confirmation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum DecisionState {
    Pending,
    Answered {
        answer: String,
        note: Option<String>,
        by: String,
        at_ms: u64,
        /// The runner has acted on the answer.
        acted: bool,
    },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub stage: String,
    /// The decision kind from the pipeline (`lanes`, `finalize`, ...).
    pub name: String,
    pub kind: DecisionKind,
    pub question: String,
    pub options: Vec<String>,
    pub recommendation: Option<String>,
    /// The attempt the decision is about, as `(stage, n)`, if one.
    #[serde(default)]
    pub attempt: Option<(String, u32)>,
    #[serde(flatten)]
    pub state: DecisionState,
    pub made_ms: u64,
}

impl Decision {
    #[must_use]
    pub fn pending(&self) -> bool {
        self.state == DecisionState::Pending
    }

    /// The answer given and not yet acted on.
    #[must_use]
    pub fn unacted_answer(&self) -> Option<&str> {
        match &self.state {
            DecisionState::Answered {
                answer,
                acted: false,
                ..
            } => Some(answer),
            _ => None,
        }
    }
}

/// One request to Switchboard: written before it is sent, its reply
/// written after. `reply: None` after a restart is what recovery
/// resolves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub op: String,
    pub kind: String,
    /// `creation`, `idempotent`, `non-replayable`.
    pub class: String,
    /// The attempt it belongs to, as `(stage, n)`, if any.
    pub attempt: Option<(String, u32)>,
    /// What the request was for (`session`, `run`, `root-project`,
    /// `lane-project:<lane>`, `space`, `set`, ...), which is how its
    /// reply's records are applied, now or in recovery.
    #[serde(default)]
    pub intent: String,
    pub sent_ms: u64,
    /// The request itself, so an idempotent one whose reply was lost can
    /// be sent again as the same operation.
    #[serde(default)]
    pub body: Option<Body>,
    pub reply: Option<Reply>,
    /// The socket failed before a reply came; recovery decides.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum TicketState {
    Active,
    /// Stopping: open attempts are cancelled and the ticket's processes
    /// killed; `Parked` once Switchboard reports every one gone.
    Parking {
        reason: String,
    },
    /// Stopped with a reason; requeue or close by hand.
    Parked {
        reason: String,
    },
    /// Every stage done, or closed by hand.
    Closed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub project: String,
    pub source: SourceSnapshot,
    pub pipeline_fingerprint: String,
    /// The pipeline as copied when the ticket was taken.
    pub pipeline_file: PathBuf,
    pub lanes: Vec<LaneRecord>,
    /// The ticket's own tree: a worktree of the project's repository on
    /// the ticket's branch. Lanes of their own repositories sit inside
    /// it. None until cut, and for a project that works in place.
    #[serde(default)]
    pub tree: Option<PathBuf>,
    /// Index of the current stage in the pipeline copy.
    pub stage: usize,
    pub attempts: Vec<Attempt>,
    pub decisions: Vec<Decision>,
    pub ledger: Vec<Operation>,
    /// Every Switchboard session made for this ticket, until removed.
    pub processes: Vec<String>,
    /// The Switchboard project for the root context, once made.
    pub root_project: Option<String>,
    #[serde(flatten)]
    pub state: TicketState,
    pub created_ms: u64,
    pub updated_ms: u64,
}

impl Ticket {
    /// The attempts of stage `stage`, latest numbers last.
    pub fn attempts_of<'a>(&'a self, stage: &'a str) -> impl Iterator<Item = &'a Attempt> + 'a {
        self.attempts.iter().filter(move |a| a.stage == stage)
    }

    /// The most recent completed attempt of any stage that wrote `name`.
    #[must_use]
    pub fn input(&self, name: &str) -> Option<&PathBuf> {
        self.attempts
            .iter()
            .rev()
            .filter(|a| a.state == AttemptState::Complete)
            .find_map(|a| a.artifacts.get(name))
    }

    #[must_use]
    pub fn pending_decisions(&self) -> Vec<&Decision> {
        self.decisions.iter().filter(|d| d.pending()).collect()
    }

    /// The session a card for this ticket should show: the latest
    /// attempt's, or none.
    #[must_use]
    pub fn current_session(&self) -> Option<&String> {
        self.attempts.iter().rev().find_map(|a| a.session.as_ref())
    }

    #[must_use]
    pub fn active(&self) -> bool {
        self.state == TicketState::Active
    }

    /// A short id for the command line: eight hex characters.
    #[must_use]
    pub fn new_id() -> String {
        uuid::Uuid::new_v4().simple().to_string()[..8].to_owned()
    }
}

/// Per project: what Dispatch made in Switchboard for it, and the queue.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectState {
    pub name: String,
    /// The Switchboard workspace named by the pipeline, once found or made.
    pub space: Option<String>,
    /// The queue's working set, once made.
    pub set: Option<String>,
    /// Ticket ids in the order they are taken from.
    pub queue: Vec<String>,
    /// What the set last showed, so it is redrawn only on a change.
    pub shown: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_come_from_the_latest_completed_attempt_that_wrote_them() {
        let mut t = Ticket {
            id: "t".into(),
            project: "p".into(),
            source: SourceSnapshot {
                kind: "manual".into(),
                identity: "x".into(),
                number: None,
                title: String::new(),
                body: String::new(),
                url: None,
                labels: vec![],
                taken_at_ms: 0,
            },
            pipeline_fingerprint: String::new(),
            pipeline_file: PathBuf::new(),
            lanes: vec![],
            tree: None,
            stage: 0,
            attempts: vec![],
            decisions: vec![],
            ledger: vec![],
            processes: vec![],
            root_project: None,
            state: TicketState::Active,
            created_ms: 0,
            updated_ms: 0,
        };
        let attempt = |stage: &str, n: u32, state: AttemptState, path: &str| Attempt {
            stage: stage.into(),
            n,
            context: "root".into(),
            kind: AttemptKind::Agent,
            state,
            project: None,
            session: Some(format!("s-{stage}-{n}")),
            run: None,
            artifacts: BTreeMap::from([("plan".to_owned(), PathBuf::from(path))]),
            settle: BTreeMap::new(),
            stop_at_ms: None,
            polls_since_stop: 0,
            head: None,
            started_ms: 0,
            ended_ms: None,
        };
        t.attempts
            .push(attempt("plan", 1, AttemptState::Complete, "/a1/plan.md"));
        t.attempts.push(attempt(
            "plan",
            2,
            AttemptState::Failed { reason: "x".into() },
            "/a2/plan.md",
        ));
        assert_eq!(t.input("plan"), Some(&PathBuf::from("/a1/plan.md")));
        assert_eq!(t.input("notes"), None);
        assert_eq!(t.current_session().unwrap(), "s-plan-2");
        assert_eq!(t.attempts_of("plan").count(), 2);
        assert_eq!(Ticket::new_id().len(), 8);
    }
}
