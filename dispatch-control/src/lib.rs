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
    /// Commits ahead of the base before and after.
    pub from: u32,
    pub to: u32,
    /// Why history was left as it was.
    pub skipped: Option<String>,
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
            }),
            ..a
        };
        let back: AttemptView = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}
