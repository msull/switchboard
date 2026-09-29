//! Git, for the lanes: a worktree per ticket, the head of a tree, whether
//! it is clean. Fixed argv only; nothing from a ticket is spliced into a
//! command line.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

pub trait Repo: Send {
    /// `git worktree add <dir> -b <branch> <base>` in `repo`, then the
    /// lane's `setup` argv inside the new worktree.
    fn worktree_add(&mut self, repo: &Path, dir: &Path, branch: &str, base: &str) -> Result<()>;
    fn head(&self, dir: &Path) -> Result<String>;
    fn is_clean(&self, dir: &Path) -> Result<bool>;
    /// Run `argv` in `dir` with `env` set; nonzero exit is an error.
    fn run(&mut self, dir: &Path, argv: &[String], env: &[(String, String)]) -> Result<()>;
}

/// The `git` on the PATH.
#[derive(Debug, Default)]
pub struct GitCli;

/// A `git` command with the caller's own `GIT_*` variables removed: run
/// from a hook, those point at the hook's repository, not ours.
fn git() -> Command {
    let mut cmd = Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    cmd
}

fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().with_context(|| format!("run {cmd:?}"))?;
    if !out.status.success() {
        bail!(
            "{cmd:?} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

impl Repo for GitCli {
    fn worktree_add(&mut self, repo: &Path, dir: &Path, branch: &str, base: &str) -> Result<()> {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        output(
            git()
                .arg("-C")
                .arg(repo)
                .args(["worktree", "add"])
                .arg(dir)
                .args(["-b", branch, base]),
        )?;
        Ok(())
    }

    fn head(&self, dir: &Path) -> Result<String> {
        output(git().arg("-C").arg(dir).args(["rev-parse", "HEAD"]))
    }

    fn is_clean(&self, dir: &Path) -> Result<bool> {
        let status = output(git().arg("-C").arg(dir).args(["status", "--porcelain"]))?;
        Ok(status.is_empty())
    }

    fn run(&mut self, dir: &Path, argv: &[String], env: &[(String, String)]) -> Result<()> {
        let Some((program, rest)) = argv.split_first() else {
            return Ok(());
        };
        let mut cmd = Command::new(program);
        cmd.args(rest).current_dir(dir);
        for (k, v) in env {
            cmd.env(k, v);
        }
        output(&mut cmd)?;
        Ok(())
    }
}

/// What tests use: worktrees are directories made on the spot, heads are
/// set by the test, commands are recorded.
#[derive(Debug, Default)]
pub struct FakeRepo {
    pub worktrees: Vec<(std::path::PathBuf, std::path::PathBuf, String, String)>,
    pub heads: std::collections::BTreeMap<std::path::PathBuf, String>,
    pub dirty: Vec<std::path::PathBuf>,
    pub ran: Vec<(std::path::PathBuf, Vec<String>)>,
    pub fail_worktree: Option<String>,
}

impl Repo for FakeRepo {
    fn worktree_add(&mut self, repo: &Path, dir: &Path, branch: &str, base: &str) -> Result<()> {
        if let Some(e) = &self.fail_worktree {
            bail!("{e}");
        }
        std::fs::create_dir_all(dir)?;
        self.worktrees.push((
            repo.to_path_buf(),
            dir.to_path_buf(),
            branch.into(),
            base.into(),
        ));
        self.heads.insert(dir.to_path_buf(), "base0000".into());
        Ok(())
    }
    fn head(&self, dir: &Path) -> Result<String> {
        self.heads
            .get(dir)
            .cloned()
            .with_context(|| format!("no fake head for {}", dir.display()))
    }
    fn is_clean(&self, dir: &Path) -> Result<bool> {
        Ok(!self.dirty.iter().any(|d| d == dir))
    }
    fn run(&mut self, dir: &Path, argv: &[String], _env: &[(String, String)]) -> Result<()> {
        self.ran.push((dir.to_path_buf(), argv.to_vec()));
        Ok(())
    }
}

/// `dispatch/<number>-<slug>`: the issue title, lowercased, non-word runs
/// as one hyphen, at most 40 characters.
#[must_use]
pub fn branch_name(number: u64, title: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        format!("dispatch/{number}")
    } else {
        format!("dispatch/{number}-{slug}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_are_short_and_safe() {
        assert_eq!(
            branch_name(12, "Escape leaves the field, not the set!"),
            "dispatch/12-escape-leaves-the-field-not-the-set"
        );
        assert_eq!(branch_name(3, "???"), "dispatch/3");
        assert!(branch_name(4, &"word ".repeat(30)).len() <= "dispatch/4-".len() + 40);
    }

    #[test]
    fn the_real_git_cuts_a_worktree_and_reads_its_head() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = git().arg("-C").arg(&repo).args(args).output().unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "-b", "main"]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "root",
        ]);
        let mut cli = GitCli;
        let wt = dir.path().join("wt").join("t1");
        cli.worktree_add(&repo, &wt, "dispatch/1-x", "main")
            .unwrap();
        assert!(cli.is_clean(&wt).unwrap());
        assert_eq!(cli.head(&wt).unwrap(), cli.head(&repo).unwrap());
        std::fs::write(wt.join("f"), "x").unwrap();
        assert!(!cli.is_clean(&wt).unwrap());
        cli.run(&wt, &["true".to_owned()], &[]).unwrap();
        assert!(cli.run(&wt, &["false".to_owned()], &[]).is_err());
    }
}
