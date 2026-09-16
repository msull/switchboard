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

use crate::core::env::SecretScope;
use crate::core::grid;
use crate::core::model::{
    Activity, AgentKind, CardState, EnvVar, GridRect, Launch, PinTarget, PinnedItem, Project,
    ProjectEnv, ProjectId, RecordId, ResumeHandle, SavedView, SessionKind, SessionRecord, SetId,
    Settings, SideTab, ThemeMode, Views, WorkingSet, Workspace,
};
use crate::ports::agent::AgentLaunch;
use crate::ports::events::SessionEvent;
use crate::ports::host::{HostId, HostStatus, Liveness, SpawnSpec};
use crate::ports::project_config::ProjectConfig;
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
    /// A file of the project, previewed read-only.
    Document(ProjectId, PathBuf),
    /// One of the user's grids of cards from any project.
    WorkingSet(SetId),
}

impl View {
    /// What to remember of this screen across a restart.
    #[must_use]
    pub fn saved(&self) -> SavedView {
        match self {
            View::Switchboard => SavedView::Switchboard,
            View::Board(id) | View::Document(id, _) => SavedView::Board(*id),
            View::Session(id) => SavedView::Session(*id),
            View::WorkingSet(id) => SavedView::Set(*id),
        }
    }
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
    /// Preview a file (absolute path) of the project.
    ShowDocument(ProjectId, PathBuf),
    ShowWorkingSet(SetId),
    /// Put a session or file on a working set, in the first free spot
    /// of a grid `columns` wide (what the window fits right now).
    AddToWorkingSet {
        set: SetId,
        target: PinTarget,
        columns: u32,
    },
    RemoveFromWorkingSet {
        set: SetId,
        target: PinTarget,
    },
    /// Move or resize a working-set card. A place that overlaps another
    /// card is refused and the card stays where it was.
    PlacePin {
        set: SetId,
        target: PinTarget,
        rect: GridRect,
    },
    /// A new working set, shown at once: empty, or a copy of `clone_of`
    /// (named after it), and holding `with` if given.
    NewWorkingSet {
        name: Option<String>,
        clone_of: Option<SetId>,
        with: Option<PinTarget>,
        columns: u32,
    },
    RenameWorkingSet {
        set: SetId,
        name: String,
    },
    /// Drop a working set; its cards were only references.
    DeleteWorkingSet(SetId),
    OpenDocument(PathBuf),
    OpenInEditor(PathBuf),
    RevealDocument(PathBuf),
    SetEditor(String),
    SetGlobalEnv(Vec<EnvVar>),
    SetProjectEnv(ProjectId, ProjectEnv),
    /// Put a secret value in the store; the value never reaches a record.
    StoreSecret {
        scope: SecretScope,
        name: String,
        value: String,
    },
    DeleteSecret {
        scope: SecretScope,
        name: String,
    },
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
    SetTheme(ThemeMode),
    /// Exclusive mode shows only the active project (screen sharing).
    SetExclusive(bool),
    /// Show or hide the file side next to sessions.
    SetFilesOpen(bool),
    /// Whether an agent's terminal window opens as it starts or resumes.
    SetOpenTerminalOnLaunch(bool),
    /// Which tab the side panel shows.
    SetSideTab(SideTab),
    /// The project's `.switchboard/project.json` was read (or is absent,
    /// or unusable). Entries become records that cannot run until
    /// approved.
    ProjectConfigRead {
        project: ProjectId,
        result: Result<Option<ProjectConfig>, String>,
    },
    /// The user approved the record's current definition.
    ApproveDefinition(RecordId),
    RevokeApproval(RecordId),
    /// Type `text` into the session's terminal and press Enter, as if the
    /// user had typed it there.
    SendInput {
        id: RecordId,
        text: String,
    },
    /// Send Escape to the session's terminal, which stops an agent's
    /// current turn without leaving the conversation view.
    Interrupt(RecordId),
    KillSession(RecordId),
    /// Stop a shell, command, or service if it runs, then start it again.
    RestartSession(RecordId),
    RemoveSession(RecordId),
    /// Fork an agent session at one of the user's prompts: a new record
    /// whose conversation is everything before turn `before`, with
    /// `prompt` (that turn's text) primed as its draft. Never launches.
    CloneSession {
        id: RecordId,
        before: usize,
        prompt: String,
    },
    // --- results from effects / workers
    /// An effect with no result of its own failed; the user is told.
    Failed(String),
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
    /// The provider-side copy for `CloneSession` was made (or not).
    TranscriptCloned {
        source: RecordId,
        prompt: String,
        result: Result<ResumeHandle, String>,
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
    SaveSettings(Settings),
    SaveViews(Views),
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
    /// Copy the transcript behind `handle` up to (not including) the
    /// `before`th prompt under a fresh provider id (reports
    /// `TranscriptCloned`).
    CloneTranscript {
        source: RecordId,
        handle: ResumeHandle,
        before: usize,
        prompt: String,
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
    /// Raw bytes to the pane, no Enter (Escape, control characters).
    SendKeys {
        host: HostId,
        bytes: Vec<u8>,
    },
    /// Read `<root>/.switchboard/project.json`; answered with
    /// `ProjectConfigRead`.
    ReadProjectConfig {
        project: ProjectId,
        root: PathBuf,
    },
    OpenPath(PathBuf),
    /// Drop what the host kept for a removed record (scrollback on disk).
    Forget(HostId),
    StoreSecret {
        account: String,
        value: String,
    },
    DeleteSecret(String),
    /// Open the file with the configured editor command.
    OpenInEditor {
        editor: String,
        path: PathBuf,
    },
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

/// The state of a project's definition file as last read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigStatus {
    /// A file exists (usable or not).
    pub present: bool,
    /// Entries that were skipped, and why.
    pub warnings: Vec<String>,
    /// The file could not be used at all.
    pub error: Option<String>,
}

/// Top-level state. Plain data and pure methods only.
///
/// Fields are `pub(super)` so the sibling transition modules can reach
/// them; nothing outside `core` sees them.
#[derive(Debug, Default)]
pub struct AppCore {
    pub(super) workspaces: Vec<Workspace>,
    pub(super) views: Views,
    pub(super) view_stack: Vec<View>,
    pub(super) notices: Vec<Notice>,
    pub(super) host_error: Option<String>,
    pub(super) read_only: bool,
    /// Latest host status per session, from the last poll.
    pub(super) host: Vec<HostStatus>,
    /// Records with a launch or resume in flight (idempotent return).
    pub(super) in_flight: Vec<Flight>,
    /// Text a new record's message box should start with, waiting for
    /// the shell to hand it to the UI (`take_primed`).
    pub(super) primed: Vec<(RecordId, String)>,
    /// What the last read of each project's definition file said.
    /// Transient: it is re-read at startup.
    pub(super) config_status: Vec<(ProjectId, ConfigStatus)>,
    /// Codex launches are serialized: at most one discovery pending.
    pub(super) codex_pending: Option<RecordId>,
    /// Codex records waiting for their turn to launch, in order.
    pub(super) codex_queue: Vec<RecordId>,
    /// The store has loaded, so the next host poll is the reconcile.
    pub(super) store_loaded: bool,
    pub(super) reconciled: bool,
    pub(super) settings: Settings,
    /// Agents without hooks (Codex) whose pane has been quiet for a
    /// while, per the last host poll: shown idle instead of working.
    pub(super) quiet: Vec<RecordId>,
}

impl AppCore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drafts queued for records the core just made (a cloned session's
    /// chosen prompt). The shell moves them into the UI's drafts; the
    /// core never reads them back.
    pub fn take_primed(&mut self) -> Vec<(RecordId, String)> {
        std::mem::take(&mut self.primed)
    }

    /// The single entry point: applies one action at `now` and returns
    /// the effects the shell must run. See the module docs for the
    /// contract; the transitions live in the sibling modules.
    pub fn dispatch(&mut self, action: AppAction, now: Clock) -> Vec<Effect> {
        let mut out = Out::default();
        match action {
            AppAction::StoreLoaded(result) => self.store_loaded(result, &mut out),
            AppAction::SaveFinished(_, Err(e)) => self.error(format!("save failed: {e}")),
            AppAction::SaveFinished(_, Ok(())) => {}
            AppAction::Failed(text) => self.error(text),
            AppAction::HostUnavailable(reason) => self.host_error = reason,
            AppAction::HostListed(statuses) => self.host_listed(statuses, now, &mut out),

            AppAction::ShowSwitchboard => self.show(View::Switchboard, now, &mut out),
            AppAction::ShowWorkingSet(_)
            | AppAction::AddToWorkingSet { .. }
            | AppAction::RemoveFromWorkingSet { .. }
            | AppAction::PlacePin { .. }
            | AppAction::NewWorkingSet { .. }
            | AppAction::RenameWorkingSet { .. }
            | AppAction::DeleteWorkingSet(_) => self.working_set_action(action, now, &mut out),
            AppAction::ShowBoard(id) => self.show(View::Board(id), now, &mut out),
            AppAction::ShowSession(id) => self.show(View::Session(id), now, &mut out),
            AppAction::RenameProject(..)
            | AppAction::PinDocument(..)
            | AppAction::UnpinDocument(..)
            | AppAction::ShowDocument(..)
            | AppAction::OpenDocument(_)
            | AppAction::RevealDocument(_)
            | AppAction::OpenInEditor(_)
            | AppAction::SetEditor(_)
            | AppAction::SetGlobalEnv(_)
            | AppAction::SetProjectEnv(..)
            | AppAction::StoreSecret { .. }
            | AppAction::DeleteSecret { .. }
            | AppAction::SetTheme(_)
            | AppAction::SetExclusive(_)
            | AppAction::SetFilesOpen(_)
            | AppAction::SetOpenTerminalOnLaunch(_)
            | AppAction::SetSideTab(_)
            | AppAction::ProjectConfigRead { .. }
            | AppAction::ApproveDefinition(_)
            | AppAction::RevokeApproval(_) => self.files_and_settings(action, now, &mut out),
            AppAction::Back => drop(self.view_stack.pop()),
            AppAction::DismissNotice => {
                if !self.notices.is_empty() {
                    self.notices.remove(0);
                }
            }
            AppAction::Tick => self.expire_notices(now),

            AppAction::AddProject { name, root } => self.add_project(name, root, now, &mut out),
            AppAction::RemoveProject(id) => self.remove_project(id, now, &mut out),

            AppAction::NewSession { .. }
            | AppAction::RenameSession(..)
            | AppAction::SetSessionNotes(..)
            | AppAction::SetAutostart(..)
            | AppAction::MoveCard { .. }
            | AppAction::ReturnToSession(_)
            | AppAction::SendInput { .. }
            | AppAction::Interrupt(_)
            | AppAction::KillSession(_)
            | AppAction::RemoveSession(_)
            | AppAction::RestartSession(_)
            | AppAction::CloneSession { .. }
            | AppAction::TranscriptCloned { .. } => self.session_action(action, now, &mut out),

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
        self.remember_view(&mut out);
        self.prune_working_set(&mut out);
        self.finish(out)
    }

    /// The session records' transitions, split out of `dispatch` for length.
    fn session_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::NewSession {
                project,
                name,
                kind,
                cwd,
                launch,
            } => self.new_session(project, name, kind, cwd, launch, now, out),
            AppAction::RenameSession(id, name) => {
                self.edit_session(id, out, |s| s.name = name);
            }
            AppAction::SetSessionNotes(id, notes) => {
                self.edit_session(id, out, |s| s.notes = notes);
            }
            AppAction::SetAutostart(id, on) => {
                self.edit_session(id, out, |s| s.autostart = on);
            }
            AppAction::MoveCard { id, order, group } => self.edit_session(id, out, |s| {
                s.layout.order = order;
                s.layout.group = group;
            }),
            AppAction::ReturnToSession(id) => self.return_to_session(id, now, out),
            AppAction::SendInput { id, text } => {
                self.aim_at_pane(id, out, |host| Effect::SendInput { host, text });
            }
            AppAction::Interrupt(id) => self.aim_at_pane(id, out, |host| Effect::SendKeys {
                host,
                bytes: vec![0x1b],
            }),
            AppAction::KillSession(id) => {
                if let Some(status) = self.host_status(id) {
                    out.push(Effect::Kill(status.id.clone()));
                }
            }
            AppAction::RemoveSession(id) => self.remove_session(id, out),
            AppAction::RestartSession(id) => self.restart_session(id, now, out),
            AppAction::CloneSession { id, before, prompt } => {
                self.clone_session(id, before, prompt, out);
            }
            AppAction::TranscriptCloned {
                source,
                prompt,
                result,
            } => self.transcript_cloned(source, prompt, result, now, out),
            _ => unreachable!("not a session action"),
        }
    }

    /// The working sets' transitions, split out of `dispatch` for length.
    fn working_set_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::ShowWorkingSet(id) => {
                if self.working_set(id).is_some() {
                    self.show(View::WorkingSet(id), now, out);
                }
            }
            AppAction::AddToWorkingSet {
                set,
                target,
                columns,
            } => self.add_to_working_set(set, target, columns, out),
            AppAction::RemoveFromWorkingSet { set, target } => {
                self.update_set(out, set, |s| s.items.retain(|i| i.target != target));
            }
            AppAction::PlacePin { set, target, rect } => {
                let rect = grid::clamp(rect);
                self.update_set(out, set, |s| {
                    if grid::fits(&s.items, &target, rect)
                        && let Some(item) = s.items.iter_mut().find(|i| i.target == target)
                    {
                        item.rect = rect;
                    }
                });
            }
            AppAction::NewWorkingSet {
                name,
                clone_of,
                with,
                columns,
            } => self.new_working_set(name, clone_of, with, columns, now, out),
            AppAction::RenameWorkingSet { set, name } => {
                let name = name.trim().to_owned();
                if !name.is_empty() {
                    self.update_set(out, set, |s| s.name = name);
                }
            }
            AppAction::DeleteWorkingSet(id) => {
                self.update_views(out, |v| v.sets.retain(|s| s.id != id));
                self.view_stack
                    .retain(|v| !matches!(v, View::WorkingSet(s) if *s == id));
            }
            _ => unreachable!("routed by `dispatch`"),
        }
    }

    /// Change the views and save them if anything changed. A read-only
    /// instance changes nothing.
    fn update_views(&mut self, out: &mut Out, change: impl FnOnce(&mut Views)) {
        let mut next = self.views.clone();
        change(&mut next);
        if next != self.views {
            self.views = next;
            if !self.read_only {
                out.push(Effect::SaveViews(self.views.clone()));
            }
        }
    }

    /// Change one working set by id; an unknown id changes nothing.
    fn update_set(&mut self, out: &mut Out, id: SetId, change: impl FnOnce(&mut WorkingSet)) {
        self.update_views(out, |v| {
            if let Some(set) = v.sets.iter_mut().find(|s| s.id == id) {
                change(set);
            }
        });
    }

    fn target_exists(&self, target: &PinTarget) -> bool {
        match target {
            PinTarget::Session(id) => self.session(*id).is_some(),
            PinTarget::File(pid, _) => self.workspace(*pid).is_some(),
        }
    }

    /// `target`'s card, placed on `set` in the first free spot, if it
    /// exists and is not there already.
    fn place_new(&self, set: &mut WorkingSet, target: PinTarget, columns: u32) {
        if !self.target_exists(&target) || set.items.iter().any(|i| i.target == target) {
            return;
        }
        let kind = match &target {
            PinTarget::Session(id) => self.session(*id).map(|s| s.kind),
            PinTarget::File(..) => None,
        };
        let (w, h) = grid::default_size(&target, kind);
        let rect = grid::first_free(&set.items, w, h, columns);
        set.items.push(PinnedItem { target, rect });
    }

    fn add_to_working_set(&mut self, id: SetId, target: PinTarget, columns: u32, out: &mut Out) {
        let Some(mut set) = self.working_set(id).cloned() else {
            return;
        };
        self.place_new(&mut set, target, columns);
        self.update_set(out, id, |s| *s = set);
    }

    fn new_working_set(
        &mut self,
        name: Option<String>,
        clone_of: Option<SetId>,
        with: Option<PinTarget>,
        columns: u32,
        now: Clock,
        out: &mut Out,
    ) {
        let source = clone_of.and_then(|id| self.working_set(id).cloned());
        let name = name
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .or_else(|| source.as_ref().map(|s| format!("{} copy", s.name)))
            .unwrap_or_else(|| match self.views.sets.len() {
                0 => "Working Set".to_owned(),
                n => format!("Working Set {}", n + 1),
            });
        let mut set = WorkingSet::named(name);
        if let Some(source) = source {
            set.items = source.items;
        }
        if let Some(target) = with {
            self.place_new(&mut set, target, columns);
        }
        let id = set.id;
        self.update_views(out, |v| v.sets.push(set));
        self.show(View::WorkingSet(id), now, out);
    }

    /// Drop working-set cards whose session or project is gone, after
    /// whatever action removed it (or the load that found it missing).
    fn prune_working_set(&mut self, out: &mut Out) {
        let stale = self
            .views
            .sets
            .iter()
            .flat_map(|s| &s.items)
            .any(|i| !self.target_exists(&i.target));
        if !stale {
            return;
        }
        let mut next = self.views.clone();
        for set in &mut next.sets {
            set.items.retain(|i| self.target_exists(&i.target));
        }
        self.update_views(out, |v| *v = next);
    }

    /// Keep `settings.last_view` equal to the screen showing, whatever
    /// action moved it (navigation, a project added or removed, a
    /// session deleted), so the next start reopens the same screen.
    fn remember_view(&mut self, out: &mut Out) {
        let saved = self.view().saved();
        self.update_settings(out, |s| s.last_view = saved);
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

    fn expire_notices(&mut self, now: Clock) {
        self.notices
            .retain(|n| n.expires_at.is_none_or(|t| t > now.mono));
    }

    pub(super) fn show(&mut self, view: View, now: Clock, out: &mut Out) {
        if let View::Board(id) | View::Document(id, _) = &view {
            let id = *id;
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
            env: ProjectEnv::default(),
            created: now.wall,
            last_active: now.wall,
        }));
        out.touch(id);
        out.push(super::definitions::read_config(
            id,
            self.workspaces
                .last()
                .map(|w| w.project.root.clone())
                .unwrap_or_default(),
        ));
        self.view_stack.push(View::Board(id));
    }

    /// Drop a project and everything the core remembers about its
    /// sessions: views, flights, and its place in the Codex queue. A
    /// pending discovery for one of its records would otherwise block
    /// every later Codex launch until it expired.
    fn remove_project(&mut self, id: ProjectId, now: Clock, out: &mut Out) {
        let Some(pos) = self.workspaces.iter().position(|w| w.project.id == id) else {
            return;
        };
        let workspace = self.workspaces.remove(pos);
        out.push(Effect::Delete(id));
        let gone = |r: RecordId| workspace.sessions.iter().any(|s| s.id == r);
        self.view_stack.retain(|v| match v {
            View::Board(p) | View::Document(p, _) => *p != id,
            View::Session(r) => !gone(*r),
            View::Switchboard | View::WorkingSet(_) => true,
        });
        self.in_flight.retain(|f| !gone(f.id));
        self.codex_queue.retain(|r| !gone(*r));
        self.quiet.retain(|r| !gone(*r));
        if self.codex_pending.is_some_and(gone) {
            self.codex_pending = None;
        }
        self.advance_codex_queue(now, out);
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
        // A running process is left alone (removing a record is not a
        // kill); only a gone pane's scrollback is dropped with the record.
        if self.host_status(id).is_none() {
            out.push(Effect::Forget(HostId(id.host_name())));
        }
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
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    /// Every working set, in the user's order.
    #[must_use]
    pub fn working_sets(&self) -> &[WorkingSet] {
        &self.views.sets
    }
    /// One working set: its cards and where they sit.
    #[must_use]
    pub fn working_set(&self, id: SetId) -> Option<&WorkingSet> {
        self.views.sets.iter().find(|s| s.id == id)
    }
    /// The sessions on a working set, for the card refreshes.
    #[must_use]
    pub fn working_set_sessions(&self, id: SetId) -> Vec<RecordId> {
        self.working_set(id)
            .map(|s| {
                s.items
                    .iter()
                    .filter_map(|i| match &i.target {
                        PinTarget::Session(id) => Some(*id),
                        PinTarget::File(..) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    /// The working sets holding `target`.
    #[must_use]
    pub fn sets_holding(&self, target: &PinTarget) -> Vec<SetId> {
        self.views
            .sets
            .iter()
            .filter(|s| s.items.iter().any(|i| i.target == *target))
            .map(|s| s.id)
            .collect()
    }

    /// The project last shown (most recent `last_active`); the one
    /// exclusive mode keeps visible.
    #[must_use]
    pub fn active_project(&self) -> Option<ProjectId> {
        self.workspaces
            .iter()
            .map(|w| &w.project)
            .max_by_key(|p| p.last_active)
            .map(|p| p.id)
    }

    /// Whether the UI may show this project at all.
    #[must_use]
    pub fn project_visible(&self, id: ProjectId) -> bool {
        !self.settings.exclusive || self.active_project() == Some(id)
    }

    /// Workspaces the UI may show, in stored order.
    pub fn visible_workspaces(&self) -> impl Iterator<Item = &Workspace> {
        self.workspaces
            .iter()
            .filter(|w| self.project_visible(w.project.id))
    }

    /// Project edits, the document hand-offs, and the preferences, split
    /// out of `dispatch` for length.
    fn files_and_settings(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::RenameProject(id, name) => {
                self.edit_project(id, out, |p| p.name = name);
            }
            AppAction::PinDocument(id, path) => self.edit_project(id, out, |p| {
                if !p.pinned.contains(&path) {
                    p.pinned.push(path);
                }
            }),
            AppAction::UnpinDocument(id, path) => {
                self.edit_project(id, out, |p| p.pinned.retain(|d| *d != path));
            }
            AppAction::ShowDocument(pid, path) => self.show(View::Document(pid, path), now, out),
            AppAction::OpenDocument(path) => out.push(Effect::OpenPath(path)),
            AppAction::RevealDocument(path) => out.push(Effect::Reveal(path)),
            AppAction::OpenInEditor(path) => out.push(Effect::OpenInEditor {
                editor: self.settings.editor.clone(),
                path,
            }),
            AppAction::SetEditor(editor) => {
                self.update_settings(out, |s| editor.trim().clone_into(&mut s.editor));
            }
            AppAction::SetGlobalEnv(env) => self.update_settings(out, |s| s.env = env),
            AppAction::SetProjectEnv(id, env) => self.edit_project(id, out, |p| p.env = env),
            AppAction::StoreSecret { scope, name, value } => out.push(Effect::StoreSecret {
                account: scope.account(&name),
                value,
            }),
            AppAction::DeleteSecret { scope, name } => {
                out.push(Effect::DeleteSecret(scope.account(&name)));
            }
            AppAction::SetTheme(theme) => self.update_settings(out, |s| s.theme = theme),
            AppAction::SetExclusive(on) => self.update_settings(out, |s| s.exclusive = on),
            AppAction::SetFilesOpen(on) => self.update_settings(out, |s| s.files_open = on),
            AppAction::SetOpenTerminalOnLaunch(on) => {
                self.update_settings(out, |s| s.open_terminal_on_launch = on);
            }
            AppAction::SetSideTab(tab) => self.update_settings(out, |s| s.side_tab = tab),
            AppAction::ProjectConfigRead { project, result } => {
                self.project_config_read(project, result, now, out);
            }
            AppAction::ApproveDefinition(id) => self.approve_definition(id, out),
            AppAction::RevokeApproval(id) => self.revoke_approval(id, out),
            AppAction::StoreLoaded(_)
            | AppAction::SaveFinished(..)
            | AppAction::HostUnavailable(_)
            | AppAction::HostListed(_)
            | AppAction::ShowSwitchboard
            | AppAction::ShowBoard(_)
            | AppAction::ShowSession(_)
            | AppAction::Back
            | AppAction::DismissNotice
            | AppAction::Tick
            | AppAction::AddProject { .. }
            | AppAction::RemoveProject(_)
            | AppAction::NewSession { .. }
            | AppAction::RenameSession(..)
            | AppAction::SetSessionNotes(..)
            | AppAction::SetAutostart(..)
            | AppAction::MoveCard { .. }
            | AppAction::ReturnToSession(_)
            | AppAction::SendInput { .. }
            | AppAction::Interrupt(_)
            | AppAction::KillSession(_)
            | AppAction::RestartSession(_)
            | AppAction::RemoveSession(_)
            | AppAction::Failed(_)
            | AppAction::LaunchPrepared { .. }
            | AppAction::TranscriptChecked { .. }
            | AppAction::CloneSession { .. }
            | AppAction::TranscriptCloned { .. }
            | AppAction::Spawned { .. }
            | AppAction::Attached { .. }
            | AppAction::Discovered { .. }
            | AppAction::ShowWorkingSet(_)
            | AppAction::AddToWorkingSet { .. }
            | AppAction::RemoveFromWorkingSet { .. }
            | AppAction::NewWorkingSet { .. }
            | AppAction::RenameWorkingSet { .. }
            | AppAction::DeleteWorkingSet(_)
            | AppAction::PlacePin { .. }
            | AppAction::Events(_) => unreachable!("dispatched by `dispatch` itself"),
        }
    }

    fn update_settings(&mut self, out: &mut Out, change: impl FnOnce(&mut Settings)) {
        let mut next = self.settings.clone();
        change(&mut next);
        if next != self.settings {
            self.settings = next;
            if !self.read_only {
                out.push(Effect::SaveSettings(self.settings.clone()));
            }
        }
    }

    #[must_use]
    pub fn host_status(&self, id: RecordId) -> Option<&HostStatus> {
        let name = id.host_name();
        self.host.iter().find(|h| h.id.0 == name)
    }
    /// Emit an effect aimed at a record's running pane, or a notice when
    /// there is none.
    fn aim_at_pane(&mut self, id: RecordId, out: &mut Out, effect: impl FnOnce(HostId) -> Effect) {
        let host = self
            .host_status(id)
            .filter(|h| matches!(h.liveness, Liveness::Running { .. }))
            .map(|h| h.id.clone());
        if let Some(host) = host {
            out.push(effect(host));
        } else {
            let name = self.session_name(id);
            self.error(format!("{name} is not running; return to it first"));
        }
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
                Activity::Working | Activity::Unknown if self.quiet.contains(&id) => {
                    CardState::Idle
                }
                Activity::Working => CardState::Working,
                Activity::Idle | Activity::Ended => CardState::Idle,
                // Nothing reported yet: a Claude Code pane is starting until
                // its first hook; Codex has no hooks, so it is simply busy;
                // a shell sits at its prompt.
                Activity::Unknown => match record.kind {
                    SessionKind::Agent(AgentKind::ClaudeCode) => CardState::Starting,
                    SessionKind::Agent(AgentKind::Codex) => CardState::Working,
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
    /// What the last read of a project's definition file found; `None`
    /// before the first read.
    #[must_use]
    pub fn config_status(&self, project: ProjectId) -> Option<&ConfigStatus> {
        self.config_status
            .iter()
            .find(|(p, _)| *p == project)
            .map(|(_, s)| s)
    }
    /// A project's commands and services: services first, then by the
    /// saved order. These are the Run tab's and run bar's entries.
    #[must_use]
    pub fn run_entries(&self, project: ProjectId) -> Vec<&SessionRecord> {
        let mut entries: Vec<&SessionRecord> = self
            .workspace(project)
            .map(|w| {
                w.sessions
                    .iter()
                    .filter(|s| matches!(s.kind, SessionKind::Command | SessionKind::Service))
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by_key(|s| (s.kind != SessionKind::Service, s.layout.order));
        entries
    }
    /// A project's agents and shells: the board's cards, waiting first.
    #[must_use]
    pub fn board_sessions(&self, project: ProjectId) -> Vec<&SessionRecord> {
        self.sessions_sorted(project)
            .into_iter()
            .filter(|s| matches!(s.kind, SessionKind::Agent(_) | SessionKind::Shell))
            .collect()
    }
    /// The card state as text, with the reason when the session waits:
    /// "waiting on you: permission for Bash".
    #[must_use]
    pub fn state_text(&self, id: RecordId) -> String {
        let state = self.card_state(id);
        let label = state.label();
        match self
            .session(id)
            .filter(|_| state == CardState::WaitingOnYou)
            .and_then(|s| s.activity_reason.as_deref())
        {
            Some(reason) => format!("{label}: {reason}"),
            None => label,
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
        self.visible_workspaces()
            .flat_map(|w| &w.sessions)
            .filter(|s| self.card_state(s.id) == CardState::WaitingOnYou)
            .count()
    }
}
