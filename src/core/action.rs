//! The state machine. `AppCore` holds the workspaces and derived state;
//! `dispatch` applies one action at an explicit time and returns effects.
//!
//! Contract for the implementation:
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

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::control::{ControlAction, ControlOutcome};
use crate::core::env::SecretScope;
use crate::core::grid;
use crate::core::model::{
    Activity, AgentKind, CardState, Dismissal, EnvVar, FileRoot, GridRect, HandoffMode, Launch,
    MonitorZoom, PinTarget, PinnedItem, Popout, Project, ProjectEnv, ProjectId, RecordId,
    ResumeHandle, SavedView, SessionKind, SessionRecord, SetId, SetRule, Settings, SideTab, Space,
    SpaceId, ThemeMode, VOICE_KEY_ACCOUNT, Views, VoiceSettings, WindowFrame, WorkflowDefinition,
    WorkflowId, WorkingSet, Workspace,
};
use crate::ports::agent::AgentLaunch;
use crate::ports::controller::{ControllerEvent, Direction};
use crate::ports::events::SessionEvent;
use crate::ports::host::{HostId, HostStatus, Liveness, SpawnSpec};
use crate::ports::project_config::ProjectConfig;
use crate::ports::round_files::Probed;
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

/// Down arrow then Enter: the trust question's "Yes" is the second
/// choice of its menu.
pub(crate) const TRUST_YES_KEYS: &[u8] = b"\x1b[B\r";

/// The Escape key, which interrupts an agent's turn.
pub(super) const ESCAPE: &[u8] = &[0x1b];

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
    /// A workflow run: its rounds, plan versions, and controls.
    Workflow(WorkflowId),
    /// Dispatch: every ticket, what waits on the user, the console.
    Dispatch,
    /// One Dispatch ticket by its id: stages, attempts, decisions.
    Ticket(String),
}

