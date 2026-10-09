//! What Dispatch's port speaks: one JSON line in, one JSON line out, on
//! `<Dispatch data dir>/dispatch.sock`, served by `dispatch run`. A
//! client sees tickets as views (never Dispatch's records), asks for an
//! artifact's text, and answers decisions, reorders a queue or takes an
//! issue exactly as the command line would. This crate depends on
//! nothing of Dispatch's or Switchboard's, so the app can show tickets
//! without knowing how they are stored, and a runner on another
//! machine looks the same through a forwarded socket.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

mod client;

pub use client::Client;

/// The socket's file name under Dispatch's data directory.
pub const SOCKET_FILE: &str = "dispatch.sock";

/// One request line: `{"op":"...","kind":"status",...}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    pub fn parse(line: &str) -> Result<Self, String> {
        parse_line(line)
    }

    #[must_use]
    pub fn to_line(&self) -> String {
        to_line(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Body {
    /// Every ticket and what the runner is.
    Status,
    /// One ticket in full.
    Ticket { id: String },
    /// The text of a file under a ticket's directory (an artifact, a
    /// review round). `path` is the absolute path a view named.
    Artifact { ticket: String, path: PathBuf },
    /// Answer a pending decision; the runner acts on it on its next pass.
    Decide {
        ticket: String,
        decision: String,
        answer: String,
        #[serde(default)]
        note: Option<String>,
    },
    /// A project's queue, reordered when `order` is given.
    Queue {
        project: String,
        #[serde(default)]
        order: Vec<String>,
    },
    /// Make a ticket from the project's source and queue it.
    Take { project: String, issue: String },
    /// A parked ticket back to active.
    Resume { ticket: String },
    /// Close a ticket: its worktrees are removed, and its branch,
    /// directory and record are kept. On a ticket already closed with
    /// its trees kept, the removal is tried again. The answer is the
    /// ticket as it stands, `closing` once the intent is saved; the
    /// runner's next pass does the rest: that work asks the caller's
    /// own control socket, so it is not done while the caller waits on
    /// this reply.
    Close {
        ticket: String,
        #[serde(default)]
        reason: Option<String>,
    },
    /// Where tickets' trees go: read it, set it (`path`), and with
    /// `migrate` move every idle ticket's tree there.
    Worktrees {
        #[serde(default)]
        path: Option<PathBuf>,
        #[serde(default)]
        migrate: bool,
    },
    /// A ticket's events after the cursor `since`, from the runner's
    /// event log.
    Events {
        ticket: String,
        #[serde(default)]
        since: u64,
    },
    /// Start a new supervisor session for the project, replacing the
    /// current one. The answer is the status at once, the intent saved;
    /// the runner's next pass does the work, which asks the caller's own
    /// control socket.
    SupervisorFresh { project: String },
}

impl Body {
    /// The request's kind, as the line names it.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Ticket { .. } => "ticket",
            Self::Artifact { .. } => "artifact",
            Self::Decide { .. } => "decide",
            Self::Queue { .. } => "queue",
            Self::Take { .. } => "take",
            Self::Resume { .. } => "resume",
            Self::Close { .. } => "close",
            Self::Worktrees { .. } => "worktrees",
            Self::Events { .. } => "events",
            Self::SupervisorFresh { .. } => "supervisor-fresh",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "kebab-case")]
pub enum Reply {
    Status(Status),
    Ticket(TicketView),
    Artifact { text: String },
    Decided(DecisionView),
    Queue { order: Vec<String> },
    Taken(TicketView),
    Worktrees(WorktreesView),
    Events(EventsView),
    Failed { reason: String },
}

/// A ticket's events after a cursor, from the runner's log.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EventsView {
    /// This ticket's events with `seq > since`, in order, none withdrawn
    /// and no `void` lines.
    pub events: Vec<EventView>,
    /// The highest seq read, of any ticket, or `since`: the next cursor.
    pub last: u64,
    /// Seqs this ticket's `void` lines in the batch withdraw, so a
    /// client drops ones it cached earlier.
    pub withdrawn: Vec<u64>,
}

/// One line of the runner's event log, as a reader needs it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EventView {
    pub seq: u64,
    pub at_ms: u64,
    /// The attempt's or decision's stage, else the ticket's stage then.
    pub stage: String,
    /// What happened, as the log's word: `stage`, `attempt-ended`,
    /// `answered`, ...
    pub kind: String,
    /// One short line for a person.
    pub text: String,
    /// The attempt it is about, as stage and number.
    pub attempt: Option<(String, u32)>,
    /// The decision it is about, by id.
    pub decision: Option<String>,
    /// The commit it names.
    pub head: Option<String>,
    /// The pull request it names.
    pub url: Option<String>,
}

