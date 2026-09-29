//! The control port's side of the core: what another process may ask
//! for, run quietly (no view changes, no terminal windows), with the
//! operation id stamped on every record made so the asker can find them
//! again. The wire format lives in the `switchboard-control` crate; the
//! app translates it to `ControlAction` and back.

use std::path::PathBuf;
use std::time::SystemTime;

use switchboard_control as wire;

use super::action::{AppAction, Out};
use super::{
    AppCore, Clock, Effect, GridRect, Launch, PinTarget, PinnedItem, ProjectId, RecordId,
    SessionKind, SetId, Space, SpaceId, WorkflowDefinition, WorkflowId, WorkingSet, grid,
};
use crate::ports::host::Liveness;

/// One command from the control port. Payloads are the core's own ids;
/// the app parses the wire's strings into them.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlAction {
    AddProject {
        space: SpaceId,
        name: String,
        root: PathBuf,
    },
    RemoveProject(ProjectId),
    NewSession {
        project: ProjectId,
        name: String,
        kind: SessionKind,
        cwd: PathBuf,
        launch: Launch,
        /// An agent's first prompt, on its command line.
        prompt: Option<String>,
        notes: String,
    },
    SendInput {
        id: RecordId,
        text: String,
    },
    Kill(RecordId),
    Remove(RecordId),
    SetNotes {
        id: RecordId,
        text: String,
    },
    /// Why the session waits on the user, or `None` to clear it.
    SetWaiting {
        id: RecordId,
        reason: Option<String>,
    },
    MoveSession {
        id: RecordId,
        project: ProjectId,
    },
    RenameProject {
        id: ProjectId,
        name: String,
    },
    NewSpace {
        name: String,
    },
    NewSet {
        space: SpaceId,
        name: String,
    },
    /// The set's cards become exactly this list, or nothing changes.
    SyncSet {
        set: SetId,
        items: Vec<PinnedItem>,
    },
    /// Add or replace the definition of that name.
    InstallDefinition(WorkflowDefinition),
    StartWorkflow {
        source: RecordId,
        plan: PathBuf,
        definition: String,
        reviewer_cwd: Option<PathBuf>,
        reviewer_args: Vec<String>,
    },
    PauseWorkflow(WorkflowId),
    ContinueWorkflow(WorkflowId),
    FinalizeWorkflow(WorkflowId),
    RemoveWorkflow(WorkflowId),
}

/// What one control command did: the records it made and the error it
/// hit, if any. Taken by the app to write the reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlOutcome {
    pub op: String,
    pub made: Vec<wire::Made>,
    pub error: Option<String>,
}

fn made(kind: wire::RecordKind, id: uuid::Uuid) -> wire::Made {
    wire::Made {
        kind,
        id: id.to_string(),
    }
}

impl AppCore {
    /// Apply one control command. Notices it would have posted become
    /// the outcome's error instead: the window stays quiet and the
    /// asker gets the reason.
    pub(super) fn control(&mut self, op: String, action: ControlAction, now: Clock, out: &mut Out) {
        self.quiet_op = Some(op.clone());
        let notices_before = self.notices.len();
        let creation = matches!(
            action,
            ControlAction::AddProject { .. }
                | ControlAction::NewSession { .. }
                | ControlAction::NewSpace { .. }
                | ControlAction::NewSet { .. }
                | ControlAction::StartWorkflow { .. }
        );
        let kind = control_kind(&action);
        let made = self.apply_control(action, now, out);
        self.quiet_op = None;
        let error = self
            .notices
            .drain(notices_before..)
            .filter(|n| n.is_error)
            .map(|n| n.text)
            .reduce(|a, b| format!("{a}; {b}"));
        if creation && error.is_none() && !made.is_empty() {
            out.push(Effect::LogOperation {
                op: op.clone(),
                kind,
                ids: made.iter().map(|m| m.id.clone()).collect(),
            });
        }
        self.control_outcomes
            .push(ControlOutcome { op, made, error });
    }

