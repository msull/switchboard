//! Git, for the lanes: a worktree per ticket, the head of a tree, whether
//! it is clean. Fixed argv only; nothing from a ticket is spliced into a
//! command line.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub trait Repo: Send {
    /// A clone of `url` at `dir`, made if it is not there yet.
    fn ensure_clone(&mut self, url: &str, dir: &Path) -> Result<()>;
    /// `git fetch <remote> --prune` in `dir`, so a cut starts from what
    /// the remote has now.
    fn fetch(&mut self, dir: &Path, remote: &str) -> Result<()>;
    /// `remote` in `dir` points at `url`: added, or its URL set.
    fn ensure_remote(&mut self, dir: &Path, remote: &str, url: &str) -> Result<()>;
    /// A GitHub pull request's head fetched from `remote` in `dir` as
    /// `<remote>/pr/<number>`, whichever fork it comes from.
    fn fetch_pull(&mut self, dir: &Path, remote: &str, number: u64) -> Result<()>;
    /// `git worktree add <dir> -b <branch> <start>` in `repo`, where
    /// `start` is a ref like `origin/main`.
    fn worktree_add(&mut self, repo: &Path, dir: &Path, branch: &str, start: &str) -> Result<()>;
    /// The commit `rev` names in `dir`.
    fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String>;
    /// The merge base of `a` and `b` in `dir`.
    fn merge_base(&self, dir: &Path, a: &str, b: &str) -> Result<String>;
    /// A worktree of `repo` at `dir` on `branch` as `remote` has it,
    /// tracking it: someone else's branch, checked out to be looked at.
    /// A local branch of that name left by an earlier ticket is reset
    /// to the remote's.
    fn worktree_track(&mut self, repo: &Path, dir: &Path, branch: &str, remote: &str)
    -> Result<()>;
    /// Whether `dir` is a finished worktree of `repo` with `branch`
    /// checked out: the same common git directory, that branch, a tree.
    fn is_worktree_of(&self, repo: &Path, dir: &Path, branch: &str) -> Result<bool>;
    fn head(&self, dir: &Path) -> Result<String>;
    fn is_clean(&self, dir: &Path) -> Result<bool>;
    /// Move a worktree of `repo` from `from` to `to`, git's own records
    /// of it included.
    fn worktree_move(&mut self, repo: &Path, from: &Path, to: &Path) -> Result<()>;
    /// Re-point `repo` at its worktree now at `dir`, after the tree was
    /// moved by something other than git (a parent directory moved).
    fn worktree_repair(&mut self, repo: &Path, dir: &Path) -> Result<()>;
    /// The `origin` remote of the repository holding `dir`, if it has one.
    fn remote_url(&self, dir: &Path) -> Result<Option<String>>;
    /// What the branch at `dir` adds over `base`: the commits, one per
    /// line, then the files changed, for a person to read.
    fn summary(&self, dir: &Path, base: &str) -> Result<String>;
    /// Run `argv` in `dir` with `env` set; nonzero exit is an error.
    fn run(&mut self, dir: &Path, argv: &[String], env: &[(String, String)]) -> Result<()>;
    /// Start a check (a command gate) in `dir` as a child of the runner,
    /// its output appended to `log`, under `key` for polling. Nothing
    /// from a template reaches the command line; values go in `env`.
    fn start_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
    ) -> Result<()>;
    /// `None` while the check runs, `Some(Ok(code))` once it exited, and
    /// `Some(Err)` for a check this runner never started or lost: a
    /// restart means the process is gone with it.
    fn poll_check(&mut self, key: &str) -> Option<Result<i32>>;
    /// A command reviewer: like a check, with stdout and stderr kept
    /// apart, since the stdout is its findings.
    fn start_reviewer(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
    ) -> Result<()>;
    /// Kill a running check or reviewer and its descendants, if this
    /// runner started it.
    fn kill_check(&mut self, key: &str);
}