/// The worktree root after a `worktrees` request, and what a migration
/// moved or left where it was (ticket id, why).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreesView {
    pub root: PathBuf,
    pub moved: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

impl Reply {
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed {
            reason: reason.into(),
        }
    }

    pub fn parse(line: &str) -> Result<Self, String> {
        parse_line(line)
    }

    #[must_use]
    pub fn to_line(&self) -> String {
        to_line(self)
    }
}

/// The runner and everything it drives.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// Dispatch's data directory, where tickets and their files live.
    pub data_dir: PathBuf,
    /// Where tickets' trees go unless a pipeline says otherwise.
    pub worktrees: PathBuf,
    /// The projects with a pipeline file, each with its queue.
    pub projects: Vec<ProjectView>,
    pub tickets: Vec<TicketView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectView {
    pub name: String,
    /// Ticket ids in queue order.
    pub queue: Vec<String>,
    /// The policy's limit on tickets with something running at once.
    pub slots: u32,
    /// The policy's limit on decisions that may wait on the user before
    /// nothing new starts.
    pub waiting_on_me: u32,
    /// Active tickets with an open attempt, against `slots`.
    pub running: u32,
    /// Pending decisions across the project's tickets, against
    /// `waiting_on_me`.
    pub pending: u32,
    /// The policy's floor for free space on the worktrees' volume, in
    /// GB; nothing new starts under it.
    #[serde(default)]
    pub min_free_gb: u32,
    /// Free space on the worktrees' volume now, in GB.
    #[serde(default)]
    pub free_gb: Option<u32>,
    /// The project's supervisor, when its live pipeline has a
    /// `[supervisor]` table.
    #[serde(default)]
    pub supervisor: Option<SupervisorView>,
}

/// A project's supervisor session as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SupervisorView {
    /// Switchboard's record id of the session now, if there is one.
    pub session: Option<String>,
    /// When the session now was asked for, in Unix ms; 0 without one.
    pub created_ms: u64,
    /// The `[supervisor]` table changed since the session was seeded.
    pub seed_stale: bool,
    /// How many sessions it replaced.
    pub replaced: u32,
    /// A fresh one is asked for and the runner has yet to start it.
    pub fresh_pending: bool,
    /// Why the last fresh, resume or kill failed.
    pub error: Option<String>,
}

impl ProjectView {
    /// Nothing new starts: every slot is taken, or too much waits.
    #[must_use]
    pub fn held(&self) -> Option<String> {
        if self.running >= self.slots {
            Some(format!("all {} slots in use", self.slots))
        } else if let Some(free) = self.free_gb.filter(|free| *free < self.min_free_gb) {
            Some(format!(
                "{free} GB free on the worktrees' volume, the policy wants {}",
                self.min_free_gb
            ))
        } else if self.pending >= self.waiting_on_me {
            Some(format!(
                "{} decision(s) waiting, the limit is {}",
                self.pending, self.waiting_on_me
            ))
        } else {
            None
        }
    }
}

/// Where a ticket stands: the record as a reader needs it, with the
/// pipeline's stage names resolved and every id as a string.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TicketView {
    pub id: String,
    pub project: String,
    /// The source's kind: `github` for an issue, `pull-request` for
    /// someone else's work, `task-file` or `manual`.
    pub kind: String,
    pub number: Option<u64>,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub labels: Vec<String>,
    /// `active`, `parking`, `parked`, `closing` or `closed`.
    pub state: String,
    /// Why, for a parked or closed ticket.
    pub reason: Option<String>,
    /// The pipeline's stages in order.
    pub stages: Vec<String>,
    /// Beside `stages`, each stage's external gate check (`pr-checks`,
    /// `pr-merged`, `review-finalized`), or `None`.
    pub stage_checks: Vec<Option<String>>,
    /// Index into `stages` of the current one; past the end when done.
    pub stage: usize,
    pub tree: Option<PathBuf>,
    /// The tree is removed: the ticket closed. `tree` still says where
    /// it was.
    pub tree_removed: bool,
    /// Why a close left the trees in place, when it did.
    pub trees_kept: Option<String>,
    /// `close` would start a close: parked, or active with nothing
    /// open. A tree with changes is refused only when it runs.
    pub closable: bool,
    /// `close` would try the removal of the kept trees again.
    pub trees_retryable: bool,
    /// The paths a close would remove, in the order it removes them:
    /// each lane with a repository of its own, then the ticket's tree.
    /// Empty for a pipeline that works in place.
    pub removes: Vec<PathBuf>,
    pub lanes: Vec<LaneView>,
    pub attempts: Vec<AttemptView>,
    pub decisions: Vec<DecisionView>,
    /// The Switchboard project the ticket's sessions live under.
    pub root_project: Option<String>,
    /// The session a card shows: the latest attempt's.
    pub current_session: Option<String>,
    /// The resources the ticket holds.
    pub holds: Vec<String>,
    /// What the ticket waits for, and who holds it:
    /// `my-dev, held by baea8dbe (#56)`.
    pub waiting_for: Option<String>,
    /// The lanes served for its stages that are not stopped.
    pub services: Vec<ServiceView>,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// The files a reader opens next, filled for a single ticket's view
    /// and left empty in a status, which reads no directories.
    pub paths: PathsView,
    /// Every restart, oldest first.
    pub restarts: Vec<RestartView>,
}

