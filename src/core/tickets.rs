//! What the remote port says of Dispatch's tickets: each ticket as a
//! device shows it at a glance, with its standing worded as the Mac's
//! own page words it, the decisions a device may answer, and in the
//! single-ticket view the body, attempts and lanes. Shaped here, in
//! the core, so the port's fingerprint and its replies are one value.

use serde::Serialize;

use super::AppCore;
use crate::ports::dispatch::{AttemptView, DecisionView, LaneView, ProjectView, TicketView};

/// A decision as a device answers it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketDecision {
    pub id: String,
    pub stage: String,
    pub name: String,
    pub question: String,
    pub options: Vec<String>,
    pub recommendation: Option<String>,
    pub multiple: bool,
    pub state: String,
    pub answer: Option<String>,
    pub note: Option<String>,
    pub made_ms: u64,
    pub answered_by: Option<String>,
    pub answered_ms: Option<u64>,
    pub needs_note: Vec<String>,
}

impl From<&DecisionView> for TicketDecision {
    fn from(d: &DecisionView) -> Self {
        Self {
            id: d.id.clone(),
            stage: d.stage.clone(),
            name: d.name.clone(),
            question: d.question.clone(),
            options: d.options.clone(),
            recommendation: d.recommendation.clone(),
            multiple: d.multiple,
            state: d.state.clone(),
            answer: d.answer.clone(),
            note: d.note.clone(),
            made_ms: d.made_ms,
            answered_by: d.answered_by.clone(),
            answered_ms: d.answered_ms,
            needs_note: d.needs_note.clone(),
        }
    }
}

/// A pull request an attempt is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketPullRequest {
    pub number: u64,
    pub url: String,
    pub checks: String,
}

/// One attempt of a stage, with the session it ran in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketAttempt {
    pub stage: String,
    pub n: u32,
    pub context: String,
    pub kind: String,
    pub state: String,
    pub reason: Option<String>,
    pub session: Option<String>,
    pub pr: Option<TicketPullRequest>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
}

impl From<&AttemptView> for TicketAttempt {
    fn from(a: &AttemptView) -> Self {
        Self {
            stage: a.stage.clone(),
            n: a.n,
            context: a.context.clone(),
            kind: a.kind.clone(),
            state: a.state.clone(),
            reason: a.reason.clone(),
            session: a.session.clone(),
            pr: a.pr.as_ref().map(|pr| TicketPullRequest {
                number: pr.number,
                url: pr.url.clone(),
                checks: pr.checks.clone(),
            }),
            started_ms: a.started_ms,
            ended_ms: a.ended_ms,
        }
    }
}

/// A lane by its branch; paths stay on the Mac.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketLane {
    pub name: String,
    pub branch: String,
    pub chosen: bool,
    pub head: Option<String>,
}

impl From<&LaneView> for TicketLane {
    fn from(l: &LaneView) -> Self {
        Self {
            name: l.name.clone(),
            branch: l.branch.clone(),
            chosen: l.chosen,
            head: l.head.clone(),
        }
    }
}

/// A ticket as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketSummary {
    pub id: String,
    pub project: String,
    pub kind: String,
    pub number: Option<u64>,
    pub title: String,
    pub url: Option<String>,
    pub labels: Vec<String>,
    pub state: String,
    pub reason: Option<String>,
    /// The current stage's name; `None` once past the last.
    pub stage: Option<String>,
    pub stages: Vec<String>,
    /// The Mac page's one line on where the ticket stands.
    pub standing: String,
    /// It waits on the owner: a decision, or an agent at a prompt.
    pub waits: bool,
    /// Drawn as needing a look: it waits, or is not simply running.
    pub urgent: bool,
    pub current_session: Option<String>,
    pub root_project: Option<String>,
    pub waiting_for: Option<String>,
    pub closable: bool,
    pub decisions: Vec<TicketDecision>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

/// A ticket read alone: the summary with what the list leaves out.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketDetail {
    #[serde(flatten)]
    pub summary: TicketSummary,
    pub body: String,
    pub attempts: Vec<TicketAttempt>,
    pub lanes: Vec<TicketLane>,
    pub holds: Vec<String>,
}

/// A project with a pipeline, and its supervisor's session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketProject {
    pub name: String,
    pub slots: u32,
    pub running: u32,
    pub pending: u32,
    pub waiting_on_me: u32,
    pub queue: Vec<String>,
    pub supervisor_session: Option<String>,
}

/// Every ticket in the last status, in an order that does not follow
/// the runner's (which lists by last write): by project, then oldest
/// first, so a list a device is about to tap in does not move.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketListing {
    /// A runner answered the last poll; without one the tickets are
    /// the last seen, if any.
    pub connected: bool,
    pub projects: Vec<TicketProject>,
    pub tickets: Vec<TicketSummary>,
}

impl AppCore {
    /// The tickets as the port lists them.
    #[must_use]
    pub fn ticket_listing(&self) -> TicketListing {
        let status = &self.dispatch.status;
        let mut tickets: Vec<TicketSummary> = status
            .tickets
            .iter()
            .map(|t| self.ticket_summary(t))
            .collect();
        tickets.sort_by(|a, b| {
            (&a.project, a.created_ms, &a.id).cmp(&(&b.project, b.created_ms, &b.id))
        });
        let projects = status.projects.iter().map(ticket_project).collect();
        TicketListing {
            connected: self.dispatch.connected,
            projects,
            tickets,
        }
    }

    /// One ticket as the list shows it.
    #[must_use]
    pub fn ticket_summary(&self, t: &TicketView) -> TicketSummary {
        TicketSummary {
            id: t.id.clone(),
            project: t.project.clone(),
            kind: t.kind.clone(),
            number: t.number,
            title: t.title.clone(),
            url: t.url.clone(),
            labels: t.labels.clone(),
            state: t.state.clone(),
            reason: t.reason.clone(),
            stage: t.stages.get(t.stage).cloned(),
            stages: t.stages.clone(),
            standing: self.ticket_standing(t),
            waits: self.ticket_waits(t),
            urgent: self.ticket_urgent(t),
            current_session: t.current_session.clone(),
            root_project: t.root_project.clone(),
            waiting_for: t.waiting_for.clone(),
            closable: t.closable,
            decisions: t.decisions.iter().map(TicketDecision::from).collect(),
            created_ms: t.created_ms,
            updated_ms: t.updated_ms,
        }
    }

    /// One ticket read alone, from the last status (a single-ticket
    /// read through the port adds only paths, which stay on the Mac).
    #[must_use]
    pub fn ticket_detail(&self, id: &str) -> Option<TicketDetail> {
        let t = self.ticket(id)?;
        Some(TicketDetail {
            summary: self.ticket_summary(t),
            body: t.body.clone(),
            attempts: t.attempts.iter().map(TicketAttempt::from).collect(),
            lanes: t.lanes.iter().map(TicketLane::from).collect(),
            holds: t.holds.clone(),
        })
    }
}

fn ticket_project(p: &ProjectView) -> TicketProject {
    TicketProject {
        name: p.name.clone(),
        slots: p.slots,
        running: p.running,
        pending: p.pending,
        waiting_on_me: p.waiting_on_me,
        queue: p.queue.clone(),
        supervisor_session: p.supervisor.as_ref().and_then(|s| s.session.clone()),
    }
}