    /// The outcome of `op`, once; `None` if no such command ran.
    pub fn take_control_outcome(&mut self, op: &str) -> Option<ControlOutcome> {
        let pos = self.control_outcomes.iter().position(|o| o.op == op)?;
        Some(self.control_outcomes.remove(pos))
    }

    #[allow(clippy::too_many_lines)] // one arm per command
    fn apply_control(
        &mut self,
        action: ControlAction,
        now: Clock,
        out: &mut Out,
    ) -> Vec<wire::Made> {
        use wire::RecordKind as K;
        match action {
            ControlAction::AddProject { space, name, root } => {
                if self.space(space).is_none() {
                    self.error("no such space");
                    return Vec::new();
                }
                let id = self.add_project(name, root, space, now, out);
                vec![made(K::Project, id.0)]
            }
            ControlAction::RemoveProject(id) => {
                self.remove_project(id, now, out);
                Vec::new()
            }
            ControlAction::NewSession {
                project,
                name,
                kind,
                cwd,
                launch,
                prompt,
                notes,
            } => {
                if let Some(reason) = self.host_error.clone() {
                    self.error(format!("cannot start {name}: {reason}"));
                    return Vec::new();
                }
                let Some(id) = self.add_record(project, name, kind, cwd, launch, now, out) else {
                    return Vec::new();
                };
                self.edit_session(id, out, |s| s.notes = notes);
                if let Some(prompt) = prompt.filter(|_| matches!(kind, SessionKind::Agent(_))) {
                    self.first_prompts.push((id, prompt));
                }
                self.launch_fresh(id, now, out);
                vec![made(K::Session, id.0)]
            }
            ControlAction::SendInput { id, text } => {
                self.session_action(AppAction::SendInput { id, text }, now, out);
                Vec::new()
            }
            ControlAction::Kill(id) => {
                self.session_action(AppAction::KillSession(id), now, out);
                Vec::new()
            }
            ControlAction::Remove(id) => {
                self.session_action(AppAction::RemoveSession(id), now, out);
                Vec::new()
            }
            ControlAction::SetNotes { id, text } => {
                self.session_action(AppAction::SetSessionNotes(id, text), now, out);
                Vec::new()
            }
            ControlAction::SetWaiting { id, reason } => {
                if self.session(id).is_none() {
                    self.error("no such session");
                }
                self.edit_session(id, out, |s| s.waiting_on = reason);
                Vec::new()
            }
            ControlAction::MoveSession { id, project } => {
                self.move_session(id, project, out);
                Vec::new()
            }
            ControlAction::RenameProject { id, name } => {
                if self.workspace(id).is_none() {
                    self.error("no such project");
                }
                self.edit_project(id, out, |p| p.name = name);
                Vec::new()
            }
            ControlAction::NewSpace { name } => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    self.error("a space needs a name");
                    return Vec::new();
                }
                let space = Space {
                    id: SpaceId::new(),
                    name,
                    op: self.quiet_op.clone(),
                };
                let id = space.id;
                self.update_views(out, |v| v.spaces.push(space));
                vec![made(K::Space, id.0)]
            }
            ControlAction::NewSet { space, name } => {
                if self.space(space).is_none() {
                    self.error("no such space");
                    return Vec::new();
                }
                let set = WorkingSet {
                    id: SetId::new(),
                    name,
                    items: Vec::new(),
                    space,
                    op: self.quiet_op.clone(),
                };
                let id = set.id;
                self.update_views(out, |v| v.sets.push(set));
                vec![made(K::Set, id.0)]
            }
            ControlAction::SyncSet { set, items } => {
                self.sync_set(set, items, out);
                Vec::new()
            }
            ControlAction::InstallDefinition(def) => {
                if def.name.trim().is_empty() {
                    self.error("a definition needs a name");
                    return Vec::new();
                }
                self.update_settings(out, |s| {
                    s.workflows.retain(|d| d.name != def.name);
                    s.workflows.push(def);
                });
                Vec::new()
            }
            ControlAction::StartWorkflow {
                source,
                plan,
                definition,
                reviewer_cwd,
                reviewer_args,
            } => {
                let before: Vec<WorkflowId> = self.workflows().map(|r| r.id).collect();
                self.start_workflow(
                    source,
                    &plan,
                    &definition,
                    reviewer_cwd,
                    reviewer_args,
                    now,
                    out,
                );
                let Some(run) = self.workflows().find(|r| !before.contains(&r.id)).cloned() else {
                    return Vec::new();
                };
                vec![made(K::Run, run.id.0), made(K::Session, run.reviewer.0)]
            }
            ControlAction::PauseWorkflow(id) => {
                self.workflow_action(AppAction::PauseWorkflow(id), now, out);
                Vec::new()
            }
            ControlAction::ContinueWorkflow(id) => {
                self.workflow_action(AppAction::ContinueWorkflow(id), now, out);
                Vec::new()
            }
            ControlAction::FinalizeWorkflow(id) => {
                self.workflow_action(AppAction::FinalizeWorkflow(id), now, out);
                Vec::new()
            }
            ControlAction::RemoveWorkflow(id) => {
                self.workflow_action(AppAction::RemoveWorkflow(id), now, out);
                Vec::new()
            }
        }
    }

    /// Replace a set's cards wholesale. Every target must be in the set's
    /// space and no two may overlap; otherwise nothing changes and the
    /// reason is an error.
    fn sync_set(&mut self, id: SetId, items: Vec<PinnedItem>, out: &mut Out) {
        let Some(set) = self.working_set(id) else {
            self.error("no such working set");
            return;
        };
        let space = set.space;
        let items: Vec<PinnedItem> = items
            .into_iter()
            .map(|i| PinnedItem {
                target: i.target,
                rect: grid::clamp(i.rect),
            })
            .collect();
        for (n, item) in items.iter().enumerate() {
            if !self.target_in(&item.target, space) {
                self.error(format!("card {n} is not in the set's space"));
                return;
            }
            if items[..n].iter().any(|other| other.target == item.target) {
                self.error(format!("card {n} appears twice"));
                return;
            }
            if items[..n]
                .iter()
                .any(|other| other.rect.overlaps(item.rect))
            {
                self.error(format!("card {n} overlaps an earlier card"));
                return;
            }
        }
        self.update_views(out, |v| {
            if let Some(s) = v.sets.iter_mut().find(|s| s.id == id) {
                s.items = items;
            }
        });
    }

    // --- read models, in the wire's shapes

    /// One session as the port reports it; `None` for an unknown id.
    #[must_use]
    pub fn session_view(&self, id: RecordId, now: SystemTime) -> Option<wire::SessionView> {
        let s = self.session(id)?;
        let status = self.host_status(id);
        let liveness = match status.map(|h| &h.liveness) {
            Some(Liveness::Running { .. }) => wire::Liveness::Running,
            Some(Liveness::Exited { code }) => wire::Liveness::Exited { code: *code },
            Some(Liveness::Missing) | None => wire::Liveness::Missing,
        };
        let quiet_secs = match liveness {
            wire::Liveness::Running => status
                .and_then(|h| h.last_activity)
                .and_then(|t| now.duration_since(t).ok())
                .map(|d| d.as_secs()),
            _ => None,
        };
        let card = self.card_state(id);
        Some(wire::SessionView {
            id: id.0.to_string(),
            project: s.project.0.to_string(),
            name: s.name.clone(),
            kind: match s.kind {
                SessionKind::Agent(super::AgentKind::ClaudeCode) => wire::SessionKind::Claude,
                SessionKind::Agent(super::AgentKind::Codex) => wire::SessionKind::Codex,
                SessionKind::Shell => wire::SessionKind::Shell,
                SessionKind::Command => wire::SessionKind::Command,
                SessionKind::Service => wire::SessionKind::Service,
            },
            cwd: s.cwd.clone(),
            notes: s.notes.clone(),
            liveness,
            card: card.label(),
            last_exit: s.last_exit,
            last_stop_at_ms: s.last_stop_at.and_then(epoch_ms),
            quiet_secs,
            waiting: card == super::CardState::WaitingOnYou,
            waiting_reason: s.waiting_on.clone().or_else(|| s.activity_reason.clone()),
            resume_id: s.resume.as_ref().map(super::ResumeHandle::provider_id),
            op: s.op.clone(),
        })
    }

    #[must_use]
    pub fn project_views(&self, space: Option<SpaceId>) -> Vec<wire::ProjectView> {
        self.workspaces
            .iter()
            .map(|w| &w.project)
            .filter(|p| space.is_none_or(|s| p.space == s))
            .map(|p| wire::ProjectView {
                id: p.id.0.to_string(),
                name: p.name.clone(),
                root: p.root.clone(),
                space: p.space.0.to_string(),
                op: p.op.clone(),
            })
            .collect()
    }

    #[must_use]
    pub fn space_views(&self) -> Vec<wire::SpaceView> {
        self.views
            .spaces
            .iter()
            .map(|s| wire::SpaceView {
                id: s.id.0.to_string(),
                name: s.name.clone(),
                op: s.op.clone(),
            })
            .collect()
    }

    #[must_use]
    pub fn set_views(&self, space: SpaceId) -> Vec<wire::SetView> {
        self.views
            .sets
            .iter()
            .filter(|s| s.space == space)
            .map(|s| wire::SetView {
                id: s.id.0.to_string(),
                name: s.name.clone(),
                space: s.space.0.to_string(),
                items: s.items.iter().map(pin_view).collect(),
                op: s.op.clone(),
            })
            .collect()
    }

    #[must_use]
    pub fn run_view(&self, id: WorkflowId) -> Option<wire::RunView> {
        let r = self.workflow(id)?;
        Some(wire::RunView {
            id: r.id.0.to_string(),
            project: r.project.0.to_string(),
            definition: r.definition.clone(),
            state: match &r.state {
                super::RunState::Starting => wire::RunState::Starting,
                super::RunState::AwaitingFeedback => wire::RunState::AwaitingFeedback,
                super::RunState::AwaitingResponse => wire::RunState::AwaitingResponse,
                super::RunState::Converged => wire::RunState::Converged,
                super::RunState::AtCap => wire::RunState::AtCap,
                super::RunState::Paused(reason) => wire::RunState::Paused {
                    reason: reason.clone(),
                },
                super::RunState::Finalized => wire::RunState::Finalized,
                super::RunState::HandedOff => wire::RunState::HandedOff,
            },
            round: r.rounds.last().map_or(0, |round| round.n),
            cap: r.cap,
            source: r.source.0.to_string(),
            plan: r.plan.clone(),
            reviewer: r.reviewer.0.to_string(),
            planner: r.planner.map(|p| p.0.to_string()),
            op: r.op.clone(),
        })
    }

    /// Every session whose card reads as waiting on the user.
    #[must_use]
    pub fn waiting_views(&self, now: SystemTime) -> Vec<wire::SessionView> {
        self.workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|s| self.card_state(s.id) == super::CardState::WaitingOnYou)
            .filter_map(|s| self.session_view(s.id, now))
            .collect()
    }

    /// Every record still present that `op` made, with its state.
    #[must_use]
    pub fn records_with_op(&self, op: &str, now: SystemTime) -> Vec<wire::Found> {
        let has = |o: &Option<String>| o.as_deref() == Some(op);
        let mut found = Vec::new();
        for s in &self.views.spaces {
            if has(&s.op) {
                found.push(found_record(wire::RecordKind::Space, s.id.0));
            }
        }
        for s in &self.views.sets {
            if has(&s.op) {
                found.push(found_record(wire::RecordKind::Set, s.id.0));
            }
        }
        for w in &self.workspaces {
            if has(&w.project.op) {
                found.push(found_record(wire::RecordKind::Project, w.project.id.0));
            }
            for s in &w.sessions {
                if has(&s.op) {
                    let mut f = found_record(wire::RecordKind::Session, s.id.0);
                    f.session = self.session_view(s.id, now);
                    found.push(f);
                }
            }
            for r in &w.workflows {
                if has(&r.op) {
                    let mut f = found_record(wire::RecordKind::Run, r.id.0);
                    f.run = self.run_view(r.id);
                    found.push(f);
                }
            }
        }
        found
    }

    /// Records whose launch was written down but never reported back: a
    /// restart happened between the two.
    #[must_use]
    pub fn interrupted_ops(&self) -> Vec<String> {
        self.workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|s| s.pending_launch && !self.in_flight.iter().any(|f| f.id == s.id))
            .filter_map(|s| s.op.clone())
            .collect()
    }
}