/// A restart: the ticket put at a stage under a fresh copy of the live
/// pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RestartView {
    /// When it applied.
    pub at_ms: u64,
    /// The stage it stood at.
    pub from: String,
    /// The stage it was put at.
    pub to: String,
    /// The copy earlier attempts ran under.
    pub before: PathBuf,
    /// The copy the restart wrote.
    pub after: PathBuf,
    /// Attempts discarded, as `stage/n`.
    pub discarded: Vec<String>,
    /// Branches moved back, as `key from → to` with short heads.
    pub reset: Vec<String>,
    /// Lanes whose setup runs again.
    pub setup_again: Vec<String>,
}

impl TicketView {
    /// How a lane's last bring-up reads in `show`: `brought_up`'s words,
    /// then what the resolution review of its conflict made of it, once
    /// one began. `None` for a lane never brought up.
    #[must_use]
    pub fn lane_brought_up(&self, l: &LaneView) -> Option<String> {
        let by = l.brought_up_by?;
        let text = brought_up(by, l.brought_up_commits, l.rebase_conflicts);
        let pass = l
            .rebase_conflicts
            .and_then(|_| self.attempts.iter().find(|a| reviews_resolution(a, l)));
        Some(match pass {
            Some(a) => format!("{text}, {}", review_outcome(a)),
            None => text,
        })
    }

    /// What a resolution review read, in the same words: the conflicted
    /// bring-up of its lane.
    #[must_use]
    pub fn resolution_conflict(&self, a: &AttemptView) -> Option<String> {
        let l = self.lanes.iter().find(|l| reviews_resolution(a, l))?;
        Some(rebased_with_conflicts(l.rebase_conflicts?, Some(a)))
    }
}

/// `a` is the resolution review of the lane's last bring-up.
fn reviews_resolution(a: &AttemptView, l: &LaneView) -> bool {
    a.stage == RESOLUTION && a.context == l.name && l.resolution == Some(a.n)
}

/// Where a ticket's documents are, and its pull request.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsView {
    /// The plan as it stands: an open review's copy, else the newest
    /// complete one; `plan_files` has it per lane.
    pub plan: Option<PathBuf>,
    /// The stage whose attempt wrote `plan`; none from an older runner.
    pub plan_stage: Option<String>,
    /// `plan` is a plan review's copy, its review open or finished.
    pub plan_reviewed: bool,
    /// `plan` is a plan review's copy whose review is still open, so the
    /// planner may still be editing it.
    pub plan_reviewing: bool,
    /// The newest round of that review, from its feedback files or a
    /// `revise` answer; none before the first round's feedback.
    pub plan_round: Option<u32>,
    /// The latest review round's findings: a code review round's
    /// aggregated feedback, else the plan review's latest round file.
    pub round_file: Option<PathBuf>,
    /// The latest code review summary.
    pub review_summary: Option<PathBuf>,
    /// The latest notes for a human gate; `notes_files` has them per
    /// lane.
    pub notes: Option<PathBuf>,
    /// The last pull request an attempt bound to.
    pub pr_url: Option<String>,
    /// That pull request's head when the attempt bound it.
    pub pr_head: Option<String>,
    /// The plan review's round files, first to last.
    pub plan_rounds: Vec<PlanRoundView>,
    /// The plan as it stands, one file per lane when the stage that last
    /// wrote it runs per lane, in pipeline lane order, a lane's open
    /// review copy standing in for its own lane; else the one `plan`
    /// names.
    pub plan_files: Vec<LaneFile>,
    /// The notes, the same way.
    pub notes_files: Vec<LaneFile>,
}

/// One lane's copy of a ticket's document, or the ticket's only one.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LaneFile {
    /// The lane it was written in; none for a stage that runs once.
    pub lane: Option<String>,
    /// The stage whose attempt wrote it.
    pub stage: String,
    /// Where it is.
    pub path: PathBuf,
    /// Written by a plan review whose review is still open, so the
    /// planner may still be editing it.
    pub reviewing: bool,
    /// The newest round of the plan review that wrote it, as
    /// `PathsView::plan_round` counts; none for any other writer.
    pub round: Option<u32>,
}

