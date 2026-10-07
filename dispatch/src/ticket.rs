//! The records: a ticket with its lanes, attempts, decisions and ledger,
//! and the per-project state (the workspace and set in Switchboard, the
//! queue). Plain data; the scheduler changes it, the store writes it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use switchboard_control::{Body, Class, Reply};

use crate::events::short;
use crate::history::Commits;

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
    /// Who took it: `supervisor`, or `None` for the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken_by: Option<String>,
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
    /// A rebase onto a moved base that stopped on a conflict and has not
    /// been brought up yet: the head that was reviewed, kept while a
    /// rebaser or a hand rebase works, so the bring-up can say what the
    /// resolution changed.
    #[serde(default)]
    pub conflict: Option<RefreshConflict>,
    /// The lane's worktree is removed from its clone: the ticket closed.
    /// The path stays, so a reader can still say where the work was.
    #[serde(default)]
    pub removed: bool,
}

/// A base that moved under a branch, and the branch brought up to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refreshed {
    /// The base the branch sat on; empty when it was not recorded and
    /// could not be read from the branch.
    pub from: String,
    pub to: String,
    /// The branch had commits of its own, so it was rebased rather
    /// than moved: the next code review checks the rebase.
    #[serde(default)]
    pub commits: bool,
    /// The rebaser's notes, when this bring-up followed a rebaser.
    #[serde(default)]
    pub notes: Option<PathBuf>,
    /// When it was recorded: tells a rebaser for an earlier move apart
    /// from one for this move. 0 is a record from before the field, after
    /// which no rebaser's notes are attached.
    #[serde(default)]
    pub at_ms: u64,
    /// The conflict this bring-up resolved, when the branch was rewritten
    /// by a rebaser or by hand after the rebase stopped; `None` for a
    /// clean rebase.
    #[serde(default)]
    pub conflict: Option<RefreshConflict>,
    /// The head the branch was brought up to.
    #[serde(default)]
    pub after: Option<String>,
}

/// A rebase of a lane's branch that stopped on a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshConflict {
    /// The branch's head when the rebase first stopped: the last head a
    /// review read.
    pub before: String,
    /// The base the branch sat on; empty when it was not recorded.
    pub from: String,
    /// The base it would not rebase onto, the latest one tried.
    pub to: String,
    /// The branch's commits whose replay onto `to` conflicts, oldest
    /// first; empty when they could not be read.
    #[serde(default)]
    pub commits: Vec<String>,
    /// The index of the pipeline stage it was last seen at, and at the
    /// bring-up the stage of the bring-up: what decides whether a code
    /// review still reads it.
    #[serde(default)]
    pub stage: usize,
    /// When the rebase first stopped: only a rebaser started at or after
    /// it worked on this conflict. 0 is a record from before the field.
    #[serde(default)]
    pub at_ms: u64,
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

/// How long an implementer's tree may stay dirty after its response
/// settled while its session still runs: a commit whose pre-commit hook
/// runs the whole test suite takes minutes, and the response can settle
/// before it lands. Five minutes, from the first pass that found the
/// tree dirty.
pub const DIRTY_WAIT_MS: u64 = 300_000;

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
    /// Asking about it again offers `check` too: its last failure was at
    /// the stage's checks, or a restart cancelled it with its checks
    /// started and, for a code review, before its rewrite.
    #[serde(default)]
    pub failed_at_checks: bool,
    /// Its last failure was the rewrite of its commits, with the branch
    /// at the reviewed head, so asking about it again offers `keep` too.
    #[serde(default)]
    pub failed_at_rewrite: bool,
    /// A code review attempt that continues an earlier attempt of the
    /// same stage and context: that attempt's last reviewed state, its
    /// settled and open points, are this one's start. Written when the
    /// attempt is made and never changed.
    #[serde(default)]
    pub carried_from: Option<(String, u32)>,
    /// The note the user sent a code review attempt's stage back with,
    /// taken off `Ticket::rework` when the attempt started: the first
    /// fix pass is given it.
    #[serde(default)]
    pub rework: Option<String>,
    /// The history rewrite a code review attempt made as it completed:
    /// written as intent (`after` unset) before git writes anything.
    #[serde(default)]
    pub rewrite: Option<Rewrite>,
    /// When each nudge was sent into the agent's session after it
    /// stopped with a dirty tree, in ms. Pushed before the send, so a
    /// restart never sends one twice.
    #[serde(default)]
    pub nudges: Vec<u64>,
    /// Check groups a previous runner left running for this attempt,
    /// signalled by a later one before the checks ran again or the
    /// attempt was cancelled.
    #[serde(default)]
    pub orphans_killed: Vec<OrphanKill>,
    /// The artifact names that are secret, from the ticket's copy of the
    /// stage when the attempt was made; never changed afterwards, so a
    /// reader knows what never to read without the pipeline.
    #[serde(default)]
    pub secret: BTreeSet<String>,
    /// Each secret artifact whose file was deleted, and why. The path
    /// stays in `artifacts`, so the name is still listed.
    #[serde(default)]
    pub forgotten: BTreeMap<String, Forgotten>,
    /// A plan review's rounds the owner opened with a `revise` answer to
    /// `finalize`, first to last. Pushed before `workflow.object` is
    /// sent and popped when Switchboard refuses it.
    #[serde(default)]
    pub revisions: Vec<Revision>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