impl View {
    /// What to remember of this screen across a restart.
    #[must_use]
    pub fn saved(&self) -> SavedView {
        match self {
            // Tickets are Dispatch's; the next launch asks it again.
            View::Switchboard | View::Dispatch | View::Ticket(_) => SavedView::Switchboard,
            View::Board(id) | View::Document(id, _) => SavedView::Board(*id),
            View::Session(id) => SavedView::Session(*id),
            View::WorkingSet(id) => SavedView::Set(*id),
            View::Workflow(id) => SavedView::Workflow(*id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
    pub expires_at: Option<Duration>,
    /// The space the notice is about, when it names something in one:
    /// shown as it is in that space, and as a neutral line elsewhere so
    /// no name crosses the space boundary.
    pub space: Option<SpaceId>,
    /// The removed session this notice can put back (`UndoRemove`).
    pub undo: Option<RecordId>,
}

impl Notice {
    /// What a notice says outside its own space.
    pub const ELSEWHERE: &'static str = "Something in another workspace needs you";
}

/// Which of the owner's boxes sent a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Composer {
    /// The conversation view's message box or a card's quick-send line,
    /// whose draft is `UiState.input_drafts[id]`.
    Line,
    /// The session's Prompt Box editor, which empties itself when it
    /// sends.
    Editor,
}

/// Inputs from UI, workers, and the host poll.
#[derive(Debug, Clone, PartialEq)]
// The largest variant carries the loaded store, dispatched once at
// startup; boxing it would change the contract for no gain.
#[allow(clippy::large_enum_variant)]
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
    /// `AddProject` into a named space, which must be a real one: what
    /// the add dialog sends while the global space is active.
    AddProjectTo {
        name: String,
        root: PathBuf,
        space: SpaceId,
    },
    RemoveProject(ProjectId),
    RenameProject(ProjectId, String),
    /// Move a session record into another project's workspace. The
    /// record, its pane and its place on any working set are unchanged;
    /// only which board lists it.
    MoveSession(RecordId, ProjectId),
    PinDocument(ProjectId, PathBuf),
    UnpinDocument(ProjectId, PathBuf),
    /// Preview a file (absolute path) of the project.
    ShowDocument(ProjectId, PathBuf),
    /// Open a file an agent named in its conversation in the full
    /// document view, at `line` when one was given. The path is
    /// absolute and was found on disk by the UI.
    ShowFileRef {
        record: RecordId,
        path: PathBuf,
        line: Option<u32>,
    },
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
    /// A new set in the active space whose cards `rule` chooses, shown
    /// at once. Named "Recent sessions" when `name` is `None`.
    NewRuleSet {
        name: Option<String>,
        rule: SetRule,
    },
    /// Change how far back a rule set looks, clamped to
    /// [`RULE_HOURS`]. A hand set changes nothing.
    SetRuleHours {
        set: SetId,
        hours: u32,
    },
    /// Show only the members of a rule set whose pane is running. A
    /// hand set changes nothing.
    SetRuleRunningOnly {
        set: SetId,
        on: bool,
    },
    /// The size of a rule set's cards in percent, clamped to
    /// [`RULE_SCALE`] and snapped to steps of 10. A hand set changes
    /// nothing.
    SetRuleCardScale {
        set: SetId,
        scale: u32,
    },
    /// Take a session off a rule set until it is active again.
    DismissFromSet {
        set: SetId,
        record: RecordId,
    },
    /// Stop a session's pane and take it off a rule set, so the end the
    /// kill sends does not bring it back.
    KillAndDismiss {
        set: SetId,
        record: RecordId,
    },
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
    /// Open or close the window in which `switchboard-env`'s setup
    /// commands (`env.set.upsert`, `env.secret.store`, `env.grant`) are
    /// accepted. It lasts `ENV_SETUP_WINDOW` and is never saved.
    SetEnvSetup {
        open: bool,
    },
    /// The app put a fresh launch token in the record's next spawn;
    /// `hash` is its SHA-256 in hex, what `env.resolve` checks.
    RecordTokenIssued {
        id: RecordId,
        hash: String,
    },
    // --- sessions
    NewSession {
        project: ProjectId,
        name: String,
        kind: SessionKind,
        cwd: PathBuf,
        launch: Launch,
        /// Glob patterns of the files a command produces (see
        /// `SessionRecord::outputs`).
        outputs: Vec<String>,
    },
    RenameSession(RecordId, String),
    /// Replace a command's declared output patterns.
    SetOutputs(RecordId, Vec<String>),
    /// The files a finished run produced (from `Effect::FindArtifacts`).
    ArtifactsFound {
        id: RecordId,
        n: u32,
        paths: Vec<PathBuf>,
    },
    SetSessionNotes(RecordId, String),
    /// The owner clears a session's own question without answering it;
    /// the session is not told.
    DismissAsk(RecordId),
    /// The owner answers the session's standing ask, taken at `at`: a
    /// confirm's `yes` or `no`, one of a choice's options, or a line of
    /// text. Sent into the pane as its next prompt once it is between
    /// turns; refused when the ask has changed since.
    AnswerAsk {
        id: RecordId,
        at: SystemTime,
        answer: String,
    },
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
    /// Whether agent sessions get the Prompt Box editor as their message box.
    SetPromptBox(bool),
    /// The embedded Prompt Box's trigger word, model, and captions.
    SetVoiceSettings(VoiceSettings),
    /// The `OpenAI` key for the embedded Prompt Box, into the Keychain
    /// under `VOICE_KEY_ACCOUNT`; blank deletes it.
    StoreVoiceKey(String),
    /// Which tab the side panel shows.
    SetSideTab(SideTab),
    /// The side panel on the left of the content (true) or the right.
    SetSideLeft(bool),
    /// Give the session a window of its own; the main window goes back
    /// to what it showed before if the session was its page.
    PopOut(RecordId),
    /// The session's window closed; its page is the main window's again.
    ClosePopout(RecordId),
    /// The session's window was moved or resized.
    PopoutMoved(RecordId, WindowFrame),
    /// The project's file side starts at this directory (relative to
    /// the root); `None` puts the project root back.
    SetFileRoot(ProjectId, Option<PathBuf>),
    /// Windows on the named display draw at this zoom, in percent.
    SetMonitorZoom(String, u32),
    /// The main window has held still at this frame.
    MainWindowMoved(WindowFrame),
    /// Put back a session removed within the undo window.
    UndoRemove(RecordId),
    /// The hand controller said something.
    Controller(ControllerEvent),
    /// The card the controller's stick starts from on this set; a click
    /// on a card sets it too.
    ActivateCard {
        set: SetId,
        target: PinTarget,
    },
    /// Move the selection one card that way on the working set shown
    /// (the keyboard's h, j, k, l).
    StepCard(Direction),
    /// How many grid units the working-set view fits across, so steps
    /// on a rule set follow the grid as drawn. A measurement, never
    /// saved.
    ViewColumns(u32),
    /// Work in this space: the rail shows it, and the screen goes to
    /// its switchboard unless what was showing is in it.
    ShowSpace(SpaceId),
    /// A new space with this name, shown at once.
    NewSpace(String),
    RenameSpace(SpaceId, String),
    /// Drop an empty space; the last one, or one with anything in it,
    /// stays.
    DeleteSpace(SpaceId),
    MoveProjectToSpace(ProjectId, SpaceId),
    MoveSetToSpace(SetId, SpaceId),
    /// Replace `<root>/.switchboard/project.json` with `text`: the config
    /// editor's Save, the one write into a project directory.
    SaveProjectConfig {
        project: ProjectId,
        text: String,
    },
    /// The write for `SaveProjectConfig` finished; on success the file
    /// is read again.
    ProjectConfigWritten {
        project: ProjectId,
        result: Result<(), String>,
    },
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
    /// The owner sent a message from one of the boxes. It is typed into
    /// the pane like `SendInput`, and the app settles the box's draft by
    /// the outcome of the write. Every other writer (control port,
    /// pipelines, console, scripts) keeps `SendInput`, so no other write
    /// touches a draft.
    SendMessage {
        id: RecordId,
        text: String,
        from: Composer,
    },
    /// The owner pressed a key in the session's embedded terminal: a
    /// draft may be in its input box, so `session.prompt` holds off for
    /// `TYPED_HOLD`. Transient; nothing is saved.
    InputTyped {
        id: RecordId,
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
    /// Cut an agent session's conversation back to before turn `before`,
    /// in place: the record resumes through a copy up to there, with
    /// `prompt` primed, and keeps what it resumed through before so
    /// `UndoDiscard` can put it back. A running agent is stopped. Never
    /// launches.
    DiscardTo {
        id: RecordId,
        before: usize,
        prompt: String,
    },
    /// Put back what the last `DiscardTo` replaced. Offered until the
    /// next message goes into the session.
    UndoDiscard(RecordId),
    // --- workflows
    /// Begin a plan review: a fresh reviewer is launched with the first
    /// prompt, and `source` is cloned whole as the planner. `plan` is
    /// absolute.
    StartWorkflow {
        source: RecordId,
        plan: PathBuf,
        definition: String,
    },
    ShowWorkflow(WorkflowId),
    /// Stop waiting; nothing is launched until `ContinueWorkflow`.
    PauseWorkflow(WorkflowId),
    /// From `Paused`: wait again, relaunching the agent if its pane is
    /// gone. From `AtCap` or `Converged`: one more reviewer round.
    ContinueWorkflow(WorkflowId),
    RaiseWorkflowCap {
        run: WorkflowId,
        cap: u32,
    },
    /// The user has reviewed the plan.
    FinalizeWorkflow(WorkflowId),
    /// Delete the round files the run named (the user confirmed).
    CleanUpWorkflow(WorkflowId),
    /// Send the finished plan back to the source session.
    HandOffWorkflow {
        run: WorkflowId,
        mode: HandoffMode,
    },
    /// The user's own feedback as one more round for the planner.
    UserFeedback {
        run: WorkflowId,
        text: String,
    },
    /// The owner's objection to a converged or capped review, from the
    /// control port: round `round` (the next one) carries `text` to the
    /// planner, and the reviewer re-reads after it answers.
    ObjectWorkflow {
        run: WorkflowId,
        round: u32,
        text: String,
    },
    /// Forget the run; its sessions stay.
    RemoveWorkflow(WorkflowId),
    SetWorkflowRoundCap(u32),
    SetWorkflowDefinitions(Vec<WorkflowDefinition>),
    /// The planner clone's transcript was made (or not).
    WorkflowCloned {
        run: WorkflowId,
        result: Result<ResumeHandle, String>,
    },
    /// One probe of the file a run waits on.
    RoundFileProbed {
        run: WorkflowId,
        path: PathBuf,
        found: Option<Probed>,
    },
    RoundSnapshotted {
        run: WorkflowId,
        n: u32,
        result: Result<(), String>,
    },
    RoundFilesRemoved {
        run: WorkflowId,
        result: Result<(), String>,
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
    /// The copy for a record made ahead of it (the control port's
    /// `session.clone`) exists (or not); the record launches from it.
    TranscriptClonedInto {
        target: RecordId,
        prompt: String,
        result: Result<ResumeHandle, String>,
    },
    /// The provider-side copy for `DiscardTo` was made (or not).
    TranscriptDiscarded {
        id: RecordId,
        before: usize,
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
    /// The shell read a Claude Code pane that has reported no hook yet:
    /// `seen` says whether it shows Claude's own folder trust prompt.
    PromptSeen {
        id: RecordId,
        seen: bool,
    },
    /// Answer Claude's folder trust question in the pane with yes.
    TrustFolder(RecordId),
    Tick,
    /// One command from the control port, run quietly under its
    /// operation id; the outcome is taken with `take_control_outcome`.
    Control {
        op: String,
        action: ControlAction,
    },
    /// Where Dispatch is: its executable and data directory, and this
    /// app's own data directory, which the runner is told to reach the
    /// app through.
    DispatchConfigured {
        command: PathBuf,
        data_dir: PathBuf,
        switchboard_data_dir: PathBuf,
    },
    /// The runner's answer to a status poll; `None` is no runner.
    DispatchStatus(Option<crate::ports::dispatch::Status>),
    ShowDispatch,
    ShowTicket(String),
    /// Answer a pending decision through Dispatch's port.
    DispatchDecide {
        ticket: String,
        decision: String,
        answer: String,
        note: Option<String>,
    },
    /// A parked ticket back to active, through the port.
    DispatchResume(String),
    /// A new supervisor session for the project, replacing the current
    /// one, through the port.
    DispatchSupervisorFresh(String),
    /// Close a ticket through the port: its worktrees removed, its
    /// branch and records kept.
    DispatchClose(String),
    /// Where Dispatch puts tickets' trees: set it (`path`), and with
    /// `migrate` have idle tickets' trees moved there.
    DispatchWorktrees {
        path: Option<PathBuf>,
        migrate: bool,
    },
    /// Read an artifact's text through the port, once.
    DispatchReadArtifact {
        ticket: String,
        path: PathBuf,
    },
    /// Read a ticket's events after the cached cursor, once per
    /// `updated_ms` the page sees: a ticket that has not changed is not
    /// asked again.
    DispatchReadEvents {
        ticket: String,
        updated_ms: u64,
    },
    /// Read one ticket in full (its paths, its lanes' clones), once per
    /// `updated_ms` the page sees.
    DispatchReadTicket {
        id: String,
        updated_ms: u64,
    },
    /// What a `DispatchCall` came back with.
    DispatchReplied {
        body: crate::ports::dispatch::Body,
        result: Result<crate::ports::dispatch::Reply, String>,
    },
    /// Make the console session if there is none, and show it.
    OpenDispatchConsole,
    /// Type a `dispatch` command line into the console; a line starting
    /// with `!` goes to the shell as it is.
    DispatchConsole(String),
    /// Run `dispatch run` as the app's runner service, and keep it
    /// running across app starts until Stop.
    DispatchRunnerStart,
    /// Kill the runner service and stop relaunching it.
    DispatchRunnerStop,
    /// The Dispatch page in a window of its own; the main window goes
    /// back to what was under it.
    PopOutDispatch,
    CloseDispatchWindow,
    DispatchWindowMoved(WindowFrame),
}

/// Work the shell performs on the core's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Bring the session's own window to the front.
    FocusWindow(RecordId),
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
    /// The whole conversation copied under a fresh id, for a workflow's
    /// planner (reports `WorkflowCloned`).
    CloneAllTranscript {
        run: WorkflowId,
        handle: ResumeHandle,
    },
    /// The whole conversation behind `handle` copied under a fresh id
    /// for the record `target`, which already exists and waits for it
    /// (reports `TranscriptClonedInto`).
    CloneTranscriptInto {
        target: RecordId,
        handle: ResumeHandle,
        prompt: String,
    },
    /// Look for the file a run waits on (reports `RoundFileProbed`).
    ProbeRoundFile {
        run: WorkflowId,
        path: PathBuf,
    },
    /// Copy the round's files into the run's snapshot directory under
    /// the data dir (reports `RoundSnapshotted`).
    SnapshotRound {
        run: WorkflowId,
        n: u32,
        files: Vec<PathBuf>,
        note: Option<String>,
    },
    /// Delete the round files (reports `RoundFilesRemoved`).
    RemoveRoundFiles {
        run: WorkflowId,
        files: Vec<PathBuf>,
    },
    /// The same copy for `DiscardTo` (reports `TranscriptDiscarded`).
    DiscardTranscript {
        id: RecordId,
        handle: ResumeHandle,
        before: usize,
        prompt: String,
    },
    /// Match a finished run's declared outputs under its directory
    /// (reports `ArtifactsFound`).
    FindArtifacts {
        id: RecordId,
        n: u32,
        cwd: PathBuf,
        patterns: Vec<String>,
        since: SystemTime,
    },
    /// Delete a run's log (`scrollback/<name>`) that fell off the record.
    RemoveLog(String),
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
    /// Write `text` to the pane, then Enter, and settle the `from` box's
    /// draft for `id`.
    SendMessage {
        id: RecordId,
        host: HostId,
        text: String,
        from: Composer,
    },
    /// Write the owner's answer to an ask to the pane, then Enter. It
    /// goes out by itself at a turn's end, not from a box, so like
    /// `SendInput` it settles no draft. Only `SendMessage` does.
    SendAnswer {
        host: HostId,
        text: String,
    },
    /// Raw bytes to the pane, no Enter (Escape, control characters).
    SendKeys {
        host: HostId,
        bytes: Vec<u8>,
    },
    /// Write the config file's text (reports `ProjectConfigWritten`).
    WriteProjectConfig {
        project: ProjectId,
        root: PathBuf,
        text: String,
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
    /// Append to the operations log, before anything else of the same
    /// action runs: `op` made the records `ids` with command `kind`.
    LogOperation {
        op: String,
        kind: String,
        ids: Vec<String>,
    },
    /// One request to Dispatch's port; the reply returns as
    /// `AppAction::DispatchReplied`.
    DispatchCall(crate::ports::dispatch::Body),
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
/// How long a removed session can be put back. Cards move as states
/// change, so a click meant for one can land on another; the record is
/// held here, untouched on disk, until the window closes.
pub(crate) const UNDO_WINDOW: Duration = Duration::from_secs(10);
/// How long environment setup stays unlocked once the owner opens it.
/// `ENV_SETUP_LOCKED` and the settings menu's Unlock label both say this
/// length in words; change them with it.
pub const ENV_SETUP_WINDOW: Duration = Duration::from_mins(5);
/// How long a key typed into a session's embedded terminal holds off
/// `session.prompt`, unless a submitted prompt clears it first: a
/// draft the owner left longer than this is pasted over.
pub const TYPED_HOLD: Duration = Duration::from_mins(1);
/// Why `switchboard-env`'s setup commands are refused outside the window.
pub const ENV_SETUP_LOCKED: &str = "environment setup is locked: choose Unlock environment setup in Switchboard's settings menu, then run this again within five minutes";
/// How far back a rule set may look, in hours: an hour to a month.
pub const RULE_HOURS: std::ops::RangeInclusive<u32> = 1..=720;
/// The width, in grid units, a rule set is laid out at where no view
/// says how wide it is: the control port, and the core before the
/// first frame reports [`AppAction::ViewColumns`].
pub const RULE_COLUMNS: u32 = 24;
/// A rule set's card size in percent: small enough to fit more, never
/// so small a card's header and footer clip.
pub const RULE_SCALE: std::ops::RangeInclusive<u32> = 90..=200;

/// `hours` kept within [`RULE_HOURS`].
fn clamp_hours(hours: u32) -> u32 {
    hours.clamp(*RULE_HOURS.start(), *RULE_HOURS.end())
}

/// `scale` kept within [`RULE_SCALE`], to the nearest 10: the value a
/// rule set's scale is stored at, and the one its slider shows.
#[must_use]
pub fn clamp_scale(scale: u32) -> u32 {
    (scale.saturating_add(5) / 10 * 10).clamp(*RULE_SCALE.start(), *RULE_SCALE.end())
}

/// A session taken off its board, kept whole until its undo window
/// closes: the record, the project it came from, and its cards on the
/// working sets.
#[derive(Debug, Clone, PartialEq)]
struct Trashed {
    record: SessionRecord,
    project: ProjectId,
    pins: Vec<(SetId, PinnedItem)>,
    /// Its dismissals from rule sets, which leave views with the record
    /// and come back with it.
    dismissals: Vec<(SetId, Dismissal)>,
    until: Duration,
}
/// How a zoom notice starts, so the next one replaces it.
const ZOOM_NOTICE: &str = "Zoom ";

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
    /// Sessions removed within the last [`UNDO_WINDOW`].
    trash: Vec<Trashed>,
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
    /// The space settings.json holds while the core works in the global
    /// space only because views came from a newer build: saves write
    /// this one, so that build reopens where it was left. Cleared once
    /// the user changes space.
    pub(super) saved_space: Option<SpaceId>,
    /// Agents without hooks (Codex) whose pane has been quiet for a
    /// while, per the last host poll: shown idle instead of working.
    pub(super) quiet: Vec<RecordId>,
    /// Claude Code panes sitting at a prompt of Claude's own before any
    /// hook has run (the folder trust question): the user's turn, though
    /// no event says so. Transient; the shell reports them from the
    /// pane's text and the first hook event clears them.
    pub(super) prompted: Vec<RecordId>,
    /// Asking sessions sent a prompt over `control.sock`: the next typed
    /// prompt event is that send arriving, not the owner answering, so
    /// it leaves the ask. Transient; a restart loses it.
    pub(super) relayed: Vec<RecordId>,
    /// Sessions whose `Working` came from a `SessionStart` outside a
    /// turn (a launch, `/clear`, `/resume`): Claude Code waits at its
    /// prompt with no turn open, and runs no `Stop` to say so (spike 17).
    /// The next hook event that sets an activity drops the mark.
    /// Transient; a restart loses it.
    pub(super) started: Vec<RecordId>,
    /// When the owner last typed into each session's embedded terminal,
    /// or a `session.prompt` typed into it, by wall clock, and whether
    /// it was the prompt: the input box may hold text until `TYPED_HOLD`
    /// passes or a prompt is submitted. Transient; a restart loses it.
    pub(super) typed: Vec<(RecordId, SystemTime, bool)>,
    /// Text to submit as the first prompt of a record's next launch,
    /// on its command line. Consumed by `launch_prepared`.
    pub(super) first_prompts: Vec<(RecordId, String)>,
    /// How long each waiting run's file has looked the same.
    pub(super) probes: Vec<crate::core::workflow::Probe>,
    /// Waiting runs whose agent's pane has printed nothing for
    /// `STALL_AFTER`: noticed once, and shown as waiting
    /// on the user until the pane moves again. Transient.
    pub(super) stalled: Vec<WorkflowId>,
    /// The hand controller: held buttons and the selected card per set.
    /// Transient.
    pub(super) controller: super::controller::ControllerState,
    /// The control-port operation being applied, while one is: no view
    /// changes, and records made carry it. Transient.
    pub(super) quiet_op: Option<String>,
    /// Outcomes of control commands not yet taken by the app.
    pub(super) control_outcomes: Vec<ControlOutcome>,
    /// What Dispatch's port last said, and the console. Transient.
    pub(super) dispatch: super::dispatch::DispatchState,
    /// The members of each rule set, newest first, as of the last
    /// action. A cache of the rule, never saved: `dispatch` works it out
    /// again after every action, so whatever changed the answer (a new
    /// set, a dismissal, a move to another space, a load, the clock on a
    /// `Tick`) shows in the same frame.
    pub(super) rule_members: HashMap<SetId, Vec<RecordId>>,
    /// How many grid units the working-set view last said it fits
    /// across; `None` until the first frame. Transient.
    pub(super) view_columns: Option<u32>,
    /// Until when, on the clock's `mono`, `switchboard-env`'s setup
    /// commands are accepted. Transient: a restart closes it.
    pub(super) env_setup_until: Option<Duration>,
    /// The line the document view should scroll to: the file, the line,
    /// and the request it came from, so a repeat click scrolls again.
    /// Cleared by every other show, the same file's too, and by Back.
    /// Transient.
    pub(super) document_line: Option<(PathBuf, u32, u64)>,
    /// Requests to scroll to a line so far; numbers `document_line`.
    pub(super) line_requests: u64,
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
    // The router names every action once; splitting it would hide the
    // one place that shows where each goes.
    #[allow(clippy::too_many_lines)]
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
            | AppAction::DeleteWorkingSet(_)
            | AppAction::NewRuleSet { .. }
            | AppAction::SetRuleHours { .. }
            | AppAction::SetRuleRunningOnly { .. }
            | AppAction::SetRuleCardScale { .. }
            | AppAction::DismissFromSet { .. }
            | AppAction::KillAndDismiss { .. } => self.working_set_action(action, now, &mut out),
            AppAction::ShowBoard(id) => self.show(View::Board(id), now, &mut out),
            AppAction::ShowSession(id) => self.show_session(id, now, &mut out),
            AppAction::RenameProject(..)
            | AppAction::MoveSession(..)
            | AppAction::PinDocument(..)
            | AppAction::UnpinDocument(..)
            | AppAction::ShowDocument(..)
            | AppAction::ShowFileRef { .. }
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
            | AppAction::SetPromptBox(_)
            | AppAction::SetVoiceSettings(_)
            | AppAction::StoreVoiceKey(_)
            | AppAction::SetSideTab(_)
            | AppAction::SetSideLeft(_)
            | AppAction::PopOut(_)
            | AppAction::ClosePopout(_)
            | AppAction::PopoutMoved(..)
            | AppAction::SetFileRoot(..)
            | AppAction::SetMonitorZoom(..)
            | AppAction::MainWindowMoved(..) => self.files_and_settings(action, now, &mut out),
            AppAction::ShowSpace(_)
            | AppAction::NewSpace(_)
            | AppAction::RenameSpace(..)
            | AppAction::DeleteSpace(_)
            | AppAction::MoveProjectToSpace(..)
            | AppAction::MoveSetToSpace(..) => self.space_action(action, &mut out),
            AppAction::ProjectConfigRead { .. }
            | AppAction::SaveProjectConfig { .. }
            | AppAction::ProjectConfigWritten { .. }
            | AppAction::ApproveDefinition(_)
            | AppAction::RevokeApproval(_) => self.definition_action(action, now, &mut out),
            AppAction::Back => {
                self.view_stack.pop();
                self.document_line = None;
            }
            AppAction::DismissNotice => self.dismiss_notice(),
            AppAction::PromptSeen { id, seen } => self.prompt_seen(id, seen),
            AppAction::TrustFolder(id) => self.trust_folder(id, &mut out),
            AppAction::Tick => self.tick(now, &mut out),
            AppAction::SetEnvSetup { open } => {
                self.env_setup_until = open.then_some(now.mono + ENV_SETUP_WINDOW);
            }
            AppAction::RecordTokenIssued { id, hash } => {
                self.edit_session(id, &mut out, |s| s.token_hash = Some(hash));
            }
            AppAction::Controller(event) => self.controller_event(event, now, &mut out),
            AppAction::ActivateCard { set, target } => self.activate_card(set, target),
            AppAction::StepCard(direction) => self.step_card(direction),
            AppAction::ViewColumns(columns) => self.view_columns = Some(columns),

            AppAction::StartWorkflow { .. }
            | AppAction::ShowWorkflow(_)
            | AppAction::PauseWorkflow(_)
            | AppAction::ContinueWorkflow(_)
            | AppAction::RaiseWorkflowCap { .. }
            | AppAction::FinalizeWorkflow(_)
            | AppAction::CleanUpWorkflow(_)
            | AppAction::HandOffWorkflow { .. }
            | AppAction::UserFeedback { .. }
            | AppAction::ObjectWorkflow { .. }
            | AppAction::RemoveWorkflow(_)
            | AppAction::SetWorkflowRoundCap(_)
            | AppAction::SetWorkflowDefinitions(_)
            | AppAction::WorkflowCloned { .. }
            | AppAction::RoundFileProbed { .. }
            | AppAction::RoundSnapshotted { .. }
            | AppAction::RoundFilesRemoved { .. } => self.workflow_action(action, now, &mut out),

            AppAction::AddProject { name, root } => {
                let space = self.add_project_space();
                self.add_project(name, root, space, now, &mut out);
            }
            AppAction::AddProjectTo { name, root, space } => {
                if self.space(space).is_some() {
                    self.add_project(name, root, space, now, &mut out);
                } else {
                    self.error("a project goes into a workspace of its own");
                }
            }
            AppAction::RemoveProject(id) => self.remove_project(id, now, &mut out),

            AppAction::NewSession { .. }
            | AppAction::SetOutputs(..)
            | AppAction::ArtifactsFound { .. }
            | AppAction::RenameSession(..)
            | AppAction::SetSessionNotes(..)
            | AppAction::DismissAsk(_)
            | AppAction::AnswerAsk { .. }
            | AppAction::SetAutostart(..)
            | AppAction::MoveCard { .. }
            | AppAction::ReturnToSession(_)
            | AppAction::SendInput { .. }
            | AppAction::SendMessage { .. }
            | AppAction::InputTyped { .. }
            | AppAction::Interrupt(_)
            | AppAction::KillSession(_)
            | AppAction::RemoveSession(_)
            | AppAction::UndoRemove(_)
            | AppAction::RestartSession(_)
            | AppAction::CloneSession { .. }
            | AppAction::TranscriptCloned { .. }
            | AppAction::TranscriptClonedInto { .. }
            | AppAction::DiscardTo { .. }
            | AppAction::UndoDiscard(_)
            | AppAction::TranscriptDiscarded { .. }
            | AppAction::LaunchPrepared { .. }
            | AppAction::Spawned { .. }
            | AppAction::Attached { .. }
            | AppAction::TranscriptChecked { .. }
            | AppAction::Discovered { .. } => self.session_action(action, now, &mut out),

            AppAction::Events(events) => self.apply_events(events, now, &mut out),
            AppAction::Control { op, action } => self.control(op, action, now, &mut out),
            AppAction::DispatchConfigured { .. }
            | AppAction::DispatchStatus(_)
            | AppAction::ShowDispatch
            | AppAction::ShowTicket(_)
            | AppAction::DispatchDecide { .. }
            | AppAction::DispatchReadArtifact { .. }
            | AppAction::DispatchReadEvents { .. }
            | AppAction::DispatchReadTicket { .. }
            | AppAction::DispatchResume(_)
            | AppAction::DispatchSupervisorFresh(_)
            | AppAction::DispatchClose(_)
            | AppAction::DispatchWorktrees { .. }
            | AppAction::DispatchReplied { .. }
            | AppAction::OpenDispatchConsole
            | AppAction::DispatchConsole(_)
            | AppAction::DispatchRunnerStart
            | AppAction::DispatchRunnerStop
            | AppAction::PopOutDispatch
            | AppAction::CloseDispatchWindow
            | AppAction::DispatchWindowMoved(_) => self.dispatch_action(action, now, &mut out),
        }
        self.remember_view(&mut out);
        self.prune_working_set(&mut out);
        self.refresh_rule_members(now.wall);
        self.finish(out)
    }

    /// The session records' transitions, split out of `dispatch` for length.
    #[allow(clippy::too_many_lines)] // one arm per session action
    pub(super) fn session_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::NewSession {
                project,
                name,
                kind,
                cwd,
                launch,
                outputs,
            } => {
                self.new_session(project, name, kind, cwd, launch, outputs, now, out);
            }
            AppAction::SetOutputs(id, outputs) => {
                self.edit_session(id, out, |s| s.outputs = outputs);
            }
            AppAction::ArtifactsFound { id, n, paths } => self.edit_session(id, out, |s| {
                if let Some(run) = s.runs.iter_mut().find(|r| r.n == n) {
                    run.artifacts = paths;
                }
            }),
            AppAction::RenameSession(id, name) => {
                self.edit_session(id, out, |s| s.name = name);
            }
            AppAction::SetSessionNotes(id, notes) => {
                self.edit_session(id, out, |s| s.notes = notes);
            }
            AppAction::DismissAsk(id) => {
                self.edit_session(id, out, |s| s.asking = None);
            }
            AppAction::AnswerAsk { id, at, answer } => {
                self.answer_ask(id, at, &answer, now, out);
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
                self.send_to_pane(id, out, |host| Effect::SendInput { host, text });
            }
            AppAction::SendMessage { id, text, from } => {
                // The editor emptied itself when it queued the text, so a
                // send that finds no pane hands the text back.
                if from == Composer::Editor && self.running_host(id).is_none() {
                    self.primed.push((id, text.clone()));
                }
                self.send_to_pane(id, out, |host| Effect::SendMessage {
                    id,
                    host,
                    text,
                    from,
                });
            }
            AppAction::InputTyped { id } => {
                if self.session(id).is_some() {
                    self.mark_typed(id, now.wall, false);
                }
            }
            AppAction::Interrupt(id) => self.aim_at_pane(id, out, |host| Effect::SendKeys {
                host,
                bytes: ESCAPE.to_vec(),
            }),
            AppAction::KillSession(id) => self.kill_pane(id, out),
            AppAction::RemoveSession(id) => self.trash_session(id, now),
            AppAction::UndoRemove(id) => self.undo_remove(id, out),
            AppAction::RestartSession(id) => self.restart_session(id, now, out),
            AppAction::CloneSession { id, before, prompt } => {
                self.clone_session(id, before, prompt, out);
            }
            AppAction::TranscriptCloned {
                source,
                prompt,
                result,
            } => self.transcript_cloned(source, prompt, result, now, out),
            AppAction::TranscriptClonedInto {
                target,
                prompt,
                result,
            } => self.transcript_cloned_into(target, prompt, result, now, out),
            AppAction::DiscardTo { id, before, prompt } => {
                self.discard_to(id, before, prompt, out);
            }
            AppAction::TranscriptDiscarded {
                id,
                before,
                prompt,
                result,
            } => self.transcript_discarded(id, before, prompt, result, now, out),
            AppAction::UndoDiscard(id) => self.undo_discard(id, now, out),
            AppAction::LaunchPrepared { id, result } => {
                self.launch_prepared(id, result, now, out);
            }
            AppAction::Spawned { id, result } => self.spawned(id, result, now, out),
            AppAction::Attached { id, result } => {
                if let Err(e) = result {
                    let name = self.session_name(id);
                    self.error(format!("could not attach to {name}: {e}"));
                }
            }
            AppAction::TranscriptChecked { id, exists } => {
                self.transcript_checked(id, exists, now, out);
            }
            AppAction::Discovered { id, result } => self.discovered(id, result, now, out),
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
                    if s.rule.is_none()
                        && grid::fits(&s.items, &target, rect)
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
            AppAction::NewRuleSet { name, rule } => {
                self.new_rule_set(name, rule, now, out);
            }
            AppAction::SetRuleHours { set, hours } => {
                let hours = clamp_hours(hours);
                self.update_set(out, set, |s| {
                    if let Some(SetRule::Recent { hours: h }) = &mut s.rule {
                        *h = hours;
                    }
                });
            }
            AppAction::SetRuleRunningOnly { set, on } => {
                self.update_set(out, set, |s| {
                    if s.rule.is_some() {
                        s.running_only = on;
                    }
                });
            }
            AppAction::SetRuleCardScale { set, scale } => {
                let scale = clamp_scale(scale);
                self.update_set(out, set, |s| {
                    if s.rule.is_some() {
                        s.card_scale = scale;
                    }
                });
            }
            AppAction::DismissFromSet { set, record } => {
                self.dismiss_from_set(set, record, out);
            }
            AppAction::KillAndDismiss { set, record } => {
                // The dismissal is stamped before the end arrives;
                // `carry_dismissals` moves it up to the end when it does.
                self.dismiss_from_set(set, record, out);
                self.kill_pane(record, out);
            }
            _ => unreachable!("routed by `dispatch`"),
        }
    }