/// One plan review round's files.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanRoundView {
    /// The round's number, from 1.
    pub n: u32,
    /// The reviewer's feedback file.
    pub feedback: PathBuf,
    /// The response, when it exists.
    pub response: Option<PathBuf>,
    /// Who opened the round with an objection to the finished review
    /// (`you` or `supervisor`, Dispatch's `BY_HAND` and
    /// `BY_SUPERVISOR`); none for the reviewer's own round. The ticket
    /// page matches `supervisor` to title the round "owner, via the
    /// supervisor" and reads any other value as the owner.
    pub by: Option<String>,
}

/// A lane served for a stage, as Switchboard's service session.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceView {
    pub lane: String,
    /// Where it answers, once it has a port.
    pub url: Option<String>,
    /// `before`, `starting`, `ready`, `failed: <why>` or `stopped`.
    pub state: String,
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)] // independent facts about the lane
pub struct LaneView {
    pub name: String,
    pub worktree: PathBuf,
    pub branch: String,
    pub chosen: bool,
    pub setup_done: bool,
    /// The lane's worktree is removed: the ticket closed.
    pub removed: bool,
    /// The commit the lane was cut from, or the base it was last
    /// brought up to.
    pub base_sha: Option<String>,
    /// The latest head an attempt in this lane recorded.
    pub head: Option<String>,
    /// The last head a refresh pushed to the lane's branch.
    pub pushed_head: Option<String>,
    /// The lane's last bring-up resolved a rebase that conflicted: how
    /// many commits conflicted, 0 when they could not be listed.
    pub rebase_conflicts: Option<u32>,
    /// The `n` of the resolution review of that bring-up, once one began.
    pub resolution: Option<u32>,
    /// Who brought the lane up last, once it was.
    pub brought_up_by: Option<BroughtUpBy>,
    /// That bring-up rebased commits of the branch's own rather than
    /// moving it.
    pub brought_up_commits: bool,
    /// Dispatch's clone holding the lane's branch; filled in a single
    /// ticket's view only.
    pub clone: Option<PathBuf>,
    /// The lanes whose pull requests merge before this one's, from the
    /// ticket's pipeline.
    pub merge_after: Vec<String>,
    /// The merge also waits for the whole base pipeline on their merge
    /// commit.
    pub merge_after_run: bool,
    /// The merge also waits for this step of that pipeline to pass.
    pub merge_after_step: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AttemptView {
    pub stage: String,
    pub n: u32,
    pub context: String,
    /// `agent`, `workflow`, `review` or `gate-only`.
    pub kind: String,
    /// `starting`, `running`, `complete`, `failed` or `cancelled`.
    pub state: String,
    pub reason: Option<String>,
    /// The Switchboard session, when the attempt has one.
    pub session: Option<String>,
    /// The Switchboard review run, when the attempt has one.
    pub run: Option<String>,
    /// Artifact name and its file.
    pub artifacts: Vec<(String, PathBuf)>,
    /// The head the attempt's result is bound to, once complete.
    pub head: Option<String>,
    /// The stage's checks, once the agent stopped and they started: the
    /// head they ran at and their exit, none while they run.
    pub checks: Option<ChecksView>,
    /// The pull request a `pr-checks` gate is bound to, once looked up.
    pub pr: Option<PullRequestView>,
    /// A code review attempt's rounds, first to last.
    pub rounds: Vec<ReviewRoundView>,
    /// The history rewrite a code review attempt made as it completed.
    pub rewrite: Option<RewriteView>,
    /// When each nudge was typed into the agent's session after it
    /// stopped with its tree not clean, in ms.
    pub nudges: Vec<u64>,
    /// The artifact names that are secret: listed, never read.
    pub secret: Vec<String>,
    /// The secret artifacts whose files were deleted.
    pub forgotten: Vec<String>,
    /// While the open attempt's merge question is held behind another
    /// lane's merge, what it waits for: `backend's merge`. `None` once
    /// released, never held, or the attempt ended.
    pub waits: Option<String>,
    /// When that wait began, in ms.
    pub waits_since_ms: Option<u64>,
    /// The open attempt's sessions whose missing artifact is not yet
    /// counted against them, because their last Stop listed background
    /// work or a wakeup the app still holds them for.
    pub held: Vec<HeldView>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

/// One session's hold, as `show` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HeldView {
    /// The held agent's Switchboard session id.
    pub session: String,
    /// Background task kinds, then `wakeup` or `recurring wakeup`.
    pub pending: Vec<String>,
    /// The earliest one-shot wakeup's fire time, in ms.
    pub wakeup_at_ms: Option<u64>,
    /// When the app stops holding it, in ms.
    pub until_ms: u64,
}

impl AttemptView {
    /// Whether the artifact `name` is secret, so nothing may ask to
    /// read it.
    #[must_use]
    pub fn is_secret(&self, name: &str) -> bool {
        self.secret.iter().any(|s| s == name)
    }