fn found_record(kind: wire::RecordKind, id: uuid::Uuid) -> wire::Found {
    wire::Found {
        kind,
        id: id.to_string(),
        removed: false,
        session: None,
        run: None,
    }
}

fn pin_view(item: &PinnedItem) -> wire::Pin {
    wire::Pin {
        target: match &item.target {
            PinTarget::Session(id) => wire::PinTarget::Session {
                session: id.0.to_string(),
            },
            PinTarget::File(project, path) => wire::PinTarget::File {
                project: project.0.to_string(),
                path: path.clone(),
            },
        },
        rect: rect_view(item.rect),
    }
}

fn rect_view(r: GridRect) -> wire::Rect {
    wire::Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

fn epoch_ms(t: SystemTime) -> Option<u64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The command's name for the operations log.
fn control_kind(action: &ControlAction) -> String {
    match action {
        ControlAction::AddProject { .. } => "project.add",
        ControlAction::RemoveProject(_) => "project.remove",
        ControlAction::NewSession { .. } => "session.new",
        ControlAction::SendInput { .. } => "session.send",
        ControlAction::Kill(_) => "session.kill",
        ControlAction::Remove(_) => "session.remove",
        ControlAction::SetNotes { .. } => "session.notes",
        ControlAction::SetWaiting { .. } => "session.waiting",
        ControlAction::MoveSession { .. } => "session.move",
        ControlAction::RenameProject { .. } => "project.rename",
        ControlAction::NewSpace { .. } => "space.new",
        ControlAction::NewSet { .. } => "set.new",
        ControlAction::SyncSet { .. } => "set.sync",
        ControlAction::InstallDefinition(_) => "workflow.definitions.install",
        ControlAction::StartWorkflow { .. } => "workflow.start",
        ControlAction::PauseWorkflow(_) => "workflow.pause",
        ControlAction::ContinueWorkflow(_) => "workflow.continue",
        ControlAction::FinalizeWorkflow(_) => "workflow.finalize",
        ControlAction::RemoveWorkflow(_) => "workflow.remove",
    }
    .to_owned()
}

// --- from the wire

fn parse_id(kind: &str, text: &str) -> Result<uuid::Uuid, String> {
    uuid::Uuid::parse_str(text).map_err(|_| format!("{kind} id {text:?} is not a uuid"))
}

fn session_kind(kind: wire::SessionKind) -> SessionKind {
    match kind {
        wire::SessionKind::Claude => SessionKind::Agent(super::AgentKind::ClaudeCode),
        wire::SessionKind::Codex => SessionKind::Agent(super::AgentKind::Codex),
        wire::SessionKind::Shell => SessionKind::Shell,
        wire::SessionKind::Command => SessionKind::Command,
        wire::SessionKind::Service => SessionKind::Service,
    }
}

fn launch(launch: wire::Launch) -> Launch {
    match launch {
        wire::Launch::Shell => Launch::Shell,
        wire::Launch::Argv(argv) => Launch::Argv(argv),
        wire::Launch::Command { command, shell } => Launch::Command { command, shell },
    }
}

fn pin(pin: wire::Pin) -> Result<PinnedItem, String> {
    let target = match pin.target {
        wire::PinTarget::Session { session } => {
            PinTarget::Session(RecordId(parse_id("session", &session)?))
        }
        wire::PinTarget::File { project, path } => {
            PinTarget::File(ProjectId(parse_id("project", &project)?), path)
        }
    };
    Ok(PinnedItem {
        target,
        rect: GridRect {
            x: pin.rect.x,
            y: pin.rect.y,
            w: pin.rect.w,
            h: pin.rect.h,
        },
    })
}

fn definition(d: wire::Definition) -> WorkflowDefinition {
    WorkflowDefinition {
        name: d.name,
        reviewer: match d.reviewer {
            wire::AgentKind::Claude => super::AgentKind::ClaudeCode,
            wire::AgentKind::Codex => super::AgentKind::Codex,
        },
        review_first: d.review_first,
        review_round: d.review_round,
        respond: d.respond,
        respond_to_user: d.respond_to_user,
        handoff: d.handoff,
        no_feedback: d.no_feedback,
        cap: d.cap,
    }
}

impl TryFrom<wire::Body> for ControlAction {
    type Error = String;

    /// A command's payload with its ids parsed. A query is an error: it
    /// is answered from the read models, never dispatched.
    fn try_from(body: wire::Body) -> Result<Self, String> {
        let session = |s: &str| parse_id("session", s).map(RecordId);
        let project = |s: &str| parse_id("project", s).map(ProjectId);
        let space = |s: &str| parse_id("space", s).map(SpaceId);
        let run = |s: &str| parse_id("run", s).map(WorkflowId);
        Ok(match body {
            wire::Body::ProjectAdd {
                space: sp,
                name,
                root,
            } => Self::AddProject {
                space: space(&sp)?,
                name,
                root,
            },
            wire::Body::ProjectRemove { project: p } => Self::RemoveProject(project(&p)?),
            wire::Body::SessionNew {
                project: p,
                name,
                session_kind: k,
                cwd,
                launch: l,
                prompt,
                notes,
            } => Self::NewSession {
                project: project(&p)?,
                name,
                kind: session_kind(k),
                cwd,
                launch: launch(l),
                prompt,
                notes,
            },
            wire::Body::SessionSend { session: s, text } => Self::SendInput {
                id: session(&s)?,
                text,
            },
            wire::Body::SessionKill { session: s } => Self::Kill(session(&s)?),
            wire::Body::SessionRemove { session: s } => Self::Remove(session(&s)?),
            wire::Body::SessionNotes { session: s, text } => Self::SetNotes {
                id: session(&s)?,
                text,
            },
            wire::Body::SessionWaiting {
                session: s,
                on,
                reason,
            } => Self::SetWaiting {
                id: session(&s)?,
                reason: on.then_some(reason),
            },
            wire::Body::SessionMove {
                session: s,
                project: p,
            } => Self::MoveSession {
                id: session(&s)?,
                project: project(&p)?,
            },
            wire::Body::ProjectRename { project: p, name } => Self::RenameProject {
                id: project(&p)?,
                name,
            },
            wire::Body::SpaceNew { name } => Self::NewSpace { name },
            wire::Body::SetNew { space: sp, name } => Self::NewSet {
                space: space(&sp)?,
                name,
            },
            wire::Body::SetSync { set, items } => Self::SyncSet {
                set: SetId(parse_id("set", &set)?),
                items: items.into_iter().map(pin).collect::<Result<_, _>>()?,
            },
            wire::Body::DefinitionInstall { definition: d } => {
                Self::InstallDefinition(definition(d))
            }
            wire::Body::WorkflowStart {
                source,
                plan,
                definition,
                reviewer_cwd,
                reviewer_args,
            } => Self::StartWorkflow {
                source: session(&source)?,
                plan,
                definition,
                reviewer_cwd,
                reviewer_args,
            },
            wire::Body::WorkflowPause { run: r } => Self::PauseWorkflow(run(&r)?),
            wire::Body::WorkflowContinue { run: r } => Self::ContinueWorkflow(run(&r)?),
            wire::Body::WorkflowFinalize { run: r } => Self::FinalizeWorkflow(run(&r)?),
            wire::Body::WorkflowRemove { run: r } => Self::RemoveWorkflow(run(&r)?),
            other => return Err(format!("{} is a query, not a command", other.kind())),
        })
    }
}