    /// Kill a session's pane, if it has one. Every kill of a session
    /// goes through here, so a step added to killing reaches them all.
    pub(super) fn kill_pane(&mut self, id: RecordId, out: &mut Out) {
        self.keep_last_output(id, out);
        if let Some(status) = self.host_status(id) {
            out.push(Effect::Kill(status.id.clone()));
        }
    }

    /// Raise `last_seen` to the pane's last output before the pane goes.
    /// A killed session or a dead server leaves no status behind, and
    /// without this a shell typed into a minute ago would fall back to
    /// its launch in `last_active` and leave every rule set at once.
    pub(super) fn keep_last_output(&mut self, id: RecordId, out: &mut Out) {
        let Some(output) = self.host_status(id).and_then(|h| h.last_activity) else {
            return;
        };
        if self.session(id).is_some_and(|s| s.last_seen < output) {
            self.edit_session(id, out, |s| s.last_seen = output);
        }
    }

    /// Change the views and save them if anything changed. A read-only
    /// instance changes nothing.
    pub(super) fn update_views(&mut self, out: &mut Out, change: impl FnOnce(&mut Views)) {
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
    pub(super) fn update_set(
        &mut self,
        out: &mut Out,
        id: SetId,
        change: impl FnOnce(&mut WorkingSet),
    ) {
        self.update_views(out, |v| {
            if let Some(set) = v.sets.iter_mut().find(|s| s.id == id) {
                change(set);
            }
        });
    }

    /// `target`'s card, placed on `set` in the first free spot, if it
    /// exists and is not there already. A rule set takes no pins: its
    /// cards are the rule's.
    fn place_new(&self, set: &mut WorkingSet, target: PinTarget, columns: u32) {
        if set.rule.is_some()
            || !self.target_in(&target, set.space)
            || set.items.iter().any(|i| i.target == target)
        {
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
            .unwrap_or_else(|| match self.visible_working_sets().count() {
                0 => "Working Set".to_owned(),
                n => format!("Working Set {}", n + 1),
            });
        let mut set = WorkingSet::named(name);
        set.space = self.settings.space;
        if let Some(source) = source {
            set.items = source.items;
            set.rule = source.rule;
            set.dismissed = source.dismissed;
            set.running_only = source.running_only;
            set.card_scale = source.card_scale;
        }
        if let Some(target) = with {
            self.place_new(&mut set, target, columns);
        }
        let id = set.id;
        self.update_views(out, |v| v.sets.push(set));
        self.show(View::WorkingSet(id), now, out);
    }

    fn new_rule_set(&mut self, name: Option<String>, rule: SetRule, now: Clock, out: &mut Out) {
        let name = name
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                match self
                    .visible_working_sets()
                    .filter(|s| s.rule.is_some())
                    .count()
                {
                    0 => "Recent sessions".to_owned(),
                    n => format!("Recent sessions {}", n + 1),
                }
            });
        let SetRule::Recent { hours } = rule;
        let mut set = WorkingSet::named(name);
        set.space = self.settings.space;
        set.rule = Some(SetRule::Recent {
            hours: clamp_hours(hours),
        });
        let id = set.id;
        self.update_views(out, |v| v.sets.push(set));
        self.show(View::WorkingSet(id), now, out);
    }

