//! The one place a runner child's `Command` is built. The runner's own
//! environment holds its launch token, and a child inherits that
//! environment, so every child starts here with the token removed.

use std::ffi::OsStr;
use std::process::Command;

use switchboard_control::RECORD_TOKEN_ENV;

/// A child's `Command` for `program` with `env` set, without the
/// runner's launch token unless `env` gives it back: the runner's own
/// environment is inherited, and the token in it would let any command
/// resolve the runner's environment sets.
pub(crate) fn child_command(program: impl AsRef<OsStr>, env: &[(String, String)]) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_remove(RECORD_TOKEN_ENV);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// A child's `Command` for `program`, without the launch token. This is
/// the default; reach for [`child_command`] only when the caller has an
/// env list to pass.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    child_command(program, &[])
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// What `cmd` does to the token: no entry when it leaves it alone,
    /// `[None]` when it removes it, `[Some(v)]` when it sets it.
    fn token(cmd: &Command) -> Vec<Option<&OsStr>> {
        cmd.get_envs()
            .filter(|(k, _)| *k == RECORD_TOKEN_ENV)
            .map(|(_, v)| v)
            .collect()
    }

    #[test]
    fn a_child_loses_the_token_unless_its_env_gives_it_back() {
        assert_eq!(token(&command("x")), vec![None]);
        assert_eq!(token(&child_command("x", &[])), vec![None]);
        let given = child_command("x", &[(RECORD_TOKEN_ENV.into(), "t0k".into())]);
        assert_eq!(token(&given), vec![Some(OsStr::new("t0k"))]);
    }

    /// Every `.rs` file under `dir`, recursively.
    fn sources(dir: &Path, into: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, into);
            } else if path.extension().is_some_and(|e| e == "rs") {
                into.push(path);
            }
        }
    }

    /// A `Command` built anywhere else would inherit the launch token. Only
    /// the code before a file's `#[cfg(test)] mod tests` is checked: tests
    /// start `kill`, `sleep` and `ps` of their own. That is enough because
    /// the crate keeps its test module last in each file.
    #[test]
    fn no_child_is_built_outside_this_module() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let mut files = Vec::new();
        sources(root, &mut files);
        assert!(files.len() > 1, "no sources found under {}", root.display());
        let needle = "Command::new(";
        for file in files {
            if file.file_name().is_some_and(|n| n == "child.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            let code = text
                .split("#[cfg(test)]\nmod tests")
                .next()
                .unwrap_or_default();
            assert!(
                !code.contains(needle),
                "{} builds a child with {needle}; use crate::child::command (or child_command with an env list)",
                file.display()
            );
        }
    }
}
