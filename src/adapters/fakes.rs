//! Test doubles for every port, for the UI and integration tests (core
//! tests need none: the core does no I/O). `FakeSecrets` is also the
//! secret store off macOS. Each is a plain struct with public fields so
//! tests can script results.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::app::Services;
use crate::core::{
    AgentKind, ProjectId, RECORD_ID_ENV, RecordId, ResumeHandle, Settings, Views, Workspace,
};
use crate::ports::agent::{AgentLaunch, AgentLauncher};
use crate::ports::changes::{BranchChanges, Changes, FileDiff};
use crate::ports::control::{OpLine, Operations};
use crate::ports::controller::{Controller, ControllerEvent};
use crate::ports::dispatch::{
    Body as DispatchBody, DispatchPort, EventView, EventsView, PathsView, Reply as DispatchReply,
    Status,
};
use crate::ports::events::{EventSource, SessionEvent};
use crate::ports::host::{HostId, HostInfo, HostStatus, ProcessHost, SpawnSpec};
use crate::ports::opener::Opener;
use crate::ports::project_config::{ProjectConfig, ProjectConfigReader};
use crate::ports::remote::{Advertisement, Advertiser, Identity, PairedDevice, RemoteStore};
use crate::ports::store::{Loaded, Store, StoreError};
use crate::ports::transcript::{Conversation, TranscriptReader, TranscriptVersion};

/// Every port as its fake's default, Dispatch included, and no wake
/// socket. Tests name the fields they script and take the rest from
/// here with struct update syntax.
#[must_use]
pub fn services() -> Services {
    Services {
        store: Box::new(MemoryStore::default()),
        host: Arc::new(FakeHost::default()),
        events: Box::new(FakeEvents),
        agents: Box::new(FakeAgents),
        opener: Box::new(FakeOpener::default()),
        remote: Box::new(FakeRemoteStore::default()),
        advertiser: Box::new(FakeAdvertiser::default()),
        transcripts: Arc::new(FakeTranscripts::default()),
        secrets: std::sync::Arc::new(FakeSecrets::default()),
        project_config: Box::new(FakeProjectConfig),
        round_files: Box::new(FakeRoundFiles),
        changes: Arc::new(FakeChanges::default()),
        artifacts: Box::new(FakeArtifacts),
        controller: Box::new(FakeController),
        operations: Box::new(FakeOperations::default()),
        dispatch: Some(Box::new(FakeDispatch::default())),
        wake: None,
        updates: Arc::new(FakeUpdates::default()),
    }
}

/// A release source that answers what the test put in it and records
/// what was fetched and installed.
#[derive(Debug)]
pub struct FakeUpdates {
    pub latest: Mutex<Result<Option<crate::ports::updates::Release>, String>>,
    /// What `download` answers; the release fetched is recorded.
    pub staged: Mutex<Result<PathBuf, String>>,
    pub downloaded: Mutex<Vec<crate::ports::updates::Release>>,
    pub installed: Mutex<Vec<PathBuf>>,
}

impl Default for FakeUpdates {
    fn default() -> Self {
        Self {
            latest: Mutex::new(Ok(None)),
            staged: Mutex::new(Ok(PathBuf::from("/tmp/staged/Switchboard.app"))),
            downloaded: Mutex::new(Vec::new()),
            installed: Mutex::new(Vec::new()),
        }
    }
}

impl crate::ports::updates::UpdateSource for FakeUpdates {
    fn latest(&self) -> Result<Option<crate::ports::updates::Release>, String> {
        self.latest.lock().unwrap().clone()
    }

    fn download(&self, release: &crate::ports::updates::Release) -> Result<PathBuf, String> {
        self.downloaded.lock().unwrap().push(release.clone());
        self.staged.lock().unwrap().clone()
    }

    fn install(&self, staged: &Path) -> Result<(), String> {
        self.installed.lock().unwrap().push(staged.to_path_buf());
        Ok(())
    }
}

/// In-memory store. Saves are not recorded: the core's `Effect::Save`
/// is what tests assert on, so a save here only succeeds or fails.
#[derive(Debug, Default)]
pub struct MemoryStore {
    pub initial: Loaded,
    pub lock_result: Option<bool>,
}

