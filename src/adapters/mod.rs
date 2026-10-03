//! Concrete implementations of the ports, plus `fakes`: one per port,
//! for tests, and the secret store off macOS.

pub mod agents;
pub mod artifacts;
pub mod control;
pub mod controller;
pub mod dispatch;
pub mod dock;
pub mod dotenv;
pub mod fakes;
pub mod files;
pub mod ghostty;
pub mod git;
pub mod hooks;
#[cfg(target_os = "macos")]
pub mod keychain;
pub mod project_config;
pub mod round_files;
pub mod scrollback;
pub mod store;
pub mod tmux;
pub mod transcript;

use std::path::{Path, PathBuf};

/// `$HOME`, or `/` when it is unset.
fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// The directories on `PATH`, in order.
fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// The first `<dir>/<name>` that is a file, trying `dirs` in order.
fn find_in(dirs: impl IntoIterator<Item = PathBuf>, name: impl AsRef<Path>) -> Option<PathBuf> {
    dirs.into_iter()
        .map(|d| d.join(name.as_ref()))
        .find(|p| p.is_file())
}