/// One owner's objection to a finished plan review, sent as round
/// `round` of its run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub round: u32,
    /// Who answered `revise`, as the decision's `by` says it: `you` or
    /// `supervisor`.
    pub by: String,
    pub at_ms: u64,
}

/// A secret artifact's file deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forgotten {
    /// When the file was deleted, in ms.
    pub at_ms: u64,
    /// Why: `"<resource> released"`, `"attempt failed"`, `"attempt
    /// cancelled"`, `"attempt replaced"`, `"parked"` or `"closed"`.
    pub why: String,
}

/// `Rewrite::skipped` when the branch is already on the remote.
pub const PUBLISHED: &str = "the branch is published";

/// `Rewrite::skipped` when the user answered `keep` to a failed rewrite.
pub const KEPT_BY_HAND: &str = "the user kept them after the rewrite failed";

/// A code review attempt's rewrite of its branch's commits, from the
/// head its checks passed at to one with the same tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rewrite {
    /// `fold` or `one`; `keep` writes no record.
    pub mode: Commits,
    /// The head the checks passed at: the branch before the rewrite.
    pub before: String,
    /// The head the rewrite produced. On a complete attempt the branch
    /// is here; on a failed one the branch is back at `before`.
    #[serde(default)]
    pub after: Option<String>,
    /// Commits ahead of the base before the rewrite.
    #[serde(default)]
    pub from: u32,
    /// Commits ahead of the base after it.
    #[serde(default)]
    pub to: u32,
    /// Why history was left as it was: [`PUBLISHED`] or [`KEPT_BY_HAND`],
    /// a clause that reads after "commits kept:".
    #[serde(default)]
    pub skipped: Option<String>,
    /// The folded commits whose messages name something neither their
    /// own diff nor the tree at the folded head has; the attempt asks
    /// about them before it completes.
    #[serde(default)]
    pub stale: Vec<StaleMessage>,
    /// What became of the stale messages once the user answered.
    #[serde(default)]
    pub message: Option<MessageFix>,
    pub at_ms: u64,
}

impl Rewrite {
    /// What became of the stale messages, as a clause: `None` while
    /// nothing is stale or the question is open.
    #[must_use]
    pub fn message_outcome(&self) -> Option<String> {
        if self.stale.is_empty() {
            return None;
        }
        let m = self.message.as_ref()?;
        // A rewording that landed and then still named something leaves
        // the branch at `to`: what an accept keeps is the reworded message.
        Some(match (m.answer.as_str(), &m.to, &m.failed) {
            (answer, Some(to), failed) => {
                let moved = format!("rewritten, {} → {}", short(&m.from), short(to));
                match (failed, answer) {
                    (None, _) => moved,
                    (Some(why), "accept") => format!("{moved}, but {why}; kept as rewritten"),
                    (Some(why), _) => format!("{moved}, but {why}"),
                }
            }
            ("accept", None, Some(why)) => format!("rewrite failed: {why}; kept as written"),
            ("accept", None, None) => "kept as written".to_owned(),
            (_, None, Some(why)) => format!("rewrite failed: {why}"),
            (_, None, None) => "being rewritten".to_owned(),
        })
    }