impl Store for MemoryStore {
    fn lock(&mut self) -> Result<bool, StoreError> {
        Ok(self.lock_result.unwrap_or(true))
    }
    fn load_all(&self) -> Result<Loaded, StoreError> {
        Ok(self.initial.clone())
    }
    fn save(&self, _workspace: &Workspace) -> Result<(), StoreError> {
        Ok(())
    }
    fn delete(&self, _id: ProjectId) -> Result<(), StoreError> {
        Ok(())
    }
    fn save_settings(&self, _settings: &Settings) -> Result<(), StoreError> {
        Ok(())
    }
    fn save_views(&self, _views: &Views) -> Result<(), StoreError> {
        Ok(())
    }
    fn data_dir(&self) -> PathBuf {
        std::env::temp_dir().join("switchboard-fake")
    }
}

/// Scripted host: `statuses` is what `list` returns. Shared handles so
/// tests keep a reference.
#[derive(Debug, Default)]
pub struct FakeHostState {
    pub statuses: Vec<HostStatus>,
    /// Every write fails with this message (a pane that died).
    pub fail_write: Option<String>,
    /// What `snapshot` returns per pane; unknown panes read as empty.
    pub snapshots: HashMap<HostId, String>,
    /// Every spec `spawn` was given, oldest first.
    pub spawned: Vec<SpawnSpec>,
    /// Every line `write_line` put in a pane, oldest first.
    pub written: Vec<(HostId, String)>,
    /// Every raw `write` into a pane, oldest first: keys as well as
    /// the bytes of each line.
    pub keys: Vec<(HostId, Vec<u8>)>,
    /// The next `snapshot` panics, once: a read that blows up.
    pub panic_snapshot: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FakeHost(pub Arc<Mutex<FakeHostState>>);

impl FakeHost {
    pub fn state(&self) -> std::sync::MutexGuard<'_, FakeHostState> {
        self.0.lock().expect("fake host lock")
    }
}

impl ProcessHost for FakeHost {
    fn probe(&self) -> Result<HostInfo, String> {
        Ok(HostInfo {
            description: "fake host".into(),
            persistent: true,
        })
    }
    fn list(&self) -> std::io::Result<Vec<HostStatus>> {
        Ok(self.state().statuses.clone())
    }
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<()> {
        self.state().spawned.push(spec.clone());
        Ok(())
    }
    fn status(&self, id: &HostId) -> std::io::Result<HostStatus> {
        self.state()
            .statuses
            .iter()
            .find(|s| &s.id == id)
            .cloned()
            .ok_or_else(|| std::io::Error::other("no such session"))
    }
    fn snapshot(&self, id: &HostId, _lines: Option<usize>) -> std::io::Result<String> {
        // Taken in its own statement so the guard is dropped before the
        // panic, which would otherwise poison the fake for the test.
        let boom = std::mem::take(&mut self.state().panic_snapshot);
        assert!(!boom, "snapshot panicked on purpose");
        Ok(self.state().snapshots.get(id).cloned().unwrap_or_default())
    }
    fn write(&self, id: &HostId, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(e) = &self.state().fail_write {
            return Err(std::io::Error::other(e.clone()));
        }
        self.state().keys.push((id.clone(), bytes.to_vec()));
        Ok(())
    }
    fn write_line(&self, id: &HostId, text: &str) -> std::io::Result<()> {
        self.write(id, text.as_bytes())?;
        self.write(id, b"\r")?;
        self.state().written.push((id.clone(), text.to_owned()));
        Ok(())
    }
    fn kill(&self, _id: &HostId) -> std::io::Result<()> {
        Ok(())
    }
    fn attach_command(&self, id: &HostId) -> Vec<String> {
        vec!["fake-attach".into(), id.0.clone()]
    }
}

/// No hook events, ever.
#[derive(Debug, Default)]
pub struct FakeEvents;

impl EventSource for FakeEvents {
    fn poll(&mut self) -> Vec<SessionEvent> {
        Vec::new()
    }
    fn checkpoint(&mut self) {}
}

/// A branch's changes as a test sets them, whatever is asked.
#[derive(Debug, Default, Clone)]
pub struct FakeChanges {
    pub changes: Changes,
    /// Each file's diff by path; a missing path answers `Err("no diff")`.
    pub diffs: HashMap<String, FileDiff>,
}

impl BranchChanges for FakeChanges {
    fn read(&self, _dir: &Path, _base: &str, _head: &str) -> Result<Changes, String> {
        Ok(self.changes.clone())
    }
    fn diff(
        &self,
        _dir: &Path,
        _base: &str,
        _head: &str,
        path: &str,
        _old_path: Option<&str>,
    ) -> Result<FileDiff, String> {
        self.diffs
            .get(path)
            .cloned()
            .ok_or_else(|| "no diff".to_owned())
    }
}

