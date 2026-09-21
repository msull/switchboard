//! Finding the files a command run produced: the record's declared
//! patterns, matched under the run's directory, kept when the file was
//! modified during the run.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub trait ArtifactFinder: Send + Sync {
    /// Files under `cwd` matching any of `patterns` (globs relative to
    /// `cwd`) whose modification time is at or after `since`, absolute,
    /// sorted, each once.
    fn find(&self, cwd: &Path, patterns: &[String], since: SystemTime) -> Vec<PathBuf>;
}
