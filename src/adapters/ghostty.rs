//! macOS opener: `open`, Finder reveal, Ghostty window launch and raise.
//! See `ports::opener` and spike 04. Everything goes through
//! `std::process::Command`; nothing here ever sends input to a window.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ports::opener::Opener;

const GHOSTTY_BUNDLE_ID: &str = "com.mitchellh.ghostty";

/// Real opener for macOS.
#[derive(Debug, Clone)]
pub struct MacOpener {
    /// The Ghostty app bundle, when one was found.
    pub ghostty: Option<PathBuf>,
}

impl MacOpener {
    /// Looks for Ghostty in the usual places, then via Spotlight.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            ghostty: find_ghostty(),
        }
    }

    /// The `open` command line for a new Ghostty window, without running
    /// it. Flags per spike 04: `-n` for a fresh instance (an existing one
    /// ignores `--args`), `--window-save-state=never` so that instance
    /// does not restore old windows, `--quit-after-last-window-closed` so
    /// instances do not pile up, and `-e` last.
    #[must_use]
    pub fn terminal_command(title: &str, argv: &[String], cwd: &Path) -> Vec<String> {
        let mut cmd = vec![
            "open".to_string(),
            "-na".into(),
            "Ghostty".into(),
            "--args".into(),
            "--window-save-state=never".into(),
            "--quit-after-last-window-closed=true".into(),
            format!("--title={title}"),
            format!("--working-directory={}", cwd.display()),
            "-e".into(),
        ];
        cmd.extend(argv.iter().cloned());
        cmd
    }

    /// The System Events script from spike 04: raise the Ghostty window
    /// whose title is exactly `title`. Window activation only.
    #[must_use]
    pub fn raise_script(title: &str) -> String {
        let quoted = title.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            r#"tell application "System Events"
  repeat with p in (every process whose name is "Ghostty")
    repeat with w in windows of p
      if title of w is "{quoted}" then
        set frontmost of p to true
        perform action "AXRaise" of w
        return "raised in pid " & (unix id of p)
      end if
    end repeat
  end repeat
  return "not found"
end tell"#
        )
    }
}

fn find_ghostty() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    let candidates = [
        PathBuf::from("/Applications/Ghostty.app"),
        home.join("Applications/Ghostty.app"),
    ];
    if let Some(found) = candidates.into_iter().find(|p| p.is_dir()) {
        return Some(found);
    }
    let out = Command::new("mdfind")
        .arg(format!(
            "kMDItemCFBundleIdentifier == '{GHOSTTY_BUNDLE_ID}'"
        ))
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .find(|p| p.is_dir())
}

fn run(cmd: &[String]) -> Result<(), String> {
    let out = Command::new(&cmd[0])
        .args(&cmd[1..])
        .output()
        .map_err(|e| format!("{}: {e}", cmd[0]))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{} failed: {}", cmd[0], err.trim()))
    }
}

impl Opener for MacOpener {
    fn open_default(&self, path: &Path) -> Result<(), String> {
        run(&["open".to_string(), path.to_string_lossy().into_owned()])
    }

    fn reveal(&self, path: &Path) -> Result<(), String> {
        run(&[
            "open".to_string(),
            "-R".into(),
            path.to_string_lossy().into_owned(),
        ])
    }

    fn open_terminal(&self, title: &str, argv: &[String], cwd: &Path) -> Result<(), String> {
        if self.ghostty.is_none() {
            return Err("Ghostty is not installed (looked in /Applications and Spotlight)".into());
        }
        if argv.is_empty() {
            return Err("open_terminal needs a command to run".into());
        }
        run(&Self::terminal_command(title, argv, cwd))
    }

    fn raise_terminal(&self, title: &str) -> Result<bool, String> {
        let out = Command::new("osascript")
            .arg("-e")
            .arg(Self::raise_script(title))
            .output()
            .map_err(|e| format!("osascript: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("osascript failed: {}", err.trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .trim()
            .starts_with("raised"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_command_matches_the_spike() {
        let argv = vec!["tmux".to_string(), "attach".into()];
        let cmd = MacOpener::terminal_command("sb-1", &argv, Path::new("/w d"));
        assert_eq!(
            cmd,
            vec![
                "open",
                "-na",
                "Ghostty",
                "--args",
                "--window-save-state=never",
                "--quit-after-last-window-closed=true",
                "--title=sb-1",
                "--working-directory=/w d",
                "-e",
                "tmux",
                "attach",
            ]
        );
    }

    #[test]
    fn raise_script_escapes_the_title() {
        let script = MacOpener::raise_script(r#"a"b\c"#);
        assert!(script.contains(r#"if title of w is "a\"b\\c" then"#));
        assert!(script.contains("AXRaise"));
        assert!(!script.contains("keystroke"));
    }

    #[test]
    fn missing_ghostty_is_an_error() {
        let o = MacOpener { ghostty: None };
        assert!(
            o.open_terminal("t", &["sleep".into()], Path::new("/"))
                .is_err()
        );
    }
}
