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
    /// The commit the lane was cut from, or the base it was last
    /// brought up to: what a code review diffs against.
    #[serde(default)]
    pub base_sha: Option<String>,
    /// The last time the branch was brought up to a moved base: told
    /// to the next agent, since its plan was written against `from`.
    #[serde(default)]
    pub refreshed: Option<Refreshed>,
    /// The last head a refresh pushed to the lane's branch: what the
    /// remote holds until an attempt records a later head, so the next
    /// refresh's lease is on Dispatch's own push.
    #[serde(default)]
    pub pushed: Option<PushedHead>,
    /// The lane's worktree is removed from its clone: the ticket closed.
    /// The path stays, so a reader can still say where the work was.
    #[serde(default)]
    pub removed: bool,
}

/// A base that moved under a branch, and the branch brought up to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refreshed {
    pub from: String,
    pub to: String,
}

/// A head pushed to a lane's branch, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushedHead {
    pub head: String,
    pub at_ms: u64,
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

/// Polls an implementer's tree may stay dirty after its stop while its
/// session still runs: a commit whose pre-commit hook runs the whole
/// test suite takes minutes, and the response can settle before it
/// lands. About five minutes at one poll a second.
pub const DIRTY_POLLS: u32 = 300;

/// Polls a stopped agent may sit idle at its prompt without its
/// artifact before it counts as finished without it: about thirty
/// seconds at one poll a second. A card that reads anything but idle
/// (background agents still working, a question to the user) starts
/// the count again.
pub const STOP_IDLE_POLLS: u32 = 30;

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
    /// Polls in a row the stopped agent has sat idle at its prompt with an
    /// artifact still missing; any other card starts the count again. At
    /// `STOP_IDLE_POLLS` the attempt failed.
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
    /// Its last failure was at the stage's checks, so asking about it
    /// again offers `check` too.
    #[serde(default)]
    pub failed_at_checks: bool,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

/// One round of a code review: every reviewer read `base..head`, then
/// the findings were addressed by a fresh implementer (or there were
/// none, or the user accepted them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRound {
    pub n: u32,
    /// The lane's base when the round began: the commit the branch was
    /// cut from, or the base a refresh moved it onto since.
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
    /// Counted as `Attempt::polls_since_stop` is, for the implementer
    /// and its response: at `STOP_IDLE_POLLS` the round failed.
    #[serde(default)]
    pub polls_since_stop: u32,
    #[serde(default)]
    pub settle: Option<Settle>,
    /// Polls the tree has been dirty since the response settled, while
    /// the implementer's session still runs (a commit in flight).
    #[serde(default)]
    pub dirty_polls: u32,
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
    /// Counted as `Attempt::polls_since_stop` is, for this reviewer and
    /// its feedback: at `STOP_IDLE_POLLS` the reviewer failed.
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

    /// A `rerun` answer to a `rerun` question, acted on: the replaced
    /// attempt is confirmed gone and its replacement may launch.
    #[must_use]
    pub fn acted_rerun(&self) -> bool {
        self.name == "rerun"
            && matches!(
                &self.state,
                DecisionState::Answered { answer, acted: true, .. } if answer == "rerun"
            )
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
    /// `space`, `set`, ...), which is how its reply's records are
    /// applied, now or in recovery. `lane-project:<lane>` is no longer
    /// sent but is still applied from older ledgers.
    #[serde(default)]
    pub intent: String,
    pub sent_ms: u64,
    /// The request itself, so an idempotent one whose reply was lost can
    /// be sent again as the same operation.
    #[serde(default)]
    pub body: Option<Body>,
    pub reply: Option<Reply>,
    /// The socket failed before a reply came; recovery decides. Once it
    /// has, this is its verdict in words, for the reader only.
    pub error: Option<String>,
    /// Its reply was lost and it may not be sent again, so the user was
    /// asked what to do; recovery leaves it to that question.
    #[serde(default)]
    pub asked: bool,
    /// Recovery gave its verdict on this unanswered operation, and a
    /// later pass must not recover it again (a lost send would raise
    /// its decision twice).
    #[serde(default)]
    pub settled: bool,
}

impl Operation {
    /// Whether recovery still has to resolve it.
    #[must_use]
    pub fn unresolved(&self) -> bool {
        self.reply.is_none() && !self.asked && !self.settled
    }
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
    /// Closing: the intent is written, and the sequence (decisions
    /// cancelled, processes gone, the session unmarked, the trees
    /// removed, the card off the set) runs from it on every pass until
    /// it is done. Nothing starts in a closing ticket.
    Closing {
        reason: String,
    },
    /// Every stage done, or closed by hand.
    Closed {
        reason: String,
    },
}

impl TicketState {
    /// The state as a person reads it: `active`, or `<state>: <reason>`.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Active => "active".to_owned(),
            Self::Parking { reason } => format!("parking: {reason}"),
            Self::Parked { reason } => format!("parked: {reason}"),
            Self::Closing { reason } => format!("closing: {reason}"),
            Self::Closed { reason } => format!("closed: {reason}"),
        }
    }
}

