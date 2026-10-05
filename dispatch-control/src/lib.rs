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
    Failed { reason: String },
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
    pub created_ms: u64,
    pub updated_ms: u64,
    /// The files a reader opens next, filled for a single ticket's view
    /// and left empty in a status, which reads no directories.
    pub paths: PathsView,
}

impl TicketView {
    /// A lane's last bring-up, when it resolved a conflict, in
    /// `rebased_with_conflicts`'s words with the review of it.
    #[must_use]
    pub fn lane_conflict(&self, l: &LaneView) -> Option<String> {
        let commits = l.rebase_conflicts?;
        let pass = self.attempts.iter().find(|a| reviews_resolution(a, l));
        Some(rebased_with_conflicts(commits, pass))
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
    /// The latest completed plan.
    pub plan: Option<PathBuf>,
    /// The latest review round's findings: a code review round's
    /// aggregated feedback, else the plan review's latest round file.
    pub round_file: Option<PathBuf>,
    /// The latest code review summary.
    pub review_summary: Option<PathBuf>,
    /// The latest notes for a human gate.
    pub notes: Option<PathBuf>,
    /// The last pull request an attempt bound to.
    pub pr_url: Option<String>,
    /// That pull request's head when the attempt bound it.
    pub pr_head: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
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
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
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

/// How a bring-up that had conflicts reads on the page and in `show`:
/// how many commits conflicted, then what its resolution review (the
/// `resolution` attempt `pass`) made of it, once one began.
#[must_use]
pub fn rebased_with_conflicts(commits: u32, pass: Option<&AttemptView>) -> String {
    let text = format!("rebased with conflicts in {}", commit_count(commits));
    let Some(a) = pass else {
        return text;
    };
    let points = |k: u32| {
        if k == 1 {
            "1 point".to_owned()
        } else {
            format!("{k} points")
        }
    };
    let last = a.rounds.last();
    let outcome = match a.state.as_str() {
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
    };
    format!("{text}, {outcome}")
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
}