    /// Stamp the session's activity as of now on the set, replacing any
    /// earlier dismissal of it, so it stays off until it does something.
    fn dismiss_from_set(&mut self, set: SetId, record: RecordId, out: &mut Out) {
        let Some(at) = self.last_active(record) else {
            return;
        };
        self.update_set(out, set, |s| {
            if s.rule.is_some() {
                s.dismissed.retain(|d| d.record != record);
                s.dismissed.push(Dismissal { record, at });
            }
        });
    }

    /// When `id` last did something, for rule sets: the latest of its
    /// hook events, its launch, and its pane's last output. A pane counts
    /// through what it prints, not by being alive, or an idle shell would
    /// never leave a set. A dead pane counts too: tmux keeps its last
    /// output time, and without it a service that printed a minute ago
    /// and then crashed would fall back to its launch and leave the set
    /// the moment it stops. A pane that is gone altogether (killed, or
    /// the server died) has no status, and its last output lives on in
    /// `last_seen`, where `keep_last_output` put it before the pane
    /// went. `None` for an unknown id.
    #[must_use]
    pub fn last_active(&self, id: RecordId) -> Option<SystemTime> {
        let s = self.session(id)?;
        let output = self.host_status(id).and_then(|h| h.last_activity);
        // `last_seen` already moves with every event and holds a vanished
        // pane's last output; the other two are named so the intent
        // reads here.
        [Some(s.last_seen), s.last_event_at, s.last_stop_at, output]
            .into_iter()
            .flatten()
            .max()
    }