/// The `git` on the PATH, and the checks this runner has started.
#[derive(Debug, Default)]
pub struct GitCli {
    checks: std::collections::HashMap<String, std::process::Child>,
}

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
    fn ensure_clone(&mut self, url: &str, dir: &Path) -> Result<()> {
        if dir.join(".git").exists() {
            return Ok(());
        }
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        output(git().args(["clone", "--quiet", url]).arg(dir))?;
        Ok(())
    }

    fn fetch(&mut self, dir: &Path, remote: &str) -> Result<()> {
        output(
            git()
                .arg("-C")
                .arg(dir)
                .args(["fetch", "--quiet", "--prune", remote]),
        )?;
        Ok(())
    }

    fn ensure_remote(&mut self, dir: &Path, remote: &str, url: &str) -> Result<()> {
        let known = git()
            .arg("-C")
            .arg(dir)
            .args(["remote", "get-url", remote])
            .output()
            .with_context(|| format!("git in {}", dir.display()))?;
        let verb = if known.status.success() {
            "set-url"
        } else {
            "add"
        };
        output(git().arg("-C").arg(dir).args(["remote", verb, remote, url]))?;
        Ok(())
    }

    fn fetch_pull(&mut self, dir: &Path, remote: &str, number: u64) -> Result<()> {
        output(git().arg("-C").arg(dir).args([
            "fetch",
            "--quiet",
            remote,
            &format!("+refs/pull/{number}/head:refs/remotes/{remote}/pr/{number}"),
        ]))?;
        Ok(())
    }

    fn worktree_add(&mut self, repo: &Path, dir: &Path, branch: &str, start: &str) -> Result<()> {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        output(
            git()
                .arg("-C")
                .arg(repo)
                .args(["worktree", "add"])
                .arg(dir)
                .args(["-b", branch, start]),
        )?;
        Ok(())
    }

    fn worktree_track(
        &mut self,
        repo: &Path,
        dir: &Path,
        branch: &str,
        remote: &str,
    ) -> Result<()> {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        output(
            git()
                .arg("-C")
                .arg(repo)
                .args(["worktree", "add"])
                .arg(dir)
                .args(["--track", "-B", branch, &format!("{remote}/{branch}")]),
        )?;
        Ok(())
    }

    fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String> {
        Ok(output(
            git()
                .arg("-C")
                .arg(dir)
                .args(["rev-parse", "--verify", rev]),
        )?
        .trim()
        .to_owned())
    }

    fn merge_base(&self, dir: &Path, a: &str, b: &str) -> Result<String> {
        Ok(output(git().arg("-C").arg(dir).args(["merge-base", a, b]))?
            .trim()
            .to_owned())
    }

    fn is_worktree_of(&self, repo: &Path, dir: &Path, branch: &str) -> Result<bool> {
        // `rev-parse` answers relative to the directory git was given.
        let show = |where_: &Path, what: &str| -> Result<Option<PathBuf>> {
            let out = git()
                .arg("-C")
                .arg(where_)
                .args(["rev-parse", what])
                .output()
                .with_context(|| format!("git in {}", where_.display()))?;
            if !out.status.success() {
                return Ok(None);
            }
            let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            Ok(where_.join(text).canonicalize().ok())
        };
        let (Some(common), Some(repo_common), Some(top)) = (
            show(dir, "--git-common-dir")?,
            show(repo, "--git-common-dir")?,
            show(dir, "--show-toplevel")?,
        ) else {
            return Ok(false);
        };
        if common != repo_common || top != dir.canonicalize()? {
            return Ok(false);
        }
        let head = output(
            git()
                .arg("-C")
                .arg(dir)
                .args(["rev-parse", "--abbrev-ref", "HEAD"]),
        )?;
        Ok(head == branch)
    }

    fn head(&self, dir: &Path) -> Result<String> {
        output(git().arg("-C").arg(dir).args(["rev-parse", "HEAD"]))
    }

    fn is_clean(&self, dir: &Path) -> Result<bool> {
        let status = output(git().arg("-C").arg(dir).args(["status", "--porcelain"]))?;
        Ok(status.is_empty())
    }

    fn worktree_move(&mut self, repo: &Path, from: &Path, to: &Path) -> Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        output(
            git()
                .arg("-C")
                .arg(repo)
                .args(["worktree", "move"])
                .arg(from)
                .arg(to),
        )?;
        Ok(())
    }

    fn remote_url(&self, dir: &Path) -> Result<Option<String>> {
        let out = git()
            .arg("-C")
            .arg(dir)
            .args(["remote", "get-url", "origin"])
            .output()
            .context("run git")?;
        if !out.status.success() {
            return Ok(None);
        }
        let url = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok((!url.is_empty()).then_some(url))
    }

    fn summary(&self, dir: &Path, base: &str) -> Result<String> {
        let range = format!("{base}..HEAD");
        let log =
            output(
                git()
                    .arg("-C")
                    .arg(dir)
                    .args(["log", "--oneline", "--no-decorate", &range]),
            )?;
        let stat = output(git().arg("-C").arg(dir).args(["diff", "--stat", &range]))?;
        Ok(format!("{log}\n{stat}").trim().to_owned())
    }

    fn worktree_repair(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        output(
            git()
                .arg("-C")
                .arg(repo)
                .args(["worktree", "repair"])
                .arg(dir),
        )?;
        Ok(())
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

    fn start_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
    ) -> Result<()> {
        let Some((program, rest)) = argv.split_first() else {
            bail!("a check with no command");
        };
        let out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .with_context(|| format!("open {}", log.display()))?;
        let err = out.try_clone()?;
        let mut cmd = Command::new(program);
        cmd.args(rest)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .stdout(out)
            .stderr(err);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("start {program} in {}", dir.display()))?;
        self.checks.insert(key.to_owned(), child);
        Ok(())
    }

    fn start_reviewer(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
    ) -> Result<()> {
        let Some((program, rest)) = argv.split_first() else {
            bail!("a reviewer with no command");
        };
        let out = std::fs::File::create(stdout)
            .with_context(|| format!("create {}", stdout.display()))?;
        let err = std::fs::File::create(stderr)
            .with_context(|| format!("create {}", stderr.display()))?;
        let mut cmd = Command::new(program);
        cmd.args(rest)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .stdout(out)
            .stderr(err)
            .process_group(0);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd
            .spawn()
            .with_context(|| format!("start {program} in {}", dir.display()))?;
        self.checks.insert(key.to_owned(), child);
        Ok(())
    }

    fn kill_check(&mut self, key: &str) {
        if let Some(mut child) = self.checks.remove(key) {
            // The group first (a reviewer runs in its own), then the
            // child itself for one started without a group.
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{}", child.id())])
                .output();
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn poll_check(&mut self, key: &str) -> Option<Result<i32>> {
        let Some(child) = self.checks.get_mut(key) else {
            return Some(Err(anyhow::anyhow!(
                "no such check in this runner (it restarted)"
            )));
        };
        match child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) => {
                self.checks.remove(key);
                Some(Ok(status.code().unwrap_or(-1)))
            }
            Err(e) => {
                self.checks.remove(key);
                Some(Err(anyhow::anyhow!("waiting on the check: {e}")))
            }
        }
    }
}

