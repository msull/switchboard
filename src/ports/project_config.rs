//! Commands and services a project declares in
//! `<root>/.switchboard/project.json`. The file is untrusted input: what
//! it declares is listed, never run, until the user approves each entry
//! (see "Trust boundary" in the design). The adapter parses; the core
//! decides what becomes a record and what may run.

use std::path::Path;
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
}

/// The parsed file: the entries it declares plus a warning per entry
/// that was skipped, naming the entry and the field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectConfig {
    pub entries: Vec<DefinedEntry>,
    pub warnings: Vec<String>,
    /// The shell that will run the commands (the user's login shell).
    pub shell: String,
}

pub trait ProjectConfigReader: Send + Sync {
    /// Read and parse the project's definition file. `Ok(None)` when
    /// there is none; `Err` for a file that exists but cannot be used
    /// (unreadable, too large, a symlink, wrong version).
    ///
    /// # Errors
    /// The message is shown to the user as the file's status.
    fn read(&self, root: &Path) -> Result<Option<ProjectConfig>, String>;
    /// The file's modification time, so a poll can skip an unchanged
    /// file. `None` when there is no file.
    fn modified(&self, root: &Path) -> Option<SystemTime>;
}
