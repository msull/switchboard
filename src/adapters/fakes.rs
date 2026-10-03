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
use crate::ports::control::{OpLine, Operations};
use crate::ports::controller::{Controller, ControllerEvent};
use crate::ports::dispatch::{Body as DispatchBody, DispatchPort, Reply as DispatchReply, Status};
use crate::ports::events::{EventSource, SessionEvent};
use crate::ports::host::{HostId, HostInfo, HostStatus, ProcessHost, SpawnSpec};
use crate::ports::opener::Opener;
use crate::ports::project_config::{ProjectConfig, ProjectConfigReader};
use crate::ports::store::{Loaded, Store, StoreError};
use crate::ports::transcript::{Conversation, TranscriptReader};

/// Every port as its fake's default, Dispatch included, and no wake
/// socket. Tests name the fields they script and take the rest from
/// here with struct update syntax.
#[must_use]
pub fn services() -> Services {
    Services {
        store: Box::new(MemoryStore::default()),
        host: Box::new(FakeHost::default()),
        events: Box::new(FakeEvents),
        agents: Box::new(FakeAgents),
        opener: Box::new(FakeOpener::default()),
        transcripts: Box::new(FakeTranscripts::default()),
        secrets: Box::new(FakeSecrets::default()),
        project_config: Box::new(FakeProjectConfig),
        round_files: Box::new(FakeRoundFiles),
        artifacts: Box::new(FakeArtifacts),
        controller: Box::new(FakeController),
        operations: Box::new(FakeOperations::default()),
        dispatch: Some(Box::new(FakeDispatch::default())),
        wake: None,
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
    fn spawn(&self, _spec: &SpawnSpec) -> std::io::Result<()> {
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
        Ok(self.state().snapshots.get(id).cloned().unwrap_or_default())
    }
    fn write(&self, _id: &HostId, _bytes: &[u8]) -> std::io::Result<()> {
        if let Some(e) = &self.state().fail_write {
            return Err(std::io::Error::other(e.clone()));
        }
        Ok(())
    }
    fn write_line(&self, id: &HostId, text: &str) -> std::io::Result<()> {
        self.write(id, text.as_bytes())?;
        self.write(id, b"\r")
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

/// Dispatch as a test sets it: a status to answer with (none means no
/// runner).
#[derive(Debug, Default, Clone)]
pub struct FakeDispatch {
    pub status: Option<Status>,
    /// Answer from the app's port thread, as the real socket does.
    pub blocks: bool,
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
                    DispatchReply::Ticket,
                ),
            DispatchBody::Artifact { .. } => DispatchReply::failed("no such file"),
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
        })
    }
}

/// The operations log in memory, shared so a test can read it back.
#[derive(Debug, Default, Clone)]
pub struct FakeOperations {
    pub lines: Arc<Mutex<Vec<OpLine>>>,
}

impl Operations for FakeOperations {
    fn append(&mut self, line: &OpLine) -> std::io::Result<()> {
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

/// Secrets in memory, shared so a test can read back what the app stored.
#[derive(Debug, Clone, Default)]
pub struct FakeSecrets(pub Arc<Mutex<HashMap<String, String>>>);

impl FakeSecrets {
    pub fn state(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.0.lock().expect("fake secrets lock")
    }
}

impl crate::ports::secrets::SecretStore for FakeSecrets {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
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
/// a missing transcript.
#[derive(Debug, Default)]
pub struct FakeTranscripts {
    pub conversation: Option<Conversation>,
}

impl TranscriptReader for FakeTranscripts {
    fn read(&self, _handle: &ResumeHandle) -> Result<Conversation, String> {
        self.conversation
            .clone()
            .ok_or_else(|| "no transcript".to_owned())
    }
    fn modified(&self, _handle: &ResumeHandle) -> Option<SystemTime> {
        // A fixed time: the fake never changes, so the app reads it once.
        self.conversation.as_ref().map(|_| std::time::UNIX_EPOCH)
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
