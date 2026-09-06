//! Durable store for workspace records. See "Durable store" in the design:
//! one JSON file per project under the private data directory, atomic
//! writes with `.bak`, flock single-writer, validation on load.

use std::path::PathBuf;

use crate::core::{ProjectId, Workspace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Another instance holds the lock; this one is read-only.
    Locked,
    /// File was unreadable; `recovered` is true when `.bak` was used.
    Corrupt {
        path: PathBuf,
        recovered: bool,
        detail: String,
    },
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Locked => write!(f, "another Switchboard instance holds the lock"),
            Self::Corrupt {
                path,
                recovered,
                detail,
            } => write!(
                f,
                "{} is corrupt ({detail}); {}",
                path.display(),
                if *recovered {
                    "recovered from backup"
                } else {
                    "no backup"
                }
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Result of loading everything: the workspaces plus any recovery notices
/// the UI must show (a corrupt file that fell back to `.bak`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub workspaces: Vec<Workspace>,
    pub notices: Vec<StoreError>,
}

pub trait Store: Send {
    /// Take the single-writer lock. `Ok(false)` means another instance
    /// holds it and this one must not write.
    fn lock(&mut self) -> Result<bool, StoreError>;
    fn load_all(&self) -> Result<Loaded, StoreError>;
    fn save(&self, workspace: &Workspace) -> Result<(), StoreError>;
    fn delete(&self, id: ProjectId) -> Result<(), StoreError>;
    /// Directory for per-session files (scrollback, event log).
    fn data_dir(&self) -> PathBuf;
}
