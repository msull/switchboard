//! The state machine. `AppCore` holds the workspaces and derived state;
//! `dispatch` applies one action at an explicit time and returns effects.
//!
//! Contract for the implementation (Milestone 1):
//! - Startup is a reconcile: `StoreLoaded` then `HostListed` produce card
//!   states, never launches, except `Effect::Spawn` for trusted
//!   `autostart` services.
//! - "Return" is idempotent: `ReturnToSession` emits `Attach` when the host
//!   status is `Running`, `PrepareResume` (then `Spawn`, then `Attach`)
//!   when it is `Missing`, and nothing when a resume is already in flight.
//! - Events older than `record.last_event_at` are ignored.
//! - `Liveness` from the host overrides event-derived activity.
//! - Every change to a workspace emits `Effect::Save` for it.
//!
//! This file holds the contract types, the dispatcher, and the read model.
//! The transitions live beside it: `reconcile` (store and host results),
//! `sessions` (launch, return, resume), and `events` (hook events).

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::model::{
    Activity, AgentKind, CardState, Launch, Project, ProjectId, RecordId, ResumeHandle,
    SessionKind, SessionRecord, Workspace,
};
use crate::ports::agent::AgentLaunch;
use crate::ports::events::SessionEvent;
use crate::ports::host::{HostId, HostStatus, Liveness, SpawnSpec};
use crate::ports::store::{Loaded, StoreError};

/// The current time as the core sees it.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub mono: Duration,
    pub wall: SystemTime,
}

impl Clock {
    #[must_use]
    pub fn at(mono_ms: u64) -> Self {
        Self {
            mono: Duration::from_millis(mono_ms),
            wall: UNIX_EPOCH + Duration::from_secs(1_700_000_000 + mono_ms / 1000),
        }
    }
}

/// Which screen is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// Every session across every project.
    Switchboard,
    Board(ProjectId),
    Session(RecordId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
    pub expires_at: Option<Duration>,
}

/// Inputs from UI, workers, and the host poll.
#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    // --- startup / persistence
    StoreLoaded(Result<Loaded, StoreError>),
    SaveFinished(ProjectId, Result<(), StoreError>),
    /// The host is unusable (tmux missing); `None` clears.
    HostUnavailable(Option<String>),
    // --- navigation
    ShowSwitchboard,
    ShowBoard(ProjectId),
    ShowSession(RecordId),
    Back,
    DismissNotice,
    // --- projects
    AddProject {
        name: String,
        root: PathBuf,
    },
    RemoveProject(ProjectId),
    RenameProject(ProjectId, String),
    PinDocument(ProjectId, PathBuf),
    UnpinDocument(ProjectId, PathBuf),
    // --- sessions
    NewSession {
        project: ProjectId,
        name: String,
        kind: SessionKind,
        cwd: PathBuf,
        launch: Launch,
    },
    RenameSession(RecordId, String),
    SetSessionNotes(RecordId, String),
    SetAutostart(RecordId, bool),
    MoveCard {
        id: RecordId,
        order: u32,
        group: Option<String>,
    },
    ReturnToSession(RecordId),
    /// Type `text` into the session's terminal and press Enter, as if the
    /// user had typed it there.
    SendInput {
        id: RecordId,
        text: String,
    },
    KillSession(RecordId),
    RemoveSession(RecordId),
    // --- results from effects / workers
    LaunchPrepared {
        id: RecordId,
        result: Result<AgentLaunch, String>,
    },
    Spawned {
        id: RecordId,
        result: Result<(), String>,
    },
    Attached {
        id: RecordId,
        result: Result<(), String>,
    },
    TranscriptChecked {
        id: RecordId,
        exists: bool,
    },
    Discovered {
        id: RecordId,
        result: Result<Option<ResumeHandle>, String>,
    },
    /// Result of one `ProcessHost::list` poll.
    HostListed(Vec<HostStatus>),
    Events(Vec<SessionEvent>),
    Tick,
}

