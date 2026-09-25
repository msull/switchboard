//! The durable data model. Everything here is serialized into the
//! workspace records, so changes need a `SCHEMA_VERSION` bump and a
//! migration in the store adapter.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bump when the on-disk shape changes incompatibly.
pub const SCHEMA_VERSION: u32 = 8;

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
    /// Show only the active project, for screen sharing. Superseded by
    /// spaces; kept so older files read and write unchanged.
    pub exclusive: bool,
    /// The space being worked in: the one the rail shows, and the one
    /// the next launch opens on.
    pub space: SpaceId,
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
    /// The side panel sits on the left of the content, between the
    /// rail and the page, instead of on the right.
    pub side_left: bool,
    /// Where the main window was last seen, so the next launch opens it
    /// there; `None` until it has reported a position.
    pub main_window: Option<WindowFrame>,
    /// Zoom per display, by the display's name: a window is drawn at
    /// the zoom of the display its centre is on, so the dashboard on a
    /// big screen and the sessions on a laptop can each be read.
    pub monitor_zoom: Vec<MonitorZoom>,
    /// Where each project's file side starts: a directory under the
    /// project root shown as the tree's top, for projects whose side
    /// has been narrowed. Absent means the project root.
    pub file_roots: Vec<FileRoot>,
    /// Sessions shown in windows of their own, brought back where they
    /// were on the next launch. A session is drawn as a page in one
    /// place only: its window while it has one, else the main window.
    pub popouts: Vec<Popout>,
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
    /// How many review rounds a plan review workflow runs before it
    /// stops and asks; a definition may override it.
    pub workflow_round_cap: u32,
    /// The user's own workflow definitions, by name. The built-in one
    /// (`WorkflowDefinition::default`) is always available under its
    /// own name and never stored.
    pub workflows: Vec<WorkflowDefinition>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::default(),
            exclusive: false,
            space: SpaceId::DEFAULT,
            editor: String::new(),
            env: Vec::new(),
            files_open: false,
            side_tab: SideTab::default(),
            side_left: false,
            main_window: None,
            monitor_zoom: Vec::new(),
            file_roots: Vec::new(),
            popouts: Vec::new(),
            last_view: SavedView::default(),
            open_terminal_on_launch: false,
            prompt_box: true,
            voice: VoiceSettings::default(),
            workflow_round_cap: DEFAULT_ROUND_CAP,
            workflows: Vec::new(),
        }
    }
}

impl Settings {
    /// The definition called `name`: the user's copy when they have one,
    /// the built-in when the name is its, otherwise none.
    #[must_use]
    pub fn workflow(&self, name: &str) -> Option<WorkflowDefinition> {
        self.workflows
            .iter()
            .find(|d| d.name == name)
            .cloned()
            .or_else(|| (name == BUILTIN_WORKFLOW).then(WorkflowDefinition::default))
    }
}

/// Rounds a review runs before stopping to ask, unless a definition
/// says otherwise.
pub const DEFAULT_ROUND_CAP: u32 = 4;

/// Name of the definition that ships with the app.
pub const BUILTIN_WORKFLOW: &str = "Plan review";

/// The first line a reviewer writes when it has nothing further to say.
/// A string compare, never a reading of prose.
pub const NO_FEEDBACK_LINE: &str = "No further feedback.";

/// What a workflow says to its agents and how far it goes. Prompts are
/// templates: `{plan}`, `{feedback}`, `{response}`, `{round}`, `{cap}`,
/// and `{no_feedback}` are filled per round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkflowDefinition {
    pub name: String,
    /// What the reviewer runs as; the planner is always a clone.
    pub reviewer: AgentKind,
    /// The reviewer's first prompt.
    pub review_first: String,
    /// The reviewer's prompt for every later round.
    pub review_round: String,
    /// The planner clone's prompt, every round.
    pub respond: String,
    /// The planner clone's prompt for a round of the user's own
    /// feedback; `{text}` is what they wrote.
    pub respond_to_user: String,
    /// What the original planning session is told at the end.
    pub handoff: String,
    /// The reviewer's "nothing further" first line.
    pub no_feedback: String,
    /// Overrides the setting when set.
    pub cap: Option<u32>,
}

