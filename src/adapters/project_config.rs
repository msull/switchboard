//! Reads `<root>/.switchboard/project.json`. Parsing is strict per entry
//! (an unknown or missing field skips that entry with a warning naming
//! it) and tolerant per file (one bad entry does not hide the rest), so
//! an agent that wrote the file gets a precise message. The file itself
//! is capped and must not be a symlink: it is untrusted input from the
//! project directory.

use std::io;
use std::path::Path;
use std::time::SystemTime;

use serde::Deserialize;

use crate::core::SessionKind;
use crate::ports::project_config::{DefinedEntry, ProjectConfig, ProjectConfigReader};

/// Relative path of the definition file under a project root.
pub const CONFIG_PATH: &str = ".switchboard/project.json";

/// Larger files are refused outright: a definitions file is a few
/// hundred bytes, and the read happens on the poll thread.
pub const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// The only file version this build understands.
const VERSION: u64 = 1;

/// Reads the file with the shell the entries will run under.
pub struct FileConfigReader {
    shell: String,
}

impl FileConfigReader {
    /// Commands run under the user's login shell, like user-created ones.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
        }
    }
}

impl Default for FileConfigReader {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectConfigReader for FileConfigReader {
    fn read(&self, root: &Path) -> Result<Option<ProjectConfig>, String> {
        let path = root.join(CONFIG_PATH);
        // `symlink_metadata` does not follow links: a link pointing outside
        // the project would let it read (and present) any file.
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{CONFIG_PATH}: {e}")),
        };
        if meta.file_type().is_symlink() {
            return Err(format!("{CONFIG_PATH} is a symlink; refusing to read it"));
        }
        if meta.len() > MAX_CONFIG_BYTES {
            return Err(format!(
                "{CONFIG_PATH} is {} bytes; the limit is {MAX_CONFIG_BYTES}",
                meta.len()
            ));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{CONFIG_PATH}: {e}"))?;
        parse(&text, self.shell.clone()).map(Some)
    }

    fn modified(&self, root: &Path) -> Option<SystemTime> {
        std::fs::symlink_metadata(root.join(CONFIG_PATH))
            .ok()
            .and_then(|m| m.modified().ok())
    }
}

/// The file's shape. Entries stay as raw JSON so each can be parsed and
/// reported on its own.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u64,
    #[serde(default)]
    commands: Vec<serde_json::Value>,
    #[serde(default)]
    services: Vec<serde_json::Value>,
}

/// `deny_unknown_fields` turns a typo like `comand` into an error for
/// that entry instead of silently running nothing.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandEntry {
    name: String,
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceEntry {
    name: String,
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    autostart: bool,
}

/// Parse the file text. Pure, so the rules are unit-tested without a
/// file system.
///
/// # Errors
/// The whole file is unusable: not JSON, not an object, or a version
/// this build does not understand.
pub fn parse(text: &str, shell: String) -> Result<ProjectConfig, String> {
    let file: File = serde_json::from_str(text).map_err(|e| format!("{CONFIG_PATH}: {e}"))?;
    if file.version != VERSION {
        return Err(format!(
            "{CONFIG_PATH}: version {} is not supported (this build reads version {VERSION})",
            file.version
        ));
    }
    let mut config = ProjectConfig {
        shell,
        ..ProjectConfig::default()
    };
    for (i, raw) in file.commands.iter().enumerate() {
        match serde_json::from_value::<CommandEntry>(raw.clone()) {
            Ok(e) => config.add(
                DefinedEntry {
                    name: e.name,
                    kind: SessionKind::Command,
                    command: e.command,
                    cwd: e.cwd,
                    env: e.env,
                    autostart: false,
                },
                "commands",
                i,
            ),
            Err(e) => config.warnings.push(format!("commands[{i}] skipped: {e}")),
        }
    }
    for (i, raw) in file.services.iter().enumerate() {
        match serde_json::from_value::<ServiceEntry>(raw.clone()) {
            Ok(e) => config.add(
                DefinedEntry {
                    name: e.name,
                    kind: SessionKind::Service,
                    command: e.command,
                    cwd: e.cwd,
                    env: e.env,
                    autostart: e.autostart,
                },
                "services",
                i,
            ),
            Err(e) => config.warnings.push(format!("services[{i}] skipped: {e}")),
        }
    }
    Ok(config)
}

impl ProjectConfig {
    /// Keep an entry that passes the value rules, or record why not.
    fn add(&mut self, entry: DefinedEntry, list: &str, i: usize) {
        match validate(&entry, &self.entries) {
            Ok(()) => self.entries.push(entry),
            Err(why) => self
                .warnings
                .push(format!("{list}[{i}] ({:?}) skipped: {why}", entry.name)),
        }
    }
}