/// A check the fake was asked to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedCheck {
    pub key: String,
    pub dir: PathBuf,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub log: PathBuf,
}

/// What tests use: worktrees are directories made on the spot, heads are
/// set by the test, commands are recorded.
#[derive(Debug, Default)]
pub struct FakeRepo {
    pub clones: Vec<(String, PathBuf)>,
    pub fetched: Vec<(PathBuf, String)>,
    pub worktrees: Vec<(PathBuf, PathBuf, String, String)>,
    /// Worktrees of someone else's branch: repo, dir, branch, remote.
    pub tracked: Vec<(PathBuf, PathBuf, String, String)>,
    /// Remotes added or re-pointed: dir, name, url.
    pub remotes_set: Vec<(PathBuf, String, String)>,
    /// Pull request heads fetched: dir, remote, number.
    pub fetched_pulls: Vec<(PathBuf, String, u64)>,
    pub heads: std::collections::BTreeMap<PathBuf, String>,
    pub dirty: Vec<PathBuf>,
    pub ran: Vec<(PathBuf, Vec<String>)>,
    pub fail_worktree: Option<String>,
    /// Worktrees moved: repo, from, to.
    pub moved: Vec<(PathBuf, PathBuf, PathBuf)>,
    /// The `origin` of a tree, for a project the pipeline names by `root`.
    pub remotes: std::collections::BTreeMap<PathBuf, String>,
    /// What a tree's branch adds over its base, as a test wrote it.
    pub summaries: std::collections::BTreeMap<PathBuf, String>,
    /// Worktrees repaired: repo, dir.
    pub repaired: Vec<(PathBuf, PathBuf)>,
    /// Checks started.
    pub checks: Vec<StartedCheck>,
    /// Exit codes a test sets for a check by key; unset means running.
    pub check_exits: std::collections::BTreeMap<String, i32>,
    /// Checks and reviewers killed by key.
    pub killed_checks: Vec<String>,
    /// Command reviewers started: the check record plus its stderr file.
    pub reviewers: Vec<(StartedCheck, PathBuf)>,
    /// What a tree's base resolves to; absent, `base0000`.
    pub bases: std::collections::BTreeMap<PathBuf, String>,
}