    /// Every name the stale messages lack, deduped in commit order.
    #[must_use]
    pub fn stale_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for n in self.stale.iter().flat_map(|s| &s.names) {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
        names
    }
}

/// A folded commit whose message names something that neither its own
/// diff nor the tree at the folded head has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleMessage {
    /// Its position in `base..after`, oldest first.
    pub index: u32,
    pub subject: String,
    /// The backticked names it lacks, in message order.
    pub names: Vec<String>,
}

/// What became of a rewrite's stale messages once the user answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFix {
    /// `accept` or `rewrite`.
    pub answer: String,
    /// The folded head the question was about.
    pub from: String,
    /// Saved before the rewriter's `session.new` is sent, so a lost or
    /// not yet applied reply never sends a second one.
    #[serde(default)]
    pub launched: bool,
    /// The rewriter's session, once its reply is applied.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub stop_at_ms: Option<u64>,
    /// Counted as `Attempt::polls_since_stop` is, for the rewriter and
    /// its files.
    #[serde(default)]
    pub polls_since_stop: u32,
    /// Each output file's settling, by path.
    #[serde(default)]
    pub settle: BTreeMap<String, Settle>,
    /// Saved before git writes the reworded commits.
    #[serde(default)]
    pub moving: bool,
    /// The head carrying the new messages, once the branch moved there.
    #[serde(default)]
    pub to: Option<String>,
    /// Why `rewrite` ended without a clean result; the question is then
    /// asked again with `accept | park` only.
    #[serde(default)]
    pub failed: Option<String>,
    pub at_ms: u64,
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
    /// Passes that found the tree dirty after the response settled, kept
    /// for the record and so the wait is logged once; it bounds nothing,
    /// `dirty_since_ms` does.
    #[serde(default)]
    pub dirty_polls: u32,
    /// When a pass first found the tree dirty after the response
    /// settled; the wait is bounded by `DIRTY_WAIT_MS` from here.
    #[serde(default)]
    pub dirty_since_ms: Option<u64>,
    /// When each nudge was sent into the implementer's session after it
    /// stopped with a dirty tree, in ms; each round starts at none.
    #[serde(default)]
    pub nudges: Vec<u64>,
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
    /// A command reviewer's process group as started, so a runner that
    /// restarts while it runs can stop it before the round fails.
    /// Cleared when a later runner finds the group gone; never read
    /// once `result` is set.
    #[serde(default)]
    pub group: Option<CheckGroup>,
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
    /// The check's process group as started, so a runner that restarts
    /// while it runs can stop it, or wait for it when it is a gate-only
    /// command. Cleared when a later runner finds the group gone or the
    /// owner answers `released`; never read once `exit` is set.
    #[serde(default)]
    pub group: Option<CheckGroup>,
    /// When a runner first found this gate-only command lost to a
    /// restart with its group still running. Saved so the stop limit
    /// counts across further restarts before `stuck` is asked; cleared
    /// with `group`.
    #[serde(default)]
    pub lost_since_ms: Option<u64>,
}

/// A check's or command reviewer's process group as started, so a
/// later runner can tell whether what it finds under the id is the
/// same group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckGroup {
    /// The group id, which is the leader's pid.
    pub pgid: u32,
    /// The leader's start time as `ps -o lstart=` printed it (under
    /// `LC_ALL=C`, `TZ=UTC`) right after the spawn. A pid is reused once
    /// its group is empty, and a reused one starts at another time.
    pub leader_started: String,
}

/// A check group a previous runner left running, signalled by this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrphanKill {
    /// The group id signalled, which was the orphan leader's pid.
    pub pgid: u32,
    /// The orphan leader's start time, copied from its `CheckGroup`:
    /// with `pgid` it names one group, since a pid comes back once its
    /// group is empty.
    pub leader_started: String,
    /// The head the orphan was checking.
    pub head: String,
    /// When the group got its first TERM: the stop limit counts from it
    /// across restarts while the leader lives, before the group gets
    /// SIGKILL.
    pub at_ms: u64,
    /// The command reviewer the group ran, by name; `None` is the
    /// checks. A reviewer's `head` is its round's.
    #[serde(default)]
    pub reviewer: Option<String>,
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
    /// Answers a supervisor gave that its `decides` did not allow; the
    /// decision still waits on the owner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<Refusal>,
}