/// What a close has done so far. Each flag is set and saved right after
/// its step is read back, so a close cut short resumes from the first
/// step not yet done.
// One flag per step of the sequence, each read back on its own; a state
// machine would lose which steps a cut-short close already did.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloseProgress {
    /// Every pending decision is cancelled.
    pub decisions_cancelled: bool,
    /// Switchboard answered `session.waiting off` for the current
    /// session, or the ticket has none.
    pub waiting_cleared: bool,
    /// The ticket's tree is removed from the project's clone.
    pub tree_removed: bool,
    /// Why a removal was refused, when one was and the close went on
    /// without it; the tree is still there.
    pub trees_kept: Option<String>,
    /// The working set was synced without this ticket.
    pub card_cleared: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ticket {
    /// The record's format; see `store::RECORD_VERSION`. A record
    /// written before records carried one reads as 0.
    #[serde(default)]
    pub version: u32,
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
    /// The stage whose start last brought the lanes up to their bases;
    /// a refresh runs once per stage entry.
    #[serde(default)]
    pub refreshed_stage: Option<usize>,
    #[serde(flatten)]
    pub state: TicketState,
    /// How far a close has got.
    #[serde(default)]
    pub close: CloseProgress,
    pub created_ms: u64,
    pub updated_ms: u64,
}

impl Ticket {
    /// The indexes of the ledger's operations recovery still has to
    /// resolve.
    #[must_use]
    pub fn unsettled(&self) -> Vec<usize> {
        self.unsettled_where(|_| true)
    }

    /// `unsettled`, launches only: what parking and closing wait on,
    /// since a launch still in flight would bring up a session after
    /// they stopped everything.
    #[must_use]
    pub fn unsettled_creations(&self) -> Vec<usize> {
        self.unsettled_where(|o| o.class == "creation")
    }

    fn unsettled_where(&self, keep: impl Fn(&Operation) -> bool) -> Vec<usize> {
        self.ledger
            .iter()
            .enumerate()
            .filter(|(_, o)| keep(o) && o.unresolved())
            .map(|(i, _)| i)
            .collect()
    }

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

    /// The pending decisions that wait on the user and count against
    /// the project's limit: none while the ticket is closing, whose
    /// pending decisions are on their way to cancelled. Every count and
    /// every view reads this, so the rule lives in one place.
    #[must_use]
    pub fn waiting_on_you(&self) -> Vec<&Decision> {
        if matches!(self.state, TicketState::Closing { .. }) {
            return Vec::new();
        }
        self.pending_decisions()
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

    /// Whether `close` would start a close, as far as the record says:
    /// parked, or active with nothing open. A tree with changes is
    /// still refused when the close runs; that needs git to tell.
    #[must_use]
    pub fn closable(&self) -> bool {
        match self.state {
            TicketState::Parked { .. } => true,
            TicketState::Active => !self.attempts.iter().any(Attempt::is_open),
            _ => false,
        }
    }

    /// Whether `close` would try the removal of kept trees again: the
    /// ticket closed and a refusal kept them.
    #[must_use]
    pub fn trees_retryable(&self) -> bool {
        matches!(self.state, TicketState::Closed { .. }) && self.close.trees_kept.is_some()
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
    /// The record's format; see `store::RECORD_VERSION`.
    pub version: u32,
    pub name: String,
    /// The Switchboard workspace named by the pipeline, once found or made.
    pub space: Option<String>,
    /// The queue's working set, once made.
    pub set: Option<String>,
    /// Ticket ids in the order they are taken from.
    pub queue: Vec<String>,
    /// Ids of the project's tickets that are closing: out of the queue,
    /// visited by every pass until their close is done.
    pub closing: Vec<String>,
    /// What the set last showed, so it is redrawn only on a change.
    pub shown: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::new_attempt;

    /// An active ticket with nothing on it.
    fn blank() -> Ticket {
        Ticket {
            version: 0,
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
            refreshed_stage: None,
            state: TicketState::Active,
            close: CloseProgress::default(),
            created_ms: 0,
            updated_ms: 0,
        }
    }

    #[test]
    fn inputs_come_from_the_latest_completed_attempt_that_wrote_them() {
        let mut t = blank();
        let attempt = |stage: &str, n: u32, state: AttemptState, path: &str| Attempt {
            session: Some(format!("s-{stage}-{n}")),
            ..new_attempt(
                stage,
                n,
                "root",
                AttemptKind::Agent,
                state,
                BTreeMap::from([("plan".to_owned(), PathBuf::from(path))]),
                0,
            )
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

    #[test]
    fn a_ticket_closes_by_hand_when_parked_or_active_with_nothing_open() {
        let mut t = blank();
        assert!(t.closable() && !t.trees_retryable());
        t.attempts.push(new_attempt(
            "plan",
            1,
            "root",
            AttemptKind::Agent,
            AttemptState::Running,
            BTreeMap::new(),
            0,
        ));
        assert!(!t.closable(), "an open attempt is parked first");
        t.state = TicketState::Parked {
            reason: "by hand".into(),
        };
        assert!(t.closable());
        for state in [
            TicketState::Parking {
                reason: String::new(),
            },
            TicketState::Closing {
                reason: String::new(),
            },
            TicketState::Closed {
                reason: String::new(),
            },
        ] {
            t.state = state;
            assert!(!t.closable(), "{:?}", t.state);
        }
        assert!(!t.trees_retryable(), "closed with its trees removed");
        t.close.trees_kept = Some("it has changes".into());
        assert!(t.trees_retryable());
    }
}