/// Dispatch as a test sets it: a status to answer with (none means no
/// runner).
#[derive(Debug, Default, Clone)]
pub struct FakeDispatch {
    pub status: Option<Status>,
    /// Answer from the app's port thread, as the real socket does.
    pub blocks: bool,
    /// The log `events` reads, every ticket's.
    pub events: Vec<EventView>,
    /// The paths a single-ticket reply carries.
    pub paths: PathsView,
    /// The text an artifact read answers, by path; any other path is
    /// `no such file`, and a text over `FILE_TEXT_MAX` is refused with
    /// its size, as Dispatch refuses it.
    pub artifacts: HashMap<PathBuf, String>,
    /// What a usage read answers, by ticket; any other ticket is
    /// answered as a runner from before usage was served.
    pub usage: HashMap<String, dispatch_control::UsageView>,
    /// What `build_on_disk` answers.
    pub on_disk: Option<dispatch_control::BuildId>,
}

impl FakeDispatch {
    /// The text held for `path`, refused with its size past the bound
    /// as Dispatch refuses it, else `no such file`.
    fn artifact_reply(&self, path: &Path) -> DispatchReply {
        match self.artifacts.get(path) {
            None => DispatchReply::failed("no such file"),
            Some(text) if text.len() as u64 > dispatch_control::FILE_TEXT_MAX => {
                DispatchReply::failed(dispatch_control::too_large_reason(path, text.len() as u64))
            }
            Some(text) => DispatchReply::Artifact { text: text.clone() },
        }
    }

    /// The usage set for `ticket`, else the answer of a runner from
    /// before usage was served.
    fn usage_reply(&self, ticket: &str) -> DispatchReply {
        self.usage.get(ticket).cloned().map_or_else(
            || DispatchReply::failed("bad request: unknown variant `usage`"),
            DispatchReply::Usage,
        )
    }
}

impl DispatchPort for FakeDispatch {
    fn command(&self) -> PathBuf {
        PathBuf::from("/opt/sb/dispatch")
    }
    fn data_dir(&self) -> PathBuf {
        PathBuf::from("/dispatch")
    }
    fn may_block(&self) -> bool {
        self.blocks
    }
    fn build_on_disk(&self) -> Option<dispatch_control::BuildId> {
        self.on_disk.clone()
    }
    fn call(&mut self, body: &DispatchBody) -> std::io::Result<DispatchReply> {
        let Some(status) = &self.status else {
            return Err(std::io::Error::other("no runner"));
        };
        Ok(match body {
            DispatchBody::Status => DispatchReply::Status(status.clone()),
            DispatchBody::Ticket { id } => status
                .tickets
                .iter()
                .find(|t| &t.id == id)
                .cloned()
                .map_or_else(
                    || DispatchReply::failed("no such ticket"),
                    |mut t| {
                        t.paths = self.paths.clone();
                        for l in &mut t.lanes {
                            l.clone = Some(l.worktree.clone());
                        }
                        DispatchReply::Ticket(t)
                    },
                ),
            // The fake's events carry no ticket of their own: each is
            // every ticket's.
            DispatchBody::Events { since, .. } => {
                let events: Vec<EventView> = self
                    .events
                    .iter()
                    .filter(|e| e.seq > *since)
                    .cloned()
                    .collect();
                let last = events.iter().map(|e| e.seq).max().unwrap_or(*since);
                DispatchReply::Events(EventsView {
                    events,
                    last,
                    withdrawn: Vec::new(),
                })
            }
            DispatchBody::Usage { ticket } => self.usage_reply(ticket),
            DispatchBody::Artifact { path, .. } => self.artifact_reply(path),
            DispatchBody::Decide {
                ticket, decision, ..
            } => status
                .tickets
                .iter()
                .find(|t| &t.id == ticket)
                .and_then(|t| t.decisions.iter().find(|d| &d.id == decision))
                .cloned()
                .map_or_else(
                    || DispatchReply::failed("no such decision"),
                    DispatchReply::Decided,
                ),
            DispatchBody::Queue { order, .. } => DispatchReply::Queue {
                order: order.clone(),
            },
            DispatchBody::Take { .. } => DispatchReply::failed("takes are not faked"),
            DispatchBody::Worktrees { path, .. } => {
                DispatchReply::Worktrees(dispatch_control::WorktreesView {
                    root: path.clone().unwrap_or_else(|| status.worktrees.clone()),
                    ..Default::default()
                })
            }
            DispatchBody::Resume { ticket } => status
                .tickets
                .iter()
                .find(|t| &t.id == ticket)
                .cloned()
                .map_or_else(
                    || DispatchReply::failed("no such ticket"),
                    DispatchReply::Ticket,
                ),
            DispatchBody::Close { ticket, reason } => status
                .tickets
                .iter()
                .find(|t| &t.id == ticket)
                .cloned()
                .map_or_else(
                    || DispatchReply::failed("no such ticket"),
                    |mut t| {
                        // As the runner answers: the intent written, the
                        // rest left to its next pass. A closed ticket
                        // whose trees were kept stays closed.
                        if t.state != "closed" {
                            t.state = "closing".into();
                            t.reason =
                                Some(reason.clone().unwrap_or_else(|| "closed by hand".into()));
                        }
                        t.closable = false;
                        DispatchReply::Ticket(t)
                    },
                ),
            // As the runner answers: the intent saved, the status at once.
            DispatchBody::SupervisorFresh { project } => {
                let mut status = status.clone();
                if let Some(s) = status
                    .projects
                    .iter_mut()
                    .find(|p| &p.name == project)
                    .and_then(|p| p.supervisor.as_mut())
                {
                    s.fresh_pending = true;
                }
                DispatchReply::Status(status)
            }
        })
    }
}

