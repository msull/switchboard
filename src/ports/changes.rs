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
    /// The name before a rename.
    pub old_path: Option<String>,
    /// numstat's `-\t-`: no line counts.
    pub binary: bool,
}

/// A branch's commits over its base, newest first, and the files
/// changed between the base and the branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub commits: Vec<Commit>,
    pub files: Vec<FileStat>,
}

/// What a file's diff did to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FileStatus {
    #[default]
    Modified,
    Added,
    Deleted,
    Renamed,
}

/// Which side of a diff a line is on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineKind {
    #[default]
    Context,
    Added,
    Removed,
}

/// One line of a hunk, numbered on the sides it is on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
    /// Byte ranges of `text` that differ from the line it replaced.
    pub marks: Vec<std::ops::Range<usize>>,
    /// Followed by `\ No newline at end of file`.
    pub no_newline: bool,
}

/// A stretch of a diff: where it starts on each side, and its lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

/// What a file's diff holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DiffBody {
    Hunks(Vec<Hunk>),
    Binary,
    /// Over the line cap; `lines` is how many the diff had.
    TooLarge {
        lines: usize,
    },
    /// A pure rename or a mode change: no line changed.
    #[default]
    Empty,
}

/// One file's diff over a branch's base.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub status: FileStatus,
    pub body: DiffBody,
}

/// Reads a branch's changes from git. `Send + Sync` because the ticket
/// page shares one reader with the threads it starts.
pub trait BranchChanges: Send + Sync {
    /// `head`'s commits after `base`, and `base...head`'s files, read in
    /// the repository at `dir`. An error says why, for the page.
    fn read(&self, dir: &Path, base: &str, head: &str) -> Result<Changes, String>;
    /// One file's `base...head` diff in `dir`; `old_path` for a rename.
    fn diff(
        &self,
        dir: &Path,
        base: &str,
        head: &str,
        path: &str,
        old_path: Option<&str>,
    ) -> Result<FileDiff, String>;
}