/// Work the shell performs on the core's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Save(Workspace),
    Delete(ProjectId),
    /// Compose a fresh launch for an agent record (worker; reports `LaunchPrepared`).
    PrepareLaunch {
        id: RecordId,
        kind: AgentKind,
        name: String,
        cwd: PathBuf,
    },
    PrepareResume {
        id: RecordId,
        handle: ResumeHandle,
        name: String,
        cwd: PathBuf,
    },
    CheckTranscript {
        id: RecordId,
        handle: ResumeHandle,
    },
    /// Discover a Codex id created in `cwd` after `since`.
    Discover {
        id: RecordId,
        kind: AgentKind,
        cwd: PathBuf,
        since: SystemTime,
    },
    Spawn {
        id: RecordId,
        spec: SpawnSpec,
    },
    /// Open (or raise) the external terminal attached to the host session.
    Attach {
        id: RecordId,
        host: HostId,
        title: String,
        cwd: PathBuf,
    },
    Kill(HostId),
    /// Write `text` to the pane, then Enter.
    SendInput {
        host: HostId,
        text: String,
    },
    OpenPath(PathBuf),
    Reveal(PathBuf),
}

/// What a record is waiting on. A record with a flight is "in flight":
/// repeated returns do nothing until the flight lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FlightKind {
    /// Fresh launch: `PrepareLaunch` (agents) or `Spawn` (everything else).
    Launch,
    /// Transcript preflight before a resume.
    Preflight,
    /// `PrepareResume` then `Spawn`.
    Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Flight {
    pub id: RecordId,
    pub kind: FlightKind,
    /// Wall time the launch began; Codex discovery looks for rollout
    /// files created after it.
    pub started: SystemTime,
}

/// Effects gathered while one action is applied. Saves are coalesced:
/// one `Effect::Save` per touched workspace, emitted before the rest.
#[derive(Debug, Default)]
pub(super) struct Out {
    pub effects: Vec<Effect>,
    pub dirty: Vec<ProjectId>,
}

impl Out {
    pub fn push(&mut self, effect: Effect) {
        self.effects.push(effect);
    }
    pub fn touch(&mut self, project: ProjectId) {
        if !self.dirty.contains(&project) {
            self.dirty.push(project);
        }
    }
}

/// How long a success notice stays up.
const NOTICE_TTL: Duration = Duration::from_secs(4);

/// Top-level state. Plain data and pure methods only.
///
/// Fields are `pub(super)` so the sibling transition modules can reach
/// them; nothing outside `core` sees them.
#[derive(Debug, Default)]
pub struct AppCore {
    pub(super) workspaces: Vec<Workspace>,
    pub(super) view_stack: Vec<View>,
    pub(super) notices: Vec<Notice>,
    pub(super) host_error: Option<String>,
    pub(super) read_only: bool,
    /// Latest host status per session, from the last poll.
    pub(super) host: Vec<HostStatus>,
    /// Records with a launch or resume in flight (idempotent return).
    pub(super) in_flight: Vec<Flight>,
    /// Codex launches are serialized: at most one discovery pending.
    pub(super) codex_pending: Option<RecordId>,
    /// Codex records waiting for their turn to launch, in order.
    pub(super) codex_queue: Vec<RecordId>,
    /// The store has loaded, so the next host poll is the reconcile.
    pub(super) store_loaded: bool,
    pub(super) reconciled: bool,
}