/// A supervisor's answer, or command, refused because it is the owner's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// Who answered: `supervisor`.
    pub by: String,
    /// The answer or command it gave, which was not applied: a
    /// decision's answer, or `runner restart` on a project's record.
    pub answer: String,
    /// When, in Unix ms.
    pub at_ms: u64,
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

/// The decision asked when a service's stop is not confirmed within
/// the stop limit: answered `wait` or `released`, and asked even while
/// the ticket parks or closes.
pub const STUCK: &str = "stuck";

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
    /// A request about to be sent, with no reply yet: what a ledger
    /// writes before the call.
    #[must_use]
    pub fn new(
        op: String,
        body: &Body,
        attempt: Option<(String, u32)>,
        intent: &str,
        sent_ms: u64,
    ) -> Self {
        let class = match body.class() {
            Class::Creation => "creation",
            Class::Idempotent => "idempotent",
            Class::NonReplayable => "non-replayable",
            Class::Query => "query",
        };
        Self {
            op,
            kind: body.kind(),
            class: class.into(),
            attempt,
            intent: intent.into(),
            sent_ms,
            body: Some(body.clone()),
            reply: None,
            error: None,
            asked: false,
            settled: false,
        }
    }

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
    /// The newest copy of the pipeline: the one taken with the ticket,
    /// or the one its last restart wrote.
    pub pipeline_file: PathBuf,
    pub lanes: Vec<LaneRecord>,
    /// The ticket's own tree: a worktree of the project's repository on
    /// the ticket's branch. Lanes of their own repositories sit inside
    /// it. None until cut, and for a project that works in place.
    #[serde(default)]
    pub tree: Option<PathBuf>,
    /// The tree's last bring-up to the project's base, for a ticket none
    /// of whose chosen lanes lives on the tree's branch.
    #[serde(default)]
    pub tree_refreshed: Option<Refreshed>,
    /// Index of the current stage in the newest pipeline copy.
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
    /// Who made the last park, resume or close: `supervisor`, or `None`
    /// for the owner and for Dispatch itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_by: Option<String>,
    /// How far a close has got.
    #[serde(default)]
    pub close: CloseProgress,
    /// Every restart, oldest first.
    #[serde(default)]
    pub restarts: Vec<Restart>,
    /// A restart asked for and not yet applied; it rides on `Parking`.
    #[serde(default)]
    pub restart: Option<RestartIntent>,
    /// What each branch stood at as the ticket advanced into a stage,
    /// oldest first; the newest entry for a stage name is the one a
    /// ranged restart resets to.
    #[serde(default)]
    pub entered: Vec<StageEntry>,
    /// The resources the ticket holds. The record is the authority:
    /// another ticket's hold is read from its record, under the writer
    /// lock, when one is to be taken.
    #[serde(default)]
    pub holds: Vec<Hold>,
    /// The lanes served for its stages, every record kept once stopped.
    #[serde(default)]
    pub services: Vec<ServiceRecord>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

/// A restart applied: the ticket put at a stage under a fresh copy of
/// the live pipeline, the later work discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restart {
    /// When it applied: the write that saved it, and its event's time.
    pub at_ms: u64,
    /// The stage the ticket stood at.
    pub from: String,
    /// The stage it stands at now: `from` unless a stage was named.
    pub to: String,
    /// The copy earlier attempts ran under.
    pub before: PathBuf,
    /// The copy this restart wrote.
    pub after: PathBuf,
    /// The attempts it discarded, as `(stage, n)`.
    #[serde(default)]
    pub discarded: Vec<(String, u32)>,
    /// The branches it moved back.
    #[serde(default)]
    pub reset: Vec<HeadReset>,
    /// The lanes whose setup changed, so it runs again.
    #[serde(default)]
    pub setup_again: Vec<String>,
}

