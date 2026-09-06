//! Process host: where session processes live. The tmux adapter is the
//! product; a fake plays scripted state in tests. From the process-host
//! spike's proposed trait, trimmed to what Milestone 1 uses.

use std::path::PathBuf;
use std::time::SystemTime;

/// Host-side name of a session: `RecordId::host_name()`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub id: HostId,
    pub cwd: PathBuf,
    /// `None` runs the user's login shell; `Some` runs this argv.
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    /// Raw output stream is appended here (`pipe-pane`).
    pub scrollback: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Liveness {
    Running { pid: u32, command: String },
    Exited { code: Option<i32> },
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostStatus {
    pub id: HostId,
    pub liveness: Liveness,
    pub cwd: Option<PathBuf>,
    pub last_activity: Option<SystemTime>,
    /// Pane title (OSC 0/2), which Claude Code and shell integration set.
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInfo {
    pub description: String,
    /// Sessions outlive the app.
    pub persistent: bool,
}

pub trait ProcessHost: Send + Sync {
    /// Usable at all? The app shows the reason when not (tmux missing).
    fn probe(&self) -> Result<HostInfo, String>;
    /// Every session on the host, warm or dead-but-kept. One call, for
    /// the whole board.
    fn list(&self) -> std::io::Result<Vec<HostStatus>>;
    /// Fails if a session with this id already exists.
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<()>;
    fn status(&self, id: &HostId) -> std::io::Result<HostStatus>;
    /// Rendered screen and history, most recent `lines` (None = all).
    fn snapshot(&self, id: &HostId, lines: Option<usize>) -> std::io::Result<String>;
    /// Raw bytes to the process (keystrokes).
    fn write(&self, id: &HostId, bytes: &[u8]) -> std::io::Result<()>;
    fn kill(&self, id: &HostId) -> std::io::Result<()>;
    /// The argv an external terminal runs to attach to this session.
    fn attach_command(&self, id: &HostId) -> Vec<String>;
}