    /// Whether the file at `path` is one of this attempt's secret
    /// artifacts.
    #[must_use]
    pub fn secret_at(&self, path: &std::path::Path) -> bool {
        self.artifacts
            .iter()
            .any(|(name, p)| p == path && self.is_secret(name))
    }
}

/// A code review attempt's rewrite of its branch's commits.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RewriteView {
    /// `fold` or `one`.
    pub mode: String,
    /// The head the checks passed at.
    pub before: String,
    /// The head the rewrite produced, once it did.
    pub after: Option<String>,
    /// Commits ahead of the base before the rewrite.
    pub from: u32,
    /// Commits ahead of the base after it.
    pub to: u32,
    /// Why history was left as it was.
    pub skipped: Option<String>,
    /// The names folded messages carried that neither their commit nor
    /// the tree has.
    pub stale: Vec<String>,
    /// What became of those messages: "being rewritten" while the
    /// rewriter runs, "rewritten, a → b", "rewritten, a → b, but …" when
    /// the reworded message still names something (with "; kept as
    /// rewritten" once accepted), "kept as written", or why a rewrite
    /// failed; absent while it is first asked.
    pub message: Option<String>,
    /// The head carrying the reworded messages, once the branch moved
    /// there.
    pub message_head: Option<String>,
    /// Whether the last rewrite failed or left a message that still names
    /// something; `message` says how.
    pub message_failed: bool,
    /// The session of the agent rewording them, once it started.
    pub message_session: Option<String>,
}

/// One round of a code review: what every reviewer read, what each
/// said, and what became of the findings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewRoundView {
    pub n: u32,
    pub base: String,
    pub head: String,
    /// `reviewing`, `converged`, `findings`, `fixing`, `fixed`,
    /// `accepted` or `failed: <why>`.
    pub state: String,
    pub open_points: u32,
    /// The head after the implementer's commits, once it stopped.
    pub head_after: Option<String>,
    /// Each reviewer's name and state: `starting`, `running`, `clean`,
    /// `findings` or `failed: <why>`.
    pub reviewers: Vec<(String, String)>,
    /// The aggregated findings, once every reviewer finished.
    pub feedback: Option<PathBuf>,
    /// The implementer's response to them, once written.
    pub response: Option<PathBuf>,
    /// When each nudge was typed into the implementer's session, in ms.
    pub nudges: Vec<u64>,
}

/// How an attempt's or a round's nudges read on the page: `None` for
/// none, else `nudged once` or `nudged N times`.
#[must_use]
pub fn nudged(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some("nudged once".into()),
        n => Some(format!("nudged {n} times")),
    }
}

/// The stage name of a resolution review's attempts: the one review
/// pass a conflicted bring-up gets after the pipeline's last code review.
pub const RESOLUTION: &str = "resolution";

/// How many commits a rebase conflicted in, as the page and a
/// resolution reviewer read it: `1 commit`, `N commits`, or `commits it
/// could not list` for 0.
#[must_use]
pub fn commit_count(commits: u32) -> String {
    match commits {
        0 => "commits it could not list".to_owned(),
        1 => "1 commit".to_owned(),
        n => format!("{n} commits"),
    }
}

/// Who rewrote a lane's branch onto its new base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BroughtUpBy {
    /// git alone: the branch moved, or rebased without a conflict.
    Git,
    /// A rebaser agent finished the rebase.
    Rebaser,
    /// A rebase finished by hand, adopted by a `recheck` or `rerun`
    /// answer.
    Hand,
    /// A rebaser failed or was cancelled with no answer since: the record
    /// cannot tell whether it or a hand rebase finished the work.
    Stopped,
}

/// How a bring-up reads in the event log and in `show`. `commits` is
/// whether the branch had commits of its own; `conflicts` is how many of
/// them conflicted (0 when they could not be listed), `None` when no
/// conflict is on record.
#[must_use]
pub fn brought_up(by: BroughtUpBy, commits: bool, conflicts: Option<u32>) -> String {
    if !commits {
        return "brought up with no commits of its own".to_owned();
    }
    let who = match by {
        BroughtUpBy::Git => return "rebased cleanly".to_owned(),
        BroughtUpBy::Rebaser => "rebased by the rebaser",
        BroughtUpBy::Hand => "rebased by hand (adopted)",
        BroughtUpBy::Stopped => "rebased after the rebaser stopped",
    };
    match conflicts {
        Some(n) => format!("{who}, conflicts in {}", commit_count(n)),
        None => who.to_owned(),
    }
}

