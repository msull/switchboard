//! Agent launchers: compose launch and resume command lines per agent,
//! check transcript availability, discover ids for agents that cannot
//! be assigned one (Codex).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::{AgentKind, RecordId, ResumeHandle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunch {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Known before spawn for Claude Code; `None` for Codex until discovered.
    pub resume: Option<ResumeHandle>,
}

pub trait AgentLauncher: Send + Sync {
    fn available(&self, kind: AgentKind) -> bool;
    /// A fresh session. Writes nothing into `cwd`.
    fn prepare_launch(
        &self,
        kind: AgentKind,
        record: RecordId,
        name: &str,
        cwd: &Path,
    ) -> Result<AgentLaunch, String>;
    fn prepare_resume(
        &self,
        handle: &ResumeHandle,
        record: RecordId,
        name: &str,
        cwd: &Path,
    ) -> Result<AgentLaunch, String>;
    /// Preflight: does the provider still have the transcript?
    fn transcript_exists(&self, handle: &ResumeHandle) -> bool;
    /// For Codex: the rollout created in `cwd` after `since`, if exactly
    /// one appeared. Launches are serialized by the caller.
    fn discover(
        &self,
        kind: AgentKind,
        cwd: &Path,
        since: SystemTime,
    ) -> Result<Option<ResumeHandle>, String>;
    /// Path of the transcript for a handle, if the provider keeps one.
    fn transcript_path(&self, handle: &ResumeHandle) -> Option<PathBuf>;
}