    /// Work out every rule set's members again as of `now`.
    pub(super) fn refresh_rule_members(&mut self, now: SystemTime) {
        let members = self
            .views
            .sets
            .iter()
            .filter_map(|set| Some((set.id, self.members_at(set, set.rule?, now))))
            .collect();
        self.rule_members = members;
    }

    /// The sessions `rule` chooses for `set` at `now`, newest first,
    /// ties by id so the order holds from tick to tick.
    fn members_at(&self, set: &WorkingSet, rule: SetRule, now: SystemTime) -> Vec<RecordId> {
        let SetRule::Recent { hours } = rule;
        let since = now
            .checked_sub(Duration::from_secs(u64::from(hours) * 3600))
            .unwrap_or(UNIX_EPOCH);
        let mut members: Vec<(SystemTime, RecordId)> = self
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter_map(|s| Some((self.last_active(s.id)?, s.id)))
            .filter(|(at, id)| {
                *at > since
                    && !set.dismissed.iter().any(|d| d.record == *id && *at <= d.at)
                    && self.target_in(&PinTarget::Session(*id), set.space)
                    && (!set.running_only || self.is_running(*id))
            })
            .collect();
        members.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        members.into_iter().map(|(_, id)| id).collect()
    }

    /// How many cards `set` shows: its pins, or its rule's members.
    #[must_use]
    pub fn set_card_count(&self, set: &WorkingSet) -> usize {
        match set.rule {
            Some(_) => self.rule_members(set.id).len(),
            None => set.items.len(),
        }
    }

    /// How many grid units wide the core lays a rule set out for the
    /// keys and the controller: the view's width once a frame has said
    /// it, [`RULE_COLUMNS`] before.
    #[must_use]
    pub fn view_columns(&self) -> u32 {
        self.view_columns.unwrap_or(RULE_COLUMNS)
    }

    /// A rule set's members as of the last action; empty for a hand set.
    #[must_use]
    pub fn rule_members(&self, set: SetId) -> &[RecordId] {
        self.rule_members.get(&set).map_or(&[], Vec::as_slice)
    }

    /// The cards of `set` as drawn: its pins for a hand set, its members
    /// laid out in order at the set's card scale for a rule set. `Cow`
    /// lends the hand set's own list and hands over a new one for a rule
    /// set.
    #[must_use]
    pub fn set_cards<'a>(&'a self, set: &'a WorkingSet, columns: u32) -> Cow<'a, [PinnedItem]> {
        if set.rule.is_none() {
            return Cow::Borrowed(&set.items);
        }
        let members = self.rule_members(set.id);
        let (w, h) = grid::rule_card(set.card_scale);
        let rects = grid::flow(members.len(), w, h, columns);
        Cow::Owned(
            members
                .iter()
                .zip(rects)
                .map(|(id, rect)| PinnedItem {
                    target: PinTarget::Session(*id),
                    rect,
                })
                .collect(),
        )
    }

    /// Drop working-set cards whose session or project is gone, after
    /// whatever action removed it (or the load that found it missing),
    /// and cards whose project is no longer in the set's space: a set
    /// shows only its own space.
    ///
    /// Dismissals go with their record, not its space: a session whose
    /// project moves out and back keeps its dismissal.
    fn prune_working_set(&mut self, out: &mut Out) {
        let stale = self.views.sets.iter().any(|s| {
            s.items.iter().any(|i| !self.target_in(&i.target, s.space))
                || s.dismissed.iter().any(|d| self.session(d.record).is_none())
        });
        if !stale {
            return;
        }
        let mut next = self.views.clone();
        for set in &mut next.sets {
            let space = set.space;
            set.items.retain(|i| self.target_in(&i.target, space));
            set.dismissed.retain(|d| self.session(d.record).is_some());
        }
        self.update_views(out, |v| *v = next);
    }

    /// Hand `old`'s pins to `new` in place, at the same rect, on every
    /// hand-placed set whose space `new` belongs to. A set that already
    /// pins `new` just loses `old`'s card. Rule sets and dismissals are
    /// left alone: a rule set holds no pins, and a dismissal belongs to
    /// its own record.
    pub(super) fn take_over_pins(&mut self, old: RecordId, new: RecordId, out: &mut Out) {
        let (old, new) = (PinTarget::Session(old), PinTarget::Session(new));
        let mut next = self.views.clone();
        for set in &mut next.sets {
            if set.rule.is_some() || !self.target_in(&new, set.space) {
                continue;
            }
            if set.items.iter().any(|i| i.target == new) {
                set.items.retain(|i| i.target != old);
            } else if let Some(item) = set.items.iter_mut().find(|i| i.target == old) {
                item.target = new.clone();
            }
        }
        self.update_views(out, |v| *v = next);
    }

    /// The target exists and its project is in `space`, or anywhere
    /// when `space` is the global one.
    pub(super) fn target_in(&self, target: &PinTarget, space: SpaceId) -> bool {
        let project = match target {
            PinTarget::Session(id) => self.session(*id).map(|s| s.project),
            PinTarget::File(pid, _) => Some(*pid),
        };
        project
            .and_then(|p| self.project_space(p))
            .is_some_and(|s| space.contains(s))
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
        // The operations log comes first: a record found on disk without
        // its log line would read as never asked for.
        let (logs, rest): (Vec<Effect>, Vec<Effect>) = out
            .effects
            .into_iter()
            .partition(|e| matches!(e, Effect::LogOperation { .. }));
        effects.extend(logs);
        if !self.read_only {
            for id in out.dirty {
                if let Some(w) = self.workspace(id) {
                    effects.push(Effect::Save(w.clone()));
                }
            }
        }
        effects.extend(
            rest.into_iter()
                .filter(|e| !(self.read_only && matches!(e, Effect::Delete(_)))),
        );
        effects
    }

    /// The views (sets and spaces) directly, bypassing dispatch. Tests
    /// only.
    #[cfg(test)]
    pub(crate) fn seed_views(&mut self, views: Views) {
        self.views = views;
    }

    /// Set a record's outside waiting reason directly. Tests only; the
    /// control port sets it through its own action.
    #[cfg(test)]
    pub(crate) fn seed_waiting_on(&mut self, id: RecordId, reason: Option<String>) {
        if let Some(s) = self.session_mut(id) {
            s.waiting_on = reason;
        }
    }

    /// Set a record's own ask directly. Tests only; a session sets it
    /// through `session.ask` with its token.
    #[cfg(test)]
    pub(crate) fn seed_asking(&mut self, id: RecordId, ask: Option<super::Ask>) {
        if let Some(s) = self.session_mut(id) {
            s.asking = ask;
        }
    }