impl AppCore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The single entry point: applies one action at `now` and returns
    /// the effects the shell must run. See the module docs for the
    /// contract; the transitions live in the sibling modules.
    pub fn dispatch(&mut self, action: AppAction, now: Clock) -> Vec<Effect> {
        let mut out = Out::default();
        match action {
            AppAction::StoreLoaded(result) => self.store_loaded(result),
            AppAction::SaveFinished(_, Err(e)) => self.error(format!("save failed: {e}")),
            AppAction::SaveFinished(_, Ok(())) => {}
            AppAction::HostUnavailable(reason) => self.host_error = reason,
            AppAction::HostListed(statuses) => self.host_listed(statuses, now, &mut out),

            AppAction::ShowSwitchboard => self.show(View::Switchboard, now, &mut out),
            AppAction::ShowBoard(id) => self.show(View::Board(id), now, &mut out),
            AppAction::ShowSession(id) => self.show(View::Session(id), now, &mut out),
            AppAction::Back => {
                self.view_stack.pop();
            }
            AppAction::DismissNotice => {
                if !self.notices.is_empty() {
                    self.notices.remove(0);
                }
            }
            AppAction::Tick => self
                .notices
                .retain(|n| n.expires_at.is_none_or(|t| t > now.mono)),

            AppAction::AddProject { name, root } => self.add_project(name, root, now, &mut out),
            AppAction::RemoveProject(id) => {
                let before = self.workspaces.len();
                self.workspaces.retain(|w| w.project.id != id);
                if self.workspaces.len() != before {
                    out.push(Effect::Delete(id));
                }
                self.view_stack
                    .retain(|v| !matches!(v, View::Board(p) if *p == id));
            }
            AppAction::RenameProject(id, name) => {
                self.edit_project(id, &mut out, |p| p.name = name);
            }
            AppAction::PinDocument(id, path) => self.edit_project(id, &mut out, |p| {
                if !p.pinned.contains(&path) {
                    p.pinned.push(path);
                }
            }),
            AppAction::UnpinDocument(id, path) => {
                self.edit_project(id, &mut out, |p| p.pinned.retain(|d| *d != path));
            }

            AppAction::NewSession {
                project,
                name,
                kind,
                cwd,
                launch,
            } => self.new_session(project, name, kind, cwd, launch, now, &mut out),
            AppAction::RenameSession(id, name) => {
                self.edit_session(id, &mut out, |s| s.name = name);
            }
            AppAction::SetSessionNotes(id, notes) => {
                self.edit_session(id, &mut out, |s| s.notes = notes);
            }
            AppAction::SetAutostart(id, on) => {
                self.edit_session(id, &mut out, |s| s.autostart = on);
            }
            AppAction::MoveCard { id, order, group } => self.edit_session(id, &mut out, |s| {
                s.layout.order = order;
                s.layout.group = group;
            }),
            AppAction::ReturnToSession(id) => self.return_to_session(id, now, &mut out),
            AppAction::SendInput { id, text } => {
                let running = self
                    .host_status(id)
                    .is_some_and(|h| matches!(h.liveness, Liveness::Running { .. }));
                if let (true, Some(status)) = (running, self.host_status(id)) {
                    out.push(Effect::SendInput {
                        host: status.id.clone(),
                        text,
                    });
                } else {
                    let name = self.session_name(id);
                    self.error(format!("{name} is not running; return to it first"));
                }
            }
            AppAction::KillSession(id) => {
                if let Some(status) = self.host_status(id) {
                    out.push(Effect::Kill(status.id.clone()));
                }
            }
            AppAction::RemoveSession(id) => self.remove_session(id, &mut out),

            AppAction::LaunchPrepared { id, result } => {
                self.launch_prepared(id, result, now, &mut out);
            }
            AppAction::Spawned { id, result } => self.spawned(id, result, now, &mut out),
            AppAction::Attached { id, result } => {
                if let Err(e) = result {
                    let name = self.session_name(id);
                    self.error(format!("could not attach to {name}: {e}"));
                }
            }
            AppAction::TranscriptChecked { id, exists } => {
                self.transcript_checked(id, exists, now, &mut out);
            }
            AppAction::Discovered { id, result } => self.discovered(id, result, now, &mut out),
            AppAction::Events(events) => self.apply_events(events, &mut out),
        }
        self.finish(out)
    }

    /// Turns the gathered output into the effect list: one `Save` per
    /// dirty workspace first, then the rest. A read-only instance never
    /// writes, so its persistence effects are dropped here.
    fn finish(&self, out: Out) -> Vec<Effect> {
        let mut effects = Vec::with_capacity(out.effects.len() + out.dirty.len());
        if !self.read_only {
            for id in out.dirty {
                if let Some(w) = self.workspace(id) {
                    effects.push(Effect::Save(w.clone()));
                }
            }
        }
        effects.extend(
            out.effects
                .into_iter()
                .filter(|e| !(self.read_only && matches!(e, Effect::Delete(_)))),
        );
        effects
    }

    /// Populate state directly, bypassing dispatch. For UI tests and the
    /// demo launcher only; the app itself always goes through `dispatch`.
    pub fn seed(&mut self, workspaces: Vec<Workspace>, host: Vec<HostStatus>) {
        self.workspaces = workspaces;
        self.host = host;
        self.store_loaded = true;
        self.reconciled = true;
    }

    // --- notices and navigation

    pub(super) fn error(&mut self, text: impl Into<String>) {
        self.notices.push(Notice {
            text: text.into(),
            is_error: true,
            expires_at: None,
        });
    }

    pub(super) fn info(&mut self, text: impl Into<String>, now: Clock) {
        self.notices.push(Notice {
            text: text.into(),
            is_error: false,
            expires_at: Some(now.mono + NOTICE_TTL),
        });
    }

    fn show(&mut self, view: View, now: Clock, out: &mut Out) {
        if let View::Board(id) = view {
            self.edit_project(id, out, |p| p.last_active = now.wall);
        }
        if self.view() != view {
            self.view_stack.push(view);
        }
    }

    // --- projects

    fn add_project(&mut self, name: String, root: PathBuf, now: Clock, out: &mut Out) {
        let id = ProjectId::new();
        self.workspaces.push(Workspace::new(Project {
            id,
            name,
            root,
            tags: Vec::new(),
            notes: String::new(),
            pinned: Vec::new(),
            created: now.wall,
            last_active: now.wall,
        }));
        out.touch(id);
        self.view_stack.push(View::Board(id));
    }

    fn edit_project(&mut self, id: ProjectId, out: &mut Out, edit: impl FnOnce(&mut Project)) {
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.project.id == id) {
            edit(&mut w.project);
            out.touch(id);
        }
    }

    /// Applies `edit` to a record and marks its workspace for saving.
    pub(super) fn edit_session(
        &mut self,
        id: RecordId,
        out: &mut Out,
        edit: impl FnOnce(&mut SessionRecord),
    ) {
        if let Some(s) = self.session_mut(id) {
            let project = s.project;
            edit(s);
            out.touch(project);
        }
    }

    fn remove_session(&mut self, id: RecordId, out: &mut Out) {
        for w in &mut self.workspaces {
            let before = w.sessions.len();
            w.sessions.retain(|s| s.id != id);
            if w.sessions.len() != before {
                out.touch(w.project.id);
            }
        }
        self.in_flight.retain(|f| f.id != id);
        self.codex_queue.retain(|q| *q != id);
        if self.codex_pending == Some(id) {
            self.codex_pending = None;
        }
        self.view_stack
            .retain(|v| !matches!(v, View::Session(s) if *s == id));
    }

    pub(super) fn session_mut(&mut self, id: RecordId) -> Option<&mut SessionRecord> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| &mut w.sessions)
            .find(|s| s.id == id)
    }

    /// The record's name for notices, falling back to the host name.
    pub(super) fn session_name(&self, id: RecordId) -> String {
        self.session(id)
            .map_or_else(|| id.host_name(), |s| s.name.clone())
    }

    /// Show `view` directly, bypassing dispatch. UI tests only.
    pub fn seed_view(&mut self, view: View) {
        self.view_stack.push(view);
    }

    /// Set the bottom-bar state directly, bypassing dispatch. UI tests only.
    pub fn seed_status(
        &mut self,
        notice: Option<Notice>,
        host_error: Option<String>,
        read_only: bool,
    ) {
        self.notices = notice.into_iter().collect();
        self.host_error = host_error;
        self.read_only = read_only;
    }

    // --- read model for the UI

    #[must_use]
    pub fn view(&self) -> View {
        self.view_stack.last().cloned().unwrap_or(View::Switchboard)
    }
    #[must_use]
    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }
    /// Workspaces most recently active first, for the switcher.
    #[must_use]
    pub fn workspaces_by_recency(&self) -> Vec<&Workspace> {
        let mut all: Vec<&Workspace> = self.workspaces.iter().collect();
        all.sort_by_key(|w| std::cmp::Reverse(w.project.last_active));
        all
    }
    #[must_use]
    pub fn workspace(&self, id: ProjectId) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.project.id == id)
    }
    #[must_use]
    pub fn session(&self, id: RecordId) -> Option<&SessionRecord> {
        self.workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .find(|s| s.id == id)
    }
    #[must_use]
    pub fn host_status(&self, id: RecordId) -> Option<&HostStatus> {
        let name = id.host_name();
        self.host.iter().find(|h| h.id.0 == name)
    }
    /// Derived card state for a record: host liveness first, then activity.
    #[must_use]
    pub fn card_state(&self, id: RecordId) -> CardState {
        let Some(record) = self.session(id) else {
            return CardState::NotRunning;
        };
        match self.host_status(id).map(|h| &h.liveness) {
            Some(Liveness::Running { .. }) => match record.activity {
                Activity::WaitingOnYou => CardState::WaitingOnYou,
                Activity::Working => CardState::Working,
                Activity::Idle | Activity::Ended => CardState::Idle,
                // Nothing reported yet: an agent that just started is busy
                // until a hook says otherwise; a shell sits at its prompt.
                Activity::Unknown => match record.kind {
                    SessionKind::Agent(_) => CardState::Working,
                    SessionKind::Command | SessionKind::Service | SessionKind::Shell => {
                        CardState::Idle
                    }
                },
            },
            Some(Liveness::Exited { code }) => CardState::Exited(*code),
            Some(Liveness::Missing) | None if record.not_resumable => CardState::NotResumable,
            Some(Liveness::Missing) | None => CardState::NotRunning,
        }
    }
    /// One project's cards: waiting first, then by the saved order.
    #[must_use]
    pub fn sessions_sorted(&self, project: ProjectId) -> Vec<&SessionRecord> {
        let mut sessions: Vec<&SessionRecord> = self
            .workspace(project)
            .map(|w| w.sessions.iter().collect())
            .unwrap_or_default();
        sessions.sort_by_key(|s| (self.card_state(s.id).rank(), s.layout.order));
        sessions
    }
    /// Every session for the switchboard view: waiting first, then by
    /// project recency, then state and saved order within a project.
    #[must_use]
    pub fn all_sessions_sorted(&self) -> Vec<&SessionRecord> {
        let mut all: Vec<(&Project, &SessionRecord)> = self
            .workspaces
            .iter()
            .flat_map(|w| w.sessions.iter().map(move |s| (&w.project, s)))
            .collect();
        all.sort_by_key(|(p, s)| {
            let rank = self.card_state(s.id).rank();
            (
                rank != CardState::WaitingOnYou.rank(),
                std::cmp::Reverse(p.last_active),
                rank,
                s.layout.order,
            )
        });
        all.into_iter().map(|(_, s)| s).collect()
    }
    /// The oldest pending notice; `notices` has them all.
    #[must_use]
    pub fn notice(&self) -> Option<&Notice> {
        self.notices.first()
    }
    #[must_use]
    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }
    #[must_use]
    pub fn host_error(&self) -> Option<&str> {
        self.host_error.as_deref()
    }
    #[must_use]
    pub fn read_only(&self) -> bool {
        self.read_only
    }
    /// A launch, preflight, or resume is under way (or queued) for this
    /// record, so a return does nothing until it lands.
    #[must_use]
    pub fn is_in_flight(&self, id: RecordId) -> bool {
        self.in_flight.iter().any(|f| f.id == id)
    }
    /// A Codex record waiting for an earlier Codex launch to be bound.
    #[must_use]
    pub fn queued_codex(&self, id: RecordId) -> bool {
        self.codex_queue.contains(&id)
    }
    /// Sessions across all projects that are waiting on the user.
    #[must_use]
    pub fn waiting_count(&self) -> usize {
        self.workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|s| self.card_state(s.id) == CardState::WaitingOnYou)
            .count()
    }
}