/// The operations log in memory, shared so a test can read it back.
#[derive(Debug, Default, Clone)]
pub struct FakeOperations {
    pub lines: Arc<Mutex<Vec<OpLine>>>,
    /// Every reply line fails to append (the disk went away between a
    /// command's run and its reply), request lines still land.
    pub fail_replies: Arc<std::sync::atomic::AtomicBool>,
}

impl Operations for FakeOperations {
    fn append(&mut self, line: &OpLine) -> std::io::Result<()> {
        if matches!(line, OpLine::Replied { .. })
            && self.fail_replies.load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(std::io::Error::other("the log is not writable"));
        }
        self.lines.lock().unwrap().push(line.clone());
        Ok(())
    }
    fn find(&self, op: &str) -> Vec<OpLine> {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.op() == op)
            .cloned()
            .collect()
    }
}

/// No controller attached: no input, and the waiting count goes
/// nowhere.
#[derive(Debug, Default)]
pub struct FakeController;

impl Controller for FakeController {
    fn poll(&mut self) -> Vec<ControllerEvent> {
        Vec::new()
    }
    fn set_waiting(&mut self, _count: usize) {}
}

/// Composes deterministic command lines; never runs anything.
#[derive(Debug, Default)]
pub struct FakeAgents;

impl AgentLauncher for FakeAgents {
    fn available(&self, _kind: AgentKind) -> bool {
        true
    }
    fn prepare_launch(
        &self,
        kind: AgentKind,
        record: RecordId,
        name: &str,
        _cwd: &Path,
    ) -> Result<AgentLaunch, String> {
        let resume = match kind {
            AgentKind::ClaudeCode => Some(ResumeHandle::ClaudeCode {
                session_id: uuid::Uuid::new_v4(),
                transcript: None,
            }),
            AgentKind::Codex => None,
        };
        Ok(AgentLaunch {
            argv: vec!["fake-agent".into(), kind.label().into(), name.into()],
            env: vec![(RECORD_ID_ENV.into(), record.0.to_string())],
            resume,
        })
    }
    fn prepare_resume(
        &self,
        handle: &ResumeHandle,
        record: RecordId,
        _name: &str,
        _cwd: &Path,
    ) -> Result<AgentLaunch, String> {
        Ok(AgentLaunch {
            argv: vec!["fake-agent".into(), "resume".into(), handle.provider_id()],
            env: vec![(RECORD_ID_ENV.into(), record.0.to_string())],
            resume: Some(handle.clone()),
        })
    }
    fn transcript_exists(&self, _handle: &ResumeHandle) -> bool {
        true
    }
    fn discover(
        &self,
        _kind: AgentKind,
        _cwd: &Path,
        _since: SystemTime,
    ) -> Result<Option<ResumeHandle>, String> {
        Ok(None)
    }
    fn transcript_path(&self, handle: &ResumeHandle) -> Option<PathBuf> {
        handle.transcript().cloned()
    }
}

