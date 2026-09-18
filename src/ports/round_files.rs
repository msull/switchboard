//! The files a workflow run exchanges with its agents: probing for the
//! one it waits on, copying a finished round into the data directory,
//! and deleting the round files the run named once the user is done.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Enough of a file's metadata to tell that it stopped changing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub modified: SystemTime,
    pub len: u64,
}

/// What a probe found: the stamp and the file's first line, trimmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probed {
    pub stamp: FileStamp,
    pub first_line: String,
}

pub trait RoundFiles: Send + Sync {
    /// `None` while the file does not exist.
    fn probe(&self, path: &Path) -> Option<Probed>;
    /// Copy each file that exists into `dir` (created as needed) under
    /// its own name; a missing file is skipped, not an error. `note`,
    /// when given, is written beside them as `user-feedback.md`.
    fn snapshot(&self, files: &[PathBuf], dir: &Path, note: Option<&str>) -> Result<(), String>;
    /// Delete the files; a file already gone is fine.
    fn remove(&self, files: &[PathBuf]) -> Result<(), String>;
}
