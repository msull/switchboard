//! The app's own updates: what a newer release is, and how one is
//! fetched, checked and put in place of the running bundle. The source
//! is GitHub Releases outside the tests; the core only compares
//! versions and moves the state.

use std::path::{Path, PathBuf};

/// One published release newer than the running app, as the source
/// describes it: the version (no `v`), its notes, and where its bundle
/// and the bundle's checksum are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub asset_url: String,
    pub asset_name: String,
    pub digest_url: Option<String>,
}

/// Where releases come from and how one is installed. Every method
/// may block; the shell runs `latest` and `download` on a thread.
pub trait UpdateSource: Send + Sync {
    /// The newest published release, or `None` when nothing is
    /// published; a release that is not newer is still returned, and
    /// the core decides.
    fn latest(&self) -> Result<Option<Release>, String>;
    /// Fetch the release's bundle, check it against its checksum and
    /// its signature, and leave it staged; the path to the staged
    /// bundle comes back.
    fn download(&self, release: &Release) -> Result<PathBuf, String>;
    /// Arrange for the staged bundle to replace the running one once
    /// the process exits, and relaunch it. Returns once that is
    /// arranged; the caller then quits.
    fn install(&self, staged: &Path) -> Result<(), String>;
}
