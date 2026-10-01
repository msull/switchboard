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
    /// For a ticket taken from someone else's work: the pull requests,
    /// one per lane. Empty for an issue.
    #[serde(default)]
    pub pull_requests: Vec<PullRequestSource>,
}

impl SourceSnapshot {
    /// Whether the ticket reviews pull requests rather than doing work
    /// of its own: its lanes are their branches and nothing pushes.
    #[must_use]
    pub fn is_pull_request(&self) -> bool {
        !self.pull_requests.is_empty()
    }

    /// How a listing names the source: `#12` for an issue, and for pull
    /// requests `pr <lane>/<n>` joined by `+`, as `take` spells them, so
    /// a PR and an issue with the same number never read alike.
    #[must_use]
    pub fn label(&self) -> String {
        if self.is_pull_request() {
            let prs: Vec<String> = self
                .pull_requests
                .iter()
                .map(|pr| format!("{}/{}", pr.lane, pr.number))
                .collect();
            format!("pr {}", prs.join("+"))
        } else {
            format!("#{}", self.number.unwrap_or(0))
        }
    }
}

/// One pull request a ticket was taken from, in the lane whose
/// repository it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestSource {
    pub lane: String,
    pub provider: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    /// The PR's own branch.
    pub branch: String,
    /// The branch it goes into.
    #[serde(default)]
    pub base: String,
    /// The remote of Dispatch's clone it is fetched from: the lane's
    /// own, or one of the pipeline's named `remotes` (a mirror).
    #[serde(default)]
    pub remote: String,
    /// The branch the lane checks out: `pr/<n>` from a GitHub pull
    /// ref (so a fork's PR works), the PR's own branch elsewhere.
    #[serde(default)]
    pub local: String,
    /// Its head when taken.
    pub head: String,
    pub title: String,
}

impl PullRequestSource {
    /// The branch the lane is on.
    #[must_use]
    pub fn local(&self) -> &str {
        if self.local.is_empty() {
            &self.branch
        } else {
            &self.local
        }
    }
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
    /// The commit the lane was cut from, resolved once at the cut:
    /// what a code review diffs against, whatever the remote has since.
    #[serde(default)]
    pub base_sha: Option<String>,
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
    /// A code review stage's attempt: rounds of reviewers and an
    /// implementer, then the stage's checks.
    Review,
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
    /// Stopped by Dispatch (the ticket parked, a rerun replaced it, or
    /// a later gate sent the work back). Nothing is asked when it stops;
    /// as a context's latest attempt after a resume, it is asked about
    /// again with a `rerun` decision that quotes the reason.
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
    /// The stage's command gate, once the agent stopped and it started.
    #[serde(default)]
    pub gate: Option<GateRun>,
    /// The pull request a `pr-checks` gate is bound to, once looked up.
    #[serde(default)]
    pub pr: Option<PullRequestRecord>,
    /// A code review attempt's rounds, first to last.
    #[serde(default)]
    pub rounds: Vec<ReviewRound>,
    /// A `review-cap` answer of `more`: one review pass past the cap
    /// is allowed.
    #[serde(default)]
    pub extra_pass: bool,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

/// One round of a code review: every reviewer read `base..head`, then
/// the findings were addressed by a fresh implementer (or there were
/// none, or the user accepted them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRound {
    pub n: u32,
    /// The commit the branch was cut from, never resolved again.
    pub base: String,
    /// The branch's head every reviewer read.
    pub head: String,
    pub reviewers: Vec<ReviewerRun>,
    #[serde(flatten)]
    pub state: RoundState,
    /// The aggregated findings, once every reviewer finished.
    pub feedback: Option<PathBuf>,
    /// Open points after aggregation: new findings plus points kept
    /// from earlier rounds.
    #[serde(default)]
    pub open_points: u32,
    /// The user authorised the fix pass (or the dial did).
    #[serde(default)]
    pub fix_authorised: bool,
    /// The implementer's session, once started.
    pub implementer: Option<String>,
    pub response: Option<PathBuf>,
    /// The head after the implementer committed.
    pub head_after: Option<String>,
    #[serde(default)]
    pub stop_at_ms: Option<u64>,
    #[serde(default)]
    pub polls_since_stop: u32,
    #[serde(default)]
    pub settle: Option<Settle>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "round_state", rename_all = "kebab-case")]
pub enum RoundState {
    /// Reviewers are running.
    Reviewing,
    /// No open point: the head is accepted; the stage's checks next.
    Converged,
    /// Open points, waiting for the fix pass to be authorised.
    Findings,
    /// The implementer is addressing the points.
    Fixing,
    /// The implementer committed; the checks at the new head next,
    /// then the next round.
    Fixed,
    /// The user accepted the reviewed head with findings left.
    Accepted,
    Failed {
        reason: String,
    },
}

/// One reviewer in one round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerRun {
    pub name: String,
    /// `claude`, `codex` or `command`.
    pub kind: String,
    pub dir: PathBuf,
    /// Where its findings go: the file an agent writes, a command's
    /// stdout.
    pub feedback: PathBuf,
    pub session: Option<String>,
    /// The intent to start a command was written before it ran.
    #[serde(default)]
    pub launched: bool,
    #[serde(default)]
    pub stop_at_ms: Option<u64>,
    #[serde(default)]
    pub polls_since_stop: u32,
    #[serde(default)]
    pub settle: Option<Settle>,
    pub result: Option<ReviewerResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum ReviewerResult {
    /// Nothing to report (the sentinel, or a command's exit 0).
    Clean,
    Findings,
    Failed {
        reason: String,
    },
}

/// A command gate run for an attempt: started on a clean tree at a
/// head, its exit bound to that head only if the tree is unchanged
/// after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateRun {
    /// The head the check ran against.
    pub head: String,
    pub argv: Vec<String>,
    /// Where the check's output goes.
    pub log: PathBuf,
    pub started_ms: u64,
    pub exit: Option<i32>,
}

/// A pull request as a gate last read it: which one, where, the head
/// it was at, and what its checks said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestRecord {
    pub provider: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub head: String,
    /// `pending`, `passed`, `failed: <names>`, `none`, `merged`,
    /// `closed`, or `error: <why>`.
    pub checks: String,
    /// When it was last read; zero after a `recheck` answer, so the
    /// next pass reads it without waiting out the poll interval.
    pub checked_ms: u64,
    /// When lookups started failing, until one succeeds.
    #[serde(default)]
    pub error_since_ms: Option<u64>,
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
    /// What the user said when sending a stage's work back from a
    /// later human gate, by `<stage>/<context>`, until the next attempt
    /// of that stage takes it into its prompt.
    #[serde(default)]
    pub rework: BTreeMap<String, String>,
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
        self.input_with_stage(name).map(|(_, path)| path)
    }

    /// The same, with the stage whose attempt wrote it: a reader is told
    /// whose notes these are when a later stage wrote none.
    #[must_use]
    pub fn input_with_stage(&self, name: &str) -> Option<(&str, &PathBuf)> {
        self.attempts
            .iter()
            .rev()
            .filter(|a| a.state == AttemptState::Complete)
            .find_map(|a| a.artifacts.get(name).map(|p| (a.stage.as_str(), p)))
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
                pull_requests: Vec::new(),
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
            rework: BTreeMap::new(),
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
            gate: None,
            pr: None,
            rounds: Vec::new(),
            extra_pass: false,
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
