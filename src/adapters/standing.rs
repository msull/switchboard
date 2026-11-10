//! Standing agents' directories under `<data dir>/standing/`. Only the
//! instructions file is written, through the store's atomic path, and
//! only `handoff.md` is looked at (its modified time); the agent's
//! notes, ledger and hand-off are never read.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::adapters::store::JsonStore;
use crate::core::AgentKind;
use crate::core::standing::{
    INSTRUCTIONS_MARKER, STANDING_DIR, base_instructions, valid_standing_name,
};
use crate::ports::standing::{PreparedStanding, StandingFiles, StandingState};

/// The largest owner's text the instructions file takes.
const MAX_INSTRUCTIONS_BYTES: usize = 64 * 1024;

/// The file a standing agent rewrites when a piece of work ends.
const HANDOFF: &str = "handoff.md";

/// The standing agents under one data directory.
#[derive(Debug, Clone)]
pub struct StandingDirs {
    root: PathBuf,
}

impl StandingDirs {
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: data_dir.into().join(STANDING_DIR),
        }
    }

    /// Errs unless `dir` is a direct child of the root, named like a
    /// standing agent and not a symlink, so a record whose `cwd` was
    /// edited cannot aim a write elsewhere.
    fn checked(&self, dir: &Path) -> Result<(), String> {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| valid_standing_name(n))
            .ok_or_else(|| format!("{} is not a standing agent's directory", dir.display()))?;
        if dir != self.root.join(name) {
            return Err(format!(
                "{} is not under {}",
                dir.display(),
                self.root.display()
            ));
        }
        refuse_symlink(&self.root)?;
        refuse_symlink(dir)
    }
}

/// An error when `path` is a symlink; a missing path is fine.
fn refuse_symlink(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{} is a symlink; refusing to use it",
            path.display()
        )),
        _ => Ok(()),
    }
}

fn hash_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// The whole file: Switchboard's base, the marker, the owner's text.
fn compose(agent: AgentKind, text: &str) -> String {
    let mut file = base_instructions(agent);
    file.push('\n');
    file.push_str(INSTRUCTIONS_MARKER);
    file.push_str("\n\n");
    file.push_str(text.trim_end());
    file.push('\n');
    file
}

/// Write `text` as the owner's part of `dir`'s instructions file.
fn write_file(dir: &Path, agent: AgentKind, text: &str) -> Result<String, String> {
    if text.len() > MAX_INSTRUCTIONS_BYTES {
        return Err(format!(
            "the text is over the {MAX_INSTRUCTIONS_BYTES} byte limit"
        ));
    }
    let path = dir.join(agent.instructions_file());
    refuse_symlink(&path)?;
    let bytes = compose(agent, text).into_bytes();
    JsonStore::write_atomic(dir, &path, &bytes, true).map_err(|e| e.to_string())?;
    Ok(hash_hex(&bytes))
}

