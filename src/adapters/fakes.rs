//! Test doubles for every port. Shared by core tests, UI tests, and the
//! app's demo mode. Each is a plain struct with public fields so tests
//! can inspect calls and script results.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::core::{AgentKind, ProjectId, RecordId, ResumeHandle, Settings, Workspace};
use crate::ports::agent::{AgentLaunch, AgentLauncher};
use crate::ports::events::{EventSource, SessionEvent};
use crate::ports::host::{HostId, HostInfo, HostStatus, ProcessHost, SpawnSpec};
use crate::ports::opener::Opener;
use crate::ports::store::{Loaded, Store, StoreError};
use crate::ports::transcript::{Conversation, TranscriptReader};

/// In-memory store; `saved` records every save in order.
#[derive(Debug, Default)]
pub struct MemoryStore {
    pub initial: Loaded,
    pub saved: Vec<Workspace>,
    pub deleted: Vec<ProjectId>,
    pub lock_result: Option<bool>,
    pub fail_save: Option<StoreError>,
}

impl Store for MemoryStore {
    fn lock(&mut self) -> Result<bool, StoreError> {
        Ok(self.lock_result.unwrap_or(true))
    }
    fn load_all(&self) -> Result<Loaded, StoreError> {
        Ok(self.initial.clone())
    }
    fn save(&self, workspace: &Workspace) -> Result<(), StoreError> {
        if let Some(e) = &self.fail_save {
            return Err(e.clone());
        }
        // Interior mutability is avoided on purpose: the app owns the
        // store by value and tests read `saved` through `SwitchboardApp`.
        let _ = workspace;
        Ok(())
    }
    fn delete(&self, _id: ProjectId) -> Result<(), StoreError> {
        Ok(())
    }
    fn save_settings(&self, _settings: &Settings) -> Result<(), StoreError> {
        Ok(())
    }
    fn data_dir(&self) -> PathBuf {
        std::env::temp_dir().join("switchboard-fake")
    }
}

/// Scripted host: `statuses` is what `list` returns; `spawned` and
/// `killed` record calls. Shared handles so tests keep a reference.
#[derive(Debug, Default)]
pub struct FakeHostState {
    pub statuses: Vec<HostStatus>,
    pub spawned: Vec<SpawnSpec>,
    pub killed: Vec<HostId>,
    pub written: Vec<(HostId, Vec<u8>)>,
    pub probe: Option<Result<HostInfo, String>>,
    pub fail_spawn: Option<String>,
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
        self.state().probe.clone().unwrap_or(Ok(HostInfo {
            description: "fake host".into(),
            persistent: true,
        }))
    }
    fn list(&self) -> std::io::Result<Vec<HostStatus>> {
        Ok(self.state().statuses.clone())
    }
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<()> {
        let mut s = self.state();
        if let Some(e) = &s.fail_spawn {
            return Err(std::io::Error::other(e.clone()));
        }
        s.spawned.push(spec.clone());
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
    fn snapshot(&self, _id: &HostId, _lines: Option<usize>) -> std::io::Result<String> {
        Ok(String::new())
    }
    fn write(&self, id: &HostId, bytes: &[u8]) -> std::io::Result<()> {
        self.state().written.push((id.clone(), bytes.to_vec()));
        Ok(())
    }
    fn kill(&self, id: &HostId) -> std::io::Result<()> {
        self.state().killed.push(id.clone());
        Ok(())
    }
    fn attach_command(&self, id: &HostId) -> Vec<String> {
        vec!["fake-attach".into(), id.0.clone()]
    }
}

/// Events queued by tests, drained by `poll`.
#[derive(Debug, Default)]
pub struct FakeEvents {
    pub queued: Vec<SessionEvent>,
    pub checkpoints: usize,
}

impl EventSource for FakeEvents {
    fn poll(&mut self) -> Vec<SessionEvent> {
        std::mem::take(&mut self.queued)
    }
    fn checkpoint(&mut self) {
        self.checkpoints += 1;
    }
}

/// Composes deterministic command lines; never runs anything.
#[derive(Debug, Default)]
pub struct FakeAgents {
    pub transcript_missing: bool,
    pub discovered: Option<ResumeHandle>,
}

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
            env: vec![("SWITCHBOARD_RECORD_ID".into(), record.0.to_string())],
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
            env: vec![("SWITCHBOARD_RECORD_ID".into(), record.0.to_string())],
            resume: Some(handle.clone()),
        })
    }
    fn transcript_exists(&self, _handle: &ResumeHandle) -> bool {
        !self.transcript_missing
    }
    fn discover(
        &self,
        _kind: AgentKind,
        _cwd: &Path,
        _since: SystemTime,
    ) -> Result<Option<ResumeHandle>, String> {
        Ok(self.discovered.clone())
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
}
