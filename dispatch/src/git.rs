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
    /// Commits `onto` has that the branch at `dir` does not.
    fn behind(&self, dir: &Path, onto: &str) -> Result<u64>;
    /// `git rebase <onto>` at `dir`; `false` when it stopped on a
    /// conflict, in which case it is aborted and the tree is as it was.
    fn rebase_onto(&mut self, dir: &Path, onto: &str) -> Result<bool>;
    /// Bytes free on the volume holding `dir`, for the preflight that
    /// keeps a full disk from failing an attempt.
    fn free_bytes(&self, dir: &Path) -> Result<u64>;
    /// Move a worktree of `repo` from `from` to `to`, git's own records
    /// of it included.
    fn worktree_move(&mut self, repo: &Path, from: &Path, to: &Path) -> Result<()>;
    /// Re-point `repo` at its worktree now at `dir`, after the tree was
    /// moved by something other than git (a parent directory moved).
    fn worktree_repair(&mut self, repo: &Path, dir: &Path) -> Result<()>;
    /// `git worktree remove <dir>` in `repo`, never forced: git refuses a
    /// tree with modified or untracked files, and so does this. Ignored
    /// files go with the tree. A directory already gone, or one git no
    /// longer lists as a worktree, is success, after `git worktree
    /// prune` so the clone forgets a tree deleted by hand. The branch
    /// stays.
    fn worktree_remove(&mut self, repo: &Path, dir: &Path) -> Result<()>;
    /// What `git status` reports in `dir`, every untracked file listed
    /// on its own, as paths relative to `dir`. A nested repository is
    /// reported once at its own path. Read-only.
    fn changes(&self, dir: &Path) -> Result<Vec<PathBuf>>;
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

    fn behind(&self, dir: &Path, onto: &str) -> Result<u64> {
        let out = output(git().arg("-C").arg(dir).args([
            "rev-list",
            "--count",
            &format!("HEAD..{onto}"),
        ]))?;
        out.trim()
            .parse()
            .with_context(|| format!("rev-list printed {out:?}"))
    }

    fn rebase_onto(&mut self, dir: &Path, onto: &str) -> Result<bool> {
        // A rebase someone else began is theirs: starting another would
        // fail, and aborting on that failure would throw their work away.
        let in_progress =
            output(
                git()
                    .arg("-C")
                    .arg(dir)
                    .args(["rev-parse", "--git-path", "rebase-merge"]),
            )?;
        if Path::new(in_progress.trim()).exists() {
            bail!("a rebase is already in progress at {}", dir.display());
        }
        let status = git()
            .arg("-C")
            .arg(dir)
            .args(["rebase", onto])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("git rebase")?;
        if status.success() {
            return Ok(true);
        }
        let _ = git()
            .arg("-C")
            .arg(dir)
            .args(["rebase", "--abort"])
            .status();
        Ok(false)
    }

    fn free_bytes(&self, dir: &Path) -> Result<u64> {
        // `df -k` is on every macOS and Linux; its last line is the
        // volume, and the fourth column is 1K blocks available.
        let out = output(Command::new("df").arg("-k").arg(dir))?;
        let line = out.lines().last().context("df printed nothing")?;
        let avail: u64 = line
            .split_whitespace()
            .nth(3)
            .and_then(|s| s.parse().ok())
            .with_context(|| format!("df line not understood: {line:?}"))?;
        Ok(avail.saturating_mul(1024))
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

    fn worktree_remove(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        let prune = || output(git().arg("-C").arg(repo).args(["worktree", "prune"]));
        if !dir.exists() {
            prune()?;
            return Ok(());
        }
        let out = git()
            .arg("-C")
            .arg(repo)
            .args(["worktree", "remove"])
            .arg(dir)
            .output()
            .with_context(|| format!("git in {}", repo.display()))?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if stderr.contains("is not a working tree") {
            prune()?;
            return Ok(());
        }
        bail!("{} not removed: {stderr}", dir.display())
    }

    fn changes(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        // Not `output`: it trims, and a record's status column may
        // begin with a space.
        let mut cmd = git();
        cmd.arg("-C")
            .arg(dir)
            .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
        let out = cmd.output().with_context(|| format!("run {cmd:?}"))?;
        if !out.status.success() {
            bail!(
                "{cmd:?} exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(parse_status_z(&out.stdout))
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

/// The paths in `git status --porcelain=v1 -z` output: each record is
/// `XY <path>`, and a rename or copy is followed by its original path,
/// which is skipped. The `/` git puts after a nested repository goes.
fn parse_status_z(bytes: &[u8]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        let Some(path) = record.get(3..) else {
            continue;
        };
        let text = String::from_utf8_lossy(path);
        paths.push(PathBuf::from(text.trim_end_matches('/')));
        if matches!(record.first(), Some(b'R' | b'C')) {
            records.next();
        }
    }
    paths
}

/// What stands in the way of removing a ticket's trees, as absolute
/// paths: every change in each lane worktree in `lanes` (a lane with a
/// repository of its own, nested in `tree`), and every change in `tree`
/// that is not one of those lanes or inside one. The plain `is_clean`
/// cannot answer this: a clean nested lane is untracked content in the
/// outer tree. A tree whose directory is already gone is skipped; its
/// removal will be a prune.
pub fn uncommitted(git: &dyn Repo, tree: Option<&Path>, lanes: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut nested: Vec<PathBuf> = Vec::new();
    for lane in lanes {
        if let Some(tree) = tree
            && let Ok(rel) = lane.strip_prefix(tree)
        {
            nested.push(rel.to_path_buf());
        }
        if !lane.exists() {
            continue;
        }
        found.extend(git.changes(lane)?.into_iter().map(|c| lane.join(c)));
    }
    if let Some(tree) = tree
        && tree.exists()
    {
        found.extend(
            git.changes(tree)?
                .into_iter()
                .filter(|c| !nested.iter().any(|lane| c.starts_with(lane)))
                .map(|c| tree.join(c)),
        );
    }
    Ok(found)
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
    /// Bytes free on the fake volume; `None` is plenty.
    pub free_bytes: Option<u64>,
    /// Commits a tree is behind its base, as a test set it; a rebase
    /// brings it to zero.
    pub behind: std::collections::BTreeMap<PathBuf, u64>,
    /// Trees whose rebase stops on a conflict.
    pub rebase_conflicts: Vec<PathBuf>,
    /// Rebases done: dir, onto.
    pub rebased: Vec<(PathBuf, String)>,
    /// What `changes` reports for a directory, as a test wrote it; a
    /// directory in `dirty` reports `.` besides.
    pub changes: std::collections::BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Worktrees removed, in order: repo, dir. A repeated removal of
    /// the same directory is recorded again.
    pub removed: Vec<(PathBuf, PathBuf)>,
    /// The next removal of this directory fails, once.
    pub fail_remove: Option<PathBuf>,
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

    fn free_bytes(&self, _dir: &Path) -> Result<u64> {
        Ok(self.free_bytes.unwrap_or(u64::MAX))
    }
    fn behind(&self, dir: &Path, _onto: &str) -> Result<u64> {
        Ok(self.behind.get(dir).copied().unwrap_or(0))
    }
    fn rebase_onto(&mut self, dir: &Path, onto: &str) -> Result<bool> {
        self.rebased.push((dir.to_path_buf(), onto.to_owned()));
        if self.rebase_conflicts.iter().any(|d| d == dir) {
            return Ok(false);
        }
        self.behind.remove(dir);
        Ok(true)
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
    fn worktree_remove(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        if self.fail_remove.as_deref() == Some(dir) {
            self.fail_remove = None;
            bail!("{} not removed: the fake was told to fail", dir.display());
        }
        if self.dirty.iter().any(|d| d == dir) {
            bail!("{} not removed: it has changes", dir.display());
        }
        self.removed.push((repo.to_path_buf(), dir.to_path_buf()));
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        self.worktrees.retain(|(_, d, _, _)| d != dir);
        self.heads.remove(dir);
        Ok(())
    }
    fn changes(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        let mut found = self.changes.get(dir).cloned().unwrap_or_default();
        if self.dirty.iter().any(|d| d == dir) {
            found.push(PathBuf::from("."));
        }
        Ok(found)
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
            // `git()`, not a bare `Command`: run from a hook, the hook's
            // `GIT_DIR` would point these at the repository being committed.
            let out = git().arg("-C").arg(&origin).args(args).output().unwrap();
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
            let ok = git()
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

    /// `git <args>` in `dir`, which must succeed. Through `git()`, so a
    /// test run from a git hook, with the hook's `GIT_DIR` and
    /// `GIT_INDEX_FILE` set, works on the test's repository and never
    /// on the one being committed to.
    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = git()
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// An origin with one commit on main, and Dispatch's clone of it.
    fn origin_and_clone(root: &Path, name: &str) -> PathBuf {
        let origin = root.join(format!("{name}-origin"));
        std::fs::create_dir_all(&origin).unwrap();
        sh(&origin, &["init", "-q", "-b", "main"]);
        sh(&origin, &["commit", "-q", "--allow-empty", "-m", "root"]);
        let clone = root.join(format!("{name}-clone"));
        let mut cli = GitCli::default();
        cli.ensure_clone(origin.to_str().unwrap(), &clone).unwrap();
        clone
    }

    #[test]
    fn the_real_git_removes_a_worktree_and_keeps_its_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = origin_and_clone(dir.path(), "p");
        let mut cli = GitCli::default();
        let wt = dir.path().join("wt").join("t1");
        cli.worktree_add(&repo, &wt, "dispatch/1-x", "origin/main")
            .unwrap();
        cli.worktree_remove(&repo, &wt).unwrap();
        assert!(!wt.exists());
        cli.rev_parse(&repo, "refs/heads/dispatch/1-x")
            .expect("the branch stays");
        cli.worktree_remove(&repo, &wt)
            .expect("a second removal finds it gone");
        // Deleted by hand: a success, and the clone forgets it.
        let gone = dir.path().join("wt").join("t2");
        cli.worktree_add(&repo, &gone, "dispatch/2-x", "origin/main")
            .unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        cli.worktree_remove(&repo, &gone).unwrap();
        let list = sh(&repo, &["worktree", "list", "--porcelain"]);
        assert!(!list.contains("t2"), "{list}");
        // An untracked file: git refuses, and the tree stays.
        let kept = dir.path().join("wt").join("t3");
        cli.worktree_add(&repo, &kept, "dispatch/3-x", "origin/main")
            .unwrap();
        std::fs::write(kept.join("notes.txt"), "mine").unwrap();
        let e = cli.worktree_remove(&repo, &kept).unwrap_err();
        assert!(e.to_string().contains("not removed"), "{e:#}");
        assert!(kept.join("notes.txt").exists());
    }

    #[test]
    fn changes_reads_renames_and_spaces_from_the_nul_separated_status() {
        let dir = tempfile::tempdir().unwrap();
        let repo = origin_and_clone(dir.path(), "p");
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        sh(&repo, &["add", "a.txt"]);
        sh(&repo, &["commit", "-q", "-m", "a"]);
        sh(&repo, &["mv", "a.txt", "b.txt"]);
        std::fs::write(repo.join("with space.txt"), "x").unwrap();
        let mut found = GitCli::default().changes(&repo).unwrap();
        found.sort();
        assert_eq!(
            found,
            [PathBuf::from("b.txt"), PathBuf::from("with space.txt")]
        );
        assert_eq!(
            parse_status_z(b" M lead.rs\0R  new.rs\0old.rs\0?? nested/\0"),
            [
                PathBuf::from("lead.rs"),
                PathBuf::from("new.rs"),
                PathBuf::from("nested")
            ]
        );
    }

    /// The ticket's tree and a lane of a second repository nested in it
    /// at `rel`, a path the outer repository does not ignore.
    fn nested(root: &Path, rel: &str) -> (PathBuf, PathBuf) {
        let mut cli = GitCli::default();
        let outer_clone = origin_and_clone(root, "outer");
        let lane_clone = origin_and_clone(root, "lane");
        let tree = root.join("wt").join("t");
        cli.worktree_add(&outer_clone, &tree, "dispatch/1-x", "origin/main")
            .unwrap();
        let lane = tree.join(rel);
        cli.worktree_add(&lane_clone, &lane, "dispatch/1-x", "origin/main")
            .unwrap();
        (tree, lane)
    }

    #[test]
    fn a_clean_nested_lane_passes_the_preflight_the_plain_status_would_fail() {
        let dir = tempfile::tempdir().unwrap();
        let (tree, lane) = nested(dir.path(), "backend");
        let cli = GitCli::default();
        assert!(
            !cli.is_clean(&tree).unwrap(),
            "the lane is untracked content"
        );
        assert_eq!(cli.changes(&tree).unwrap(), [PathBuf::from("backend")]);
        let lanes = [lane.clone()];
        assert!(uncommitted(&cli, Some(&tree), &lanes).unwrap().is_empty());
        // Beside the lane: refused, naming that file and not the lane.
        std::fs::write(tree.join("stray.txt"), "x").unwrap();
        assert_eq!(
            uncommitted(&cli, Some(&tree), &lanes).unwrap(),
            [tree.join("stray.txt")]
        );
        std::fs::remove_file(tree.join("stray.txt")).unwrap();
        // Inside the lane: refused, naming the lane's file.
        std::fs::write(lane.join("todo.txt"), "x").unwrap();
        assert_eq!(
            uncommitted(&cli, Some(&tree), &lanes).unwrap(),
            [lane.join("todo.txt")]
        );
    }

    #[test]
    fn a_lane_under_an_untracked_parent_is_named_at_its_own_path() {
        let dir = tempfile::tempdir().unwrap();
        let (tree, lane) = nested(dir.path(), "packages/backend");
        let cli = GitCli::default();
        assert_eq!(
            cli.changes(&tree).unwrap(),
            [PathBuf::from("packages/backend")],
            "not packages/"
        );
        let lanes = [lane];
        assert!(uncommitted(&cli, Some(&tree), &lanes).unwrap().is_empty());
        std::fs::write(tree.join("packages").join("notes.txt"), "x").unwrap();
        assert_eq!(
            uncommitted(&cli, Some(&tree), &lanes).unwrap(),
            [tree.join("packages/notes.txt")],
            "the lane's path is taken out, nothing above or beside it"
        );
    }

    #[test]
    fn a_tree_deleted_by_hand_is_skipped_by_the_preflight_and_pruned_on_removal() {
        let dir = tempfile::tempdir().unwrap();
        let (tree, lane) = nested(dir.path(), "backend");
        let mut cli = GitCli::default();
        std::fs::remove_dir_all(&tree).unwrap();
        let lanes = [lane.clone()];
        assert!(uncommitted(&cli, Some(&tree), &lanes).unwrap().is_empty());
        let lane_clone = dir.path().join("lane-clone");
        let outer_clone = dir.path().join("outer-clone");
        cli.worktree_remove(&lane_clone, &lane).unwrap();
        cli.worktree_remove(&outer_clone, &tree).unwrap();
        for clone in [&lane_clone, &outer_clone] {
            let list = sh(clone, &["worktree", "list", "--porcelain"]);
            assert_eq!(list.matches("worktree ").count(), 1, "{list}");
        }
    }
}