impl Default for WorkflowDefinition {
    fn default() -> Self {
        Self {
            name: BUILTIN_WORKFLOW.into(),
            reviewer: AgentKind::Codex,
            review_first: "Review the plan at {plan}. Write all of your feedback into {feedback} \
                (it is not committed). If the plan needs no changes, write {feedback} with \
                exactly this first line and nothing else: {no_feedback}"
                .into(),
            review_round: "The plan's author responded to your feedback in {response} and updated \
                {plan} where they agreed. Read the response, re-review the plan, and write any \
                further feedback into {feedback}. If nothing further is needed, write {feedback} \
                with exactly this first line and nothing else: {no_feedback}"
                .into(),
            respond: "I've used an external reviewer on the plan at {plan}. Their feedback is in \
                {feedback}. Look at it and provide a response in markdown at {response}. Accept \
                valid items by updating the plan; reject the rest with your reasoning in the \
                response. Never mention the reviewer in the plan."
                .into(),
            respond_to_user: "I've reviewed the plan at {plan} myself. My feedback:\n\n{text}\n\n\
                Provide a response in markdown at {response}. Accept valid items by updating \
                the plan; reject the rest with your reasoning in the response."
                .into(),
            handoff:
                "I've revised the plan at {plan}. Enter plan mode and prepare to implement it."
                    .into(),
            no_feedback: NO_FEEDBACK_LINE.into(),
            cap: None,
        }
    }
}

impl WorkflowDefinition {
    /// Fill a template for one round.
    #[must_use]
    pub fn render(
        &self,
        template: &str,
        round: &Round,
        plan: &std::path::Path,
        cap: u32,
    ) -> String {
        template
            .replace("{plan}", &plan.display().to_string())
            .replace("{feedback}", &round.feedback.display().to_string())
            .replace("{response}", &round.response.display().to_string())
            .replace("{round}", &round.n.to_string())
            .replace("{cap}", &cap.to_string())
            .replace("{no_feedback}", &self.no_feedback)
    }
}

/// Switchboard's own id for a workflow run. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WorkflowId(pub Uuid);

impl WorkflowId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new()
    }
}

/// One plan review in progress or finished: which sessions it drives,
/// which file it is about, and every round so far. Lives in the
/// project's workspace file beside the sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: WorkflowId,
    pub project: ProjectId,
    /// Name of the definition used; its prompts are read each round, so
    /// an edit applies to the next one.
    pub definition: String,
    /// The planning session, never touched until the handoff.
    pub source: RecordId,
    pub plan: PathBuf,
    /// The clone of `source` that answers feedback; `None` until the
    /// provider-side copy exists.
    #[serde(default)]
    pub planner: Option<RecordId>,
    pub reviewer: RecordId,
    #[serde(default)]
    pub rounds: Vec<Round>,
    pub state: RunState,
    pub cap: u32,
    /// The round files have been deleted from the project.
    #[serde(default)]
    pub cleaned: bool,
    pub created: SystemTime,
    pub updated: SystemTime,
}

impl WorkflowRun {
    /// The round in progress or last finished.
    #[must_use]
    pub fn current(&self) -> Option<&Round> {
        self.rounds.last()
    }
    /// Which record the run is waiting on, if any.
    #[must_use]
    pub fn awaiting(&self) -> Option<RecordId> {
        match self.state {
            RunState::AwaitingFeedback => Some(self.reviewer),
            RunState::AwaitingResponse => self.planner,
            _ => None,
        }
    }
    /// The file the run is waiting for, if any.
    #[must_use]
    pub fn awaited_file(&self) -> Option<&PathBuf> {
        let round = self.current()?;
        match self.state {
            RunState::AwaitingFeedback => Some(&round.feedback),
            RunState::AwaitingResponse => Some(&round.response),
            _ => None,
        }
    }
    /// Every round file the run named, for cleanup.
    #[must_use]
    pub fn round_files(&self) -> Vec<PathBuf> {
        self.rounds
            .iter()
            .flat_map(|r| [r.feedback.clone(), r.response.clone()])
            .collect()
    }
}

