//! The durable data model. Everything here is serialized into the
//! workspace records, so changes need a `SCHEMA_VERSION` bump and a
//! migration in the store adapter.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bump when the on-disk shape changes incompatibly.
pub const SCHEMA_VERSION: u32 = 5;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)] // independent preferences
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
    /// Which tab the side panel shows.
    pub side_tab: SideTab,
    /// The screen that was showing when the app last ran, so it reopens
    /// there. A project or session that no longer exists falls back to
    /// the switchboard.
    pub last_view: SavedView,
    /// Open the terminal window when an agent starts or resumes. Off,
    /// the agent runs in its pane and the window opens only on Open.
    pub open_terminal_on_launch: bool,
    /// Agent sessions get the Prompt Box editor (voice, AI tools, Save)
    /// as their message box instead of the plain one.
    pub prompt_box: bool,
    /// What the embedded Prompt Box needs beyond the key.
    pub voice: VoiceSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default(),
            exclusive: false,
            editor: String::new(),
            env: Vec::new(),
            files_open: false,
            side_tab: SideTab::default(),
            last_view: SavedView::default(),
            open_terminal_on_launch: false,
            prompt_box: true,
            voice: VoiceSettings::default(),
        }
    }
}

/// Settings for the embedded Prompt Box, kept here rather than in the
/// standalone app's file so the two never share state. The `OpenAI` key
/// is in the Keychain under [`VOICE_KEY_ACCOUNT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceSettings {
    /// The word that starts a voice command; blank means Prompt Box's own.
    pub trigger: String,
    /// The `OpenAI` model for AI rewrites; blank means Prompt Box's own.
    pub openai_model: String,
    /// Show the on-screen captions while listening.
    pub captions: bool,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            trigger: String::new(),
            openai_model: String::new(),
            captions: true,
        }
    }
}

/// Keychain account of the `OpenAI` key the embedded Prompt Box uses.
pub const VOICE_KEY_ACCOUNT: &str = "switchboard/openai-api-key";

/// A screen as remembered in `settings.json`: only what can be found
/// again after a restart. A document preview remembers its board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SavedView {
    #[default]
    Switchboard,
    Board(ProjectId),
    Session(RecordId),
    /// The first working set, from before sets had ids; kept so an
    /// older `settings.json` still reads.
    WorkingSet,
    Set(SetId),
}

/// Schema of `views.json`, bumped like [`SCHEMA_VERSION`] when a type
/// below changes shape. v2 gave every set an id; a v1 file reads with
/// fresh ids.
pub const VIEWS_SCHEMA_VERSION: u32 = 2;

/// Switchboard's own id for a working set. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SetId(pub Uuid);

impl SetId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SetId {
    fn default() -> Self {
        Self::new()
    }
}

/// The user-arranged views, one file for all of them: `views.json` in
/// the data directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Views {
    pub schema_version: u32,
    #[serde(default)]
    pub sets: Vec<WorkingSet>,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            schema_version: VIEWS_SCHEMA_VERSION,
            sets: Vec::new(),
        }
    }
}

/// A grid of the things the user is working on right now, across
/// projects. Items refer to records and files; the set owns nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingSet {
    #[serde(default)]
    pub id: SetId,
    pub name: String,
    #[serde(default)]
    pub items: Vec<PinnedItem>,
}

impl Default for WorkingSet {
    fn default() -> Self {
        Self {
            id: SetId::new(),
            name: "Working Set".into(),
            items: Vec::new(),
        }
    }
}

impl WorkingSet {
    /// An empty set called `name`.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            id: SetId::new(),
            name: name.into(),
            items: Vec::new(),
        }
    }
}

/// One card of a working set and where it sits on the grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedItem {
    pub target: PinTarget,
    pub rect: GridRect,
}

/// What a working-set card shows. A target appears in a set at most
/// once, so it is also the item's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PinTarget {
    Session(RecordId),
    /// A file of a project, by its path relative to the root.
    File(ProjectId, PathBuf),
}

/// A rectangle in grid units: the working set's grid, not pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl GridRect {
    #[must_use]
    pub fn overlaps(self, other: GridRect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

/// The tabs of the side panel next to a board or session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SideTab {
    #[default]
    Files,
    /// Commands and services: definitions, approval, output.
    Run,
    /// The session's notes. Only beside a session; elsewhere the side
    /// shows Files instead.
    Notes,
}

