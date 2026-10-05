//! What a ticket's branch changed over its base: its commits and the
//! files they touch, for the ticket page's Changes tab. Read from a
//! tree on this machine, off the UI thread.

use std::path::Path;

/// One commit of the branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    /// The committer's time.
    pub at_ms: u64,
}

/// One file the branch changed, with lines added and removed; both 0
/// for a binary file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileStat {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// A branch's commits over its base, newest first, and the files
/// changed between the base and the branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub commits: Vec<Commit>,
    pub files: Vec<FileStat>,
}

/// Reads a branch's changes from git. `Send + Sync` because the ticket
/// page shares one reader with the threads it starts.
pub trait BranchChanges: Send + Sync {
    /// `head`'s commits after `base`, and `base...head`'s files, read in
    /// the repository at `dir`. An error says why, for the page.
    fn read(&self, dir: &Path, base: &str, head: &str) -> Result<Changes, String>;
}
