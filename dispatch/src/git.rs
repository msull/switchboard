//! Git, for the lanes: Dispatch's clones and their fetches, a worktree
//! per ticket, branch heads and merge bases, rebases and pushes, whether
//! a tree is clean, commits replayed into a folded history and a branch
//! moved onto it. Fixed argv only; nothing from a ticket is spliced into a
//! command line.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use switchboard_control::RECORD_TOKEN_ENV;

use crate::history::{self, Commit, Group};
use crate::ticket::CheckGroup;

/// What a pipeline command may touch. The adapter turns this into the
/// platform's mechanism; the scheduler never sees a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confine {
    /// Directories the command may write under, besides the base set
    /// the adapter always allows (the system temp directory, the
    /// devices a shell needs, a linked worktree's own git directory).
    pub writable: Vec<PathBuf>,
    /// Whether the command may reach off this machine. Loopback stays
    /// open under `Deny`, so a test can talk to a service it started.
    pub network: Network,
}

/// Whether a confined command may open connections off this machine,
/// as a pipeline spells it: `allow` or `deny`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// Any connection.
    #[default]
    Allow,
    /// Loopback only.
    Deny,
}

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
    /// Whether `refs/heads/<branch>` exists in `repo`.
    fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool>;
    /// Commits on `branch` that `start` does not have, read locally.
    fn branch_ahead(&self, repo: &Path, branch: &str, start: &str) -> Result<u64>;
    /// `git branch -D <branch>`: only for a branch with nothing beyond its
    /// start. Git refuses one checked out in a worktree, and so does this.
    fn delete_branch(&mut self, repo: &Path, branch: &str) -> Result<()>;
    /// `git branch -m <from> <to>`, never `-M`: an existing `to` is refused.
    fn rename_branch(&mut self, repo: &Path, from: &str, to: &str) -> Result<()>;
    /// `git worktree add <dir> <branch>`: the existing branch at its head.
    fn worktree_checkout(&mut self, repo: &Path, dir: &Path, branch: &str) -> Result<()>;
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
    /// Whether `dir` has a rebase stopped part way, of either backend.
    fn rebase_in_progress(&self, dir: &Path) -> Result<bool>;
    /// The commit `branch` names when it is what `dir` has checked out;
    /// `None` when `HEAD` is detached (a stopped rebase, a hand
    /// `checkout --detach`) or on another branch.
    fn branch_head(&self, dir: &Path, branch: &str) -> Result<Option<String>>;
    /// Commits `onto` has that `HEAD` at `dir` does not. It counts from
    /// `HEAD`, so it is the branch's count only when `branch_head`
    /// returned `Some`.
    fn behind(&self, dir: &Path, onto: &str) -> Result<u64>;
    /// `git rebase <onto>` at `dir`; `false` when it stopped on a
    /// conflict, in which case it is aborted and the tree is as it was.
    fn rebase_onto(&mut self, dir: &Path, onto: &str) -> Result<bool>;
    /// The commits of `base..head` whose replay onto `onto` conflicts,
    /// oldest first, read without touching the tree or any ref: each
    /// commit is merged onto the result of the one before, so once one
    /// conflicts the rest are read against a tree with its markers.
    fn conflicting_commits(
        &self,
        dir: &Path,
        base: &str,
        head: &str,
        onto: &str,
    ) -> Result<Vec<String>>;
    /// The files a merge of `head` and `onto` conflicts in, read the way
    /// a provider decides whether a pull request can merge, without
    /// touching the tree or any ref; empty when it merges cleanly.
    fn conflicting_files(&self, dir: &Path, head: &str, onto: &str) -> Result<Vec<String>>;
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
    /// Whether `port` can be bound on this machine: on `0.0.0.0`, on
    /// `127.0.0.1`, and on `[::1]` when IPv6 is up. A dev server bound
    /// to any of them, or to the wildcard, makes it busy.
    fn port_free(&self, port: u16) -> bool;
    /// Whether something on `localhost:<port>` answers `GET <path>` with
    /// an HTTP status line, whatever the status.
    fn answers_http(&self, port: u16, path: &str) -> bool;
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
    /// `run` under `confine`. `Ok` is the header line saying what the
    /// command ran under (an empty `argv` runs nothing, as with `run`,
    /// and still gets the header); an `Err` starts with it, then the
    /// command's stderr, which ends with any writes the sandbox refused.
    fn run_confined(
        &mut self,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        confine: &Confine,
    ) -> Result<String>;
    /// Start a check (a command gate) in `dir` as a child of the runner,
    /// in its own process group so a kill reaches its descendants, its
    /// output appended to `log`, under `key` for polling. Nothing from a
    /// template reaches the command line; values go in `env`. A
    /// non-empty `outer` (`switchboard-env exec --`) is put in front of
    /// the command as it is spawned.
    fn start_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        outer: &[String],
    ) -> Result<()>;
    /// `start_check` under `confine`: the log starts with a header line
    /// saying what the check runs under, and a failed check's log ends
    /// with any writes the sandbox refused. `outer` goes outside the
    /// confinement, so it can reach Switchboard's socket while the
    /// command itself stays confined.
    #[allow(clippy::too_many_arguments)]
    fn start_check_confined(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        confine: &Confine,
        outer: &[String],
    ) -> Result<()>;
    /// `None` while the check runs, `Some(Ok(code))` once it exited, and
    /// `Some(Err)` for a check this runner never started, or one it can
    /// no longer wait on. A check this runner started stays its own after
    /// an error, so `adopt_check` answers `Known` for it and `kill_check`
    /// still stops it; only a previous runner's check can be adopted.
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
    /// `start_reviewer` under `confine`; the header and any refused
    /// writes go to `stderr`, so the findings on `stdout` stay clean.
    #[allow(clippy::too_many_arguments)]
    fn start_reviewer_confined(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
        confine: &Confine,
    ) -> Result<()>;
    /// Kill a running check or reviewer and its descendants, if this
    /// runner started it. A check already killed gets no second signal;
    /// `escalate_check` is the step-up.
    fn kill_check(&mut self, key: &str);
    /// After `kill_check` or an `adopt_check` that answered `Killed`:
    /// true once nothing of the check is left (its process group has no
    /// member) or this runner neither started nor adopted it (a restart
    /// lost it). False while the check or anything in its group still
    /// runs.
    fn check_gone(&mut self, key: &str) -> bool;
    /// SIGKILL to the group of a check `kill_check` stopped, sent once and
    /// only if the group still has a member; the group is forgotten after.
    fn escalate_check(&mut self, key: &str);
    /// The process group of a check or reviewer this runner started.
    fn check_group(&self, key: &str) -> Option<CheckGroup>;
    /// A check or command reviewer a previous runner started and left
    /// running, signalled by its recorded group if it is still ours.
    /// `vouched` says the caller holds a kill of this group by an
    /// earlier runner of this ticket younger than the stop limit, which
    /// lets a leaderless group be killed again: a pgid cannot be reused
    /// while any member lives, and that kill proved the group ours. The
    /// one hole is the group emptying, its id going to a new leader
    /// that exits and leaves members, all within the limit. A leader
    /// whose start time `ps` cannot read counts as gone here.
    fn adopt_check(&mut self, key: &str, group: &CheckGroup, vouched: bool) -> Adopted;
    /// Whether a group a previous runner started may still be running a
    /// command of ours; sends no signal. A gate-only command lost to a
    /// restart is waited for through this, never stopped.
    fn group_running(&self, key: &str, group: &CheckGroup) -> bool;
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
    /// The branch checked out at `dir` reset to `to` with `git reset
    /// --keep`: refused when `dir` has no branch checked out, when its
    /// head is not `expected_from`, or when git would lose a change in
    /// the tree. Local; nothing is fetched or pushed.
    fn reset_branch(&mut self, dir: &Path, to: &str, expected_from: &str) -> Result<()>;
    /// Whether `remote`'s copy of the branch checked out at `dir`, as
    /// last fetched or pushed, holds a commit of `base..head`. No fetch.
    fn published(&self, dir: &Path, remote: &str, base: &str, head: &str) -> Result<bool>;
    /// The size of `base..head` in `dir`: its commits, and the files and
    /// lines its diff changes. Read in Dispatch's clone, which outlives
    /// the ticket's trees.
    fn range_size(&self, dir: &Path, base: &str, head: &str) -> Result<RangeSize>;
    /// The names among `names` that neither the diff of `rev` against its
    /// parent nor the tree at `head` has, as `history::occurs` and
    /// `history::is_path` judge them. Read-only.
    fn absent(&self, dir: &Path, rev: &str, head: &str, names: &[String]) -> Result<Vec<String>>;
}

/// How big a range of commits is, as a report shows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RangeSize {
    /// Commits in `base..head`.
    pub commits: u32,
    /// Files the range's diff changes.
    pub files: u32,
    /// Lines the diff adds.
    pub insertions: u32,
    /// Lines the diff removes.
    pub deletions: u32,
}

/// `git diff --shortstat`'s line read as numbers: ` 3 files changed, 10
/// insertions(+), 2 deletions(-)`, any part of which may be absent.
fn parse_shortstat(line: &str) -> (u32, u32, u32) {
    let mut out = (0, 0, 0);
    for part in line.split(',') {
        let mut words = part.split_whitespace();
        let Some(n) = words.next().and_then(|n| n.parse().ok()) else {
            continue;
        };
        match words.next() {
            Some(w) if w.starts_with("file") => out.0 = n,
            Some(w) if w.starts_with("insertion") => out.1 = n,
            Some(w) if w.starts_with("deletion") => out.2 = n,
            _ => {}
        }
    }
    out
}

/// What `adopt_check` found under a recorded group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adopted {
    /// This runner already holds the key: nothing to adopt.
    Known,
    /// Nothing of ours is left under the id, or nothing there can be
    /// shown to be ours (its leader started at another time, or is gone
    /// or could not be read and the caller did not vouch for the
    /// group); the record's group can go.
    Gone,
    /// Ours and still running: TERM sent to the group, which
    /// `check_gone` and `escalate_check` now follow under `key`.
    Killed,
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
    /// Checks killed whose process group may still have members, by key:
    /// the group id, which is the killed child's pid.
    killed: std::collections::HashMap<String, u32>,
}

