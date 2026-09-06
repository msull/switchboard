//! Hand-offs to other apps: default app, Finder, and an external
//! terminal (Ghostty) attached to a host session.

use std::path::Path;

pub trait Opener: Send + Sync {
    fn open_default(&self, path: &Path) -> Result<(), String>;
    fn reveal(&self, path: &Path) -> Result<(), String>;
    /// Open `path` with `editor` (a command name or path; blank means
    /// the system text editor).
    fn open_editor(&self, editor: &str, path: &Path) -> Result<(), String>;
    /// Open an external terminal window titled `title` running `argv` in
    /// `cwd`. Ghostty: `open -na Ghostty --args --title=... -e ...`.
    fn open_terminal(&self, title: &str, argv: &[String], cwd: &Path) -> Result<(), String>;
    /// Bring an existing terminal window with this title to the front.
    /// `Ok(false)` when no such window exists.
    fn raise_terminal(&self, title: &str) -> Result<bool, String>;
}