impl StandingFiles for StandingDirs {
    fn prepare(
        &self,
        name: &str,
        agent: AgentKind,
        instructions: Option<&str>,
    ) -> Result<PreparedStanding, String> {
        if !valid_standing_name(name) {
            return Err(format!("{name:?} is not a standing agent's name"));
        }
        let dir = self.root.join(name);
        self.checked(&dir)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(agent.instructions_file());
        refuse_symlink(&path)?;
        let hash = match (instructions, std::fs::read(&path)) {
            (Some(text), _) => write_file(&dir, agent, text)?,
            (None, Ok(bytes)) => hash_hex(&bytes),
            (None, Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                write_file(&dir, agent, "")?
            }
            (None, Err(e)) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(PreparedStanding {
            dir,
            root: self.root.clone(),
            hash,
        })
    }

    fn inspect(&self, dir: &Path, agent: AgentKind) -> StandingState {
        let path = dir.join(agent.instructions_file());
        let hash = std::fs::symlink_metadata(&path)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .and_then(|_| std::fs::read(&path).ok())
            .as_deref()
            .map(hash_hex);
        let handoff_at = std::fs::metadata(dir.join(HANDOFF))
            .ok()
            .and_then(|m| m.modified().ok());
        StandingState { hash, handoff_at }
    }

    fn read_instructions(&self, dir: &Path, agent: AgentKind) -> Result<Option<String>, String> {
        let path = dir.join(agent.instructions_file());
        refuse_symlink(&path)?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(Some(match text.split_once(INSTRUCTIONS_MARKER) {
            Some((_, owner)) => owner.trim().to_owned(),
            None => text,
        }))
    }

    fn write_instructions(
        &self,
        dir: &Path,
        agent: AgentKind,
        text: &str,
    ) -> Result<String, String> {
        self.checked(dir)?;
        if !dir.is_dir() {
            return Err(format!("{} does not exist", dir.display()));
        }
        write_file(dir, agent, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> (tempfile::TempDir, StandingDirs) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dirs = StandingDirs::new(tmp.path());
        (tmp, dirs)
    }

    #[test]
    fn prepare_makes_the_directory_and_the_kinds_file() {
        let (_tmp, dirs) = dirs();
        let p = dirs
            .prepare("ops", AgentKind::ClaudeCode, Some("Watch the builds."))
            .expect("prepared");
        assert_eq!(p.dir, dirs.root.join("ops"));
        assert_eq!(p.root, dirs.root);
        let text = std::fs::read_to_string(p.dir.join("CLAUDE.md")).expect("file");
        assert!(text.contains(INSTRUCTIONS_MARKER));
        assert!(text.ends_with("Watch the builds.\n"));
        assert_eq!(p.hash, hash_hex(text.as_bytes()));
        let codex = dirs
            .prepare("rev", AgentKind::Codex, None)
            .expect("prepared");
        assert!(codex.dir.join("AGENTS.md").is_file());
        assert!(!codex.dir.join("CLAUDE.md").exists());
    }

    #[test]
    fn prepare_keeps_an_existing_file_without_text() {
        let (_tmp, dirs) = dirs();
        let first = dirs
            .prepare("ops", AgentKind::ClaudeCode, Some("Mine."))
            .expect("first");
        let again = dirs
            .prepare("ops", AgentKind::ClaudeCode, None)
            .expect("again");
        assert_eq!(first.hash, again.hash);
        assert_eq!(
            dirs.read_instructions(&again.dir, AgentKind::ClaudeCode)
                .expect("read"),
            Some("Mine.".to_owned())
        );
    }

    #[test]
    fn prepare_refuses_a_name_that_escapes_and_a_symlink() {
        let (tmp, dirs) = dirs();
        for name in ["../out", "a/b", "", ".hidden", "/abs"] {
            assert!(
                dirs.prepare(name, AgentKind::ClaudeCode, None).is_err(),
                "{name:?}"
            );
        }
        std::fs::create_dir_all(&dirs.root).expect("root");
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        std::os::unix::fs::symlink(&elsewhere, dirs.root.join("linked")).expect("symlink");
        assert!(dirs.prepare("linked", AgentKind::ClaudeCode, None).is_err());
        assert!(!elsewhere.join("CLAUDE.md").exists());
        let real = dirs
            .prepare("real", AgentKind::ClaudeCode, None)
            .expect("real");
        std::fs::remove_file(real.dir.join("CLAUDE.md")).expect("rm");
        std::os::unix::fs::symlink(elsewhere.join("x"), real.dir.join("CLAUDE.md"))
            .expect("file symlink");
        assert!(
            dirs.prepare("real", AgentKind::ClaudeCode, Some("x"))
                .is_err()
        );
    }

    #[test]
    fn write_instructions_refuses_a_directory_outside_the_root() {
        let (tmp, dirs) = dirs();
        let outside = tmp.path().join("ops");
        std::fs::create_dir_all(&outside).expect("outside");
        assert!(
            dirs.write_instructions(&outside, AgentKind::ClaudeCode, "x")
                .is_err()
        );
        assert!(!outside.join("CLAUDE.md").exists());
        let nested = dirs.root.join("ops").join("deeper");
        assert!(
            dirs.write_instructions(&nested, AgentKind::ClaudeCode, "x")
                .is_err()
        );
    }

    #[test]
    fn write_instructions_replaces_atomically_and_keeps_a_backup() {
        let (_tmp, dirs) = dirs();
        let p = dirs
            .prepare("ops", AgentKind::ClaudeCode, Some("Old."))
            .expect("prepared");
        let hash = dirs
            .write_instructions(&p.dir, AgentKind::ClaudeCode, "New.")
            .expect("written");
        assert_ne!(hash, p.hash);
        let file = p.dir.join("CLAUDE.md");
        assert!(
            std::fs::read_to_string(&file)
                .expect("file")
                .ends_with("New.\n")
        );
        let backup = std::fs::read_to_string(p.dir.join("CLAUDE.md.bak")).expect("bak");
        assert!(backup.ends_with("Old.\n"));
        assert_eq!(dirs.inspect(&p.dir, AgentKind::ClaudeCode).hash, Some(hash));
        assert!(
            dirs.write_instructions(&p.dir, AgentKind::ClaudeCode, &"x".repeat(65 * 1024))
                .is_err()
        );
    }

    #[test]
    fn inspect_sees_the_handoff_time() {
        let (_tmp, dirs) = dirs();
        let p = dirs
            .prepare("ops", AgentKind::ClaudeCode, None)
            .expect("prepared");
        assert_eq!(dirs.inspect(&p.dir, AgentKind::ClaudeCode).handoff_at, None);
        std::fs::write(p.dir.join(HANDOFF), "done").expect("handoff");
        assert!(
            dirs.inspect(&p.dir, AgentKind::ClaudeCode)
                .handoff_at
                .is_some()
        );
    }
}