/// A restart asked for and not yet applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestartIntent {
    /// The stage named, or `None` for the current one.
    pub stage: Option<String>,
    /// When it was asked. Nothing reads it: it is there for whoever
    /// opens the record of a held restart and wants to know how long it
    /// has been held.
    pub made_ms: u64,
    /// Resets already done, saved one by one, so a cut-short apply
    /// carries on rather than moving a branch twice.
    #[serde(default)]
    pub reset: Vec<HeadReset>,
}

/// One branch moved back by a restart: `root` for the ticket's tree, a
/// lane's name for a lane with a repository of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadReset {
    /// `root` or the lane's name.
    pub key: String,
    /// The head the branch was at before the reset, a commit and not a
    /// stage name, unlike `Restart::from`.
    pub from: String,
    /// The head it was reset to: the one recorded at the stage's entry.
    pub to: String,
}

/// `root abc1234 → def5678`: how the CLI, the view and an error show it.
impl std::fmt::Display for HeadReset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} → {}",
            self.key,
            short(&self.from),
            short(&self.to)
        )
    }
}

/// What each branch stood at as the ticket advanced into a stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageEntry {
    /// The stage advanced into, by name.
    pub stage: String,
    /// When the ticket advanced into it.
    pub at_ms: u64,
    /// Heads by key: `root` for the ticket's tree, a lane's name for
    /// each lane not removed. A head that could not be read is absent.
    #[serde(default)]
    pub heads: BTreeMap<String, String>,
    /// Each lane's own fields at entry, by lane name.
    #[serde(default)]
    pub lanes: BTreeMap<String, LaneAtEntry>,
    /// The ticket's `tree_refreshed` then.
    #[serde(default)]
    pub tree_refreshed: Option<Refreshed>,
}

/// The lane fields that describe its head at a stage entry, restored
/// with it by a ranged restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneAtEntry {
    /// The lane's `base_sha` then.
    #[serde(default)]
    pub base_sha: Option<String>,
    /// The lane's `refreshed` then.
    #[serde(default)]
    pub refreshed: Option<Refreshed>,
    /// The lane's `conflict` then.
    #[serde(default)]
    pub conflict: Option<RefreshConflict>,
}

/// A resource this ticket holds over a run of consecutive stages that
/// name it in `needs`, from the run's first stage to its last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    pub resource: String,
    /// The stage it was taken at, or the stage of another run a
    /// send-back carried it to.
    pub stage: String,
    pub taken_ms: u64,
}

/// A lane served for a stage: its `before`, its port, its Switchboard
/// service session, and how far it got.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRecord {
    pub lane: String,
    /// 1 for the first record of this stage and lane on the ticket, then
    /// one more per retry or re-entry: it names the record's ledger
    /// intent and its `before` check key.
    pub n: u32,
    /// The stage that started it.
    pub stage: String,
    /// The last stage it lives through: the end of the starting stage's
    /// `needs` range.
    pub until: String,
    /// The lane's `before` command, run as a child of the runner; its
    /// head is the lane's when it started.
    #[serde(default)]
    pub before: Option<GateRun>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub url: Option<String>,
    /// The `session.new` operation, copied from the ledger entry under
    /// this record's intent.
    #[serde(default)]
    pub op: Option<String>,
    /// The session it made, also on `Ticket::processes`, so parking and
    /// closing kill it with the rest.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(flatten)]
    pub state: ServiceState,
    /// When the record was made, and again when its launch was sent:
    /// the readiness limit counts from the launch.
    pub started_ms: u64,
    #[serde(default)]
    pub ready_ms: Option<u64>,
    /// When a stop first found the record still to stop; the stop limit
    /// counts from here, and a `wait` answer resets it.
    #[serde(default)]
    pub stopping_ms: Option<u64>,
    /// What the stop found still alive when it asked `stuck`; a
    /// `released` answer records it as what the user stopped.
    #[serde(default)]
    pub stuck_on: Option<String>,
    /// What the user said was stopped by hand (a `released` answer to
    /// `stuck`), when the stop was not confirmed by the runner.
    #[serde(default)]
    pub released: Option<String>,
}

