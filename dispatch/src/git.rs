//! Git, for the lanes: Dispatch's clones and their fetches, a worktree
//! per ticket, branch heads and merge bases, rebases and pushes, whether
//! a tree is clean, commits replayed into a folded history and a branch
//! moved onto it. Fixed argv only; nothing from a ticket is spliced into a
//! command line.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::history::{Commit, Group};

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
    /// `git push --force-with-lease` of `branch` to `remote` from `dir`,
    /// replacing the remote branch only while it is at `expected`.
    fn push_with_lease(
        &mut self,
        dir: &Path,
        remote: &str,
        branch: &str,
        expected: &str,
    ) -> Result<Push>;
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
    /// files go with the tree. A directory deleted by hand is removed
    /// from the clone's records the same way, and one git no longer
    /// lists as a worktree is success. Never `git worktree prune`: that
    /// would also forget every other tree whose directory is missing
    /// right now, such as one on a volume that is not mounted. The
    /// branch stays.
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
    /// The commits of `base..head` in `dir`, oldest first.
    fn commits(&self, dir: &Path, base: &str, head: &str) -> Result<Vec<Commit>>;
    /// The tree `rev` names in `dir`.
    fn tree(&self, dir: &Path, rev: &str) -> Result<String>;
    /// `groups` built as new commits on `onto`, in the object database
    /// only: the new head is returned and nothing moves. Each group's
    /// commit keeps the author and committer of its `author_of`. A pick
    /// that does not apply is an error naming it.
    fn replay(&mut self, dir: &Path, onto: &str, groups: &[Group]) -> Result<String>;
    /// The branch checked out at `dir` moved from `old` to `new`, and
    /// refused when it is not at `old` or `dir` has no branch checked
    /// out. The index and files are left alone: the caller moves only
    /// between heads with the same tree.
    fn set_head(&mut self, dir: &Path, new: &str, old: &str) -> Result<()>;
    /// Whether `remote`'s copy of the branch checked out at `dir`, as
    /// last fetched or pushed, holds a commit of `base..head`. No fetch.
    fn published(&self, dir: &Path, remote: &str, base: &str, head: &str) -> Result<bool>;
}

/// What a leased push did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// The remote branch moved to the local head.
    Pushed,
    /// The remote branch was already at the local head; git wrote
    /// nothing and did not check the lease.
    UpToDate,
    /// The lease refused it: the remote branch is not at `expected`, or
    /// does not exist.
    Refused,
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