/// How a bring-up that had conflicts reads on the page:
/// how many commits conflicted, then what its resolution review (the
/// `resolution` attempt `pass`) made of it, once one began.
#[must_use]
pub fn rebased_with_conflicts(commits: u32, pass: Option<&AttemptView>) -> String {
    let text = format!("rebased with conflicts in {}", commit_count(commits));
    match pass {
        Some(a) => format!("{text}, {}", review_outcome(a)),
        None => text,
    }
}

/// What a resolution review (the `resolution` attempt `pass`) made of
/// the conflict it read.
fn review_outcome(a: &AttemptView) -> String {
    let points = |k: u32| {
        if k == 1 {
            "1 point".to_owned()
        } else {
            format!("{k} points")
        }
    };
    let last = a.rounds.last();
    match a.state.as_str() {
        "complete" => match last {
            Some(r) if r.state == "accepted" => {
                format!("accepted with {} open", points(r.open_points))
            }
            Some(r) if r.head_after.is_some() => {
                format!("reviewed: {} fixed", points(r.open_points))
            }
            _ => "reviewed".to_owned(),
        },
        "failed" | "cancelled" => "review failed".to_owned(),
        _ => "under review".to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PullRequestView {
    pub provider: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    /// The head the PR was at when last read.
    pub head: String,
    /// What its checks said then: `pending`, `passed`, `failed: <names>`,
    /// `none`, `merged`, `closed`, or `error: <why>`.
    pub checks: String,
    /// The commit it merged as, once merged and where the provider says.
    pub merge_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChecksView {
    pub head: String,
    pub exit: Option<i32>,
    pub log: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionView {
    pub id: String,
    pub ticket: String,
    pub stage: String,
    /// The decision's name in the pipeline (`lanes`, `finalize`, ...).
    pub name: String,
    pub question: String,
    pub options: Vec<String>,
    /// The suggested answer; for a decision that takes several, the
    /// chosen options joined by commas.
    pub recommendation: Option<String>,
    /// Several options may be chosen at once (the `lanes` decision);
    /// the answer is the chosen ones joined by commas.
    pub multiple: bool,
    /// `pending`, `answered`, `acted` or `cancelled`; `cancelling` for
    /// one still pending on a closing ticket, which waits on no one.
    pub state: String,
    pub answer: Option<String>,
    pub note: Option<String>,
    pub made_ms: u64,
    /// Who answered it, once answered: `user` or Dispatch's own word.
    pub answered_by: Option<String>,
    /// When it was answered.
    pub answered_ms: Option<u64>,
    /// The options that are refused without a note; a client asks for
    /// the note before it sends one of these.
    pub needs_note: Vec<String>,
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

    #[test]
    fn paths_from_an_older_runner_read_with_no_plan_label() {
        let v: PathsView = serde_json::from_str(r#"{"plan": "/plan/1/plan.md"}"#).unwrap();
        assert_eq!(v.plan, Some(PathBuf::from("/plan/1/plan.md")));
        assert_eq!(v.plan_stage, None);
        assert!(!v.plan_reviewed && !v.plan_reviewing);
        assert_eq!(v.plan_round, None);
    }

    #[test]
    fn every_request_and_reply_round_trips_through_a_line() {
        let bodies = [
            Body::Status,
            Body::Ticket { id: "t1".into() },
            Body::Artifact {
                ticket: "t1".into(),
                path: "/d/t1/notes.md".into(),
            },
            Body::Decide {
                ticket: "t1".into(),
                decision: "d1".into(),
                answer: "frontend".into(),
                note: Some("only that".into()),
            },
            Body::Queue {
                project: "Orchard".into(),
                order: vec!["t2".into(), "t1".into()],
            },
            Body::Take {
                project: "Orchard".into(),
                issue: "104".into(),
            },
            Body::Resume {
                ticket: "t1".into(),
            },
            Body::Close {
                ticket: "t1".into(),
                reason: None,
            },
            Body::Close {
                ticket: "t1".into(),
                reason: Some("done elsewhere".into()),
            },
            Body::Worktrees {
                path: Some("/wt2".into()),
                migrate: true,
            },
        ];
        for body in bodies {
            let request = Request::new("op-1", body);
            assert_eq!(Request::parse(request.to_line().trim_end()), Ok(request));
        }
        let ticket = TicketView {
            id: "t1".into(),
            stages: vec!["investigate".into(), "lanes".into()],
            stage: 1,
            decisions: vec![DecisionView {
                id: "d1".into(),
                ticket: "t1".into(),
                state: "pending".into(),
                options: vec!["backend".into()],
                ..DecisionView::default()
            }],
            ..TicketView::default()
        };
        let replies = [
            Reply::Status(Status {
                data_dir: "/d".into(),
                worktrees: "/wt".into(),
                projects: vec![ProjectView {
                    name: "Orchard".into(),
                    queue: vec!["t1".into()],
                    slots: 2,
                    waiting_on_me: 2,
                    running: 1,
                    pending: 1,
                    min_free_gb: 0,
                    free_gb: None,
                    supervisor: None,
                }],
                tickets: vec![ticket.clone()],
            }),
            Reply::Ticket(ticket.clone()),
            Reply::Artifact {
                text: "# notes".into(),
            },
            Reply::Decided(ticket.decisions[0].clone()),
            Reply::Queue {
                order: vec!["t1".into()],
            },
            Reply::Taken(ticket),
            Reply::failed("no"),
        ];
        for reply in replies {
            assert_eq!(Reply::parse(reply.to_line().trim_end()), Ok(reply));
        }
    }

    #[test]
    fn a_ticket_from_a_runner_without_the_close_fields_reads_them_as_unset() {
        let line = r#"{"reply":"ticket","id":"t","state":"closed","tree":"/wt/t","lanes":[{"name":"repo","worktree":"/wt/t"}]}"#;
        let Reply::Ticket(t) = Reply::parse(line).unwrap() else {
            panic!("a ticket")
        };
        assert!(!t.tree_removed && t.trees_kept.is_none() && !t.lanes[0].removed);
        let line = r#"{"op":"1","kind":"close","ticket":"t"}"#;
        assert_eq!(
            Request::parse(line).unwrap().body,
            Body::Close {
                ticket: "t".into(),
                reason: None
            }
        );
    }

    #[test]
    fn the_events_read_round_trips_through_a_line() {
        let request = Request::new(
            "e",
            Body::Events {
                ticket: "t1".into(),
                since: 41,
            },
        );
        assert_eq!(Request::parse(request.to_line().trim_end()), Ok(request));
        let reply = Reply::Events(EventsView {
            events: vec![EventView {
                seq: 42,
                at_ms: 1_000,
                stage: "investigate".into(),
                kind: "attempt-ended".into(),
                text: "investigate/1 complete".into(),
                attempt: Some(("investigate".into(), 1)),
                decision: None,
                head: Some("abc1234".into()),
                url: None,
            }],
            last: 44,
            withdrawn: vec![40],
        });
        assert_eq!(Reply::parse(reply.to_line().trim_end()), Ok(reply));
    }

    #[test]
    fn a_reply_without_the_page_fields_reads_them_as_unset() {
        let line = r#"{"reply":"ticket","id":"t","lanes":[{"name":"repo"}],"decisions":[{"id":"d1","state":"answered"}],"paths":{"plan":"/d/plan.md"}}"#;
        let Reply::Ticket(t) = Reply::parse(line).unwrap() else {
            panic!("a ticket")
        };
        assert_eq!(t.lanes[0].clone, None);
        assert_eq!(
            (
                t.decisions[0].answered_by.as_ref(),
                t.decisions[0].answered_ms
            ),
            (None, None)
        );
        assert_eq!(t.paths.plan_rounds, Vec::new());
        let Reply::Events(v) = Reply::parse(r#"{"reply":"events"}"#).unwrap() else {
            panic!("events")
        };
        assert_eq!(v, EventsView::default());
        let line = r#"{"op":"1","kind":"events","ticket":"t"}"#;
        assert_eq!(
            Request::parse(line).unwrap().body,
            Body::Events {
                ticket: "t".into(),
                since: 0
            }
        );
    }

    #[test]
    fn a_line_with_fields_this_build_does_not_know_still_reads() {
        let line =
            r#"{"reply":"status","data_dir":"/d","tickets":[{"id":"t","extra":1}],"later":true}"#;
        let Reply::Status(status) = Reply::parse(line).unwrap() else {
            panic!("a status")
        };
        assert_eq!(status.tickets[0].id, "t");
    }

    #[test]
    fn an_attempt_without_a_rewrite_reads_as_none() {
        let a: AttemptView =
            serde_json::from_str(r#"{"stage": "review-code", "n": 1, "state": "complete"}"#)
                .unwrap();
        assert_eq!(a.rewrite, None);
        let a = AttemptView {
            rewrite: Some(RewriteView {
                mode: "fold".into(),
                before: "aaaa".into(),
                after: Some("bbbb".into()),
                from: 4,
                to: 2,
                skipped: None,
                stale: vec!["old_name".into()],
                message: Some("kept as written".into()),
                message_head: None,
                message_failed: false,
                message_session: None,
            }),
            ..a
        };
        let back: AttemptView = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn nudges_read_as_once_or_a_count() {
        assert_eq!(nudged(0), None);
        assert_eq!(nudged(1).as_deref(), Some("nudged once"));
        assert_eq!(nudged(3).as_deref(), Some("nudged 3 times"));
    }

    #[test]
    fn a_conflicted_bring_up_reads_with_its_review() {
        let round = |state: &str, open: u32, fixed: bool| ReviewRoundView {
            n: 1,
            state: state.into(),
            open_points: open,
            head_after: fixed.then(|| "fix00001".to_owned()),
            ..ReviewRoundView::default()
        };
        let pass = |state: &str, r: ReviewRoundView| AttemptView {
            stage: RESOLUTION.into(),
            n: 1,
            state: state.into(),
            rounds: vec![r],
            ..AttemptView::default()
        };
        assert_eq!(
            rebased_with_conflicts(1, None),
            "rebased with conflicts in 1 commit"
        );
        assert_eq!(
            rebased_with_conflicts(0, None),
            "rebased with conflicts in commits it could not list"
        );
        let cases = [
            (
                2,
                pass("complete", round("converged", 0, false)),
                "rebased with conflicts in 2 commits, reviewed",
            ),
            (
                1,
                pass("complete", round("fixed", 1, true)),
                "rebased with conflicts in 1 commit, reviewed: 1 point fixed",
            ),
            (
                2,
                pass("complete", round("fixed", 2, true)),
                "rebased with conflicts in 2 commits, reviewed: 2 points fixed",
            ),
            (
                2,
                pass("complete", round("accepted", 1, false)),
                "rebased with conflicts in 2 commits, accepted with 1 point open",
            ),
            (
                2,
                pass("complete", round("accepted", 2, false)),
                "rebased with conflicts in 2 commits, accepted with 2 points open",
            ),
            (
                2,
                pass("running", round("reviewing", 0, false)),
                "rebased with conflicts in 2 commits, under review",
            ),
            (
                2,
                pass("failed", round("failed: gone", 0, false)),
                "rebased with conflicts in 2 commits, review failed",
            ),
        ];
        for (commits, a, want) in cases {
            assert_eq!(rebased_with_conflicts(commits, Some(&a)), want);
        }
    }

    #[test]
    fn a_bring_up_reads_by_who_rewrote_it() {
        use BroughtUpBy::{Git, Hand, Rebaser, Stopped};
        let cases = [
            (
                Rebaser,
                false,
                Some(2),
                "brought up with no commits of its own",
            ),
            (Git, true, Some(0), "rebased cleanly"),
            (
                Rebaser,
                true,
                Some(2),
                "rebased by the rebaser, conflicts in 2 commits",
            ),
            (
                Hand,
                true,
                Some(1),
                "rebased by hand (adopted), conflicts in 1 commit",
            ),
            (
                Stopped,
                true,
                Some(2),
                "rebased after the rebaser stopped, conflicts in 2 commits",
            ),
            (
                Rebaser,
                true,
                Some(0),
                "rebased by the rebaser, conflicts in commits it could not list",
            ),
            (Rebaser, true, None, "rebased by the rebaser"),
            (Hand, true, None, "rebased by hand (adopted)"),
            (Stopped, true, None, "rebased after the rebaser stopped"),
        ];
        for (by, commits, conflicts, want) in cases {
            assert_eq!(brought_up(by, commits, conflicts), want);
        }
        assert_eq!(serde_json::to_string(&Rebaser).unwrap(), r#""rebaser""#);
    }

    #[test]
    fn a_lane_reads_its_bring_up_with_the_review_of_its_conflict() {
        let conflicted = LaneView {
            name: "repo".into(),
            rebase_conflicts: Some(2),
            resolution: Some(1),
            brought_up_by: Some(BroughtUpBy::Rebaser),
            brought_up_commits: true,
            ..LaneView::default()
        };
        let clean = LaneView {
            name: "web".into(),
            brought_up_by: Some(BroughtUpBy::Git),
            brought_up_commits: true,
            ..LaneView::default()
        };
        let view = TicketView {
            attempts: vec![AttemptView {
                stage: RESOLUTION.into(),
                n: 1,
                context: "repo".into(),
                state: "complete".into(),
                rounds: vec![ReviewRoundView {
                    n: 1,
                    state: "fixed".into(),
                    open_points: 1,
                    head_after: Some("fix00001".into()),
                    ..ReviewRoundView::default()
                }],
                ..AttemptView::default()
            }],
            lanes: vec![conflicted.clone(), clean.clone()],
            ..TicketView::default()
        };
        assert_eq!(
            view.lane_brought_up(&conflicted).as_deref(),
            Some("rebased by the rebaser, conflicts in 2 commits, reviewed: 1 point fixed")
        );
        assert_eq!(
            view.lane_brought_up(&clean).as_deref(),
            Some("rebased cleanly")
        );
        assert_eq!(view.lane_brought_up(&LaneView::default()), None);
    }
}
