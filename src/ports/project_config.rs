//! Commands and services a project declares in
//! `<root>/.switchboard/project.json`. The file is untrusted input: what
//! it declares is listed, never run, until the user approves each entry
//! (see "Trust boundary" in the design). The adapter parses; the core
//! decides what becomes a record and what may run.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::SessionKind;

/// One entry of the file, validated but not yet trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedEntry {
    pub name: String,
    /// `Command` or `Service`; the parser admits nothing else.
    pub kind: SessionKind,
    pub command: String,
    /// Relative to the project root; `None` means the root itself.
    pub cwd: Option<String>,
    /// Names of variables the command expects (never values).
    pub env: Vec<String>,
    /// Requested for services; honored only once approved.
    pub autostart: bool,
    /// Glob patterns, relative to the entry's directory, of the files a
    /// command produces (`output`: a string or a list).
    pub outputs: Vec<String>,
}

/// The parsed file: the entries it declares plus a warning per entry
/// that was skipped, naming the entry and the field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectConfig {
    pub entries: Vec<DefinedEntry>,
    pub warnings: Vec<String>,
    /// The shell that will run the commands (the user's login shell).
    pub shell: String,
    /// Folders under the root the file side shows even when the root's
    /// `.gitignore` hides them (sub-repositories, generated trees).
    /// Relative, validated: no absolute paths, no `..`.
    pub show: Vec<PathBuf>,
}

pub trait ProjectConfigReader: Send + Sync {
    /// Read and parse the project's definition file. `Ok(None)` when
    /// there is none; `Err` for a file that exists but cannot be used
    /// (unreadable, too large, a symlink, wrong version).
    ///
    /// # Errors
    /// The message is shown to the user as the file's status.
    fn read(&self, root: &Path) -> Result<Option<ProjectConfig>, String>;
    /// The file's text as it is, for editing. `Ok(None)` when there is
    /// none; the same refusals as `read`.
    ///
    /// # Errors
    /// The file exists but cannot be used.
    fn read_text(&self, root: &Path) -> Result<Option<String>, String>;
    /// Replace the file with `text` (creating `.switchboard/` if needed),
    /// on the user's explicit Save in the config editor: the one write
    /// Switchboard makes into a project directory.
    ///
    /// # Errors
    /// The directory or file cannot be written, or the path is a symlink.
    fn write_text(&self, root: &Path, text: &str) -> Result<(), String>;
    /// The file's modification time, so a poll can skip an unchanged
    /// file. `None` when there is no file.
    fn modified(&self, root: &Path) -> Option<SystemTime>;
}