/// `git -C <dir>`, as `git` makes it.
fn git_in(dir: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = git();
    cmd.arg("-C").arg(dir);
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
        output(git_in(dir).args(["fetch", "--quiet", "--prune", remote]))?;
        Ok(())
    }

    fn ensure_remote(&mut self, dir: &Path, remote: &str, url: &str) -> Result<()> {
        let known = git_in(dir)
            .args(["remote", "get-url", remote])
            .output()
            .with_context(|| format!("git in {}", dir.display()))?;
        let verb = if known.status.success() {
            "set-url"
        } else {
            "add"
        };
        output(git_in(dir).args(["remote", verb, remote, url]))?;
        Ok(())
    }

    fn fetch_pull(&mut self, dir: &Path, remote: &str, number: u64) -> Result<()> {
        output(git_in(dir).args([
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
            git_in(repo)
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
        output(git_in(repo).args(["worktree", "add"]).arg(dir).args([
            "--track",
            "-B",
            branch,
            &format!("{remote}/{branch}"),
        ]))?;
        Ok(())
    }

    fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String> {
        output(git_in(dir).args(["rev-parse", "--verify", rev]))
    }

    fn merge_base(&self, dir: &Path, a: &str, b: &str) -> Result<String> {
        output(git_in(dir).args(["merge-base", a, b]))
    }

    fn is_worktree_of(&self, repo: &Path, dir: &Path, branch: &str) -> Result<bool> {
        // `rev-parse` answers relative to the directory git was given.
        let show = |where_: &Path, what: &str| -> Result<Option<PathBuf>> {
            let out = git_in(where_)
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
        let head = output(git_in(dir).args(["rev-parse", "--abbrev-ref", "HEAD"]))?;
        Ok(head == branch)
    }

    fn head(&self, dir: &Path) -> Result<String> {
        output(git_in(dir).args(["rev-parse", "HEAD"]))
    }

    fn is_clean(&self, dir: &Path) -> Result<bool> {
        let status = output(git_in(dir).args(["status", "--porcelain"]))?;
        Ok(status.is_empty())
    }

    fn behind(&self, dir: &Path, onto: &str) -> Result<u64> {
        let out = output(git_in(dir).args(["rev-list", "--count", &format!("HEAD..{onto}")]))?;
        out.trim()
            .parse()
            .with_context(|| format!("rev-list printed {out:?}"))
    }

    fn rebase_onto(&mut self, dir: &Path, onto: &str) -> Result<bool> {
        // A rebase someone else began is theirs: starting another would
        // fail, and aborting on that failure would throw their work away.
        let in_progress = output(git_in(dir).args(["rev-parse", "--git-path", "rebase-merge"]))?;
        if Path::new(in_progress.trim()).exists() {
            bail!("a rebase is already in progress at {}", dir.display());
        }
        let status = git_in(dir)
            .args(["rebase", onto])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("git rebase")?;
        if status.success() {
            return Ok(true);
        }
        let _ = git_in(dir).args(["rebase", "--abort"]).status();
        Ok(false)
    }

    fn push_with_lease(
        &mut self,
        dir: &Path,
        remote: &str,
        branch: &str,
        expected: &str,
    ) -> Result<Push> {
        let refname = format!("refs/heads/{branch}");
        let spec = format!("{refname}:{refname}");
        let mut cmd = git_in(dir);
        cmd.args(["push", "--porcelain"])
            .arg(format!("--force-with-lease={refname}:{expected}"))
            .arg(remote)
            .arg(&spec);
        let out = cmd.output().with_context(|| format!("run {cmd:?}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        // Porcelain lines are `<flag>\t<from>:<to>\t<summary>`; the flag
        // says what happened to the ref whatever the exit status.
        let line = stdout
            .lines()
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .find(|fields| fields.get(1) == Some(&spec.as_str()));
        let ok = out.status.success();
        match line.as_deref() {
            Some(["=", ..]) if ok => Ok(Push::UpToDate),
            Some(["+" | " " | "*", ..]) if ok => Ok(Push::Pushed),
            Some(["!", _, summary, ..]) if summary.contains("(stale info)") => Ok(Push::Refused),
            _ => bail!(
                "{cmd:?} exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        }
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
        output(git_in(repo).args(["worktree", "move"]).arg(from).arg(to))?;
        Ok(())
    }

    fn remote_url(&self, dir: &Path) -> Result<Option<String>> {
        let out = git_in(dir)
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
        let log = output(git_in(dir).args(["log", "--oneline", "--no-decorate", &range]))?;
        let stat = output(git_in(dir).args(["diff", "--stat", &range]))?;
        Ok(format!("{log}\n{stat}").trim().to_owned())
    }

    fn worktree_repair(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        output(git_in(repo).args(["worktree", "repair"]).arg(dir))?;
        Ok(())
    }

    fn worktree_remove(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        // A missing directory still goes through `remove`, which drops
        // only this tree's entry from the clone. Git cannot resolve a
        // missing path's symlinks to match its record, so the deepest
        // part that exists is resolved here.
        let dir = resolved(dir);
        let out = git_in(repo)
            .args(["worktree", "remove"])
            .arg(&dir)
            .output()
            .with_context(|| format!("git in {}", repo.display()))?;
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if stderr.contains("is not a working tree") {
            return Ok(());
        }
        bail!("{} not removed: {stderr}", dir.display())
    }

    fn changes(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        // Not `output`: it trims, and a record's status column may
        // begin with a space.
        let mut cmd = git_in(dir);
        cmd.args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
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

    fn commits(&self, dir: &Path, base: &str, head: &str) -> Result<Vec<Commit>> {
        let out = output(git_in(dir).args([
            "log",
            "--reverse",
            "--format=%H%x00%P%x00%B%x1e",
            &format!("{base}..{head}"),
        ]))?;
        Ok(parse_log(&out))
    }

    fn tree(&self, dir: &Path, rev: &str) -> Result<String> {
        output(git_in(dir).args(["rev-parse", "--verify", &format!("{rev}^{{tree}}")]))
    }

    fn replay(&mut self, dir: &Path, onto: &str, groups: &[Group]) -> Result<String> {
        let mut tip = onto.to_owned();
        for group in groups {
            let Some(leader) = group.picks.first() else {
                continue;
            };
            let who = output(git_in(dir).args([
                "show",
                "-s",
                "--date=raw",
                "--format=%an%x00%ae%x00%ad%x00%cn%x00%ce",
                &group.author_of,
            ]))?;
            let fields: Vec<&str> = who.split('\0').collect();
            let [an, ae, ad, cn, ce] = fields[..] else {
                bail!("git show printed {who:?} for {}", group.author_of);
            };
            let env = [
                ("GIT_AUTHOR_NAME", an),
                ("GIT_AUTHOR_EMAIL", ae),
                ("GIT_AUTHOR_DATE", ad),
                ("GIT_COMMITTER_NAME", cn),
                ("GIT_COMMITTER_EMAIL", ce),
            ];
            // Picks after the first are applied on throwaway commits, so
            // each three-way merge has a commit on both sides.
            let mut working = tip.clone();
            let mut tree = String::new();
            for (i, pick) in group.picks.iter().enumerate() {
                tree = merge_pick(dir, &working, pick, leader)?;
                if i + 1 < group.picks.len() {
                    working = commit_tree(dir, &tree, &working, "dispatch: fold", &env)?;
                }
            }
            tip = commit_tree(dir, &tree, &tip, &group.message, &env)?;
        }
        Ok(tip)
    }

    fn set_head(&mut self, dir: &Path, new: &str, old: &str) -> Result<()> {
        // `update-ref HEAD` follows the symbolic ref to the branch; on a
        // detached head it would move HEAD alone.
        output(git_in(dir).args(["symbolic-ref", "-q", "HEAD"]))
            .with_context(|| format!("{} has no branch checked out", dir.display()))?;
        output(git_in(dir).args([
            "update-ref",
            "-m",
            "dispatch: rewrite commits",
            "HEAD",
            new,
            old,
        ]))?;
        Ok(())
    }

    fn published(&self, dir: &Path, remote: &str, base: &str, head: &str) -> Result<bool> {
        let branch = output(git_in(dir).args(["symbolic-ref", "--short", "HEAD"]))?;
        let remote_ref = format!("refs/remotes/{remote}/{branch}");
        let known = git_in(dir)
            .args(["rev-parse", "--verify", "-q", &remote_ref])
            .output()
            .context("run git rev-parse")?;
        if !known.status.success() {
            return Ok(false);
        }
        let Ok(meet) = output(git_in(dir).args(["merge-base", &remote_ref, head])) else {
            return Ok(false);
        };
        let base =
            output(git_in(dir).args(["rev-parse", "--verify", &format!("{base}^{{commit}}")]))?;
        if meet == base {
            return Ok(false);
        }
        let status = git_in(dir)
            .args(["merge-base", "--is-ancestor", &base, &meet])
            .status()
            .context("run git merge-base")?;
        Ok(status.success())
    }
}

/// `pick` applied on `onto` as a cherry-pick would, by a three-way
/// merge with its parent as the base: the resulting tree.
fn merge_pick(dir: &Path, onto: &str, pick: &str, leader: &str) -> Result<String> {
    let mut cmd = git_in(dir);
    cmd.args(["merge-tree", "--write-tree", "--merge-base"])
        .arg(format!("{pick}^"))
        .args([onto, pick]);
    let out = cmd.output().with_context(|| format!("run {cmd:?}"))?;
    match out.status.code() {
        Some(0) => {}
        Some(1) => {
            let subject =
                output(git_in(dir).args(["show", "-s", "--format=%s", pick])).unwrap_or_default();
            bail!("folding {pick} \"{subject}\" into {leader} conflicts");
        }
        _ => bail!(
            "{cmd:?} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .next()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .with_context(|| format!("{cmd:?} printed no tree"))
}

/// A commit of `tree` on `parent` with `message`, through `commit-tree`
/// rather than `commit`, so no hook runs; `env` sets who made it.
fn commit_tree(
    dir: &Path,
    tree: &str,
    parent: &str,
    message: &str,
    env: &[(&str, &str)],
) -> Result<String> {
    use std::io::Write as _;
    let mut cmd = git_in(dir);
    cmd.args(["commit-tree", tree, "-p", parent, "-F", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Set after `git()` removed the caller's `GIT_*`, so these stay.
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().with_context(|| format!("run {cmd:?}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(message.as_bytes())?;
        stdin.write_all(b"\n")?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "{cmd:?} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The commits in `git log --format=%H%x00%P%x00%B%x1e` output.
fn parse_log(text: &str) -> Vec<Commit> {
    text.split('\x1e')
        .map(|entry| entry.trim_start_matches('\n'))
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let mut fields = entry.splitn(3, '\0');
            let sha = fields.next()?.trim().to_owned();
            let parents = fields.next()?.split_whitespace().count();
            let message = fields.next().unwrap_or("").trim().to_owned();
            Some(Commit {
                sha,
                parents: u32::try_from(parents).unwrap_or(u32::MAX),
                message,
            })
        })
        .collect()
}

/// `dir` with its deepest existing ancestor's symlinks resolved, and
/// the missing rest appended as written.
fn resolved(dir: &Path) -> PathBuf {
    for base in dir.ancestors() {
        if let Ok(real) = base.canonicalize() {
            let rest = dir.strip_prefix(base).unwrap_or(Path::new(""));
            return if rest.as_os_str().is_empty() {
                real
            } else {
                real.join(rest)
            };
        }
    }
    dir.to_path_buf()
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
/// removal only drops git's record of it.
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
    /// The head a successful rebase leaves in a tree; a tree not listed
    /// keeps its head.
    pub rebase_heads: std::collections::BTreeMap<PathBuf, String>,
    /// Leased pushes, refused or not: dir, remote, branch, expected.
    pub pushed: Vec<(PathBuf, String, String, String)>,
    /// Trees whose leased push is refused; any other is pushed.
    pub lease_stale: Vec<PathBuf>,
    /// What `changes` reports for a directory, as a test wrote it; a
    /// directory in `dirty` reports `.` besides.
    pub changes: std::collections::BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Worktrees removed, in order: repo, dir. A repeated removal of
    /// the same directory is recorded again.
    pub removed: Vec<(PathBuf, PathBuf)>,
    /// The next removal of this directory fails, once.
    pub fail_remove: Option<PathBuf>,
    /// Trees whose merge base with anything cannot be read.
    pub no_merge_base: Vec<PathBuf>,
    /// What `commits` returns for a tree, whatever the range.
    pub commits: std::collections::BTreeMap<PathBuf, Vec<Commit>>,
    /// A commit's tree by sha; an unlisted sha is `tree0000`, so a
    /// replay has the same tree unless a test says otherwise.
    pub trees: std::collections::BTreeMap<String, String>,
    /// The head `replay` returns for a tree; absent, `fold0001`.
    pub replay_heads: std::collections::BTreeMap<PathBuf, String>,
    /// Trees whose replay stops on a conflict.
    pub replay_conflicts: Vec<PathBuf>,
    /// Replays asked for: dir, onto, groups.
    pub replayed: Vec<(PathBuf, String, Vec<Group>)>,
    /// Heads moved: dir, new, old. A move is refused unless the tree's
    /// head is `old`, and a refused one is not recorded.
    pub head_sets: Vec<(PathBuf, String, String)>,
    /// Trees left dirty by the next head move, once.
    pub dirty_on_set: Vec<PathBuf>,
    /// Branches the remote already holds: dir, base, head. Asked of any
    /// other range, a tree is not published.
    pub published: Vec<(PathBuf, String, String)>,
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
        if let Some(head) = self.rebase_heads.get(dir) {
            self.heads.insert(dir.to_path_buf(), head.clone());
        }
        Ok(true)
    }
    fn push_with_lease(
        &mut self,
        dir: &Path,
        remote: &str,
        branch: &str,
        expected: &str,
    ) -> Result<Push> {
        self.pushed.push((
            dir.to_path_buf(),
            remote.to_owned(),
            branch.to_owned(),
            expected.to_owned(),
        ));
        if self.lease_stale.iter().any(|d| d == dir) {
            return Ok(Push::Refused);
        }
        Ok(Push::Pushed)
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
        if self.no_merge_base.iter().any(|d| d == dir) {
            bail!("no merge base in {}: the fake was told so", dir.display());
        }
        Ok(self
            .bases
            .get(dir)
            .cloned()
            .unwrap_or_else(|| "base0000".into()))
    }
    fn commits(&self, dir: &Path, _base: &str, _head: &str) -> Result<Vec<Commit>> {
        Ok(self.commits.get(dir).cloned().unwrap_or_default())
    }
    fn tree(&self, dir: &Path, rev: &str) -> Result<String> {
        let sha = if rev == "HEAD" {
            self.head(dir)?
        } else {
            rev.to_owned()
        };
        Ok(self
            .trees
            .get(&sha)
            .cloned()
            .unwrap_or_else(|| "tree0000".into()))
    }
    fn replay(&mut self, dir: &Path, onto: &str, groups: &[Group]) -> Result<String> {
        self.replayed
            .push((dir.to_path_buf(), onto.to_owned(), groups.to_vec()));
        if self.replay_conflicts.iter().any(|d| d == dir) {
            let pick = groups
                .iter()
                .find_map(|g| g.picks.get(1))
                .cloned()
                .unwrap_or_default();
            bail!("folding {pick} into a commit conflicts: the fake was told so");
        }
        Ok(self
            .replay_heads
            .get(dir)
            .cloned()
            .unwrap_or_else(|| "fold0001".into()))
    }
    fn set_head(&mut self, dir: &Path, new: &str, old: &str) -> Result<()> {
        let head = self.head(dir)?;
        if head != old {
            bail!("{} is at {head}, not {old}", dir.display());
        }
        self.heads.insert(dir.to_path_buf(), new.to_owned());
        self.head_sets
            .push((dir.to_path_buf(), new.to_owned(), old.to_owned()));
        if let Some(i) = self.dirty_on_set.iter().position(|d| d == dir) {
            self.dirty_on_set.remove(i);
            self.dirty.push(dir.to_path_buf());
        }
        Ok(())
    }
    fn published(&self, dir: &Path, _remote: &str, base: &str, head: &str) -> Result<bool> {
        Ok(self
            .published
            .iter()
            .any(|(d, b, h)| d == dir && b == base && h == head))
    }
}

/// A fake repository shared with a test, so a runner can be replaced (a
/// restart) over the same heads and checks.
impl Repo for std::sync::Arc<std::sync::Mutex<FakeRepo>> {
    fn ensure_clone(&mut self, url: &str, dir: &Path) -> Result<()> {
        self.lock().unwrap().ensure_clone(url, dir)
    }
    fn fetch(&mut self, dir: &Path, remote: &str) -> Result<()> {
        self.lock().unwrap().fetch(dir, remote)
    }
    fn ensure_remote(&mut self, dir: &Path, remote: &str, url: &str) -> Result<()> {
        self.lock().unwrap().ensure_remote(dir, remote, url)
    }
    fn fetch_pull(&mut self, dir: &Path, remote: &str, number: u64) -> Result<()> {
        self.lock().unwrap().fetch_pull(dir, remote, number)
    }
    fn worktree_add(&mut self, repo: &Path, dir: &Path, branch: &str, start: &str) -> Result<()> {
        self.lock().unwrap().worktree_add(repo, dir, branch, start)
    }
    fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String> {
        self.lock().unwrap().rev_parse(dir, rev)
    }
    fn merge_base(&self, dir: &Path, a: &str, b: &str) -> Result<String> {
        self.lock().unwrap().merge_base(dir, a, b)
    }
    fn worktree_track(
        &mut self,
        repo: &Path,
        dir: &Path,
        branch: &str,
        remote: &str,
    ) -> Result<()> {
        self.lock()
            .unwrap()
            .worktree_track(repo, dir, branch, remote)
    }
    fn is_worktree_of(&self, repo: &Path, dir: &Path, branch: &str) -> Result<bool> {
        self.lock().unwrap().is_worktree_of(repo, dir, branch)
    }
    fn head(&self, dir: &Path) -> Result<String> {
        self.lock().unwrap().head(dir)
    }
    fn is_clean(&self, dir: &Path) -> Result<bool> {
        self.lock().unwrap().is_clean(dir)
    }
    fn behind(&self, dir: &Path, onto: &str) -> Result<u64> {
        self.lock().unwrap().behind(dir, onto)
    }
    fn rebase_onto(&mut self, dir: &Path, onto: &str) -> Result<bool> {
        self.lock().unwrap().rebase_onto(dir, onto)
    }
    fn push_with_lease(
        &mut self,
        dir: &Path,
        remote: &str,
        branch: &str,
        expected: &str,
    ) -> Result<Push> {
        self.lock()
            .unwrap()
            .push_with_lease(dir, remote, branch, expected)
    }
    fn free_bytes(&self, dir: &Path) -> Result<u64> {
        self.lock().unwrap().free_bytes(dir)
    }
    fn worktree_move(&mut self, repo: &Path, from: &Path, to: &Path) -> Result<()> {
        self.lock().unwrap().worktree_move(repo, from, to)
    }
    fn worktree_repair(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        self.lock().unwrap().worktree_repair(repo, dir)
    }
    fn worktree_remove(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        self.lock().unwrap().worktree_remove(repo, dir)
    }
    fn changes(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        self.lock().unwrap().changes(dir)
    }
    fn remote_url(&self, dir: &Path) -> Result<Option<String>> {
        self.lock().unwrap().remote_url(dir)
    }
    fn summary(&self, dir: &Path, base: &str) -> Result<String> {
        self.lock().unwrap().summary(dir, base)
    }
    fn run(&mut self, dir: &Path, argv: &[String], env: &[(String, String)]) -> Result<()> {
        self.lock().unwrap().run(dir, argv, env)
    }
    fn start_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
    ) -> Result<()> {
        self.lock().unwrap().start_check(key, dir, argv, env, log)
    }
    fn poll_check(&mut self, key: &str) -> Option<Result<i32>> {
        self.lock().unwrap().poll_check(key)
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
        self.lock()
            .unwrap()
            .start_reviewer(key, dir, argv, env, stdout, stderr)
    }
    fn kill_check(&mut self, key: &str) {
        self.lock().unwrap().kill_check(key);
    }
    fn commits(&self, dir: &Path, base: &str, head: &str) -> Result<Vec<Commit>> {
        self.lock().unwrap().commits(dir, base, head)
    }
    fn tree(&self, dir: &Path, rev: &str) -> Result<String> {
        self.lock().unwrap().tree(dir, rev)
    }
    fn replay(&mut self, dir: &Path, onto: &str, groups: &[Group]) -> Result<String> {
        self.lock().unwrap().replay(dir, onto, groups)
    }
    fn set_head(&mut self, dir: &Path, new: &str, old: &str) -> Result<()> {
        self.lock().unwrap().set_head(dir, new, old)
    }
    fn published(&self, dir: &Path, remote: &str, base: &str, head: &str) -> Result<bool> {
        self.lock().unwrap().published(dir, remote, base, head)
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
        // The clone is the only checkout Dispatch touches.
        let repo = origin_and_clone(dir.path(), "p");
        let origin = dir.path().join("p-origin");
        let mut cli = GitCli::default();
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
        sh(&other, &["init", "-q", "-b", "dispatch/1-x"]);
        sh(&other, &["commit", "-q", "--allow-empty", "-m", "x"]);
        assert!(
            !cli.is_worktree_of(&repo, &other, "dispatch/1-x").unwrap(),
            "another repository"
        );
        std::fs::write(wt.join("f"), "x").unwrap();
        assert!(!cli.is_clean(&wt).unwrap());
        cli.run(&wt, &["true".to_owned()], &[]).unwrap();
        assert!(cli.run(&wt, &["false".to_owned()], &[]).is_err());
    }

    /// `git <args>` in `dir`, which must succeed. Through `git_in`, so a
    /// test run from a git hook, with the hook's `GIT_DIR` and
    /// `GIT_INDEX_FILE` set, works on the test's repository and never
    /// on the one being committed to.
    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = git_in(dir)
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
    fn the_real_git_pushes_with_a_lease_and_is_refused_when_the_remote_moved() {
        let dir = tempfile::tempdir().unwrap();
        let clone = origin_and_clone(dir.path(), "p");
        let origin = dir.path().join("p-origin");
        let mut cli = GitCli::default();
        let origin_f = || {
            sh(&origin, &["rev-parse", "refs/heads/f"])
                .trim()
                .to_owned()
        };
        sh(&clone, &["checkout", "-q", "-b", "f"]);
        sh(&clone, &["commit", "-q", "--allow-empty", "-m", "one"]);
        sh(&clone, &["push", "-q", "origin", "f"]);
        let old = sh(&clone, &["rev-parse", "HEAD"]).trim().to_owned();

        sh(
            &clone,
            &["commit", "-q", "--amend", "--allow-empty", "-m", "two"],
        );
        let two = sh(&clone, &["rev-parse", "HEAD"]).trim().to_owned();
        let push = cli.push_with_lease(&clone, "origin", "f", &old).unwrap();
        assert_eq!(push, Push::Pushed);
        assert_eq!(origin_f(), two);

        // Already there: git reports it up to date without checking the
        // lease, and writes nothing.
        let push = cli.push_with_lease(&clone, "origin", "f", &old).unwrap();
        assert_eq!(push, Push::UpToDate);
        assert_eq!(origin_f(), two);

        // Someone else moves the branch; the lease on what we last saw
        // refuses to overwrite it.
        let tree = sh(&origin, &["rev-parse", "refs/heads/f^{tree}"]);
        let theirs = sh(
            &origin,
            &[
                "commit-tree",
                "-p",
                "refs/heads/f",
                "-m",
                "theirs",
                tree.trim(),
            ],
        )
        .trim()
        .to_owned();
        sh(&origin, &["update-ref", "refs/heads/f", &theirs]);
        sh(
            &clone,
            &["commit", "-q", "--amend", "--allow-empty", "-m", "three"],
        );
        let push = cli.push_with_lease(&clone, "origin", "f", &two).unwrap();
        assert_eq!(push, Push::Refused);
        assert_eq!(origin_f(), theirs);

        // A branch the remote does not have is refused too.
        sh(&clone, &["branch", "g"]);
        let push = cli.push_with_lease(&clone, "origin", "g", &old).unwrap();
        assert_eq!(push, Push::Refused);
        let status = git_in(&origin)
            .args(["rev-parse", "--verify", "-q", "refs/heads/g"])
            .output()
            .unwrap()
            .status;
        assert!(!status.success());
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
        // Deleted by hand: a success, and the clone forgets it, but not
        // another tree that is missing too (an unmounted volume).
        let gone = dir.path().join("wt").join("t2");
        let unmounted = dir.path().join("wt").join("t4");
        for (tree, branch) in [(&gone, "dispatch/2-x"), (&unmounted, "dispatch/4-x")] {
            cli.worktree_add(&repo, tree, branch, "origin/main")
                .unwrap();
            std::fs::remove_dir_all(tree).unwrap();
        }
        cli.worktree_remove(&repo, &gone).unwrap();
        let list = sh(&repo, &["worktree", "list", "--porcelain"]);
        assert!(!list.contains("t2"), "{list}");
        assert!(list.contains("t4"), "{list}");
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
    fn a_tree_deleted_by_hand_is_skipped_by_the_preflight_and_forgotten_on_removal() {
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

    /// A worktree of a fresh clone on `dispatch/1-x`, and the commit it
    /// was cut at.
    fn cut(root: &Path) -> (PathBuf, String) {
        let repo = origin_and_clone(root, "p");
        let wt = root.join("wt").join("t1");
        GitCli::default()
            .worktree_add(&repo, &wt, "dispatch/1-x", "origin/main")
            .unwrap();
        let base = GitCli::default().head(&wt).unwrap();
        (wt, base)
    }

    /// `file` written with `text` and committed with `args` after
    /// `commit`; the new head.
    fn commit(dir: &Path, file: &str, text: &str, args: &[&str]) -> String {
        std::fs::write(dir.join(file), text).unwrap();
        sh(dir, &["add", file]);
        let mut all = vec!["commit", "-q"];
        all.extend_from_slice(args);
        sh(dir, &all);
        sh(dir, &["rev-parse", "HEAD"]).trim().to_owned()
    }

    #[test]
    fn the_real_git_folds_fix_rounds_into_the_commits_they_amend() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        let a = commit(&wt, "a", "a1\n", &["-m", "A", "-m", "why A"]);
        let b = commit(&wt, "b", "b1\n", &["-m", "B"]);
        let f1 = commit(&wt, "a", "a2\n", &["--fixup", &a]);
        let p = commit(&wt, "b", "b2\n", &["-m", "plain fix"]);
        let mut cli = GitCli::default();
        let tree_before = cli.tree(&wt, "HEAD").unwrap();
        let commits = cli.commits(&wt, &base, &p).unwrap();
        assert_eq!(
            commits.iter().map(|c| c.sha.as_str()).collect::<Vec<_>>(),
            [a.as_str(), b.as_str(), f1.as_str(), p.as_str()]
        );
        assert_eq!(commits[0].message, "A\n\nwhy A");
        assert_eq!(commits[2].subject(), "fixup! A");
        let ranges = [(b.clone(), f1.clone()), (f1.clone(), p.clone())];
        let groups = crate::history::fold_plan(&commits, &base, &ranges).unwrap();
        let after = cli.replay(&wt, &base, &groups).unwrap();
        assert_eq!(cli.head(&wt).unwrap(), p, "a replay moves nothing");
        assert_eq!(cli.tree(&wt, &after).unwrap(), tree_before);
        cli.set_head(&wt, &after, &p).unwrap();
        assert_eq!(cli.head(&wt).unwrap(), after);
        assert!(cli.is_clean(&wt).unwrap());
        let count = sh(&wt, &["rev-list", "--count", &format!("{base}..HEAD")]);
        assert_eq!(count.trim(), "2");
        assert_eq!(
            sh(&wt, &["log", "-1", "--format=%B", "HEAD~1"]).trim(),
            "A\n\nwhy A"
        );
        assert_eq!(sh(&wt, &["show", "HEAD~1:a"]), "a2\n");
        assert_eq!(
            sh(&wt, &["diff", "--name-only", "HEAD~2", "HEAD~1"]).trim(),
            "a"
        );
        assert_eq!(sh(&wt, &["log", "-1", "--format=%s", "HEAD"]).trim(), "B");
        assert_eq!(sh(&wt, &["show", "HEAD:b"]), "b2\n");
        assert_eq!(cli.tree(&wt, "HEAD").unwrap(), tree_before);
        let reflog = sh(&wt, &["reflog", "--format=%H"]);
        assert!(
            reflog.contains(&p),
            "the old head stays reachable: {reflog}"
        );
    }

    #[test]
    fn the_real_git_squashes_to_one_commit_with_the_first_message() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        let a = commit(
            &wt,
            "a",
            "a1\n",
            &[
                "--author",
                "Ann <ann@example.com>",
                "-m",
                "A",
                "-m",
                "why A",
            ],
        );
        commit(&wt, "b", "b1\n", &["-m", "B"]);
        let head = commit(&wt, "a", "a2\n", &["-m", "plain fix"]);
        let mut cli = GitCli::default();
        let tree_before = cli.tree(&wt, "HEAD").unwrap();
        let commits = cli.commits(&wt, &base, &head).unwrap();
        let groups = crate::history::one_plan(&commits, &base, &[]).unwrap();
        let after = cli.replay(&wt, &base, &groups).unwrap();
        cli.set_head(&wt, &after, &head).unwrap();
        let count = sh(&wt, &["rev-list", "--count", &format!("{base}..HEAD")]);
        assert_eq!(count.trim(), "1");
        assert_eq!(sh(&wt, &["log", "-1", "--format=%B"]).trim(), "A\n\nwhy A");
        assert_eq!(
            sh(&wt, &["log", "-1", "--format=%an <%ae> %ad"]),
            sh(&wt, &["log", "-1", "--format=%an <%ae> %ad", &a])
        );
        assert_eq!(cli.tree(&wt, "HEAD").unwrap(), tree_before);
    }

    #[test]
    fn the_real_git_refuses_a_conflicting_fold_and_moves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        let a = commit(&wt, "a", "1\n", &["-m", "A"]);
        commit(&wt, "a", "2\n", &["-m", "B"]);
        let fixup = commit(&wt, "a", "3\n", &["--fixup", &a]);
        let mut cli = GitCli::default();
        let commits = cli.commits(&wt, &base, &fixup).unwrap();
        let groups = crate::history::fold_plan(&commits, &base, &[]).unwrap();
        let e = cli.replay(&wt, &base, &groups).unwrap_err().to_string();
        assert!(
            e.contains(&format!("folding {fixup} \"fixup! A\" into {a} conflicts")),
            "{e}"
        );
        assert_eq!(cli.head(&wt).unwrap(), fixup);
        assert!(cli.is_clean(&wt).unwrap());
    }

    #[test]
    fn set_head_is_refused_when_the_branch_moved() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        let one = commit(&wt, "a", "1\n", &["-m", "one"]);
        let mut cli = GitCli::default();
        assert!(
            cli.set_head(&wt, &base, &base).is_err(),
            "not at the old head"
        );
        assert_eq!(cli.head(&wt).unwrap(), one);
        sh(&wt, &["checkout", "-q", "--detach"]);
        let e = cli.set_head(&wt, &base, &one).unwrap_err();
        assert!(format!("{e:#}").contains("no branch checked out"), "{e:#}");
        assert_eq!(cli.head(&wt).unwrap(), one);
    }

    #[test]
    fn the_real_git_reads_a_branch_as_published_only_with_its_own_commits() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        let cli = GitCli::default();
        assert!(
            !cli.published(&wt, "origin", &base, "HEAD").unwrap(),
            "no remote ref"
        );
        sh(&wt, &["push", "-q", "origin", "dispatch/1-x"]);
        assert!(
            !cli.published(&wt, "origin", &base, "HEAD").unwrap(),
            "the remote holds only the base"
        );
        let head = commit(&wt, "a", "1\n", &["-m", "one"]);
        assert!(
            !cli.published(&wt, "origin", &base, &head).unwrap(),
            "the commit is not pushed yet"
        );
        sh(&wt, &["push", "-q", "origin", "dispatch/1-x"]);
        assert!(cli.published(&wt, "origin", &base, &head).unwrap());
    }
}