/// How far a served lane got. Tagged `service_state` so it reads apart
/// from the ticket's and the attempts' flattened `state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "service_state", rename_all = "kebab-case")]
pub enum ServiceState {
    /// The lane's `before` runs, or is about to.
    Before,
    /// The launch is sent; the readiness probe has not answered yet.
    Starting,
    Ready,
    Failed {
        reason: String,
    },
    /// Its `before` exited, its session is gone and removed, and its
    /// port binds again; or the user said so.
    Stopped,
}

impl ServiceState {
    /// The state as a word, with a failure's reason.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Before => "before".to_owned(),
            Self::Starting => "starting".to_owned(),
            Self::Ready => "ready".to_owned(),
            Self::Failed { reason } => format!("failed: {reason}"),
            Self::Stopped => "stopped".to_owned(),
        }
    }
}

impl ServiceRecord {
    /// The ledger intent its launch is sent under.
    #[must_use]
    pub fn intent(&self) -> String {
        format!("service:{}:{}:{}", self.stage, self.lane, self.n)
    }
}

impl Ticket {
    /// The name of the secret artifact at `file`, already canonical,
    /// when it is one of any attempt's: what Dispatch never reads.
    #[must_use]
    pub fn secret_at(&self, file: &Path) -> Option<&str> {
        self.attempts.iter().find_map(|a| {
            a.secret.iter().find_map(|name| {
                let path = a.artifacts.get(name)?;
                let path = path.canonicalize().unwrap_or_else(|_| path.clone());
                (path == file).then_some(name.as_str())
            })
        })
    }

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
        self.input_where(name, |_| true)
    }

    /// The most recent completed attempt that wrote `name` among those
    /// `keep` accepts, with its stage: a lane's reader keeps only the
    /// attempts it can see.
    #[must_use]
    pub fn input_where(
        &self,
        name: &str,
        keep: impl Fn(&Attempt) -> bool,
    ) -> Option<(&str, &PathBuf)> {
        self.attempts
            .iter()
            .rev()
            .filter(|a| a.state == AttemptState::Complete && !a.forgotten.contains_key(name))
            .filter(|a| keep(a))
            .find_map(|a| a.artifacts.get(name).map(|p| (a.stage.as_str(), p)))
    }

    #[must_use]
    pub fn pending_decisions(&self) -> Vec<&Decision> {
        self.decisions.iter().filter(|d| d.pending()).collect()
    }

    /// The pending decisions that wait on the user and count against
    /// the project's limit: while the ticket is closing, only a `stuck`
    /// question, which the close waits on; its other pending decisions
    /// are on their way to cancelled. Every count and every view reads
    /// this, so the rule lives in one place.
    #[must_use]
    pub fn waiting_on_you(&self) -> Vec<&Decision> {
        let closing = matches!(self.state, TicketState::Closing { .. });
        self.pending_decisions()
            .into_iter()
            .filter(|d| !closing || d.name == STUCK)
            .collect()
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// The project's supervisor session and its workspace.
    pub supervisor: Supervision,
}

/// A project's supervisor: where it works, the session now, the ones it
/// replaced, and the one request to Switchboard in flight for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Supervision {
    /// The Switchboard project rooted at the workspace, made once.
    pub project: Option<String>,
    /// The workspace, recorded the first time it is set up.
    pub workspace: Option<PathBuf>,
    /// The session now, if one was made and not killed.
    pub current: Option<SupervisorRecord>,
    /// The sessions it replaced and the ones killed, oldest first.
    pub past: Vec<PastSupervisor>,
    /// Written by the port's `supervisor-fresh`; done and cleared by the
    /// runner.
    pub intent: Option<SupervisorIntent>,
    /// The one request to Switchboard in flight; this project's ledger.
    pub op: Option<Operation>,
    /// Why the last fresh, resume or kill failed, until one succeeds.
    pub error: Option<String>,
    /// Commands the supervisor was refused because its table does not
    /// allow them, newest last; only the last `REFUSALS_KEPT`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<Refusal>,
}

/// How many refused commands a project's supervision keeps.
pub const REFUSALS_KEPT: usize = 20;