impl SideTab {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Run => "Run",
            Self::Notes => "Notes",
        }
    }
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
    /// Folders the file side shows despite the root's `.gitignore`, as
    /// `.switchboard/project.json` last declared them (`show`).
    #[serde(default)]
    pub shown: Vec<PathBuf>,
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
    /// Set for services only, by the user. `effective_autostart` is what
    /// the reconcile honors; a defined record's request lives in `source`.
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub layout: CardLayout,
    /// Last activity derived from events, and when it was applied. Events
    /// older than `last_event_at` are ignored (ordering rule).
    #[serde(default)]
    pub activity: Activity,
    /// Why the activity is what it is, in a few words the card can show:
    /// "question", "permission for Bash", "rate limit". Set with the
    /// activity and cleared with it, so it is never stale.
    #[serde(default)]
    pub activity_reason: Option<String>,
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
    /// Set when the record comes from the project's definition file. The
    /// file owns name, command, and cwd; the record owns everything else.
    #[serde(default)]
    pub source: Option<Definition>,
    /// The definition hash the user approved. The record may run only
    /// while this equals `source.hash`; an edit to the file changes the
    /// hash and so drops the approval.
    #[serde(default)]
    pub approved_hash: Option<String>,
    /// What the last discard replaced, so it can be undone until the
    /// next message goes into the session.
    #[serde(default)]
    pub discard: Option<Discarded>,
}

/// A discard cut the conversation back to before one of the user's
/// prompts, in place: the record resumes through a copy and this keeps
/// what it resumed through before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discarded {
    /// The handle the session resumed through before the discard; the
    /// provider's file behind it is never touched.
    pub previous: ResumeHandle,
    /// The turn the conversation was cut before.
    pub before: usize,
    /// That turn's prompt, primed as the draft.
    pub prompt: String,
}

/// Where a defined record's definition stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// A user-created record: nothing to approve.
    NotApplicable,
    /// Listed from the file, never approved.
    Pending,
    Approved,
    /// Approved once, but the definition changed since.
    Changed,
    /// No longer in the file.
    Orphaned,
}

impl SessionRecord {
    #[must_use]
    pub fn approval(&self) -> Approval {
        match &self.source {
            None => Approval::NotApplicable,
            Some(d) if d.orphaned => Approval::Orphaned,
            Some(d) => match &self.approved_hash {
                None => Approval::Pending,
                Some(h) if *h == d.hash => Approval::Approved,
                Some(_) => Approval::Changed,
            },
        }
    }
    /// Whether the record may be launched at all.
    #[must_use]
    pub fn runnable(&self) -> bool {
        matches!(
            self.approval(),
            Approval::NotApplicable | Approval::Approved
        )
    }
    /// Autostart as the reconcile sees it: the user's own flag for their
    /// records, the file's request for defined ones once approved.
    #[must_use]
    pub fn effective_autostart(&self) -> bool {
        match &self.source {
            None => self.autostart,
            Some(d) => d.autostart && self.runnable(),
        }
    }
}

/// What the definition file said about a record, as last read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    /// The entry's name in the file; the join key across reads.
    pub name: String,
    /// Content hash of the entry (`core::entry_hash`).
    pub hash: String,
    /// Variable names the entry asks for (never values).
    #[serde(default)]
    pub env: Vec<String>,
    /// The file asked for autostart; honored only once approved.
    #[serde(default)]
    pub autostart: bool,
    /// The entry is gone from the file; the record stays for its history.
    #[serde(default)]
    pub orphaned: bool,
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
    /// A Claude Code pane is up but no hook has reported yet.
    Starting,
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
            Self::Starting => "starting".into(),
            Self::Working => "working".into(),
            Self::WaitingOnYou => "waiting on you".into(),
            Self::Idle => "idle".into(),
            Self::Exited(Some(c)) => format!("exited ({c})"),
            Self::Exited(None) => "exited".into(),
        }
    }
    /// Sort key: waiting first, then working, starting, idle, exited,
    /// not running.
    #[must_use]
    pub fn rank(&self) -> u8 {
        match self {
            Self::WaitingOnYou => 0,
            Self::Working => 1,
            Self::Starting => 2,
            Self::Idle => 3,
            Self::Exited(_) => 4,
            Self::NotRunning => 5,
            Self::NotResumable => 6,
        }
    }
}