impl Repo for FakeRepo {
    fn ensure_clone(&mut self, url: &str, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        self.clones.push((url.to_owned(), dir.to_path_buf()));
        Ok(())
    }
    fn ensure_remote(&mut self, dir: &Path, remote: &str, url: &str) -> Result<()> {
        self.remotes_set
            .push((dir.to_path_buf(), remote.to_owned(), url.to_owned()));
        Ok(())
    }
    fn fetch_pull(&mut self, dir: &Path, remote: &str, number: u64) -> Result<()> {
        self.fetched_pulls
            .push((dir.to_path_buf(), remote.to_owned(), number));
        Ok(())
    }
    fn fetch(&mut self, dir: &Path, remote: &str) -> Result<()> {
        self.fetched.push((dir.to_path_buf(), remote.to_owned()));
        Ok(())
    }
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
    fn worktree_track(
        &mut self,
        repo: &Path,
        dir: &Path,
        branch: &str,
        remote: &str,
    ) -> Result<()> {
        if let Some(e) = &self.fail_worktree {
            bail!("{e}");
        }
        std::fs::create_dir_all(dir)?;
        // Someone else's branch is at whatever it is at; a test that
        // cares sets `heads` afterwards.
        self.heads.insert(dir.to_path_buf(), "theirs00".into());
        self.tracked.push((
            repo.to_path_buf(),
            dir.to_path_buf(),
            branch.to_owned(),
            remote.to_owned(),
        ));
        Ok(())
    }
    fn is_worktree_of(&self, repo: &Path, dir: &Path, branch: &str) -> Result<bool> {
        Ok(self
            .worktrees
            .iter()
            .any(|(r, d, b, _)| r == repo && d == dir && b == branch))
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
    fn worktree_move(&mut self, repo: &Path, from: &Path, to: &Path) -> Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, to)?;
        let moved = |p: &Path| p.strip_prefix(from).map(|rest| to.join(rest)).ok();
        for (_, dir, _, _) in &mut self.worktrees {
            if let Some(new) = moved(dir) {
                *dir = new;
            }
        }
        self.heads = std::mem::take(&mut self.heads)
            .into_iter()
            .map(|(dir, head)| (moved(&dir).unwrap_or(dir), head))
            .collect();
        self.moved
            .push((repo.to_path_buf(), from.to_path_buf(), to.to_path_buf()));
        Ok(())
    }
    fn worktree_repair(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        self.repaired.push((repo.to_path_buf(), dir.to_path_buf()));
        Ok(())
    }
    fn remote_url(&self, dir: &Path) -> Result<Option<String>> {
        Ok(self.remotes.get(dir).cloned())
    }
    fn summary(&self, dir: &Path, _base: &str) -> Result<String> {
        Ok(self.summaries.get(dir).cloned().unwrap_or_default())
    }
    fn run(&mut self, dir: &Path, argv: &[String], _env: &[(String, String)]) -> Result<()> {
        self.ran.push((dir.to_path_buf(), argv.to_vec()));
        Ok(())
    }
    fn start_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
    ) -> Result<()> {
        std::fs::write(log, "checks ran\n")?;
        self.checks.push(StartedCheck {
            key: key.to_owned(),
            dir: dir.to_path_buf(),
            argv: argv.to_vec(),
            env: env.to_vec(),
            log: log.to_path_buf(),
        });
        Ok(())
    }
    fn poll_check(&mut self, key: &str) -> Option<Result<i32>> {
        if let Some(code) = self.check_exits.get(key) {
            return Some(Ok(*code));
        }
        if self.checks.iter().any(|c| c.key == key) {
            None
        } else {
            Some(Err(anyhow::anyhow!("no such check in this runner")))
        }
    }
    fn start_reviewer(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
    ) -> Result<()> {
        if !stdout.exists() {
            std::fs::write(stdout, "")?;
        }
        std::fs::write(stderr, "")?;
        let started = StartedCheck {
            key: key.to_owned(),
            dir: dir.to_path_buf(),
            argv: argv.to_vec(),
            env: env.to_vec(),
            log: stdout.to_path_buf(),
        };
        self.checks.push(started.clone());
        self.reviewers.push((started, stderr.to_path_buf()));
        Ok(())
    }
    fn kill_check(&mut self, key: &str) {
        self.checks.retain(|c| c.key != key);
        self.killed_checks.push(key.to_owned());
    }
    fn rev_parse(&self, dir: &Path, _rev: &str) -> Result<String> {
        Ok(self
            .bases
            .get(dir)
            .cloned()
            .unwrap_or_else(|| "base0000".into()))
    }
    fn merge_base(&self, dir: &Path, _a: &str, _b: &str) -> Result<String> {
        Ok(self
            .bases
            .get(dir)
            .cloned()
            .unwrap_or_else(|| "base0000".into()))
    }
}

