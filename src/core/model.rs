//! The durable data model. Everything here is serialized into the
//! workspace records, so changes need a `SCHEMA_VERSION` bump and a
//! migration in the store adapter.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bump when the on-disk shape changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;

/// How the UI picks its colours: follow the system, or force one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Auto,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Light, Self::Dark];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

/// One environment variable of a layer. A plain variable keeps its value
/// here; a secret keeps only its name, the value lives in the secret
/// store under the layer's account name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
    pub secret: bool,
}

/// A project's environment: its own variables and whether its `.env`
/// files are read (opt-in, since a repository ships them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectEnv {
    pub vars: Vec<EnvVar>,
    pub load_dotenv: bool,
    /// Relative to the root; `.env` when empty.
    pub dotenv_files: Vec<String>,
}

impl ProjectEnv {
    /// The files to read when `load_dotenv` is on.
    #[must_use]
    pub fn files(&self) -> Vec<String> {
        if self.dotenv_files.is_empty() {
            vec![".env".into()]
        } else {
            self.dotenv_files.clone()
        }
    }
}

/// App-wide preferences: one `settings.json` per data directory, not per
/// project. Unknown fields are kept out and missing ones default, so the
/// file needs no schema version.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    /// Show only the active project, for screen sharing.
    pub exclusive: bool,
    /// Command that opens a file in the editor (`code`, `zed`, `cursor`,
    /// `subl`); it gets the path as its one argument. Blank means the
    /// system text editor via `open -t`.
    pub editor: String,
    /// Variables every session gets, under the project's own.
    pub env: Vec<EnvVar>,
    /// The file side is shown next to sessions (boards always have it).
    pub files_open: bool,
}

/// Switchboard's own id for a project. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProjectId(pub Uuid);

/// Switchboard's own id for a session record. Distinct from the host id
/// (tmux session name derives from it) and the provider's resume id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecordId(pub Uuid);

impl RecordId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    /// The name of the host (tmux) session for this record.
    #[must_use]
    pub fn host_name(&self) -> String {
        format!("sb-{}", self.0.simple())
    }
}

impl Default for RecordId {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub root: PathBuf,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    /// Pinned document cards, paths relative to `root`.
    #[serde(default)]
    pub pinned: Vec<PathBuf>,
    #[serde(default)]
    pub env: ProjectEnv,
    pub created: SystemTime,
    /// Most recent time this project was active; drives switcher order.
    pub last_active: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    ClaudeCode,
    Codex,
}

impl AgentKind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionKind {
    Agent(AgentKind),
    Command,
    Service,
    Shell,
}

/// What to run. Agents get a composed argv; user-authored commands stay
/// as the string the user wrote plus the shell that runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Launch {
    /// The user's login shell, interactive.
    Shell,
    Argv(Vec<String>),
    Command {
        command: String,
        shell: String,
    },
}

/// The provider's handle for resuming an agent conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResumeHandle {
    ClaudeCode {
        session_id: Uuid,
        /// Transcript path as last seen; existence is the resume preflight.
        transcript: Option<PathBuf>,
    },
    Codex {
        rollout_id: String,
        transcript: Option<PathBuf>,
    },
}

impl ResumeHandle {
    #[must_use]
    pub fn transcript(&self) -> Option<&PathBuf> {
        match self {
            Self::ClaudeCode { transcript, .. } | Self::Codex { transcript, .. } => {
                transcript.as_ref()
            }
        }
    }
    #[must_use]
    pub fn provider_id(&self) -> String {
        match self {
            Self::ClaudeCode { session_id, .. } => session_id.to_string(),
            Self::Codex { rollout_id, .. } => rollout_id.clone(),
        }
    }
}

/// Where a card sits on the board.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardLayout {
    pub order: u32,
    #[serde(default)]
    pub group: Option<String>,
}

/// Coarse state derived from hook events, remembered between polls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Activity {
    #[default]
    Unknown,
    Working,
    WaitingOnYou,
    Idle,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: RecordId,
    pub project: ProjectId,
    pub name: String,
    pub kind: SessionKind,
    pub cwd: PathBuf,
    pub launch: Launch,
    #[serde(default)]
    pub env_profile: Option<String>,
    pub created: SystemTime,
    pub last_seen: SystemTime,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub resume: Option<ResumeHandle>,
    /// Set for services only; honored only for records from the private store.
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub layout: CardLayout,
    /// Last activity derived from events, and when it was applied. Events
    /// older than `last_event_at` are ignored (ordering rule).
    #[serde(default)]
    pub activity: Activity,
    #[serde(default)]
    pub last_event_at: Option<SystemTime>,
    /// Last exit code seen from the host.
    #[serde(default)]
    pub last_exit: Option<i32>,
    /// Marked when a resume was attempted and failed, or the transcript
    /// preflight failed.
    #[serde(default)]
    pub not_resumable: bool,
    #[serde(default)]
    pub scrollback: Option<PathBuf>,
}

/// One project with its sessions: one file in the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub schema_version: u32,
    pub project: Project,
    #[serde(default)]
    pub sessions: Vec<SessionRecord>,
}

impl Workspace {
    #[must_use]
    pub fn new(project: Project) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            project,
            sessions: Vec::new(),
        }
    }
}

/// What a session card shows. Derived, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardState {
    /// Record exists, no host process. Click brings it back.
    NotRunning,
    /// Provider transcript is gone or a resume failed.
    NotResumable,
    Working,
    WaitingOnYou,
    /// Alive at a prompt, nothing pending.
    Idle,
    /// Process ended; pane kept.
    Exited(Option<i32>),
}

impl CardState {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::NotRunning => "not running".into(),
            Self::NotResumable => "not resumable".into(),
            Self::Working => "working".into(),
            Self::WaitingOnYou => "waiting on you".into(),
            Self::Idle => "idle".into(),
            Self::Exited(Some(c)) => format!("exited ({c})"),
            Self::Exited(None) => "exited".into(),
        }
    }
    /// Sort key: waiting first, then working, idle, exited, not running.
    #[must_use]
    pub fn rank(&self) -> u8 {
        match self {
            Self::WaitingOnYou => 0,
            Self::Working => 1,
            Self::Idle => 2,
            Self::Exited(_) => 3,
            Self::NotRunning => 4,
            Self::NotResumable => 5,
        }
    }
}