fn validate(entry: &DefinedEntry, seen: &[DefinedEntry]) -> Result<(), String> {
    let name = entry.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("name must be 1 to 64 characters".into());
    }
    if seen.iter().any(|e| e.name == entry.name) {
        return Err("name is used twice".into());
    }
    if entry.command.trim().is_empty() {
        return Err("command is empty".into());
    }
    if let Some(cwd) = &entry.cwd {
        let p = Path::new(cwd);
        if p.is_absolute() || p.components().any(|c| c == std::path::Component::ParentDir) {
            return Err("cwd must be relative to the project and not use `..`".into());
        }
    }
    for var in &entry.env {
        let mut chars = var.chars();
        let ok = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            return Err(format!("env name {var:?} is not a variable name"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> ProjectConfig {
        parse(text, "/bin/zsh".into()).unwrap()
    }

    #[test]
    fn minimal_file_yields_commands_and_services() {
        let c = parsed(
            r#"{"version":1,
                "commands":[{"name":"rebundle","command":"./scripts/bundle.sh","cwd":"sub","env":["FOO"]}],
                "services":[{"name":"web","command":"npm run dev","autostart":true}]}"#,
        );
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        assert_eq!(c.shell, "/bin/zsh");
        assert_eq!(c.entries.len(), 2);
        assert_eq!(c.entries[0].kind, SessionKind::Command);
        assert_eq!(c.entries[0].cwd.as_deref(), Some("sub"));
        assert_eq!(c.entries[0].env, vec!["FOO".to_string()]);
        assert!(!c.entries[0].autostart);
        assert_eq!(c.entries[1].kind, SessionKind::Service);
        assert!(c.entries[1].autostart);
    }

    #[test]
    fn a_bad_entry_is_skipped_with_a_warning_naming_it() {
        let c = parsed(
            r#"{"version":1,"commands":[
                {"name":"ok","command":"true"},
                {"name":"typo","comand":"true"},
                {"name":"","command":"true"},
                {"name":"ok","command":"dup"},
                {"name":"up","command":"x","cwd":"/etc"},
                {"name":"dots","command":"x","cwd":"../x"},
                {"name":"env","command":"x","env":["1BAD"]},
                {"name":"blank","command":"  "}
            ]}"#,
        );
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.warnings.len(), 7, "{:?}", c.warnings);
        assert!(c.warnings[0].contains("commands[1]"));
        assert!(c.warnings[0].contains("comand"));
        assert!(c.warnings[2].contains("\"ok\""));
        assert!(c.warnings[2].contains("twice"));
        assert!(c.warnings[3].contains("relative"));
        assert!(c.warnings[5].contains("1BAD"));
    }

    #[test]
    fn unsupported_version_or_shape_is_a_file_error() {
        assert!(
            parse(r#"{"version":2}"#, String::new())
                .unwrap_err()
                .contains("version 2")
        );
        assert!(parse("[]", String::new()).is_err());
        assert!(parse("{\"version\":1,\"extra\":1}", String::new()).is_err());
        assert!(parse("not json", String::new()).is_err());
    }

    #[test]
    fn reader_handles_missing_symlinked_and_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let reader = FileConfigReader {
            shell: "/bin/sh".into(),
        };
        assert_eq!(reader.read(dir.path()).unwrap(), None);
        assert_eq!(reader.modified(dir.path()), None);

        let sub = dir.path().join(".switchboard");
        std::fs::create_dir_all(&sub).unwrap();
        let real = dir.path().join("elsewhere.json");
        std::fs::write(&real, r#"{"version":1}"#).unwrap();
        std::os::unix::fs::symlink(&real, sub.join("project.json")).unwrap();
        assert!(reader.read(dir.path()).unwrap_err().contains("symlink"));
        std::fs::remove_file(sub.join("project.json")).unwrap();

        std::fs::write(
            sub.join("project.json"),
            format!(
                r#"{{"version":1,"commands":[{{"name":"x","command":"{}"}}]}}"#,
                "y".repeat(usize::try_from(MAX_CONFIG_BYTES).unwrap())
            ),
        )
        .unwrap();
        assert!(reader.read(dir.path()).unwrap_err().contains("bytes"));

        std::fs::write(sub.join("project.json"), r#"{"version":1}"#).unwrap();
        let c = reader.read(dir.path()).unwrap().unwrap();
        assert!(c.entries.is_empty());
        assert_eq!(c.shell, "/bin/sh");
        assert!(reader.modified(dir.path()).is_some());
    }
}