/// One exchange: feedback from the reviewer (or the user), the planner's
/// response, and what the reviewer decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    /// 1-based.
    pub n: u32,
    pub feedback: PathBuf,
    pub response: PathBuf,
    /// The reviewer's decision once the feedback file settled.
    #[serde(default)]
    pub verdict: Option<Verdict>,
    /// The user wrote this round's feedback themselves; it is sent in
    /// the prompt and kept here rather than in a file.
    #[serde(default)]
    pub user_feedback: Option<String>,
    /// The response file settled.
    #[serde(default)]
    pub responded: bool,
    /// The plan, feedback, and response were copied into the data
    /// directory at the end of the round.
    #[serde(default)]
    pub snapshot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// The feedback file has points to answer.
    Changes,
    /// The reviewer wrote the no-feedback line.
    Nothing,
}

/// Where a run stands. The round in question is the last of `rounds`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunState {
    /// The reviewer was launched; the planner clone is being made.
    Starting,
    /// The reviewer is writing this round's feedback.
    AwaitingFeedback,
    /// The planner is writing this round's response.
    AwaitingResponse,
    /// The reviewer said nothing further.
    Converged,
    /// The cap was reached with feedback still coming.
    AtCap,
    /// Stopped by the user or by a failure; the reason is shown.
    Paused(String),
    /// The user has reviewed the plan.
    Finalized,
    /// The plan went back to the source session.
    HandedOff,
}

impl RunState {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Starting => "starting".into(),
            Self::AwaitingFeedback => "reviewing".into(),
            Self::AwaitingResponse => "responding".into(),
            Self::Converged => "converged".into(),
            Self::AtCap => "at cap".into(),
            Self::Paused(why) => format!("paused: {why}"),
            Self::Finalized => "finalized".into(),
            Self::HandedOff => "handed off".into(),
        }
    }
    /// The run is waiting on an agent.
    #[must_use]
    pub fn waiting(&self) -> bool {
        matches!(self, Self::AwaitingFeedback | Self::AwaitingResponse)
    }
}

/// How the finished plan goes back to the source session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandoffMode {
    /// Send the handoff prompt as is.
    AsIs,
    /// Send `/compact` first, then the prompt.
    Compact,
    /// A fresh session in the same directory with the prompt.
    Fresh,
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
    /// Display name of the screen for the caption bar and the preview
    /// panel; blank means the screen the window is on.
    pub overlay_screen: String,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            trigger: String::new(),
            openai_model: String::new(),
            captions: true,
            overlay_screen: String::new(),
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
    Workflow(WorkflowId),
}

/// The zoom of windows on one display, in percent (100 is native).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorZoom {
    /// The display's name as the system gives it; "main" where the
    /// system lists no displays.
    pub monitor: String,
    pub percent: u32,
}

/// A project's file side narrowed to a directory under its root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRoot {
    pub project: ProjectId,
    /// Relative to the project root.
    pub dir: PathBuf,
}

/// A session in a window of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Popout {
    pub session: RecordId,
    /// Where the window was last seen, in screen points; `None` until
    /// the window has reported a position.
    #[serde(default)]
    pub frame: Option<WindowFrame>,
}

/// A window's outer rectangle in whole native screen points, and the
/// display it was on, so the frame is used again only while that
/// display is attached ("" when unknown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowFrame {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    #[serde(default)]
    pub monitor: String,
}