#[cfg(test)]
mod check_tests {
    use super::*;

    #[test]
    fn a_check_runs_as_a_child_with_its_output_in_the_log_and_is_lost_to_a_new_runner() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("gate.log");
        let mut cli = GitCli::default();
        cli.start_check(
            "k",
            dir.path(),
            &[
                "sh".into(),
                "-c".into(),
                "echo $DISPATCH_LANE; exit 3".into(),
            ],
            &[("DISPATCH_LANE".into(), "backend".into())],
            &log,
        )
        .unwrap();
        let mut code = None;
        for _ in 0..200 {
            if let Some(result) = cli.poll_check("k") {
                code = Some(result.unwrap());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(code, Some(3));
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "backend");
        assert!(GitCli::default().poll_check("k").unwrap().is_err());
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
        // An "origin" with one commit on main, cloned the way Dispatch
        // clones: the clone is the only checkout Dispatch touches.
        let origin = dir.path().join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        let og = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&origin)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        og(&["init", "-q", "-b", "main"]);
        og(&[
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
        let repo = dir.path().join("clone");
        let mut cli = GitCli::default();
        cli.ensure_clone(origin.to_str().unwrap(), &repo).unwrap();
        cli.ensure_clone(origin.to_str().unwrap(), &repo)
            .expect("a second call finds the clone");
        cli.fetch(&repo, "origin").unwrap();
        let wt = dir.path().join("wt").join("t1");
        cli.worktree_add(&repo, &wt, "dispatch/1-x", "origin/main")
            .unwrap();
        assert!(cli.is_clean(&wt).unwrap());
        assert_eq!(cli.head(&wt).unwrap(), cli.head(&repo).unwrap());
        assert!(cli.is_worktree_of(&repo, &wt, "dispatch/1-x").unwrap());
        assert!(
            !cli.is_worktree_of(&repo, &wt, "main").unwrap(),
            "wrong branch"
        );
        let empty = dir.path().join("wt").join("t2");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(
            !cli.is_worktree_of(&repo, &empty, "dispatch/2-x").unwrap(),
            "no worktree"
        );
        let other = dir.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        let other_git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&other)
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "{args:?}");
        };
        other_git(&["init", "-q", "-b", "dispatch/1-x"]);
        other_git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ]);
        assert!(
            !cli.is_worktree_of(&repo, &other, "dispatch/1-x").unwrap(),
            "another repository"
        );
        std::fs::write(wt.join("f"), "x").unwrap();
        assert!(!cli.is_clean(&wt).unwrap());
        cli.run(&wt, &["true".to_owned()], &[]).unwrap();
        assert!(cli.run(&wt, &["false".to_owned()], &[]).is_err());
    }
}