impl Supervision {
    /// Whether `session` is this project's supervisor, now or before.
    #[must_use]
    pub fn knows(&self, session: &str) -> bool {
        self.current.as_ref().is_some_and(|c| c.session == session)
            || self.past.iter().any(|p| p.session == session)
    }

    /// Whether the current session was seeded from something other than
    /// `table`; false with no session.
    #[must_use]
    pub fn seed_stale(&self, project: &str, table: &crate::pipeline::Supervisor) -> bool {
        self.current
            .as_ref()
            .is_some_and(|c| c.seed_hash != crate::supervisor::seed_hash(project, table))
    }
}

/// The supervisor session now. `session` is Switchboard's record id,
/// never a resume handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupervisorRecord {
    /// Switchboard's record id of the session.
    pub session: String,
    /// The hash of the owner's inputs it was seeded from.
    pub seed_hash: String,
    /// When its `session.new` was sent, in Unix ms.
    pub created_ms: u64,
    /// The model its flags name, when the table set one.
    #[serde(default)]
    pub model: Option<String>,
}

/// A supervisor session that was replaced or killed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PastSupervisor {
    /// Switchboard's record id of the session; still a supervisor's for
    /// the actor check.
    pub session: String,
    /// The hash it was seeded from.
    pub seed_hash: String,
    /// When it was asked for, in Unix ms.
    pub created_ms: u64,
    /// When it was killed or replaced, in Unix ms.
    pub replaced_ms: u64,
    /// `fresh`, or `kill: <reason>`.
    pub why: String,
}

/// What the runner is asked to do for the supervisor on its next pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SupervisorIntent {
    /// A new session, replacing the current one; the workspace is set
    /// up again only when it is missing.
    Fresh,
}

/// An active ticket with nothing on it, for tests.
#[cfg(test)]
pub(crate) fn blank() -> Ticket {
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
            taken_by: None,
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
        tree_refreshed: None,
        state: TicketState::Active,
        state_by: None,
        close: CloseProgress::default(),
        restarts: vec![],
        restart: None,
        entered: vec![],
        holds: Vec::new(),
        services: Vec::new(),
        created_ms: 0,
        updated_ms: 0,
    }
}

/// A chosen lane `name` cut at `/wt/<name>`, for tests.
#[cfg(test)]
pub(crate) fn chosen_lane(name: &str) -> LaneRecord {
    LaneRecord {
        name: name.into(),
        worktree: PathBuf::from(format!("/wt/{name}")),
        branch: "dispatch/1-x".into(),
        project: None,
        chosen: true,
        setup_done: false,
        base_sha: None,
        refreshed: None,
        pushed: None,
        removed: false,
        conflict: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::new_attempt;

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
    fn a_message_outcome_says_which_message_the_branch_keeps() {
        let fix = |answer: &str, to: Option<&str>, failed: Option<&str>| Rewrite {
            mode: Commits::Fold,
            before: "head0001".into(),
            after: Some("fold0001".into()),
            from: 2,
            to: 1,
            skipped: None,
            stale: vec![StaleMessage {
                index: 0,
                subject: "A".into(),
                names: vec!["old_name".into()],
            }],
            message: Some(MessageFix {
                answer: answer.into(),
                from: "fold0001".into(),
                to: to.map(Into::into),
                failed: failed.map(Into::into),
                ..MessageFix::default()
            }),
            at_ms: 1,
        };
        let outcome = |r: Rewrite| r.message_outcome().unwrap();
        let still = Some("the rewritten message still names `old_name`");
        assert_eq!(outcome(fix("rewrite", None, None)), "being rewritten");
        assert_eq!(
            outcome(fix("rewrite", Some("word0001"), None)),
            "rewritten, fold000 → word000"
        );
        assert_eq!(
            outcome(fix("accept", Some("word0001"), still)),
            "rewritten, fold000 → word000, but the rewritten message still names `old_name`; kept as rewritten"
        );
        assert_eq!(
            outcome(fix("accept", None, Some("no file"))),
            "rewrite failed: no file; kept as written"
        );
        assert_eq!(outcome(fix("accept", None, None)), "kept as written");
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