    /// Populate state directly, bypassing dispatch. For tests only; the
    /// app itself always goes through `dispatch`.
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
            space: None,
            undo: None,
        });
    }

    pub(super) fn info(&mut self, text: impl Into<String>, now: Clock) {
        self.notices.push(Notice {
            text: text.into(),
            is_error: false,
            expires_at: Some(now.mono + NOTICE_TTL),
            space: None,
            undo: None,
        });
    }

    /// An error naming something in a project: shown as it is in the
    /// project's space only.
    pub(super) fn error_in(&mut self, project: ProjectId, text: impl Into<String>) {
        let space = self.project_space(project);
        self.error(text);
        if let Some(n) = self.notices.last_mut() {
            n.space = space;
        }
    }

    /// An info notice naming something in a project.
    pub(super) fn info_in(&mut self, project: ProjectId, text: impl Into<String>, now: Clock) {
        let space = self.project_space(project);
        self.info(text, now);
        if let Some(n) = self.notices.last_mut() {
            n.space = space;
        }
    }

    /// An error naming a session, scoped to its project's space.
    pub(super) fn error_about(&mut self, id: RecordId, text: impl Into<String>) {
        match self.session(id).map(|s| s.project) {
            Some(p) => self.error_in(p, text),
            None => self.error(text),
        }
    }

    /// An info notice naming a session, scoped to its project's space.
    pub(super) fn info_about(&mut self, id: RecordId, text: impl Into<String>, now: Clock) {
        match self.session(id).map(|s| s.project) {
            Some(p) => self.info_in(p, text, now),
            None => self.info(text, now),
        }
    }

    fn dismiss_notice(&mut self) {
        if !self.notices.is_empty() {
            self.notices.remove(0);
        }
    }

    /// Once a second: notices age out, the controller's dwell runs,
    /// trashed records past their undo window are removed, and waiting
    /// workflows probe.
    fn tick(&mut self, now: Clock, out: &mut Out) {
        self.expire_notices(now);
        if self.env_setup_until.is_some_and(|t| t <= now.mono) {
            self.env_setup_until = None;
        }
        self.controller_tick(now, out);
        let expired: Vec<(RecordId, ProjectId)> = self
            .trash
            .iter()
            .filter(|t| t.until <= now.mono)
            .map(|t| (t.record.id, t.project))
            .collect();
        self.trash.retain(|t| t.until > now.mono);
        for (id, project) in expired {
            self.finish_removal(id, project, out);
        }
        self.workflow_tick(now, out);
    }

    fn expire_notices(&mut self, now: Clock) {
        self.notices
            .retain(|n| n.expires_at.is_none_or(|t| t > now.mono));
    }

    pub(super) fn show(&mut self, view: View, now: Clock, out: &mut Out) {
        // A control command changes nothing the user is looking at.
        if self.quiet_op.is_some() {
            return;
        }
        if let View::Board(id) | View::Document(id, _) = &view {
            let id = *id;
            self.edit_project(id, out, |p| p.last_active = now.wall);
        }
        // Showing something is an explicit step into its space.
        if !self.view_shown_in(self.settings.space, &view)
            && let Some(space) = self.view_space(&view)
        {
            self.update_settings(out, |s| s.space = space);
        }
        // Even the same file shown again comes up at the top; only
        // `show_file_ref` sets a line, after this.
        self.document_line = None;
        if self.view() != view {
            self.view_stack.push(view);
        }
    }

    /// Show the file an agent named, under the project whose root holds
    /// it most closely (so Pin works), else under the session's own.
    fn show_file_ref(
        &mut self,
        record: RecordId,
        path: PathBuf,
        line: Option<u32>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(own) = self.session(record).map(|s| s.project) else {
            return;
        };
        if self.quiet_op.is_some() {
            return;
        }
        let pid = self
            .workspaces
            .iter()
            .filter(|w| path.starts_with(&w.project.root))
            .max_by_key(|w| w.project.root.components().count())
            .map_or(own, |w| w.project.id);
        self.show(View::Document(pid, path.clone()), now, out);
        self.line_requests += 1;
        self.document_line = line.map(|l| (path, l, self.line_requests));
    }

    /// The space a view is in; the switchboard is in every space.
    pub(super) fn view_space(&self, view: &View) -> Option<SpaceId> {
        match view {
            View::Switchboard | View::Dispatch | View::Ticket(_) => None,
            View::Board(pid) | View::Document(pid, _) => self.project_space(*pid),
            View::Session(id) => self
                .session(*id)
                .and_then(|s| self.project_space(s.project)),
            View::Workflow(id) => self
                .workflow(*id)
                .and_then(|r| self.project_space(r.project)),
            View::WorkingSet(id) => self.working_set(*id).map(|s| s.space),
        }
    }

    /// Whether `view` belongs on screen while `space` is active. A
    /// working set belongs to exactly one space, global's included; a
    /// page of a project (board, session, workflow) shows anywhere
    /// [`SpaceId::contains`] says its project's space is.
    pub(super) fn view_shown_in(&self, space: SpaceId, view: &View) -> bool {
        match (view, self.view_space(view)) {
            (_, None) => true,
            (View::WorkingSet(_), Some(s)) => s == space,
            (_, Some(s)) => space.contains(s),
        }
    }

    /// Work in a space that exists: the setting changes, and a page of
    /// another space gives way to the switchboard.
    fn enter_space(&mut self, id: SpaceId, out: &mut Out) {
        if self.settings.space != id {
            self.update_settings(out, |s| s.space = id);
        }
        let view = self.view();
        if !self.view_shown_in(id, &view) {
            self.view_stack.push(View::Switchboard);
        }
    }

    /// The spaces' transitions, split out of `dispatch` for length.
    fn space_action(&mut self, action: AppAction, out: &mut Out) {
        match action {
            AppAction::ShowSpace(id) => {
                if id.is_global() || self.space(id).is_some() {
                    self.enter_space(id, out);
                }
            }
            AppAction::NewSpace(name) => {
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return;
                }
                let space = Space {
                    id: SpaceId::new(),
                    name,
                    op: None,
                };
                let id = space.id;
                self.update_views(out, |v| v.spaces.push(space));
                self.enter_space(id, out);
            }
            AppAction::RenameSpace(id, name) => {
                // The global space is no record and has no name to change.
                // It is refused by name, so the rule does not rest on the
                // lookup below happening to miss.
                let name = name.trim().to_owned();
                if !name.is_empty() && !id.is_global() {
                    self.update_views(out, |v| {
                        if let Some(s) = v.spaces.iter_mut().find(|s| s.id == id) {
                            s.name = name;
                        }
                    });
                }
            }
            AppAction::DeleteSpace(id) => {
                if id.is_global() || !self.space_empty(id) || self.views.spaces.len() < 2 {
                    return;
                }
                self.update_views(out, |v| v.spaces.retain(|s| s.id != id));
                if self.settings.space == id {
                    let first = self.views.spaces[0].id;
                    self.update_settings(out, |s| s.space = first);
                }
            }
            AppAction::MoveProjectToSpace(pid, space) => {
                if self.space(space).is_some() {
                    self.edit_project(pid, out, |p| p.space = space);
                    // Its cards leave the sets of the space it left, and
                    // its board, if showing, gives way.
                    self.prune_working_set(out);
                    self.enter_space(self.settings.space, out);
                }
            }
            AppAction::MoveSetToSpace(set, space) => {
                if self.space(space).is_some() && self.set_movable(set) {
                    self.update_set(out, set, |s| s.space = space);
                    self.prune_working_set(out);
                    self.enter_space(self.settings.space, out);
                }
            }
            _ => unreachable!("routed by `dispatch`"),
        }
    }

    // --- projects

    pub(super) fn add_project(
        &mut self,
        name: String,
        root: PathBuf,
        space: SpaceId,
        now: Clock,
        out: &mut Out,
    ) -> ProjectId {
        let id = self.add_project_record(name, root, space, now, out);
        if self.quiet_op.is_none() {
            self.view_stack.push(View::Board(id));
        }
        id
    }

    /// The project's record and its first config read, without
    /// showing it.
    pub(super) fn add_project_record(
        &mut self,
        name: String,
        root: PathBuf,
        space: SpaceId,
        now: Clock,
        out: &mut Out,
    ) -> ProjectId {
        let id = ProjectId::new();
        self.workspaces.push(Workspace::new(Project {
            id,
            name,
            root,
            tags: Vec::new(),
            notes: String::new(),
            pinned: Vec::new(),
            env: ProjectEnv::default(),
            env_sets: Vec::new(),
            shown: Vec::new(),
            created: now.wall,
            last_active: now.wall,
            space,
            op: self.quiet_op.clone(),
        }));
        out.touch(id);
        out.push(super::definitions::read_config(
            id,
            self.workspaces
                .last()
                .map(|w| w.project.root.clone())
                .unwrap_or_default(),
        ));
        id
    }

    /// Drop a project and everything the core remembers about its
    /// sessions: views, flights, and its place in the Codex queue. A
    /// pending discovery for one of its records would otherwise block
    /// every later Codex launch until it expired.
    pub(super) fn remove_project(&mut self, id: ProjectId, now: Clock, out: &mut Out) {
        let Some(pos) = self.workspaces.iter().position(|w| w.project.id == id) else {
            return;
        };
        let workspace = self.workspaces.remove(pos);
        out.push(Effect::Delete(id));
        let gone = |r: RecordId| workspace.sessions.iter().any(|s| s.id == r);
        self.view_stack.retain(|v| match v {
            View::Board(p) | View::Document(p, _) => *p != id,
            View::Session(r) => !gone(*r),
            View::Workflow(w) => !workspace.workflows.iter().any(|r| r.id == *w),
            View::Switchboard | View::WorkingSet(_) | View::Dispatch | View::Ticket(_) => true,
        });
        self.in_flight.retain(|f| !gone(f.id));
        self.codex_queue.retain(|r| !gone(*r));
        self.quiet.retain(|r| !gone(*r));
        self.prompted.retain(|r| !gone(*r));
        self.relayed.retain(|r| !gone(*r));
        self.started.retain(|r| !gone(*r));
        self.typed.retain(|(r, ..)| !gone(*r));
        if self.codex_pending.is_some_and(gone) {
            self.codex_pending = None;
        }
        self.advance_codex_queue(now, out);
        self.update_settings(out, |s| s.file_roots.retain(|r| r.project != id));
    }

    /// Lift a record out of its workspace into another's; both are
    /// saved. Nothing happens when the target is missing or the same.
    pub(super) fn move_session(&mut self, id: RecordId, project: ProjectId, out: &mut Out) {
        let Some(from) = self
            .workspaces
            .iter()
            .position(|w| w.sessions.iter().any(|s| s.id == id))
        else {
            self.error("no such session");
            return;
        };
        let Some(to) = self.workspaces.iter().position(|w| w.project.id == project) else {
            self.error("no such project");
            return;
        };
        if from == to {
            return;
        }
        let source = self.workspaces[from].project.id;
        let pos = self.workspaces[from]
            .sessions
            .iter()
            .position(|s| s.id == id)
            .expect("found above");
        let mut record = self.workspaces[from].sessions.remove(pos);
        record.project = project;
        self.workspaces[to].sessions.push(record);
        out.touch(source);
        out.touch(project);
    }

    pub(super) fn edit_project(
        &mut self,
        id: ProjectId,
        out: &mut Out,
        edit: impl FnOnce(&mut Project),
    ) {
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

    /// Removing takes the record off its board at once but keeps it,
    /// and its record on disk, until the undo window closes; the toast
    /// offers to put it back.
    fn trash_session(&mut self, id: RecordId, now: Clock) {
        let mut taken = None;
        for w in &mut self.workspaces {
            if let Some(pos) = w.sessions.iter().position(|s| s.id == id) {
                taken = Some((w.sessions.remove(pos), w.project.id));
            }
        }
        let Some((record, project)) = taken else {
            return;
        };
        let pins = self
            .views
            .sets
            .iter()
            .flat_map(|s| {
                s.items
                    .iter()
                    .filter(|i| i.target == PinTarget::Session(id))
                    .map(move |i| (s.id, i.clone()))
            })
            .collect();
        let dismissals = self
            .views
            .sets
            .iter()
            .flat_map(|s| {
                s.dismissed
                    .iter()
                    .filter(|d| d.record == id)
                    .map(move |d| (s.id, d.clone()))
            })
            .collect();
        self.view_stack
            .retain(|v| !matches!(v, View::Session(s) if *s == id));
        self.info_in(project, format!("Removed {}", record.name), now);
        if let Some(n) = self.notices.last_mut() {
            n.expires_at = Some(now.mono + UNDO_WINDOW);
            n.undo = Some(id);
        }
        self.trash.push(Trashed {
            record,
            project,
            pins,
            dismissals,
            until: now.mono + UNDO_WINDOW,
        });
    }

    fn undo_remove(&mut self, id: RecordId, out: &mut Out) {
        let Some(pos) = self.trash.iter().position(|t| t.record.id == id) else {
            return;
        };
        let t = self.trash.remove(pos);
        let Some(w) = self
            .workspaces
            .iter_mut()
            .find(|w| w.project.id == t.project)
        else {
            return;
        };
        w.sessions.push(t.record);
        if !t.pins.is_empty() || !t.dismissals.is_empty() {
            self.update_views(out, |v| {
                for (set, item) in t.pins {
                    if let Some(s) = v.sets.iter_mut().find(|s| s.id == set)
                        && !s.items.iter().any(|i| i.target == item.target)
                    {
                        s.items.push(item);
                    }
                }
                for (set, dismissal) in t.dismissals {
                    if let Some(s) = v.sets.iter_mut().find(|s| s.id == set)
                        && !s.dismissed.iter().any(|d| d.record == dismissal.record)
                    {
                        s.dismissed.push(dismissal);
                    }
                }
            });
        }
        self.notices.retain(|n| n.undo != Some(id));
    }

    /// The undo window closed: the record is dropped for good.
    fn finish_removal(&mut self, id: RecordId, project: ProjectId, out: &mut Out) {
        // A running process is left alone (removing a record is not a
        // kill); only a gone pane's scrollback is dropped with the record.
        if self.host_status(id).is_none() {
            out.push(Effect::Forget(HostId(id.host_name())));
        }
        out.touch(project);
        self.in_flight.retain(|f| f.id != id);
        self.codex_queue.retain(|q| *q != id);
        if self.codex_pending == Some(id) {
            self.codex_pending = None;
        }
        self.view_stack
            .retain(|v| !matches!(v, View::Session(s) if *s == id));
        self.update_settings(out, |s| s.popouts.retain(|p| p.session != id));
    }

    /// A session with a window of its own is shown there, not here.
    pub(super) fn show_session(&mut self, id: RecordId, now: Clock, out: &mut Out) {
        if self.popped_out(id) {
            out.push(Effect::FocusWindow(id));
        } else {
            self.show(View::Session(id), now, out);
        }
    }

    fn set_file_root(&mut self, pid: ProjectId, dir: Option<PathBuf>, out: &mut Out) {
        self.update_settings(out, |s| {
            s.file_roots.retain(|r| r.project != pid);
            let dir = dir.filter(|d| !d.as_os_str().is_empty() && *d != Path::new("."));
            if let Some(dir) = dir {
                s.file_roots.push(FileRoot { project: pid, dir });
            }
        });
    }

    fn popout_moved(&mut self, id: RecordId, frame: WindowFrame, out: &mut Out) {
        self.update_settings(out, |s| {
            if let Some(p) = s.popouts.iter_mut().find(|p| p.session == id) {
                p.frame = Some(frame);
            }
        });
    }

    /// The zoom of windows on the named display, in percent.
    #[must_use]
    pub fn monitor_zoom(&self, monitor: &str) -> u32 {
        self.settings
            .monitor_zoom
            .iter()
            .find(|z| z.monitor == monitor)
            .map_or(100, |z| z.percent)
    }

    /// Where the project's file side starts, relative to its root, when
    /// it has been narrowed.
    #[must_use]
    pub fn file_root(&self, pid: ProjectId) -> Option<&PathBuf> {
        self.settings
            .file_roots
            .iter()
            .find(|r| r.project == pid)
            .map(|r| &r.dir)
    }

    /// Whether the session has a window of its own.
    #[must_use]
    pub fn popped_out(&self, id: RecordId) -> bool {
        self.settings.popouts.iter().any(|p| p.session == id)
    }

    /// The session gets a window; if it was the main window's page,
    /// that goes back to what was before, so the page is in one place.
    pub(super) fn pop_out(&mut self, id: RecordId, out: &mut Out) {
        if self.session(id).is_none() {
            return;
        }
        if self.popped_out(id) {
            out.push(Effect::FocusWindow(id));
            return;
        }
        self.update_settings(out, |s| {
            s.popouts.push(Popout {
                session: id,
                frame: None,
            });
        });
        while self.view() == View::Session(id) {
            self.view_stack.pop();
        }
    }

    /// Windows of sessions that no longer exist are forgotten.
    pub(super) fn prune_popouts(&mut self, out: &mut Out) {
        let gone: Vec<RecordId> = self
            .settings
            .popouts
            .iter()
            .map(|p| p.session)
            .filter(|id| self.session(*id).is_none())
            .collect();
        if !gone.is_empty() {
            self.update_settings(out, |s| s.popouts.retain(|p| !gone.contains(&p.session)));
        }
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
    #[cfg(test)]
    #[must_use]
    pub(crate) fn workspaces_by_recency(&self) -> Vec<&Workspace> {
        let mut all: Vec<&Workspace> = self.workspaces.iter().collect();
        all.sort_by_key(|w| std::cmp::Reverse(w.project.last_active));
        all
    }
    #[must_use]
    pub fn workspace(&self, id: ProjectId) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.project.id == id)
    }
    /// The line the document view should scroll to, if any: the file,
    /// the line (from 1), and the request number, which changes on every
    /// request even for the same file and line.
    #[must_use]
    pub fn document_line(&self) -> Option<(&Path, u32, u64)> {
        self.document_line
            .as_ref()
            .map(|(p, l, n)| (p.as_path(), *l, *n))
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
    /// Until when environment setup is unlocked, while it is; `Tick`
    /// clears it once it has passed.
    #[must_use]
    pub fn env_setup_until(&self) -> Option<Duration> {
        self.env_setup_until
    }
    /// Every environment set for `env.sets`: plain values included,
    /// secret ones never.
    #[must_use]
    pub fn env_set_views(&self) -> Vec<switchboard_control::EnvSetView> {
        self.settings
            .env_sets
            .iter()
            .map(|set| switchboard_control::EnvSetView {
                name: set.name.clone(),
                vars: set
                    .vars
                    .iter()
                    .map(|v| switchboard_control::EnvVarView {
                        name: v.name.clone(),
                        value: if v.secret {
                            String::new()
                        } else {
                            v.value.clone()
                        },
                        secret: v.secret,
                    })
                    .collect(),
                aws: set.aws.as_ref().map(crate::core::control::aws_view),
            })
            .collect()
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
    /// The sessions on a working set, for the card refreshes: a rule
    /// set's members, or a hand set's session cards.
    #[must_use]
    pub fn working_set_sessions(&self, id: SetId) -> Vec<RecordId> {
        if self.working_set(id).is_some_and(|s| s.rule.is_some()) {
            return self.rule_members(id).to_vec();
        }
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
    /// The hand sets holding `target`; a rule set holds no pins.
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

    /// Whether the UI may show this project at all: it is in the space
    /// being worked in, or the global space is.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn project_visible(&self, id: ProjectId) -> bool {
        self.project_space(id)
            .is_some_and(|s| self.settings.space.contains(s))
    }

    /// Workspaces the UI may show, in stored order: the active space's,
    /// or every one in the global space.
    pub fn visible_workspaces(&self) -> impl Iterator<Item = &Workspace> {
        self.workspaces
            .iter()
            .filter(|w| self.settings.space.contains(w.project.space))
    }

    /// Whether the space menu offers the global space. With one space
    /// and no global set it is that space over again, so it is left out,
    /// unless it is where the user already is or the only way to reach
    /// a project whose space this build does not list.
    #[must_use]
    pub fn global_space_offered(&self) -> bool {
        self.settings.space.is_global()
            || self.views.spaces.len() > 1
            || self.views.sets.iter().any(|s| s.space.is_global())
            || self.views.schema_version > super::model::VIEWS_SCHEMA_VERSION
            || self
                .workspaces
                .iter()
                .any(|w| self.space(w.project.space).is_none())
    }

    /// The space `AddProject` puts a project in: the active one, or in
    /// the global space, which holds no project, the first listed. The
    /// add dialog preselects it.
    #[must_use]
    pub fn add_project_space(&self) -> SpaceId {
        match self.settings.space {
            SpaceId::GLOBAL => self.views.spaces.first().map_or(SpaceId::DEFAULT, |s| s.id),
            space => space,
        }
    }

    /// Whether the switcher searches every space: when asked to, or
    /// always in the global space, which already sees everything.
    #[must_use]
    pub fn switcher_covers_all(&self, all_spaces: bool) -> bool {
        all_spaces || self.settings.space.is_global()
    }

    /// Whether the switcher offers to search every space: only where it
    /// does not already, and there is another space to search.
    #[must_use]
    pub fn switcher_offers_all(&self) -> bool {
        !self.settings.space.is_global() && self.views.spaces.len() > 1
    }

    /// Visible projects in the rail's order, which Cmd+1..9 count: most
    /// recently active first, and in the global space grouped by space
    /// in the user's order of spaces, so the digits count the list as
    /// drawn.
    #[must_use]
    pub fn projects_in_rail_order(&self) -> Vec<&Project> {
        let mut projects: Vec<_> = self.visible_workspaces().map(|w| &w.project).collect();
        if self.settings.space.is_global() {
            // Spaces this build does not list (views from a newer one)
            // share one group after the listed ones.
            let rank = |space: SpaceId| {
                self.views
                    .spaces
                    .iter()
                    .position(|s| s.id == space)
                    .unwrap_or(usize::MAX)
            };
            projects.sort_by_key(|p| (rank(p.space), std::cmp::Reverse(p.last_active)));
        } else {
            projects.sort_by_key(|p| std::cmp::Reverse(p.last_active));
        }
        projects
    }

    /// [`Self::projects_in_rail_order`] cut where the space changes: in
    /// the global space one group per listed space, then one `None`
    /// group for every space this build does not list. Outside global
    /// it is a single group. The rail draws a kicker per group, and the
    /// digits count across the groups, so both read the one order.
    #[must_use]
    pub fn projects_in_rail_groups(&self) -> Vec<(Option<&Space>, Vec<&Project>)> {
        let mut groups: Vec<(Option<&Space>, Vec<&Project>)> = Vec::new();
        for project in self.projects_in_rail_order() {
            let space = self.space(project.space);
            match groups.last_mut() {
                Some((last, projects)) if last.map(|s| s.id) == space.map(|s| s.id) => {
                    projects.push(project);
                }
                _ => groups.push((space, vec![project])),
            }
        }
        groups
    }

    /// The working sets of the active space, in the user's order. The
    /// global space lists only its own sets, not every space's.
    pub fn visible_working_sets(&self) -> impl Iterator<Item = &WorkingSet> {
        self.views
            .sets
            .iter()
            .filter(|s| s.space == self.settings.space)
    }

    /// The working sets of the active space as the rail lists them: rule
    /// sets first, then hand sets, each group in record order.
    #[must_use]
    pub fn working_sets_in_rail_order(&self) -> Vec<&WorkingSet> {
        let mut sets: Vec<_> = self.visible_working_sets().collect();
        sets.sort_by_key(|s| s.rule.is_none());
        sets
    }

    // --- spaces

    /// Every space, in the user's order.
    #[must_use]
    pub fn spaces(&self) -> &[Space] {
        &self.views.spaces
    }
    #[must_use]
    pub fn space(&self, id: SpaceId) -> Option<&Space> {
        self.views.spaces.iter().find(|s| s.id == id)
    }
    /// The space being worked in.
    #[must_use]
    pub fn active_space(&self) -> SpaceId {
        self.settings.space
    }
    #[must_use]
    pub fn project_space(&self, id: ProjectId) -> Option<SpaceId> {
        self.workspace(id).map(|w| w.project.space)
    }
    /// A space with no project and no working set in it.
    #[must_use]
    pub fn space_empty(&self, id: SpaceId) -> bool {
        !self.workspaces.iter().any(|w| w.project.space == id)
            && !self.views.sets.iter().any(|s| s.space == id)
    }
    /// Sessions waiting on the user in one space (every space, for the
    /// global one): the switcher's count per space, a number and nothing
    /// more. Dispatch's decisions keep their own count.
    #[must_use]
    pub fn waiting_count_in(&self, space: SpaceId) -> usize {
        self.workspaces
            .iter()
            .filter(|w| space.contains(w.project.space))
            .flat_map(|w| &w.sessions)
            .filter(|s| self.counts_as_waiting(s.id))
            .count()
    }

    /// Whether a session adds to the waiting counts. A session that
    /// waits only because Dispatch marked it (`waiting_on`, no waiting
    /// activity of its own) is Dispatch's decision, counted once from
    /// the status once one has arrived, not again here; an agent that
    /// is itself asking (a permission, a question) always counts.
    #[must_use]
    pub fn counts_as_waiting(&self, id: RecordId) -> bool {
        if self.card_state(id) != CardState::WaitingOnYou {
            return false;
        }
        let Some(record) = self.session(id) else {
            return false;
        };
        let only_dispatch = record.waiting_on.is_some()
            && record.asking.is_none()
            && record.activity != Activity::WaitingOnYou
            && !self.prompted.contains(&id);
        !(only_dispatch && self.dispatch.seen)
    }

    /// Whether the pane sits at Claude's folder trust prompt, as last
    /// reported by the shell.
    #[must_use]
    pub fn at_trust_prompt(&self, id: RecordId) -> bool {
        self.prompted.contains(&id)
    }

    /// The question's menu starts on "No, exit"; one step down is
    /// "Yes, I trust this folder", and Enter confirms. Only while the
    /// pane was last seen showing the question, so the keys land on
    /// nothing else; the mark is dropped at once and the next read of
    /// the pane restores it if the question is still there.
    pub(super) fn trust_folder(&mut self, id: RecordId, out: &mut Out) {
        if !self.prompted.contains(&id) {
            let name = self.session_name(id);
            self.error(format!("{name} is not at the trust question"));
            return;
        }
        self.prompted.retain(|r| *r != id);
        self.aim_at_pane(id, out, |host| Effect::SendKeys {
            host,
            bytes: TRUST_YES_KEYS.to_vec(),
        });
    }

    fn prompt_seen(&mut self, id: RecordId, seen: bool) {
        let marked = self.prompted.contains(&id);
        if seen && !marked && self.session(id).is_some() {
            self.prompted.push(id);
        } else if !seen && marked {
            self.prompted.retain(|r| *r != id);
        }
    }

    /// The project config file: read, edited and saved, and its entries'
    /// approvals.
    fn definition_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::ProjectConfigRead { project, result } => {
                self.project_config_read(project, result, now, out);
            }
            AppAction::SaveProjectConfig { project, text } => {
                self.save_project_config(project, text, out);
            }
            AppAction::ProjectConfigWritten { project, result } => {
                self.project_config_written(project, result, now, out);
            }
            AppAction::ApproveDefinition(id) => self.approve_definition(id, out),
            AppAction::RevokeApproval(id) => self.revoke_approval(id, out),
            _ => unreachable!("not a definition action"),
        }
    }

    /// Project edits, the document hand-offs, and the preferences, split
    /// out of `dispatch` for length.
    fn files_and_settings(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::RenameProject(id, name) => {
                self.edit_project(id, out, |p| p.name = name);
            }
            AppAction::MoveSession(id, project) => self.move_session(id, project, out),
            AppAction::PinDocument(id, path) => self.edit_project(id, out, |p| {
                if !p.pinned.contains(&path) {
                    p.pinned.push(path);
                }
            }),
            AppAction::UnpinDocument(id, path) => {
                self.edit_project(id, out, |p| p.pinned.retain(|d| *d != path));
            }
            AppAction::ShowDocument(pid, path) => self.show(View::Document(pid, path), now, out),
            AppAction::ShowFileRef { record, path, line } => {
                self.show_file_ref(record, path, line, now, out);
            }
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
            AppAction::SetPromptBox(on) => self.update_settings(out, |s| s.prompt_box = on),
            AppAction::SetVoiceSettings(voice) => self.update_settings(out, |s| s.voice = voice),
            AppAction::StoreVoiceKey(value) => {
                let account = VOICE_KEY_ACCOUNT.to_owned();
                if value.trim().is_empty() {
                    out.push(Effect::DeleteSecret(account));
                } else {
                    out.push(Effect::StoreSecret {
                        account,
                        value: value.trim().to_owned(),
                    });
                }
            }
            AppAction::SetSideTab(tab) => self.update_settings(out, |s| s.side_tab = tab),
            AppAction::SetSideLeft(left) => self.update_settings(out, |s| s.side_left = left),
            AppAction::SetFileRoot(pid, dir) => self.set_file_root(pid, dir, out),
            AppAction::MainWindowMoved(main) => {
                self.update_settings(out, |s| s.main_window = Some(main));
            }
            AppAction::SetMonitorZoom(monitor, percent) => {
                // One zoom notice at a time: a run of key presses reads
                // as a changing number, not a queue of toasts.
                self.notices.retain(|n| !n.text.starts_with(ZOOM_NOTICE));
                self.info(format!("{ZOOM_NOTICE}{percent}% on {monitor}"), now);
                self.update_settings(out, |s| {
                    s.monitor_zoom.retain(|z| z.monitor != monitor);
                    if percent != 100 {
                        s.monitor_zoom.push(MonitorZoom { monitor, percent });
                    }
                });
            }
            AppAction::PopOut(id) => self.pop_out(id, out),
            AppAction::ClosePopout(id) => {
                self.update_settings(out, |s| s.popouts.retain(|p| p.session != id));
            }
            AppAction::PopoutMoved(id, frame) => self.popout_moved(id, frame, out),
            // Everything else is routed by `dispatch` itself.
            _ => unreachable!("dispatched by `dispatch` itself"),
        }
    }

    pub(super) fn update_settings(&mut self, out: &mut Out, change: impl FnOnce(&mut Settings)) {
        let mut next = self.settings.clone();
        change(&mut next);
        if next != self.settings {
            if next.space != self.settings.space {
                self.saved_space = None;
            }
            self.settings = next;
            if !self.read_only {
                let mut saved = self.settings.clone();
                if let Some(space) = self.saved_space {
                    saved.space = space;
                }
                out.push(Effect::SaveSettings(saved));
            }
        }
    }

    #[must_use]
    pub fn host_status(&self, id: RecordId) -> Option<&HostStatus> {
        let name = id.host_name();
        self.host.iter().find(|h| h.id.0 == name)
    }
    /// Type a message into the record's pane. A message that reaches the
    /// pane ends the chance to undo a discard; one that finds no pane
    /// does not.
    fn send_to_pane(&mut self, id: RecordId, out: &mut Out, effect: impl FnOnce(HostId) -> Effect) {
        if self.host_status(id).is_some() && self.session(id).is_some_and(|s| s.discard.is_some()) {
            self.edit_session(id, out, |s| s.discard = None);
        }
        self.aim_at_pane(id, out, effect);
    }
    /// Emit an effect aimed at a record's running pane, or a notice when
    /// there is none.
    pub(super) fn aim_at_pane(
        &mut self,
        id: RecordId,
        out: &mut Out,
        effect: impl FnOnce(HostId) -> Effect,
    ) {
        if let Some(host) = self.running_host(id) {
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
                // An outside process (Dispatch) says a decision waits here.
                _ if record.waiting_on.is_some() => CardState::WaitingOnYou,
                // The session asked the owner with `switchboard-ask`.
                _ if record.asking.is_some() => CardState::WaitingOnYou,
                // Claude's own trust question, before any hook can say so.
                _ if self.prompted.contains(&id) => CardState::WaitingOnYou,
                // A review's agent gone quiet mid-round is most likely
                // sitting at an approval prompt: the user's turn.
                _ if self.stalled_agents().any(|a| a == id) => CardState::WaitingOnYou,
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
    /// Whether `set` exists and may move to another space. A global set
    /// may not: moved out, it would lose its cards from every other
    /// space. The set header offers the move only when this is true.
    #[must_use]
    pub fn set_movable(&self, set: SetId) -> bool {
        self.working_set(set).is_some_and(|s| !s.space.is_global())
    }
    /// The oldest notice as the active space may show it: one about a
    /// space it does not contain says only [`Notice::ELSEWHERE`], so no
    /// name crosses the space boundary. The global space contains every
    /// space and shows every notice as written.
    #[must_use]
    pub fn notice_shown(&self) -> Option<Notice> {
        let mut notice = self.notice()?.clone();
        if notice
            .space
            .is_some_and(|s| !self.settings.space.contains(s))
        {
            Notice::ELSEWHERE.clone_into(&mut notice.text);
        }
        Some(notice)
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
    /// Sessions waiting on the user in every space, and Dispatch's
    /// pending decisions: the Dock badge.
    #[must_use]
    pub fn waiting_count(&self) -> usize {
        self.waiting_count_in(SpaceId::GLOBAL) + self.pending_decisions().len()
    }
}