/// Records every hand-off instead of performing it.
#[derive(Debug, Default)]
pub struct FakeOpenerState {
    pub opened: Vec<PathBuf>,
    pub revealed: Vec<PathBuf>,
    pub edited: Vec<(String, PathBuf)>,
    pub terminals: Vec<(String, Vec<String>, PathBuf)>,
    pub raised: Vec<String>,
    /// Titles `raise_terminal` reports as existing.
    pub existing: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FakeOpener(pub Arc<Mutex<FakeOpenerState>>);

impl FakeOpener {
    pub fn state(&self) -> std::sync::MutexGuard<'_, FakeOpenerState> {
        self.0.lock().expect("fake opener lock")
    }
}

impl Opener for FakeOpener {
    fn open_default(&self, path: &Path) -> Result<(), String> {
        self.state().opened.push(path.to_path_buf());
        Ok(())
    }
    fn reveal(&self, path: &Path) -> Result<(), String> {
        self.state().revealed.push(path.to_path_buf());
        Ok(())
    }
    fn open_editor(&self, editor: &str, path: &Path) -> Result<(), String> {
        self.state()
            .edited
            .push((editor.into(), path.to_path_buf()));
        Ok(())
    }
    fn open_terminal(&self, title: &str, argv: &[String], cwd: &Path) -> Result<(), String> {
        self.state()
            .terminals
            .push((title.into(), argv.to_vec(), cwd.to_path_buf()));
        Ok(())
    }
    fn raise_terminal(&self, title: &str) -> Result<bool, String> {
        let mut s = self.state();
        s.raised.push(title.into());
        Ok(s.existing.iter().any(|t| t == title))
    }
}

/// The identity and device files in memory, shared so a test can read
/// back what the app saved or pre-fill what it loads.
#[derive(Debug, Clone, Default)]
pub struct FakeRemoteStore(pub Arc<Mutex<FakeRemoteState>>);

#[derive(Debug, Default)]
pub struct FakeRemoteState {
    pub identity: Option<Identity>,
    pub devices: Vec<PairedDevice>,
    /// Every load and save fails with this message.
    pub fail: Option<String>,
}

impl FakeRemoteStore {
    pub fn state(&self) -> std::sync::MutexGuard<'_, FakeRemoteState> {
        self.0.lock().expect("fake remote store lock")
    }
}

impl RemoteStore for FakeRemoteStore {
    fn load_identity(&self) -> Result<Option<Identity>, String> {
        let state = self.state();
        state.fail.clone().map_or(Ok(state.identity.clone()), Err)
    }
    fn save_identity(&self, identity: &Identity) -> Result<(), String> {
        let mut state = self.state();
        if let Some(e) = &state.fail {
            return Err(e.clone());
        }
        state.identity = Some(identity.clone());
        Ok(())
    }
    fn load_devices(&self) -> Result<Vec<PairedDevice>, String> {
        let state = self.state();
        state.fail.clone().map_or(Ok(state.devices.clone()), Err)
    }
    fn save_devices(&self, devices: &[PairedDevice]) -> Result<(), String> {
        let mut state = self.state();
        if let Some(e) = &state.fail {
            return Err(e.clone());
        }
        state.devices = devices.to_vec();
        Ok(())
    }
}

/// Secrets in memory, shared so a test can read back what the app stored.
#[derive(Debug, Clone, Default)]
pub struct FakeSecrets {
    values: Arc<Mutex<HashMap<String, String>>>,
    /// Every read fails while set, as a locked Keychain would.
    pub failing: Arc<std::sync::atomic::AtomicBool>,
}

impl FakeSecrets {
    pub fn state(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.values.lock().expect("fake secrets lock")
    }

    pub fn set_failing(&self, failing: bool) {
        self.failing
            .store(failing, std::sync::atomic::Ordering::SeqCst);
    }
}

impl crate::ports::secrets::SecretStore for FakeSecrets {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        if self.failing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("the store is locked".into());
        }
        Ok(self.state().get(account).cloned())
    }
    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        self.state().insert(account.into(), value.into());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        self.state().remove(account);
        Ok(())
    }
}

/// Serves one scripted conversation for every handle; `None` reads as
/// a missing transcript. Shared handles so a test can replace the
/// conversation under a running app, which moves the version.
#[derive(Debug, Default, Clone)]
pub struct FakeTranscripts {
    state: Arc<Mutex<FakeTranscriptState>>,
}

