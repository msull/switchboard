//! What the remote port says of Dispatch's tickets: each ticket as a
//! device shows it at a glance, with its standing worded as the Mac's
//! own page words it, the decisions a device may answer, and in the
//! single-ticket view the body, attempts and lanes, each attempt with
//! the files it wrote or kept, named by a key that stands for the path.
//! Shaped here, in the core, so the port's fingerprint and its replies
//! are one value.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::AppCore;
use crate::ports::dispatch::{
    AttemptView, DecisionView, EventView, LaneView, ProjectView, TicketView,
};

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

/// A file of an attempt: an artifact it wrote (its notes, a plan, a
/// review summary), a review round's findings or response, the log of
/// the stage's checks, or a file of its evidence directory. A device
/// reads one by its `key`, which stands for the path; the path stays
/// on the Mac.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketFile {
    pub key: String,
    pub stage: String,
    pub n: u32,
    /// The artifact's name, `checks`, `round N findings` or `round N
    /// response`, or an evidence file's path under its directory.
    pub name: String,
    /// `artifact`, `checks`, `findings`, `response`, `evidence`, or
    /// `secret` for an artifact Dispatch never reads.
    pub kind: String,
    /// Known for evidence, which the runner recorded when the writer
    /// finished.
    pub bytes: Option<u64>,
    pub modified_ms: Option<u64>,
    /// Text the port serves.
    pub inline: bool,
    /// An image the port serves.
    pub image: bool,
}

/// The stage's checks of an attempt, once they ran.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketChecks {
    pub head: String,
    /// None while they run.
    pub exit: Option<i32>,
}

/// One round of a code review attempt.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketRound {
    pub n: u32,
    pub state: String,
    pub open_points: u32,
}

/// One attempt of a stage, with the session it ran in and its files.
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
    pub checks: Option<TicketChecks>,
    pub rounds: Vec<TicketRound>,
    pub files: Vec<TicketFile>,
    /// When the evidence directory was removed, in ms.
    pub evidence_swept_ms: Option<u64>,
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
            checks: a.checks.as_ref().map(|c| TicketChecks {
                head: c.head.clone(),
                exit: c.exit,
            }),
            rounds: a
                .rounds
                .iter()
                .map(|r| TicketRound {
                    n: r.n,
                    state: r.state.clone(),
                    open_points: r.open_points,
                })
                .collect(),
            files: attempt_files(a).into_iter().map(|(f, _)| f).collect(),
            evidence_swept_ms: a.evidence_swept_ms,
            started_ms: a.started_ms,
            ended_ms: a.ended_ms,
        }
    }
}

/// Evidence a device reads as text: what an agent's probes and the
/// tester's notes are written as.
const EVIDENCE_TEXT: [&str; 6] = ["md", "txt", "log", "json", "csv", "tsv"];
/// Evidence a device shows as a picture: the tester's screenshots.
const EVIDENCE_IMAGES: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

fn extension_in(path: &Path, list: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| list.contains(&e.to_ascii_lowercase().as_str()))
}

/// The media type of an evidence image, by its extension.
#[must_use]
pub fn image_media(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    })
}

/// The key a device names a file by: a hash of its path, so the path
/// itself stays on the Mac. Stable for the life of the process, which
/// is as long as a device holds a listing from it.
#[must_use]
pub fn file_key(path: &Path) -> String {
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Every file of an attempt with its path, in the order the Mac's page
/// lists them: artifacts, the checks log, each round's files, then the
/// evidence.
fn attempt_files(a: &AttemptView) -> Vec<(TicketFile, PathBuf)> {
    let file = |name: String, kind: &str, path: &PathBuf, inline: bool| TicketFile {
        key: file_key(path),
        stage: a.stage.clone(),
        n: a.n,
        name,
        kind: kind.to_owned(),
        bytes: None,
        modified_ms: None,
        inline,
        image: false,
    };
    let mut files = Vec::new();
    for (name, path) in &a.artifacts {
        let secret = a.is_secret(name);
        let kind = if secret { "secret" } else { "artifact" };
        files.push((file(name.clone(), kind, path, !secret), path.clone()));
    }
    if let Some(c) = &a.checks {
        files.push((file("checks".into(), "checks", &c.log, true), c.log.clone()));
    }
    for r in &a.rounds {
        if let Some(p) = &r.feedback {
            let name = format!("round {} findings", r.n);
            files.push((file(name, "findings", p, true), p.clone()));
        }
        if let Some(p) = &r.response {
            let name = format!("round {} response", r.n);
            files.push((file(name, "response", p, true), p.clone()));
        }
    }
    for e in &a.evidence {
        let inline = extension_in(&e.path, &EVIDENCE_TEXT);
        let mut f = file(e.rel.clone(), "evidence", &e.path, inline);
        f.bytes = Some(e.bytes);
        f.modified_ms = Some(e.modified_ms);
        f.image = extension_in(&e.path, &EVIDENCE_IMAGES);
        files.push((f, e.path.clone()));
    }
    files
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

/// One line of the runner's event log about a ticket.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketEvent {
    pub seq: u64,
    pub at_ms: u64,
    pub stage: String,
    /// The log's word: `stage`, `attempt-ended`, `answered`, ...
    pub kind: String,
    pub text: String,
    /// The attempt it is about, as `stage/n`.
    pub attempt: Option<String>,
    pub decision: Option<String>,
    pub head: Option<String>,
    pub url: Option<String>,
}

impl From<&EventView> for TicketEvent {
    fn from(e: &EventView) -> Self {
        Self {
            seq: e.seq,
            at_ms: e.at_ms,
            stage: e.stage.clone(),
            kind: e.kind.clone(),
            text: e.text.clone(),
            attempt: e.attempt.as_ref().map(|(s, n)| format!("{s}/{n}")),
            decision: e.decision.clone(),
            head: e.head.clone(),
            url: e.url.clone(),
        }
    }
}

/// A ticket's events as the Mac holds them, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TicketEvents {
    pub events: Vec<TicketEvent>,
    /// The cursor the next read continues from.
    pub last: u64,
    /// The runner does not serve events (an older build).
    pub unavailable: bool,
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

    /// The file a device named by its key on ticket `id`, with the path
    /// the Mac reads it at.
    #[must_use]
    pub fn ticket_file(&self, id: &str, key: &str) -> Option<(TicketFile, PathBuf)> {
        self.ticket(id)?
            .attempts
            .iter()
            .flat_map(attempt_files)
            .find(|(f, _)| f.key == key)
    }

    /// A ticket's events as read so far; none before the first read.
    #[must_use]
    pub fn ticket_events(&self, id: &str) -> TicketEvents {
        let Some(e) = self.dispatch.events.get(id) else {
            return TicketEvents {
                events: Vec::new(),
                last: 0,
                unavailable: false,
            };
        };
        TicketEvents {
            events: e.events.iter().map(TicketEvent::from).collect(),
            last: e.last,
            unavailable: e.unavailable,
        }
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
