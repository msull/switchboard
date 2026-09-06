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

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::model::{
    AgentKind, CardState, Launch, ProjectId, RecordId, ResumeHandle, SessionKind, SessionRecord,
    Workspace,
};
use crate::ports::agent::AgentLaunch;
use crate::ports::events::SessionEvent;
use crate::ports::host::{HostId, HostStatus, SpawnSpec};
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
    OpenPath(PathBuf),
    Reveal(PathBuf),
}

/// Top-level state. Plain data and pure methods only.
#[derive(Debug, Default)]
pub struct AppCore {
    workspaces: Vec<Workspace>,
    view_stack: Vec<View>,
    notice: Option<Notice>,
    host_error: Option<String>,
    read_only: bool,
    /// Latest host status per session, from the last poll.
    host: Vec<HostStatus>,
    /// Records with a launch or resume in flight (idempotent return).
    in_flight: Vec<RecordId>,
    /// Codex launches are serialized: at most one discovery pending.
    codex_pending: Option<RecordId>,
}

impl AppCore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The single entry point. Implemented in Milestone 1 by the core
    /// work item; see the module docs for the contract.
    pub fn dispatch(&mut self, action: AppAction, now: Clock) -> Vec<Effect> {
        let _ = (action, now, &self.in_flight, &self.codex_pending);
        Vec::new()
    }

    /// Populate state directly, bypassing dispatch. For UI tests and the
    /// demo launcher only; the app itself always goes through `dispatch`.
    pub fn seed(&mut self, workspaces: Vec<Workspace>, host: Vec<HostStatus>) {
        self.workspaces = workspaces;
        self.host = host;
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
        let _ = id;
        CardState::NotRunning
    }
    #[must_use]
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }
    #[must_use]
    pub fn host_error(&self) -> Option<&str> {
        self.host_error.as_deref()
    }
    #[must_use]
    pub fn read_only(&self) -> bool {
        self.read_only
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