#[derive(Debug, Default)]
pub struct FakeTranscriptState {
    pub conversation: Option<Conversation>,
    /// Bumped by every `set`, so a version names each conversation.
    pub version: u64,
}

impl FakeTranscripts {
    #[must_use]
    pub fn with(conversation: Conversation) -> Self {
        let fake = Self::default();
        fake.set(Some(conversation));
        fake
    }

    /// Replace the conversation every handle reads; the version moves.
    pub fn set(&self, conversation: Option<Conversation>) {
        let mut state = self.state.lock().expect("fake transcripts");
        state.conversation = conversation;
        state.version += 1;
    }
}

impl TranscriptReader for FakeTranscripts {
    fn read(&self, _handle: &ResumeHandle) -> Result<Conversation, String> {
        self.state
            .lock()
            .expect("fake transcripts")
            .conversation
            .clone()
            .ok_or_else(|| "no transcript".to_owned())
    }
    fn modified(&self, handle: &ResumeHandle) -> Option<SystemTime> {
        self.version(handle)?.modified
    }
    fn version(&self, _handle: &ResumeHandle) -> Option<TranscriptVersion> {
        let state = self.state.lock().expect("fake transcripts");
        // A fixed time and the set count as the size: the app reads a
        // conversation once per `set`.
        state.conversation.as_ref().map(|_| TranscriptVersion {
            size: state.version,
            modified: Some(std::time::UNIX_EPOCH),
        })
    }
    fn clone_all(&self, handle: &ResumeHandle) -> Result<ResumeHandle, String> {
        self.clone_before(handle, 0)
    }
    fn clone_before(&self, handle: &ResumeHandle, _before: usize) -> Result<ResumeHandle, String> {
        match handle {
            ResumeHandle::ClaudeCode { transcript, .. } => Ok(ResumeHandle::ClaudeCode {
                session_id: uuid::Uuid::new_v4(),
                transcript: transcript.clone(),
            }),
            ResumeHandle::Codex { .. } => Err("Codex sessions cannot be cloned".into()),
        }
    }
}

/// No project has a definition file; writes succeed and are dropped.
#[derive(Debug, Default)]
pub struct FakeProjectConfig;

impl ProjectConfigReader for FakeProjectConfig {
    fn read(&self, _root: &Path) -> Result<Option<ProjectConfig>, String> {
        Ok(None)
    }
    fn read_text(&self, _root: &Path) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn write_text(&self, _root: &Path, _text: &str) -> Result<(), String> {
        Ok(())
    }
    fn modified(&self, _root: &Path) -> Option<SystemTime> {
        None
    }
}

/// No round file is ever there; snapshots and removals succeed.
#[derive(Debug, Default, Clone)]
pub struct FakeRoundFiles;

impl crate::ports::round_files::RoundFiles for FakeRoundFiles {
    fn probe(&self, _path: &Path) -> Option<crate::ports::round_files::Probed> {
        None
    }
    fn snapshot(&self, _files: &[PathBuf], _dir: &Path, _note: Option<&str>) -> Result<(), String> {
        Ok(())
    }
    fn remove(&self, _files: &[PathBuf]) -> Result<(), String> {
        Ok(())
    }
}

/// No declared output is ever found.
#[derive(Debug, Default, Clone)]
pub struct FakeArtifacts;

impl crate::ports::artifacts::ArtifactFinder for FakeArtifacts {
    fn find(&self, _cwd: &Path, _patterns: &[String], _since: SystemTime) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Bonjour as a log: `Some` for each advertisement, `None` for each
/// withdrawal, in order.
#[derive(Debug, Clone, Default)]
pub struct FakeAdvertiser(pub Arc<Mutex<Vec<Option<Advertisement>>>>);

impl FakeAdvertiser {
    pub fn log(&self) -> std::sync::MutexGuard<'_, Vec<Option<Advertisement>>> {
        self.0.lock().expect("fake advertiser lock")
    }

    /// The advertisement in force, if the last call was not a withdrawal.
    #[must_use]
    pub fn current(&self) -> Option<Advertisement> {
        self.log().last().cloned().flatten()
    }
}

impl Advertiser for FakeAdvertiser {
    fn advertise(&mut self, ad: &Advertisement) -> Result<(), String> {
        self.log().push(Some(ad.clone()));
        Ok(())
    }
    fn withdraw(&mut self) {
        self.log().push(None);
    }
}
