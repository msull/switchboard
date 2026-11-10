//! A standing agent's directory under `<data dir>/standing/`: the
//! instructions file Switchboard writes (a base text, a marker line, then
//! the owner's text) and the `handoff.md` it stats. The notes, ledger and
//! hand-off are the agent's own; Switchboard never reads them.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::AgentKind;

/// A standing agent's directory, made or found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedStanding {
    /// `<data dir>/standing/<name>/`, the agent's working directory.
    pub dir: PathBuf,
    /// `<data dir>/standing/`, the root of the standing agents' project.
    pub root: PathBuf,
    /// Hash of the instructions file as written or found.
    pub hash: String,
}

/// What a poll of the directory saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StandingState {
    /// SHA-256, in hex, of the instructions file; `None` when it is
    /// missing or unreadable.
    pub hash: Option<String>,
    /// Modified time of `handoff.md`; `None` when there is none.
    pub handoff_at: Option<SystemTime>,
}

pub trait StandingFiles: Send + Sync {
    /// Create `<data dir>/standing/<name>/` if needed and its
    /// instructions file. With `Some(text)` the file is the base text,
    /// the marker and `text`; with `None` an existing file is kept, or
    /// the base text and marker alone are written.
    ///
    /// # Errors
    /// A name that is not one path component, a symlinked directory or
    /// file, text over the size limit, or a failed write.
    fn prepare(
        &self,
        name: &str,
        agent: AgentKind,
        instructions: Option<&str>,
    ) -> Result<PreparedStanding, String>;
    /// The instructions file's hash and `handoff.md`'s modified time.
    fn inspect(&self, dir: &Path, agent: AgentKind) -> StandingState;
    /// The owner's text, after the marker, or the whole file when the
    /// marker is missing; `Ok(None)` when there is no file.
    ///
    /// # Errors
    /// The file exists but cannot be read, or is a symlink.
    fn read_instructions(&self, dir: &Path, agent: AgentKind) -> Result<Option<String>, String>;
    /// Replace the instructions file with the base text, the marker and
    /// `text`, keeping the old one as `.bak`. Returns the new hash.
    ///
    /// # Errors
    /// `dir` is not a direct child of the standing root, a symlink is in
    /// the way, the text is too large, or the write failed.
    fn write_instructions(
        &self,
        dir: &Path,
        agent: AgentKind,
        text: &str,
    ) -> Result<String, String>;
}
