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
        serde_json::from_str(line).map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
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
    Failed { reason: String },
}

impl Reply {
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed {
            reason: reason.into(),
        }
    }

    pub fn parse(line: &str) -> Result<Self, String> {
        serde_json::from_str(line).map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }
}

/// The runner and everything it drives.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    /// Dispatch's data directory, where tickets and their files live.
    pub data_dir: PathBuf,
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
    /// The policy's limits: tickets with something running at once,
    /// and decisions that may wait on the user before nothing new
    /// starts.
    pub slots: u32,
    pub waiting_on_me: u32,
    /// Where the project stands against them: active tickets with an
    /// open attempt, and pending decisions across its tickets.
    pub running: u32,
    pub pending: u32,
}

impl ProjectView {
    /// Nothing new starts: every slot is taken, or too much waits.
    #[must_use]
    pub fn held(&self) -> Option<String> {
        if self.running >= self.slots {
            Some(format!("all {} slots in use", self.slots))
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
    pub number: Option<u64>,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub labels: Vec<String>,
    /// `active`, `parking`, `parked` or `closed`.
    pub state: String,
    /// Why, for a parked or closed ticket.
    pub reason: Option<String>,
    /// The pipeline's stages in order.
    pub stages: Vec<String>,
    /// Index into `stages` of the current one; past the end when done.
    pub stage: usize,
    pub tree: Option<PathBuf>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AttemptView {
    pub stage: String,
    pub n: u32,
    pub context: String,
    /// `agent`, `workflow` or `gate-only`.
    pub kind: String,
    /// `starting`, `running`, `complete`, `failed` or `cancelled`.
    pub state: String,
    pub reason: Option<String>,
    /// The Switchboard session and review run, when the attempt has one.
    pub session: Option<String>,
    pub run: Option<String>,
    /// Artifact name and its file.
    pub artifacts: Vec<(String, PathBuf)>,
    /// The head the attempt's result is bound to, once complete.
    pub head: Option<String>,
    /// The stage's checks, once the agent stopped and they started: the
    /// head they ran at and their exit, none while they run.
    pub checks: Option<ChecksView>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
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
    /// `pending`, `answered` or `acted`.
    pub state: String,
    pub answer: Option<String>,
    pub note: Option<String>,
    pub made_ms: u64,
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
                project: "Delta".into(),
                order: vec!["t2".into(), "t1".into()],
            },
            Body::Take {
                project: "Delta".into(),
                issue: "104".into(),
            },
            Body::Resume {
                ticket: "t1".into(),
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
                projects: vec![ProjectView {
                    name: "Delta".into(),
                    queue: vec!["t1".into()],
                    slots: 2,
                    waiting_on_me: 2,
                    running: 1,
                    pending: 1,
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
    fn a_line_with_fields_this_build_does_not_know_still_reads() {
        let line =
            r#"{"reply":"status","data_dir":"/d","tickets":[{"id":"t","extra":1}],"later":true}"#;
        let Reply::Status(status) = Reply::parse(line).unwrap() else {
            panic!("a status")
        };
        assert_eq!(status.tickets[0].id, "t");
    }
}