/// Whether process group `pgid` has a member. Signal 0 delivers nothing;
/// `kill` succeeds only if some process in the group could be signalled.
/// A command, not a syscall, since the crate forbids `unsafe`.
fn group_alive(pgid: u32) -> bool {
    Command::new("kill")
        .args(["-0", "--", &format!("-{pgid}")])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// When process `pid` started, as `ps` prints it with the locale and the
/// zone fixed, so every runner on the machine prints the same text for
/// the same process; `None` if there is no such process or `ps` could
/// not say.
fn leader_started(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

impl GitCli {
    /// Forget every killed group that is now empty, so an entry nothing
    /// reads back (a reviewer, a failed round's checks) does not outlive
    /// its group by more than one port call.
    fn sweep_killed(&mut self) {
        self.killed.retain(|_, pgid| group_alive(*pgid));
    }

    /// A check's spawn, confined or not: its output appended to `log`,
    /// the header first when it is confined, and `outer` in front of
    /// whatever confines it.
    #[allow(clippy::too_many_arguments)]
    fn spawn_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        confine: Option<&Confine>,
        outer: &[String],
    ) -> Result<()> {
        if argv.is_empty() {
            bail!("a check with no command");
        }
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .with_context(|| format!("open {}", log.display()))?;
        let argv = with_header(argv, confine, &mut out)?;
        let argv: Vec<String> = outer.iter().cloned().chain(argv).collect();
        let err = out.try_clone()?;
        self.spawn(key, dir, &argv, env, out, err)
    }

    /// A reviewer's spawn, confined or not: the header, when confined,
    /// goes to `stderr`.
    #[allow(clippy::too_many_arguments)]
    fn spawn_reviewer(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
        confine: Option<&Confine>,
    ) -> Result<()> {
        if argv.is_empty() {
            bail!("a reviewer with no command");
        }
        let out = std::fs::File::create(stdout)
            .with_context(|| format!("create {}", stdout.display()))?;
        let mut err = std::fs::File::create(stderr)
            .with_context(|| format!("create {}", stderr.display()))?;
        let argv = with_header(argv, confine, &mut err)?;
        self.spawn(key, dir, &argv, env, out, err)
    }

    /// `argv` in `dir` as the leader of its own process group, kept under
    /// `key` for polling.
    fn spawn(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        out: std::fs::File,
        err: std::fs::File,
    ) -> Result<()> {
        let (program, rest) = argv
            .split_first()
            .expect("spawn_check and spawn_reviewer refuse an empty argv");
        let mut cmd = child_command(program, env);
        cmd.args(rest)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .stdout(out)
            .stderr(err)
            .process_group(0);
        let child = cmd
            .spawn()
            .with_context(|| format!("start {program} in {}", dir.display()))?;
        self.checks.insert(key.to_owned(), child);
        Ok(())
    }
}

/// A child's `Command` for `program` with `env` set, without the
/// runner's launch token unless `env` gives it back: the runner's own
/// environment is inherited, and the token in it would let any command
/// resolve the runner's environment sets.
fn child_command(program: impl AsRef<std::ffi::OsStr>, env: &[(String, String)]) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_remove(RECORD_TOKEN_ENV);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// `argv` as it is spawned under `confine`, its header line written to
/// `to` first; `argv` unchanged when there is no `confine`.
fn with_header(
    argv: &[String],
    confine: Option<&Confine>,
    to: &mut std::fs::File,
) -> Result<Vec<String>> {
    use std::io::Write as _;
    let Some(confine) = confine else {
        return Ok(argv.to_vec());
    };
    let (wrapped, header) = crate::confine::wrap(argv, confine)?;
    writeln!(to, "{header}")?;
    Ok(wrapped)
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

    fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool> {
        // A nonzero exit means the ref is absent, not that git failed.
        let out = git_in(repo)
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .output()
            .with_context(|| format!("git in {}", repo.display()))?;
        Ok(out.status.success())
    }

    fn branch_ahead(&self, repo: &Path, branch: &str, start: &str) -> Result<u64> {
        let count = output(git_in(repo).args([
            "rev-list",
            "--count",
            &format!("{start}..refs/heads/{branch}"),
        ]))?;
        count
            .parse()
            .with_context(|| format!("rev-list --count printed {count:?}"))
    }

    fn delete_branch(&mut self, repo: &Path, branch: &str) -> Result<()> {
        output(git_in(repo).args(["branch", "-D", branch]))?;
        Ok(())
    }

    fn rename_branch(&mut self, repo: &Path, from: &str, to: &str) -> Result<()> {
        output(git_in(repo).args(["branch", "-m", from, to]))?;
        Ok(())
    }

    fn worktree_checkout(&mut self, repo: &Path, dir: &Path, branch: &str) -> Result<()> {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        output(git_in(repo).args(["worktree", "add"]).arg(dir).arg(branch))?;
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

    fn rebase_in_progress(&self, dir: &Path) -> Result<bool> {
        // `--git-path` answers relative to `dir` in a main checkout and
        // absolute in a linked worktree; `join` takes both.
        for state in ["rebase-merge", "rebase-apply"] {
            let path = output(git_in(dir).args(["rev-parse", "--git-path", state]))?;
            if dir.join(path.trim()).exists() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn branch_head(&self, dir: &Path, branch: &str) -> Result<Option<String>> {
        let out = git_in(dir)
            .args(["symbolic-ref", "-q", "HEAD"])
            .output()
            .with_context(|| format!("git symbolic-ref in {}", dir.display()))?;
        match out.status.code() {
            Some(0) => {}
            // Detached.
            Some(1) => return Ok(None),
            _ => bail!(
                "git symbolic-ref in {} exited {}: {}",
                dir.display(),
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        }
        let full = format!("refs/heads/{branch}");
        if String::from_utf8_lossy(&out.stdout).trim() != full {
            return Ok(None);
        }
        output(git_in(dir).args(["rev-parse", &full])).map(Some)
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
        if self.rebase_in_progress(dir)? {
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

    fn conflicting_commits(
        &self,
        dir: &Path,
        base: &str,
        head: &str,
        onto: &str,
    ) -> Result<Vec<String>> {
        let picks =
            output(git_in(dir).args(["rev-list", "--reverse", &format!("{base}..{head}")]))?;
        // A fixed identity, so a clone with no `user.name` can still
        // write the throwaway commits; no ref ever names them.
        let env = [
            ("GIT_AUTHOR_NAME", "dispatch"),
            ("GIT_AUTHOR_EMAIL", "dispatch@localhost"),
            ("GIT_COMMITTER_NAME", "dispatch"),
            ("GIT_COMMITTER_EMAIL", "dispatch@localhost"),
        ];
        let mut tip = onto.to_owned();
        let mut conflicting = Vec::new();
        for pick in picks.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let mut cmd = git_in(dir);
            cmd.args(["merge-tree", "--write-tree", "--merge-base"])
                .arg(format!("{pick}^"))
                .args([tip.as_str(), pick]);
            let out = cmd.output().with_context(|| format!("run {cmd:?}"))?;
            match out.status.code() {
                Some(0) => {}
                Some(1) => conflicting.push(pick.to_owned()),
                _ => bail!(
                    "{cmd:?} exited {}: {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            }
            // Written even with conflict markers in it.
            let stdout = String::from_utf8_lossy(&out.stdout);
            let tree = stdout
                .lines()
                .next()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .with_context(|| format!("{cmd:?} printed no tree"))?
                .to_owned();
            tip = commit_tree(dir, &tree, &tip, "dispatch: conflict probe", &env)?;
        }
        Ok(conflicting)
    }

    fn conflicting_files(&self, dir: &Path, head: &str, onto: &str) -> Result<Vec<String>> {
        let mut cmd = git_in(dir);
        cmd.args([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            onto,
            head,
        ]);
        let out = cmd.output().with_context(|| format!("run {cmd:?}"))?;
        match out.status.code() {
            Some(0) => Ok(Vec::new()),
            // The tree id, then one conflicted path per line up to the
            // blank line that would open the messages.
            Some(1) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let mut files: Vec<String> = Vec::new();
                for line in stdout.lines().skip(1) {
                    let line = line.trim();
                    if line.is_empty() {
                        break;
                    }
                    if !files.iter().any(|f| f == line) {
                        files.push(line.to_owned());
                    }
                }
                Ok(files)
            }
            _ => bail!(
                "{cmd:?} exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        }
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

    fn port_free(&self, port: u16) -> bool {
        use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};
        // std binds with SO_REUSEADDR, under which a specific address
        // binds beside a wildcard listener; each family's wildcard and
        // loopback are tried, which together see every shape a dev
        // server binds in (spike 11).
        let v4 = [Ipv4Addr::UNSPECIFIED, Ipv4Addr::LOCALHOST]
            .iter()
            .all(|ip| TcpListener::bind((*ip, port)).is_ok());
        let v6_up = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).is_ok();
        v4 && (!v6_up || TcpListener::bind((Ipv6Addr::LOCALHOST, port)).is_ok())
    }

    fn answers_http(&self, port: u16, path: &str) -> bool {
        use std::io::{Read as _, Write as _};
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
        use std::time::{Duration, Instant};
        let hosts = [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ];
        let Some(mut stream) = hosts.iter().find_map(|ip| {
            TcpStream::connect_timeout(&SocketAddr::new(*ip, port), Duration::from_millis(200)).ok()
        }) else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_millis(500);
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        let request = format!("GET {path} HTTP/1.0\r\nHost: localhost:{port}\r\n\r\n");
        if stream.write_all(request.as_bytes()).is_err() {
            return false;
        }
        let mut got = Vec::new();
        let mut buf = [0u8; 64];
        while got.len() < 5 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
                return false;
            }
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return false,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
        got.starts_with(b"HTTP/")
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
        let mut cmd = child_command(program, env);
        cmd.args(rest).current_dir(dir);
        output(&mut cmd)?;
        Ok(())
    }

    fn run_confined(
        &mut self,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        confine: &Confine,
    ) -> Result<String> {
        let Some((program, _)) = argv.split_first() else {
            return Ok(crate::confine::header(confine));
        };
        let (wrapped, header) = crate::confine::wrap(argv, confine)?;
        let mut cmd = child_command(&wrapped[0], env);
        cmd.args(&wrapped[1..]).current_dir(dir);
        let out = cmd
            .output()
            .with_context(|| format!("{header}: run {program}"))?;
        // Built from the status and stderr alone: the wrapped command's
        // own text is the reporting script and the profile, which would
        // bury the stderr and its deny lines.
        if !out.status.success() {
            bail!(
                "{header}: {argv:?} exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(header)
    }

    fn start_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        outer: &[String],
    ) -> Result<()> {
        self.spawn_check(key, dir, argv, env, log, None, outer)
    }

    fn start_check_confined(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        confine: &Confine,
        outer: &[String],
    ) -> Result<()> {
        self.spawn_check(key, dir, argv, env, log, Some(confine), outer)
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
        self.spawn_reviewer(key, dir, argv, env, stdout, stderr, None)
    }

    fn start_reviewer_confined(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
        confine: &Confine,
    ) -> Result<()> {
        self.spawn_reviewer(key, dir, argv, env, stdout, stderr, Some(confine))
    }

    fn kill_check(&mut self, key: &str) {
        self.sweep_killed();
        if let Some(mut child) = self.checks.remove(key) {
            // Every check and reviewer leads its own group, so the TERM
            // reaches its descendants; the leader itself is killed and
            // reaped outright so no zombie keeps its pid.
            let pgid = child.id();
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{pgid}")])
                .output();
            let _ = child.kill();
            let _ = child.wait();
            if group_alive(pgid) {
                self.killed.insert(key.to_owned(), pgid);
            }
        }
    }

    fn check_gone(&mut self, key: &str) -> bool {
        self.sweep_killed();
        if let Some(child) = self.checks.get_mut(key) {
            if matches!(child.try_wait(), Ok(None)) {
                return false;
            }
            // Exited, but what it started may still run in its group.
            let pgid = child.id();
            self.checks.remove(key);
            if !group_alive(pgid) {
                return true;
            }
            self.killed.insert(key.to_owned(), pgid);
        }
        match self.killed.get(key) {
            Some(&pgid) if group_alive(pgid) => false,
            Some(_) => {
                self.killed.remove(key);
                true
            }
            None => true,
        }
    }

    fn escalate_check(&mut self, key: &str) {
        self.sweep_killed();
        // Probed just before the signal: a group id with a live member
        // cannot have been handed to anyone else. A group this runner
        // did not start is here either because `adopt_check` found its
        // leader alive with the recorded start time, or because it was
        // leaderless and a fresh `orphans_killed` entry vouched for it;
        // in both cases `group_alive`, probed just before the SIGKILL,
        // shows a member still holds the id, so it has not been handed
        // on since.
        if let Some(pgid) = self.killed.remove(key)
            && group_alive(pgid)
        {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{pgid}")])
                .output();
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
            // The child is kept: it is still this runner's to kill.
            Err(e) => Some(Err(anyhow::anyhow!("waiting on the check: {e}"))),
        }
    }

    fn check_group(&self, key: &str) -> Option<CheckGroup> {
        let pgid = self.checks.get(key)?.id();
        Some(CheckGroup {
            pgid,
            leader_started: leader_started(pgid)?,
        })
    }

    fn adopt_check(&mut self, key: &str, group: &CheckGroup, vouched: bool) -> Adopted {
        self.sweep_killed();
        if self.checks.contains_key(key) || self.killed.contains_key(key) {
            return Adopted::Known;
        }
        if !group_alive(group.pgid) {
            return Adopted::Gone;
        }
        // A live leader with the recorded start time proves the group
        // ours. A leader with another start time proves the pid reused,
        // vouched or not. A gone or unreadable leader proves nothing by
        // itself, so the group is left alone unless the caller vouches
        // for it with a kill of its own younger than the stop limit,
        // which a pgid cannot outlive while a member lives.
        let started = leader_started(group.pgid);
        if started.is_none() && vouched {
            log::info!(
                "{key}: group {}: leader gone, vouched for by this ticket's kill within the stop limit; killed again",
                group.pgid
            );
        } else if started.as_deref() != Some(group.leader_started.as_str()) {
            log::warn!(
                "{key}: group {} is led by {}, not a process started {}; left alone",
                group.pgid,
                started.map_or_else(
                    || "no process whose start time could be read".to_owned(),
                    |s| format!("a process started {s}")
                ),
                group.leader_started
            );
            return Adopted::Gone;
        }
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", group.pgid)])
            .output();
        self.killed.insert(key.to_owned(), group.pgid);
        Adopted::Killed
    }

    fn group_running(&self, _key: &str, group: &CheckGroup) -> bool {
        if !group_alive(group.pgid) {
            return false;
        }
        // The reverse of `adopt_check`'s rule for a leader whose start
        // time cannot be read. There a wrong guess signals someone else's
        // group; here the only action is waiting. A deploy shell's leader
        // can exit while its children keep applying, and a group id is
        // not reused while the group has a member, so a live group with
        // no readable leader is taken as still ours. A wrong guess costs
        // a wait, which the `stuck` question bounds. Only a leader that
        // started at another time proves the id reused.
        leader_started(group.pgid).is_none_or(|s| s == group.leader_started)
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

    fn reset_branch(&mut self, dir: &Path, to: &str, expected_from: &str) -> Result<()> {
        // On a detached head the reset would move HEAD alone and leave
        // the branch where it was.
        output(git_in(dir).args(["symbolic-ref", "-q", "HEAD"]))
            .with_context(|| format!("{} has no branch checked out", dir.display()))?;
        let head = output(git_in(dir).args(["rev-parse", "HEAD"]))?;
        if head != expected_from {
            bail!("{} is at {head}, not {expected_from}", dir.display());
        }
        output(git_in(dir).args(["reset", "-q", "--keep", to]))?;
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

    fn range_size(&self, dir: &Path, base: &str, head: &str) -> Result<RangeSize> {
        let commits =
            output(git_in(dir).args(["rev-list", "--count", &format!("{base}..{head}")]))?
                .parse()
                .context("a commit count")?;
        let stat = output(git_in(dir).args(["diff", "--shortstat", base, head]))?;
        let (files, insertions, deletions) = parse_shortstat(&stat);
        Ok(RangeSize {
            commits,
            files,
            insertions,
            deletions,
        })
    }

    fn absent(&self, dir: &Path, rev: &str, head: &str, names: &[String]) -> Result<Vec<String>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let diff = output(git_in(dir).args([
            "diff",
            "-U0",
            "--no-color",
            "--no-ext-diff",
            &format!("{rev}^"),
            rev,
        ]))?;
        let listing = output(git_in(dir).args(["ls-tree", "-r", "--name-only", "-z", head]))?;
        let paths: Vec<&str> = listing.split('\0').filter(|p| !p.is_empty()).collect();
        let mut missing = Vec::new();
        for name in names {
            let path = history::is_path(name, &paths);
            if history::occurs(&diff, name, path) {
                continue;
            }
            let suffix = format!("/{name}");
            if path && paths.iter().any(|p| *p == name || p.ends_with(&suffix)) {
                continue;
            }
            if !grep_occurs(dir, head, name, path)? {
                missing.push(name.clone());
            }
        }
        Ok(missing)
    }
}

/// Whether `name` occurs, as `history::occurs` judges it, on a line of
/// the tree at `head`. `-e` keeps a name like `--flag` from being read
/// as an option; exit 1 is no line.
fn grep_occurs(dir: &Path, head: &str, name: &str, path: bool) -> Result<bool> {
    let mut cmd = git_in(dir);
    cmd.args(["grep", "-F", "-I", "-h", "--no-color", "-e", name]);
    if let Some(last) = history::segment(name, path) {
        cmd.args(["-e", last]);
    }
    cmd.args([head, "--"]);
    let out = cmd.output().with_context(|| format!("run {cmd:?}"))?;
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| history::occurs(l, name, path))),
        Some(1) => Ok(false),
        _ => bail!(
            "{cmd:?} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
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
    for lane in lanes {
        if !lane.exists() {
            continue;
        }
        found.extend(git.changes(lane)?.into_iter().map(|c| lane.join(c)));
    }
    if let Some(tree) = tree
        && tree.exists()
    {
        found.extend(tree_changes(git, tree, lanes)?);
    }
    Ok(found)
}

/// Every change in `tree` that is not one of the worktrees in `lanes`
/// or inside one, as absolute paths. What is inside a nested lane is
/// that lane's repository's business, not the tree's.
pub fn tree_changes(git: &dyn Repo, tree: &Path, lanes: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let nested: Vec<&Path> = lanes
        .iter()
        .filter_map(|lane| lane.strip_prefix(tree).ok())
        .collect();
    Ok(git
        .changes(tree)?
        .into_iter()
        .filter(|c| !nested.iter().any(|lane| c.starts_with(lane)))
        .map(|c| tree.join(c))
        .collect())
}

/// A check the fake was asked to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedCheck {
    pub key: String,
    pub dir: PathBuf,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub log: PathBuf,
    /// What it was confined to, `None` when started unconfined.
    pub confine: Option<Confine>,
    /// What it was wrapped in outside any confinement
    /// (`switchboard-env exec --`); empty for none and for reviewers.
    pub outer: Vec<String>,
}

/// A point a fake call stops at until the test lets it go, so a test
/// can act while a slow call (a tree removal) is in progress. Both
/// waits panic after ten seconds: a test that never reaches its release
/// fails instead of passing late.
#[derive(Debug, Default)]
pub struct Gate {
    state: std::sync::Mutex<GateState>,
    changed: std::sync::Condvar,
}

#[derive(Debug, Default)]
struct GateState {
    entered: bool,
    released: bool,
}

impl Gate {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

    /// The call side: mark the gate entered, then wait for `release`.
    pub fn pass(&self) {
        let mut state = self.state.lock().unwrap();
        state.entered = true;
        self.changed.notify_all();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Self::LIMIT, |s| !s.released)
            .unwrap();
        drop(state);
        assert!(!timeout.timed_out(), "a gate was never released");
    }

    /// The test side: wait until a call is stopped at the gate.
    pub fn wait_entered(&self) {
        let state = self.state.lock().unwrap();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Self::LIMIT, |s| !s.entered)
            .unwrap();
        drop(state);
        assert!(!timeout.timed_out(), "no call reached the gate");
    }

    /// Let every call at the gate, and every later one, through.
    pub fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.changed.notify_all();
    }
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
    /// Trees with a rebase stopped part way; their `HEAD` is detached,
    /// as git leaves it.
    pub mid_rebase: Vec<PathBuf>,
    /// Trees whose `HEAD` is detached off their branch.
    pub detached: Vec<PathBuf>,
    pub ran: Vec<(PathBuf, Vec<String>)>,
    /// Commands run confined: dir, argv, what they were confined to.
    pub ran_confined: Vec<(PathBuf, Vec<String>, Confine)>,
    /// The next `run` or `run_confined` fails with this, once.
    pub fail_run: Option<String>,
    pub fail_worktree: Option<String>,
    /// Every `fetch` fails with this while it is set.
    pub fail_fetch: Option<String>,
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
    /// Checks and reviewers killed by key, one entry per call.
    pub killed_checks: Vec<String>,
    /// Checks whose kill is recorded but which keep running, as a check
    /// that ignores TERM would; only `escalate_check` removes them. Once
    /// killed, `poll_check` reads them as lost, as `GitCli` does a key
    /// its kill took out of its children.
    pub stubborn_checks: Vec<String>,
    /// Checks `escalate_check` was called on, by key.
    pub escalated_checks: Vec<String>,
    /// Keys of check groups a previous runner left running: `adopt_check`
    /// kills them and `check_gone` is false until a test removes them or
    /// `escalate_check` does.
    pub orphans: std::collections::BTreeSet<String>,
    /// Keys `adopt_check` answered `Killed` for, one entry per answer.
    pub adopted: Vec<String>,
    /// Orphan keys whose leader is gone: `adopt_check` kills them only
    /// when vouched for, and otherwise leaves them in `left_alone`.
    pub leaderless: std::collections::BTreeSet<String>,
    /// Keys `adopt_check` answered `Killed` for under a vouch, one entry
    /// per answer.
    pub vouched: Vec<String>,
    /// Leaderless orphans `adopt_check` left alone: their members run
    /// on, and `check_gone` reads them as no longer this runner's.
    pub left_alone: std::collections::BTreeSet<String>,
    /// The group id the next check or reviewer gets; 0 reads as 4000. A
    /// test that wants a pid reused sets it back.
    pub next_pgid: u32,
    /// The group of each check or reviewer started, by key, as it
    /// started.
    pub groups: std::collections::BTreeMap<String, CheckGroup>,
    /// Checks and reviewers started, counted, so two groups under one
    /// group id still have different leader start times.
    pub starts: u32,
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
    /// What `conflicting_commits` lists for a tree; absent, nothing.
    pub conflicting: std::collections::BTreeMap<PathBuf, Vec<String>>,
    /// What `conflicting_files` lists for a tree; absent, nothing.
    pub conflicting_files: std::collections::BTreeMap<PathBuf, Vec<String>>,
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
    /// Branches reset by a restart: dir, to, from. A refused reset is
    /// not recorded.
    pub resets: Vec<(PathBuf, String, String)>,
    /// The next `reset_branch` fails with this, once, as git refusing.
    pub fail_reset: Option<String>,
    /// Branches the remote already holds: dir, base, head. Asked of any
    /// other range, a tree is not published.
    pub published: Vec<(PathBuf, String, String)>,
    /// Local branches per clone: (repo, branch) -> (head, commits beyond
    /// its start). `worktree_add` makes one at `("base0000", 0)`; a test
    /// moves one by editing it.
    pub branches: std::collections::BTreeMap<(PathBuf, String), (String, u64)>,
    /// Branches deleted: repo, branch.
    pub deleted_branches: Vec<(PathBuf, String)>,
    /// Branches renamed: repo, from, to.
    pub renamed_branches: Vec<(PathBuf, String, String)>,
    /// What `merge_base` answers in a clone, read before `bases`: a
    /// reused branch's fork point, which a test sets apart from the
    /// `start` that `rev_parse` resolves.
    pub fork_points: std::collections::BTreeMap<PathBuf, String>,
    /// `branch_ahead` fails with this, as a missing `<remote>/<base>`
    /// makes git fail.
    pub fail_ahead: Option<String>,
    /// What `range_size` answers by `(dir, base, head)`; any other range
    /// is an error, as git's would be for a clone without the commits.
    pub ranges: std::collections::BTreeMap<(PathBuf, String, String), RangeSize>,
    /// The names a tree lacks, whatever the rev and head; any other
    /// name is present.
    pub absent_names: std::collections::BTreeMap<PathBuf, Vec<String>>,
    /// Gates by method name; only `worktree_remove` honours one, and
    /// only through the shared `Arc<Mutex<FakeRepo>>`.
    pub gates: std::collections::BTreeMap<&'static str, std::sync::Arc<Gate>>,
    /// Ports something else holds: `port_free` refuses them.
    pub busy_ports: std::collections::BTreeSet<u16>,
    /// Ports where a server answers HTTP.
    pub answering: std::collections::BTreeSet<u16>,
}

impl FakeRepo {
    #[allow(clippy::too_many_arguments)]
    fn record_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        confine: Option<Confine>,
        outer: &[String],
    ) -> Result<()> {
        std::fs::write(log, "checks ran\n")?;
        self.take_pgid(key);
        self.checks.push(StartedCheck {
            key: key.to_owned(),
            dir: dir.to_path_buf(),
            argv: argv.to_vec(),
            env: env.to_vec(),
            log: log.to_path_buf(),
            confine,
            outer: outer.to_vec(),
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn record_reviewer(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
        confine: Option<Confine>,
    ) -> Result<()> {
        if !stdout.exists() {
            std::fs::write(stdout, "")?;
        }
        std::fs::write(stderr, "")?;
        self.take_pgid(key);
        let started = StartedCheck {
            key: key.to_owned(),
            dir: dir.to_path_buf(),
            argv: argv.to_vec(),
            env: env.to_vec(),
            log: stdout.to_path_buf(),
            confine,
            outer: Vec::new(),
        };
        self.checks.push(started.clone());
        self.reviewers.push((started, stderr.to_path_buf()));
        Ok(())
    }

    /// One more start: the next group id and a start time of its own,
    /// recorded under `key`.
    fn take_pgid(&mut self, key: &str) {
        if self.next_pgid == 0 {
            self.next_pgid = 4000;
        }
        self.starts += 1;
        self.groups.insert(
            key.to_owned(),
            CheckGroup {
                pgid: self.next_pgid,
                leader_started: format!("fake-{}", self.starts),
            },
        );
        self.next_pgid += 1;
    }
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
        if let Some(e) = &self.fail_fetch {
            bail!("{e}");
        }
        self.fetched.push((dir.to_path_buf(), remote.to_owned()));
        Ok(())
    }
    fn worktree_add(&mut self, repo: &Path, dir: &Path, branch: &str, base: &str) -> Result<()> {
        if let Some(e) = &self.fail_worktree {
            bail!("{e}");
        }
        let key = (repo.to_path_buf(), branch.to_owned());
        if self.branches.contains_key(&key) {
            bail!("fatal: a branch named '{branch}' already exists");
        }
        self.branches.insert(key, ("base0000".into(), 0));
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
    fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool> {
        Ok(self
            .branches
            .contains_key(&(repo.to_path_buf(), branch.to_owned())))
    }
    fn branch_ahead(&self, repo: &Path, branch: &str, _start: &str) -> Result<u64> {
        if let Some(e) = &self.fail_ahead {
            bail!("{e}");
        }
        Ok(self
            .branches
            .get(&(repo.to_path_buf(), branch.to_owned()))
            .map_or(0, |(_, n)| *n))
    }
    fn delete_branch(&mut self, repo: &Path, branch: &str) -> Result<()> {
        if let Some((_, dir, ..)) = self
            .worktrees
            .iter()
            .find(|(r, _, b, _)| r == repo && b == branch)
        {
            bail!(
                "error: cannot delete branch '{branch}' used by worktree at '{}'",
                dir.display()
            );
        }
        self.branches
            .remove(&(repo.to_path_buf(), branch.to_owned()))
            .with_context(|| format!("error: branch '{branch}' not found"))?;
        self.deleted_branches
            .push((repo.to_path_buf(), branch.to_owned()));
        Ok(())
    }
    fn rename_branch(&mut self, repo: &Path, from: &str, to: &str) -> Result<()> {
        let to_key = (repo.to_path_buf(), to.to_owned());
        if self.branches.contains_key(&to_key) {
            bail!("fatal: a branch named '{to}' already exists");
        }
        let entry = self
            .branches
            .remove(&(repo.to_path_buf(), from.to_owned()))
            .with_context(|| format!("error: refname refs/heads/{from} not found"))?;
        self.branches.insert(to_key, entry);
        self.renamed_branches
            .push((repo.to_path_buf(), from.to_owned(), to.to_owned()));
        Ok(())
    }
    fn worktree_checkout(&mut self, repo: &Path, dir: &Path, branch: &str) -> Result<()> {
        if let Some(e) = &self.fail_worktree {
            bail!("{e}");
        }
        let head = self
            .branches
            .get(&(repo.to_path_buf(), branch.to_owned()))
            .map(|(head, _)| head.clone())
            .with_context(|| format!("fatal: invalid reference: {branch}"))?;
        if let Some((_, other, ..)) = self
            .worktrees
            .iter()
            .find(|(r, _, b, _)| r == repo && b == branch)
        {
            bail!(
                "fatal: '{branch}' is already used by worktree at '{}'",
                other.display()
            );
        }
        std::fs::create_dir_all(dir)?;
        self.worktrees.push((
            repo.to_path_buf(),
            dir.to_path_buf(),
            branch.into(),
            branch.into(),
        ));
        self.heads.insert(dir.to_path_buf(), head);
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
    fn rebase_in_progress(&self, dir: &Path) -> Result<bool> {
        Ok(self.mid_rebase.iter().any(|d| d == dir))
    }
    fn branch_head(&self, dir: &Path, _branch: &str) -> Result<Option<String>> {
        if self
            .mid_rebase
            .iter()
            .chain(&self.detached)
            .any(|d| d == dir)
        {
            return Ok(None);
        }
        self.head(dir).map(Some)
    }

    fn free_bytes(&self, _dir: &Path) -> Result<u64> {
        Ok(self.free_bytes.unwrap_or(u64::MAX))
    }
    fn port_free(&self, port: u16) -> bool {
        !self.busy_ports.contains(&port)
    }
    fn answers_http(&self, port: u16, _path: &str) -> bool {
        self.answering.contains(&port)
    }
    fn behind(&self, dir: &Path, _onto: &str) -> Result<u64> {
        Ok(self.behind.get(dir).copied().unwrap_or(0))
    }
    fn rebase_onto(&mut self, dir: &Path, onto: &str) -> Result<bool> {
        if self.mid_rebase.iter().any(|d| d == dir) {
            bail!("a rebase is already in progress at {}", dir.display());
        }
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
    fn conflicting_commits(
        &self,
        dir: &Path,
        _base: &str,
        _head: &str,
        _onto: &str,
    ) -> Result<Vec<String>> {
        Ok(self.conflicting.get(dir).cloned().unwrap_or_default())
    }
    fn conflicting_files(&self, dir: &Path, _head: &str, _onto: &str) -> Result<Vec<String>> {
        Ok(self.conflicting_files.get(dir).cloned().unwrap_or_default())
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
        if let Some(why) = self.fail_run.take() {
            bail!("{why}");
        }
        Ok(())
    }
    fn run_confined(
        &mut self,
        dir: &Path,
        argv: &[String],
        _env: &[(String, String)],
        confine: &Confine,
    ) -> Result<String> {
        self.ran_confined
            .push((dir.to_path_buf(), argv.to_vec(), confine.clone()));
        let header = crate::confine::header(confine);
        if let Some(why) = self.fail_run.take() {
            return Err(anyhow::anyhow!("{why}").context(header));
        }
        Ok(header)
    }
    fn start_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        outer: &[String],
    ) -> Result<()> {
        self.record_check(key, dir, argv, env, log, None, outer)
    }
    fn start_check_confined(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        confine: &Confine,
        outer: &[String],
    ) -> Result<()> {
        self.record_check(key, dir, argv, env, log, Some(confine.clone()), outer)
    }
    fn start_reviewer_confined(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
        confine: &Confine,
    ) -> Result<()> {
        self.record_reviewer(key, dir, argv, env, stdout, stderr, Some(confine.clone()))
    }
    fn poll_check(&mut self, key: &str) -> Option<Result<i32>> {
        if let Some(code) = self.check_exits.get(key) {
            return Some(Ok(*code));
        }
        let killed = self.stubborn_checks.iter().any(|k| k == key)
            && self.killed_checks.iter().any(|k| k == key);
        if !killed && self.checks.iter().any(|c| c.key == key) {
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
        self.record_reviewer(key, dir, argv, env, stdout, stderr, None)
    }
    fn kill_check(&mut self, key: &str) {
        if !self.stubborn_checks.iter().any(|k| k == key) {
            self.checks.retain(|c| c.key != key);
        }
        self.killed_checks.push(key.to_owned());
    }
    fn check_gone(&mut self, key: &str) -> bool {
        (!self.orphans.contains(key) || self.left_alone.contains(key))
            && (!self.checks.iter().any(|c| c.key == key) || self.check_exits.contains_key(key))
    }
    fn escalate_check(&mut self, key: &str) {
        self.checks.retain(|c| c.key != key);
        self.orphans.remove(key);
        self.escalated_checks.push(key.to_owned());
    }
    fn check_group(&self, key: &str) -> Option<CheckGroup> {
        if !self.checks.iter().any(|c| c.key == key) {
            return None;
        }
        self.groups.get(key).cloned()
    }
    fn adopt_check(&mut self, key: &str, _group: &CheckGroup, vouched: bool) -> Adopted {
        if self.checks.iter().any(|c| c.key == key) {
            Adopted::Known
        } else if !self.orphans.contains(key) {
            Adopted::Gone
        } else if self.leaderless.contains(key) && !vouched {
            self.left_alone.insert(key.to_owned());
            Adopted::Gone
        } else {
            self.left_alone.remove(key);
            self.adopted.push(key.to_owned());
            if vouched {
                self.vouched.push(key.to_owned());
            }
            Adopted::Killed
        }
    }
    fn group_running(&self, key: &str, _group: &CheckGroup) -> bool {
        self.orphans.contains(key)
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
        if let Some(fork) = self.fork_points.get(dir) {
            return Ok(fork.clone());
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
    fn reset_branch(&mut self, dir: &Path, to: &str, expected_from: &str) -> Result<()> {
        if self
            .mid_rebase
            .iter()
            .chain(&self.detached)
            .any(|d| d == dir)
        {
            bail!("{} has no branch checked out", dir.display());
        }
        if self.dirty.iter().any(|d| d == dir) {
            bail!("{} has local changes the reset would lose", dir.display());
        }
        let head = self.head(dir)?;
        if head != expected_from {
            bail!("{} is at {head}, not {expected_from}", dir.display());
        }
        if let Some(why) = self.fail_reset.take() {
            bail!("{why}");
        }
        self.heads.insert(dir.to_path_buf(), to.to_owned());
        self.resets
            .push((dir.to_path_buf(), to.to_owned(), expected_from.to_owned()));
        Ok(())
    }
    fn published(&self, dir: &Path, _remote: &str, base: &str, head: &str) -> Result<bool> {
        Ok(self
            .published
            .iter()
            .any(|(d, b, h)| d == dir && b == base && h == head))
    }
    fn range_size(&self, dir: &Path, base: &str, head: &str) -> Result<RangeSize> {
        self.ranges
            .get(&(dir.to_path_buf(), base.to_owned(), head.to_owned()))
            .copied()
            .with_context(|| format!("no range {base}..{head} in {}", dir.display()))
    }
    fn absent(&self, dir: &Path, _rev: &str, _head: &str, names: &[String]) -> Result<Vec<String>> {
        let lacks = self.absent_names.get(dir).cloned().unwrap_or_default();
        Ok(names
            .iter()
            .filter(|n| lacks.contains(n))
            .cloned()
            .collect())
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
    fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool> {
        self.lock().unwrap().branch_exists(repo, branch)
    }
    fn branch_ahead(&self, repo: &Path, branch: &str, start: &str) -> Result<u64> {
        self.lock().unwrap().branch_ahead(repo, branch, start)
    }
    fn delete_branch(&mut self, repo: &Path, branch: &str) -> Result<()> {
        self.lock().unwrap().delete_branch(repo, branch)
    }
    fn rename_branch(&mut self, repo: &Path, from: &str, to: &str) -> Result<()> {
        self.lock().unwrap().rename_branch(repo, from, to)
    }
    fn worktree_checkout(&mut self, repo: &Path, dir: &Path, branch: &str) -> Result<()> {
        self.lock().unwrap().worktree_checkout(repo, dir, branch)
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
    fn rebase_in_progress(&self, dir: &Path) -> Result<bool> {
        self.lock().unwrap().rebase_in_progress(dir)
    }
    fn branch_head(&self, dir: &Path, branch: &str) -> Result<Option<String>> {
        self.lock().unwrap().branch_head(dir, branch)
    }
    fn behind(&self, dir: &Path, onto: &str) -> Result<u64> {
        self.lock().unwrap().behind(dir, onto)
    }
    fn rebase_onto(&mut self, dir: &Path, onto: &str) -> Result<bool> {
        self.lock().unwrap().rebase_onto(dir, onto)
    }
    fn conflicting_commits(
        &self,
        dir: &Path,
        base: &str,
        head: &str,
        onto: &str,
    ) -> Result<Vec<String>> {
        self.lock()
            .unwrap()
            .conflicting_commits(dir, base, head, onto)
    }
    fn conflicting_files(&self, dir: &Path, head: &str, onto: &str) -> Result<Vec<String>> {
        self.lock().unwrap().conflicting_files(dir, head, onto)
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
    fn port_free(&self, port: u16) -> bool {
        self.lock().unwrap().port_free(port)
    }
    fn answers_http(&self, port: u16, path: &str) -> bool {
        self.lock().unwrap().answers_http(port, path)
    }
    fn worktree_move(&mut self, repo: &Path, from: &Path, to: &Path) -> Result<()> {
        self.lock().unwrap().worktree_move(repo, from, to)
    }
    fn worktree_repair(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        self.lock().unwrap().worktree_repair(repo, dir)
    }
    fn worktree_remove(&mut self, repo: &Path, dir: &Path) -> Result<()> {
        // The guard is dropped before the gate is passed: a gate that
        // blocked holding the mutex would freeze the test's own
        // `repo.lock()` while it acts in the gap.
        let gate = self.lock().unwrap().gates.get("worktree_remove").cloned();
        if let Some(gate) = gate {
            gate.pass();
        }
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
    fn run_confined(
        &mut self,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        confine: &Confine,
    ) -> Result<String> {
        self.lock().unwrap().run_confined(dir, argv, env, confine)
    }
    fn start_check_confined(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        confine: &Confine,
        outer: &[String],
    ) -> Result<()> {
        self.lock()
            .unwrap()
            .start_check_confined(key, dir, argv, env, log, confine, outer)
    }
    fn start_reviewer_confined(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        stdout: &Path,
        stderr: &Path,
        confine: &Confine,
    ) -> Result<()> {
        self.lock()
            .unwrap()
            .start_reviewer_confined(key, dir, argv, env, stdout, stderr, confine)
    }
    fn start_check(
        &mut self,
        key: &str,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        log: &Path,
        outer: &[String],
    ) -> Result<()> {
        self.lock()
            .unwrap()
            .start_check(key, dir, argv, env, log, outer)
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
    fn check_gone(&mut self, key: &str) -> bool {
        self.lock().unwrap().check_gone(key)
    }
    fn escalate_check(&mut self, key: &str) {
        self.lock().unwrap().escalate_check(key);
    }
    fn check_group(&self, key: &str) -> Option<CheckGroup> {
        self.lock().unwrap().check_group(key)
    }
    fn adopt_check(&mut self, key: &str, group: &CheckGroup, vouched: bool) -> Adopted {
        self.lock().unwrap().adopt_check(key, group, vouched)
    }
    fn group_running(&self, key: &str, group: &CheckGroup) -> bool {
        self.lock().unwrap().group_running(key, group)
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
    fn reset_branch(&mut self, dir: &Path, to: &str, expected_from: &str) -> Result<()> {
        self.lock().unwrap().reset_branch(dir, to, expected_from)
    }
    fn published(&self, dir: &Path, remote: &str, base: &str, head: &str) -> Result<bool> {
        self.lock().unwrap().published(dir, remote, base, head)
    }
    fn range_size(&self, dir: &Path, base: &str, head: &str) -> Result<RangeSize> {
        self.lock().unwrap().range_size(dir, base, head)
    }
    fn absent(&self, dir: &Path, rev: &str, head: &str, names: &[String]) -> Result<Vec<String>> {
        self.lock().unwrap().absent(dir, rev, head, names)
    }
}

/// `yyyymmdd` of `ms` since the epoch, in UTC: the suffix a branch
/// renamed out of a retake's way carries.
#[must_use]
pub fn yyyymmdd(ms: u64) -> String {
    // Hinnant's `civil_from_days`, for days since 1970-01-01 (never
    // negative here, since `ms` is unsigned).
    let z = ms / 86_400_000 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}")
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
    fn a_child_never_inherits_the_record_token_unless_its_env_gives_it() {
        // Read from the `Command`, not the process environment, which
        // other tests share.
        let bare = child_command("sh", &[]);
        assert!(
            bare.get_envs()
                .any(|(k, v)| k == RECORD_TOKEN_ENV && v.is_none()),
            "removed from a child without it in env"
        );
        let given = child_command("sh", &[(RECORD_TOKEN_ENV.into(), "t0k".into())]);
        assert!(
            given
                .get_envs()
                .any(|(k, v)| k == RECORD_TOKEN_ENV && v == Some("t0k".as_ref()))
        );
        let dir = tempfile::tempdir().unwrap();
        let echo: Vec<String> = vec![
            "sh".into(),
            "-c".into(),
            "echo ${SWITCHBOARD_RECORD_TOKEN-unset}".into(),
        ];
        let mut cli = GitCli::default();
        let bare_log = dir.path().join("bare.log");
        cli.start_check("bare", dir.path(), &echo, &[], &bare_log, &[])
            .unwrap();
        let given_log = dir.path().join("given.log");
        let outer: Vec<String> = vec![
            "sh".into(),
            "-c".into(),
            "echo outer; exec \"$@\"".into(),
            "outer".into(),
        ];
        cli.start_check(
            "given",
            dir.path(),
            &echo,
            &[(RECORD_TOKEN_ENV.into(), "t0k".into())],
            &given_log,
            &outer,
        )
        .unwrap();
        assert_eq!(
            poll_to("the bare exit", || cli.poll_check("bare")).unwrap(),
            0
        );
        assert_eq!(
            poll_to("the given exit", || cli.poll_check("given")).unwrap(),
            0
        );
        assert_eq!(std::fs::read_to_string(&bare_log).unwrap(), "unset\n");
        assert_eq!(std::fs::read_to_string(&given_log).unwrap(), "outer\nt0k\n");
    }
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
            &[],
        )
        .unwrap();
        let code = poll_to("the check's exit", || cli.poll_check("k")).unwrap();
        assert_eq!(code, 3);
        assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "backend");
        assert!(GitCli::default().poll_check("k").unwrap().is_err());
    }

    /// Polls `ready` every 25 ms until it gives a value; panics naming
    /// `what` after ten seconds. A deadline, not a count, so a loaded
    /// machine only makes the test slower.
    fn poll_to<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(value) = ready() {
                return value;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "never saw {what} within 10 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    /// The pid a check's script wrote to `pid` in `dir`, once it has.
    fn written_pid(dir: &Path) -> u32 {
        poll_to("the check's pid", || {
            std::fs::read_to_string(dir.join("pid"))
                .ok()
                .and_then(|text| text.trim().parse().ok())
        })
    }

    fn ps_field(field: &str, pid: u32) -> u32 {
        let out = Command::new("ps")
            .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    fn alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    #[test]
    fn a_killed_check_takes_its_process_group_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = GitCli::default();
        cli.start_check(
            "k",
            dir.path(),
            &[
                "sh".into(),
                "-c".into(),
                "sleep 30 & echo $! > pid; wait".into(),
            ],
            &[],
            &dir.path().join("gate.log"),
            &[],
        )
        .unwrap();
        let sleep = written_pid(dir.path());
        let sh = ps_field("ppid", sleep);
        assert_eq!(ps_field("pgid", sleep), sh, "the check leads its group");
        cli.kill_check("k");
        poll_to("the check gone", || cli.check_gone("k").then_some(()));
        assert!(!alive(sleep));
    }

    /// A `sleep` check started in one runner, its group as recorded, and
    /// its child handed to a thread that reaps it, as init reaps what a
    /// dead runner left. Forgetting the child instead would leave a
    /// killed leader a zombie of the test process, and `kill -0` reaches
    /// a zombie, so the group would never read as gone.
    fn orphaned_sleep(dir: &Path) -> CheckGroup {
        let mut previous = GitCli::default();
        previous
            .start_check(
                "k",
                dir,
                &["sleep".into(), "30".into()],
                &[],
                &dir.join("gate.log"),
                &[],
            )
            .unwrap();
        let group = previous.check_group("k").unwrap();
        let mut child = previous.checks.remove("k").unwrap();
        std::thread::spawn(move || child.wait());
        group
    }

    #[test]
    fn an_adopted_orphan_is_stopped_by_its_group() {
        let dir = tempfile::tempdir().unwrap();
        let group = orphaned_sleep(dir.path());
        assert_eq!(ps_field("pgid", group.pgid), group.pgid);
        let mut cli = GitCli::default();
        assert_eq!(cli.adopt_check("k", &group, false), Adopted::Killed);
        poll_to("the orphan gone", || cli.check_gone("k").then_some(()));
        assert!(!alive(group.pgid));
    }

    #[test]
    fn a_group_whose_leader_started_at_another_time_is_not_signalled() {
        let dir = tempfile::tempdir().unwrap();
        let group = orphaned_sleep(dir.path());
        let other = CheckGroup {
            leader_started: "never".into(),
            ..group.clone()
        };
        let mut cli = GitCli::default();
        assert_eq!(cli.adopt_check("k", &other, false), Adopted::Gone);
        assert!(group_alive(group.pgid));
        assert!(cli.killed.is_empty());
        let _ = Command::new("kill")
            .args(["-KILL", &group.pgid.to_string()])
            .output();
        poll_to("the sleeper gone", || (!alive(group.pgid)).then_some(()));
    }

    #[test]
    fn a_group_whose_leader_is_gone_is_not_signalled() {
        let dir = tempfile::tempdir().unwrap();
        let mut previous = GitCli::default();
        previous
            .start_check(
                "k",
                dir.path(),
                &["sh".into(), "-c".into(), "sleep 30 & echo $! > pid".into()],
                &[],
                &dir.path().join("gate.log"),
                &[],
            )
            .unwrap();
        let group = previous.check_group("k").unwrap();
        let sleep = written_pid(dir.path());
        let mut leader = previous.checks.remove("k").unwrap();
        leader.wait().unwrap();
        assert!(group_alive(group.pgid), "the sleep is left in the group");
        let mut cli = GitCli::default();
        assert_eq!(cli.adopt_check("k", &group, false), Adopted::Gone);
        assert!(alive(sleep));
        assert!(cli.killed.is_empty());
        let _ = Command::new("kill")
            .args(["-KILL", &sleep.to_string()])
            .output();
        poll_to("the sleeper gone", || (!alive(sleep)).then_some(()));
    }

    #[test]
    fn a_vouched_leaderless_group_is_killed_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut previous = GitCli::default();
        previous
            .start_check(
                "k",
                dir.path(),
                &["sh".into(), "-c".into(), "sleep 30 & echo $! > pid".into()],
                &[],
                &dir.path().join("gate.log"),
                &[],
            )
            .unwrap();
        let group = previous.check_group("k").unwrap();
        let sleep = written_pid(dir.path());
        let mut leader = previous.checks.remove("k").unwrap();
        leader.wait().unwrap();
        assert!(group_alive(group.pgid), "the sleep is left in the group");
        let mut cli = GitCli::default();
        assert_eq!(cli.adopt_check("k", &group, true), Adopted::Killed);
        poll_to("the sleeper gone", || (!alive(sleep)).then_some(()));
        poll_to("the group gone", || cli.check_gone("k").then_some(()));
    }

    #[test]
    fn an_empty_group_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut previous = GitCli::default();
        previous
            .start_check(
                "k",
                dir.path(),
                &["sleep".into(), "30".into()],
                &[],
                &dir.path().join("gate.log"),
                &[],
            )
            .unwrap();
        let group = previous.check_group("k").unwrap();
        previous.kill_check("k");
        poll_to("the check gone", || previous.check_gone("k").then_some(()));
        let mut cli = GitCli::default();
        assert_eq!(cli.adopt_check("k", &group, false), Adopted::Gone);
        assert!(!GitCli::default().group_running("k", &group));
    }

    #[test]
    fn a_group_whose_leader_exited_with_a_child_left_reads_running_and_is_not_signalled() {
        let dir = tempfile::tempdir().unwrap();
        let mut previous = GitCli::default();
        previous
            .start_check(
                "k",
                dir.path(),
                &["sh".into(), "-c".into(), "sleep 30 & echo $! > pid".into()],
                &[],
                &dir.path().join("gate.log"),
                &[],
            )
            .unwrap();
        let group = previous.check_group("k").unwrap();
        let sleep = written_pid(dir.path());
        let mut leader = previous.checks.remove("k").unwrap();
        leader.wait().unwrap();
        let cli = GitCli::default();
        assert!(cli.group_running("k", &group));
        assert!(alive(sleep), "reading the group sends it nothing");
        let _ = Command::new("kill")
            .args(["-KILL", &sleep.to_string()])
            .output();
        poll_to("the sleeper gone", || (!alive(sleep)).then_some(()));
        assert!(!cli.group_running("k", &group));
    }

    #[test]
    fn a_group_led_by_a_process_started_at_another_time_reads_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let group = orphaned_sleep(dir.path());
        let cli = GitCli::default();
        assert!(cli.group_running("k", &group));
        let other = CheckGroup {
            leader_started: "never".into(),
            ..group.clone()
        };
        assert!(!cli.group_running("k", &other));
        assert!(alive(group.pgid));
        let _ = Command::new("kill")
            .args(["-KILL", &group.pgid.to_string()])
            .output();
        poll_to("the sleeper gone", || (!alive(group.pgid)).then_some(()));
    }

    #[test]
    fn a_second_kill_of_a_check_sends_nothing_and_only_escalate_does() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = GitCli::default();
        cli.start_check(
            "k",
            dir.path(),
            &[
                "sh".into(),
                "-c".into(),
                "sh -c 'trap \"\" TERM; sleep 30' & echo $! > pid; wait".into(),
            ],
            &[],
            &dir.path().join("gate.log"),
            &[],
        )
        .unwrap();
        let inner = written_pid(dir.path());
        cli.kill_check("k");
        cli.kill_check("k");
        assert!(alive(inner), "a second kill sends no KILL");
        assert!(!cli.check_gone("k"));
        cli.escalate_check("k");
        poll_to("the step-up reached the group", || {
            (!alive(inner)).then_some(())
        });
        assert!(!cli.killed.contains_key("k"));
        cli.escalate_check("k");
        assert!(!alive(inner));
        assert!(cli.killed.is_empty());
    }

    #[test]
    fn port_free_refuses_a_bound_port() {
        let cli = GitCli::default();
        let wildcard = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = wildcard.local_addr().unwrap().port();
        assert!(!cli.port_free(port), "a wildcard listener holds it");
        drop(wildcard);
        let loopback = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = loopback.local_addr().unwrap().port();
        assert!(!cli.port_free(port), "a loopback listener holds it");
        drop(loopback);
        // Another test's connection may take a freed port as its own
        // source port, so freedom is read over a few fresh ones.
        let freed = (0..5).any(|_| {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = l.local_addr().unwrap().port();
            drop(l);
            cli.port_free(port)
        });
        assert!(freed, "a port reads free once closed");
    }

    #[test]
    fn answers_http_reads_a_status_line() {
        use std::io::{Read as _, Write as _};
        let cli = GitCli::default();
        let server = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        let serving = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            let mut buf = [0u8; 256];
            let n = stream.read(&mut buf).unwrap();
            let asked = String::from_utf8_lossy(&buf[..n]).into_owned();
            stream.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n").unwrap();
            asked
        });
        assert!(cli.answers_http(port, "/health"), "any status answers");
        assert!(
            serving
                .join()
                .unwrap()
                .starts_with("GET /health HTTP/1.0\r\n")
        );
        // Nothing listening: refused at once.
        assert!(!cli.answers_http(port, "/"));
        // A listener that never answers is given up on.
        let mute = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = mute.local_addr().unwrap().port();
        let started = std::time::Instant::now();
        assert!(!cli.answers_http(port, "/"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
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
        // Match with the parent: the temp directory's random name can
        // hold "t2" on its own.
        assert!(!list.contains("wt/t2"), "{list}");
        assert!(list.contains("wt/t4"), "{list}");
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
    fn the_real_git_reads_deletes_renames_and_checks_out_a_kept_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = origin_and_clone(dir.path(), "p");
        let mut cli = GitCli::default();
        let wt = dir.path().join("wt").join("t1");
        let branch = "dispatch/1-x";
        cli.worktree_add(&repo, &wt, branch, "origin/main").unwrap();
        cli.worktree_remove(&repo, &wt).unwrap();
        assert!(cli.branch_exists(&repo, branch).unwrap());
        assert_eq!(cli.branch_ahead(&repo, branch, "origin/main").unwrap(), 0);
        assert!(!cli.branch_exists(&repo, "dispatch/9-missing").unwrap());

        cli.delete_branch(&repo, branch).unwrap();
        cli.worktree_add(&repo, &wt, branch, "origin/main").unwrap();
        let old = commit(&wt, "f", "work\n", &["-m", "work"]);
        let e = cli.delete_branch(&repo, branch).unwrap_err();
        assert!(e.to_string().contains("worktree"), "{e:#}");
        cli.worktree_remove(&repo, &wt).unwrap();
        assert_eq!(cli.branch_ahead(&repo, branch, "origin/main").unwrap(), 1);

        sh(&repo, &["branch", "taken", "origin/main"]);
        assert!(cli.rename_branch(&repo, branch, "taken").is_err());
        cli.rename_branch(&repo, branch, "dispatch/1-x.closed")
            .unwrap();
        assert!(!cli.branch_exists(&repo, branch).unwrap());
        assert!(cli.branch_exists(&repo, "dispatch/1-x.closed").unwrap());

        cli.worktree_checkout(&repo, &wt, "dispatch/1-x.closed")
            .unwrap();
        assert_eq!(cli.head(&wt).unwrap(), old);
        assert!(
            cli.is_worktree_of(&repo, &wt, "dispatch/1-x.closed")
                .unwrap()
        );
    }

    #[test]
    fn yyyymmdd_is_the_utc_day() {
        assert_eq!(yyyymmdd(0), "19700101");
        assert_eq!(yyyymmdd(1_790_985_600_000), "20261003");
        assert_eq!(yyyymmdd(951_782_400_000), "20000229");
        assert_eq!(yyyymmdd(951_868_799_999), "20000229");
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
    fn a_range_is_counted_in_commits_files_and_lines() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        commit(&wt, "a", "a1\na2\n", &["-m", "A"]);
        let head = commit(&wt, "b", "b1\n", &["-m", "B"]);
        let size = GitCli::default().range_size(&wt, &base, &head).unwrap();
        assert_eq!(
            size,
            RangeSize {
                commits: 2,
                files: 2,
                insertions: 3,
                deletions: 0
            }
        );
        assert!(GitCli::default().range_size(&wt, "nope", &head).is_err());
        assert_eq!(
            parse_shortstat(" 1 file changed, 2 deletions(-)"),
            (1, 0, 2)
        );
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

    /// Two branch commits over a base that moved: only the second
    /// touches what the base changed, so only it is listed, and nothing
    /// in the tree or its refs moves.
    #[test]
    fn the_real_git_lists_only_the_commits_whose_replay_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        commit(&wt, "a", "1\n", &["-m", "A"]);
        let b = commit(&wt, "shared", "branch\n", &["-m", "B"]);
        sh(&wt, &["checkout", "-q", "--detach", &base]);
        let onto = commit(&wt, "shared", "base\n", &["-m", "moved"]);
        sh(&wt, &["checkout", "-q", "dispatch/1-x"]);
        let cli = GitCli::default();
        let refs = sh(&wt, &["for-each-ref"]);
        let listed = cli.conflicting_commits(&wt, &base, &b, &onto).unwrap();
        assert_eq!(listed, vec![b.clone()]);
        assert_eq!(cli.head(&wt).unwrap(), b);
        assert!(cli.is_clean(&wt).unwrap());
        assert_eq!(sh(&wt, &["for-each-ref"]), refs, "no ref was written");
        assert!(
            cli.conflicting_commits(&wt, &base, &b, &base)
                .unwrap()
                .is_empty(),
            "onto its own base nothing conflicts"
        );
    }

    /// A base that moved under the branch: the file both changed is
    /// named, a change elsewhere is not, and nothing in the tree or its
    /// refs moves.
    #[test]
    fn the_real_git_names_the_files_a_merge_with_the_base_conflicts_in() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        commit(&wt, "a", "1\n", &["-m", "A"]);
        let b = commit(&wt, "shared", "branch\n", &["-m", "B"]);
        sh(&wt, &["checkout", "-q", "--detach", &base]);
        let onto = commit(&wt, "shared", "base\n", &["-m", "moved"]);
        let elsewhere = commit(&wt, "other", "base\n", &["-m", "elsewhere"]);
        sh(&wt, &["checkout", "-q", "--detach", &base]);
        let disjoint = commit(&wt, "other", "base\n", &["-m", "disjoint"]);
        sh(&wt, &["checkout", "-q", "dispatch/1-x"]);
        let cli = GitCli::default();
        let refs = sh(&wt, &["for-each-ref"]);
        assert_eq!(
            cli.conflicting_files(&wt, &b, &onto).unwrap(),
            vec!["shared".to_owned()]
        );
        assert_eq!(
            cli.conflicting_files(&wt, &b, &elsewhere).unwrap(),
            vec!["shared".to_owned()]
        );
        assert!(
            cli.conflicting_files(&wt, &b, &disjoint)
                .unwrap()
                .is_empty()
        );
        assert_eq!(cli.head(&wt).unwrap(), b);
        assert!(cli.is_clean(&wt).unwrap());
        assert_eq!(sh(&wt, &["for-each-ref"]), refs, "no ref was written");
    }

    /// A branch and a moved base that conflict on `shared`: the
    /// worktree on the branch, and the base's commit.
    fn diverged(root: &Path) -> (PathBuf, String) {
        let (wt, base) = cut(root);
        commit(&wt, "shared", "branch\n", &["-m", "B"]);
        sh(&wt, &["checkout", "-q", "--detach", &base]);
        let onto = commit(&wt, "shared", "base\n", &["-m", "moved"]);
        sh(&wt, &["checkout", "-q", "dispatch/1-x"]);
        (wt, onto)
    }

    /// `git rebase` with `args`, which must stop on the conflict.
    fn rebase_stops(dir: &Path, args: &[&str]) {
        let status = git_in(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "rebase"])
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "{args:?} stopped");
    }

    #[test]
    fn the_real_git_sees_a_stopped_rebase_of_either_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, onto) = diverged(dir.path());
        let mut cli = GitCli::default();
        assert!(!cli.rebase_in_progress(&wt).unwrap());
        rebase_stops(&wt, &[&onto]);
        assert!(cli.rebase_in_progress(&wt).unwrap(), "rebase-merge");
        sh(&wt, &["rebase", "--abort"]);
        assert!(!cli.rebase_in_progress(&wt).unwrap());
        rebase_stops(&wt, &["--apply", &onto]);
        assert!(cli.rebase_in_progress(&wt).unwrap(), "rebase-apply");
        let err = cli.rebase_onto(&wt, &onto).unwrap_err();
        assert!(err.to_string().contains("already in progress"), "{err}");
        assert!(cli.rebase_in_progress(&wt).unwrap(), "left as it was");
        // A main checkout's `--git-path` is relative to it, not to the
        // process's directory.
        let main = dir.path().join("p-origin");
        commit(&main, "shared", "main\n", &["-m", "main"]);
        sh(&main, &["checkout", "-q", "-b", "side", "HEAD~1"]);
        commit(&main, "shared", "side\n", &["-m", "side"]);
        assert!(!cli.rebase_in_progress(&main).unwrap());
        rebase_stops(&main, &["main"]);
        assert!(cli.rebase_in_progress(&main).unwrap(), "a main checkout");
    }

    #[test]
    fn the_real_git_branch_head_is_none_off_the_branch() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, onto) = diverged(dir.path());
        let cli = GitCli::default();
        let tip = cli.head(&wt).unwrap();
        let branch = "dispatch/1-x";
        assert_eq!(cli.branch_head(&wt, branch).unwrap(), Some(tip.clone()));
        sh(&wt, &["checkout", "-q", "--detach", &onto]);
        assert_eq!(cli.branch_head(&wt, branch).unwrap(), None);
        assert_eq!(cli.behind(&wt, &onto).unwrap(), 0, "HEAD is not the branch");
        sh(&wt, &["checkout", "-q", branch]);
        rebase_stops(&wt, &[&onto]);
        assert_eq!(cli.branch_head(&wt, branch).unwrap(), None);
        sh(&wt, &["rebase", "--abort"]);
        sh(&wt, &["checkout", "-q", "-b", "other"]);
        assert_eq!(cli.branch_head(&wt, branch).unwrap(), None);
        assert_eq!(
            sh(&wt, &["rev-parse", branch]).trim(),
            tip,
            "the branch untouched"
        );
    }

    #[test]
    fn the_real_git_finds_a_name_absent_only_when_neither_diff_nor_tree_has_it() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, _) = cut(dir.path());
        commit(&wt, "gone.rs", "fn removed_name() {}\n", &["-m", "old"]);
        sh(&wt, &["rm", "-q", "gone.rs"]);
        sh(&wt, &["commit", "-q", "-m", "drop"]);
        std::fs::create_dir_all(wt.join("src")).unwrap();
        let a = commit(
            &wt,
            "src/lib.rs",
            "fn old_name() {}\nfn kept() {}\n",
            &["-m", "A"],
        );
        let fixup = commit(
            &wt,
            "src/lib.rs",
            "fn new_name() {}\nfn kept() {}\n",
            &["--fixup", &a],
        );
        let cli = GitCli::default();
        let names = |n: &[&str]| n.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        // `A`'s own diff adds `old_name`, so judged on `A` it is present
        // although the fixup renamed it away.
        let asked = names(&[
            "old_name",
            "kept",
            "lib.rs",
            "src/lib.rs",
            "removed_name",
            "--nope",
            "Thing.kept",
            "gone.rs",
        ]);
        assert_eq!(
            cli.absent(&wt, &format!("{fixup}~1"), &fixup, &asked)
                .unwrap(),
            ["removed_name", "--nope", "gone.rs"],
            "a field is found by its last segment; a removed file in a tree of its kind is a path, absent"
        );
        let drop = format!("{fixup}~2");
        assert_eq!(
            cli.absent(
                &wt,
                &drop,
                &fixup,
                &names(&["removed_name", "old_name", "kept"])
            )
            .unwrap(),
            ["old_name"],
            "renamed away and not in this diff: absent; one the commit removed is in its diff, one in the tree is present"
        );
        assert!(cli.absent(&wt, &fixup, &fixup, &[]).unwrap().is_empty());
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
    fn reset_branch_is_refused_when_the_branch_moved_the_tree_is_dirty_or_detached() {
        let dir = tempfile::tempdir().unwrap();
        let (wt, base) = cut(dir.path());
        let one = commit(&wt, "a", "1\n", &["-m", "one"]);
        let mut cli = GitCli::default();
        assert!(
            cli.reset_branch(&wt, &base, &base).is_err(),
            "not at the expected head"
        );
        assert_eq!(cli.head(&wt).unwrap(), one);
        // A change to a file the reset would rewrite is kept by refusing.
        std::fs::write(wt.join("a"), "edited\n").unwrap();
        assert!(cli.reset_branch(&wt, &base, &one).is_err(), "dirty");
        assert_eq!(cli.head(&wt).unwrap(), one);
        sh(&wt, &["checkout", "-q", "--", "a"]);
        sh(&wt, &["checkout", "-q", "--detach"]);
        let e = cli.reset_branch(&wt, &base, &one).unwrap_err();
        assert!(format!("{e:#}").contains("no branch checked out"), "{e:#}");
        sh(&wt, &["checkout", "-q", "dispatch/1-x"]);
        cli.reset_branch(&wt, &base, &one).unwrap();
        assert_eq!(cli.head(&wt).unwrap(), base);
        assert!(cli.is_clean(&wt).unwrap());
    }

    #[test]
    fn the_fake_reset_branch_refuses_as_git_does() {
        let dir = PathBuf::from("/wt");
        let mut fake = FakeRepo::default();
        fake.heads.insert(dir.clone(), "head0002".into());
        assert!(fake.reset_branch(&dir, "head0001", "head0009").is_err());
        fake.dirty.push(dir.clone());
        assert!(fake.reset_branch(&dir, "head0001", "head0002").is_err());
        fake.dirty.clear();
        fake.detached.push(dir.clone());
        assert!(fake.reset_branch(&dir, "head0001", "head0002").is_err());
        fake.detached.clear();
        fake.mid_rebase.push(dir.clone());
        assert!(fake.reset_branch(&dir, "head0001", "head0002").is_err());
        fake.mid_rebase.clear();
        assert!(fake.resets.is_empty() && fake.head(&dir).unwrap() == "head0002");
        fake.reset_branch(&dir, "head0001", "head0002").unwrap();
        assert_eq!(fake.head(&dir).unwrap(), "head0001");
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

    /// Whether this Mac has `sandbox-exec`; the confined tests skip
    /// without it.
    #[cfg(target_os = "macos")]
    fn sandbox_exec() -> bool {
        Path::new("/usr/bin/sandbox-exec").exists()
    }

    /// A tree under `$HOME`, outside the temp directory the sandbox
    /// always allows, so a write that lands in it was let through by
    /// the tree's own grant.
    #[cfg(target_os = "macos")]
    fn home_tree() -> tempfile::TempDir {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let dir = tempfile::Builder::new()
            .prefix(".dispatch-tree-")
            .tempdir_in(home)
            .unwrap();
        assert!(
            !dir.path()
                .starts_with(std::env::temp_dir().canonicalize().unwrap())
        );
        dir
    }

    #[cfg(target_os = "macos")]
    fn confined_to(dir: &Path) -> Confine {
        Confine {
            writable: vec![dir.to_path_buf()],
            network: Network::Allow,
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_confined_check_writes_its_tree_and_is_refused_outside_it() {
        if !sandbox_exec() {
            return;
        }
        let dir = home_tree();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let probe = home.join(format!(".dispatch-probe-{}", std::process::id()));
        let log = dir.path().join("checks.log");
        let mut cli = GitCli::default();
        cli.start_check_confined(
            "k",
            dir.path(),
            &[
                "sh".into(),
                "-c".into(),
                "touch ok && touch \"$HOME/.dispatch-probe-$PROBE\"".into(),
            ],
            &[("PROBE".into(), std::process::id().to_string())],
            &log,
            &confined_to(dir.path()),
            &[],
        )
        .unwrap();
        let code = poll_to("the check's exit", || cli.poll_check("k")).unwrap();
        let exists = probe.exists();
        let _ = std::fs::remove_file(&probe);
        assert_ne!(code, 0);
        assert!(dir.path().join("ok").exists());
        assert!(!exists, "the write outside the tree was refused");
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.starts_with("dispatch: confined; writable "), "{text}");
        assert!(text.contains("Operation not permitted"), "{text}");
        let deny = format!("deny(1) file-write-create {}", probe.display());
        assert!(text.contains(&deny), "{text}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_confined_check_killed_by_a_signal_polls_as_minus_one() {
        if !sandbox_exec() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let argv: Vec<String> = vec!["sh".into(), "-c".into(), "kill -KILL $$".into()];
        let log = dir.path().join("checks.log");
        let mut cli = GitCli::default();
        cli.start_check_confined(
            "c",
            dir.path(),
            &argv,
            &[],
            &log,
            &confined_to(dir.path()),
            &[],
        )
        .unwrap();
        cli.start_check(
            "u",
            dir.path(),
            &argv,
            &[],
            &dir.path().join("bare.log"),
            &[],
        )
        .unwrap();
        let confined = poll_to("the confined exit", || cli.poll_check("c")).unwrap();
        let bare = poll_to("the bare exit", || cli.poll_check("u")).unwrap();
        assert_eq!((confined, bare), (-1, -1));
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(!text.contains("dispatch: sandbox:"), "{text}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_confined_check_that_exits_over_128_keeps_its_code() {
        if !sandbox_exec() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("checks.log");
        let mut cli = GitCli::default();
        cli.start_check_confined(
            "c",
            dir.path(),
            &["sh".into(), "-c".into(), "exit 255".into()],
            &[],
            &log,
            &confined_to(dir.path()),
            &[],
        )
        .unwrap();
        let code = poll_to("the check's exit", || cli.poll_check("c")).unwrap();
        assert_eq!(code, 255);
        let text = std::fs::read_to_string(&log).unwrap();
        assert_eq!(text.lines().count(), 1, "only the header: {text}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_confined_run_writes_its_tree_and_returns_the_header() {
        if !sandbox_exec() {
            return;
        }
        let dir = home_tree();
        let mut cli = GitCli::default();
        let header = cli
            .run_confined(
                dir.path(),
                &["touch".into(), "ok".into()],
                &[],
                &confined_to(dir.path()),
            )
            .unwrap();
        assert!(
            header.starts_with("dispatch: confined; writable "),
            "{header}"
        );
        assert!(dir.path().join("ok").exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_failed_confined_run_is_the_header_the_command_and_its_stderr() {
        if !sandbox_exec() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut cli = GitCli::default();
        let err = cli
            .run_confined(
                dir.path(),
                &["sh".into(), "-c".into(), "echo nope >&2; exit 3".into()],
                &[],
                &confined_to(dir.path()),
            )
            .unwrap_err();
        let text = format!("{err:#}");
        assert!(text.starts_with("dispatch: confined; writable "), "{text}");
        assert!(text.ends_with("exited exit status: 3: nope"), "{text}");
        assert!(!text.contains("sandbox-exec"), "{text}");
    }
}