/// Schema of `views.json`, bumped like [`SCHEMA_VERSION`] when a type
/// below changes shape. v2 gave every set an id; a v1 file reads with
/// fresh ids. v3 added spaces and put every set in one; a v2 file
/// reads with everything in the default space.
pub const VIEWS_SCHEMA_VERSION: u32 = 3;

/// Switchboard's own id for a space. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SpaceId(pub Uuid);

impl SpaceId {
    /// The space everything is in until the user makes another: records
    /// from before spaces existed read as its members without a step.
    pub const DEFAULT: Self = Self(Uuid::from_u128(1));

    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SpaceId {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What the UI calls a workspace: the top level, owning projects and
/// working sets, each of which is in exactly one. The rail shows one
/// space at a time and nothing of the others, so a shared screen gives
/// away only the one being worked in. (`Workspace` is the older name of
/// a project's record, which this does not replace.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
}

impl Space {
    /// The space records land in before any other exists.
    #[must_use]
    pub fn default_space() -> Self {
        Self {
            id: SpaceId::DEFAULT,
            name: "Default".into(),
        }
    }
}

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
    /// In the user's order; never empty once loaded (the default space
    /// is put back if a file lists none).
    #[serde(default)]
    pub spaces: Vec<Space>,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            schema_version: VIEWS_SCHEMA_VERSION,
            sets: Vec::new(),
            spaces: vec![Space::default_space()],
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
    /// The space the set belongs to.
    #[serde(default)]
    pub space: SpaceId,
}

impl Default for WorkingSet {
    fn default() -> Self {
        Self {
            id: SetId::new(),
            name: "Working Set".into(),
            items: Vec::new(),
            space: SpaceId::DEFAULT,
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
            space: SpaceId::DEFAULT,
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
    /// The space the project belongs to.
    #[serde(default)]
    pub space: SpaceId,
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
    /// Every execution of a command or service, oldest first, the last
    /// `RUNS_KEPT` of them. Empty for agents and shells.
    #[serde(default)]
    pub runs: Vec<Run>,
    /// Glob patterns, relative to `cwd`, of the files a command
    /// produces; what a run's `artifacts` are matched against when it
    /// ends. A defined entry's come from the file (`output`).
    #[serde(default)]
    pub outputs: Vec<String>,
}

/// How many runs a record keeps; the logs of older ones are deleted
/// with them.
pub const RUNS_KEPT: usize = 20;

/// One execution of a command or service: when it ran, how it ended,
/// where its output is, and the files it declared and produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// 1-based, counting up for the record's life.
    pub n: u32,
    pub started: SystemTime,
    /// Set by the host poll that sees the pane exit, or the reconcile
    /// that finds it gone.
    #[serde(default)]
    pub ended: Option<SystemTime>,
    /// The exit code, when the host reported one.
    #[serde(default)]
    pub exit: Option<i32>,
    /// The run's log file name under the data directory's `scrollback/`.
    pub log: String,
    /// Files matching the record's `outputs` that were modified during
    /// the run, absolute, found when the run ended.
    #[serde(default)]
    pub artifacts: Vec<PathBuf>,
}

impl Run {
    /// How long the run took, or has been running as of `now`.
    #[must_use]
    pub fn duration(&self, now: SystemTime) -> Option<std::time::Duration> {
        self.ended.unwrap_or(now).duration_since(self.started).ok()
    }
    /// Still running as far as the record knows.
    #[must_use]
    pub fn open(&self) -> bool {
        self.ended.is_none()
    }
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
    /// The most recent run, if the record ever ran.
    #[must_use]
    pub fn last_run(&self) -> Option<&Run> {
        self.runs.last()
    }
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
    #[serde(default)]
    pub workflows: Vec<WorkflowRun>,
}

impl Workspace {
    #[must_use]
    pub fn new(project: Project) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            project,
            sessions: Vec::new(),
            workflows: Vec::new(),
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
