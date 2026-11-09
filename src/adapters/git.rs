//! Git awareness for a project: which repositories it holds (the root
//! itself, or repositories one or two directories down), their branch and
//! how many paths are changed, and per-path status for tree decorations.
//! All through the `git` command; nothing is written.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::diff::{self, DIFF_LINE_CAP};
use crate::ports::changes::{
    BaseKind, BranchChanges, Changes, Commit, DiffBody, FileDiff, FileStat, FileStatus, WorkBase,
    WorkingChanges,
};

/// How a path differs from the index and HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Modified,
    Untracked,
    Conflict,
}

impl Change {
    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Untracked => "?",
            Self::Conflict => "!",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// Relative to the project root; empty when the root is the repo.
    pub rel: PathBuf,
    /// Branch name, or a short commit id when detached.
    pub branch: String,
    pub changed: usize,
}

/// What `inspect` found for a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitState {
    pub repos: Vec<Repo>,
    /// Changed paths, relative to the project root.
    pub changes: HashMap<PathBuf, Change>,
}

impl GitState {
    /// The status of a path, or of the most severe change under it when
    /// it is a directory.
    #[must_use]
    pub fn status_of(&self, rel: &Path, is_dir: bool) -> Option<Change> {
        if !is_dir {
            return self.changes.get(rel).copied();
        }
        self.changes
            .iter()
            .filter(|(p, _)| p.starts_with(rel))
            .map(|(_, c)| *c)
            .max_by_key(|c| match c {
                Change::Conflict => 2,
                Change::Modified => 1,
                Change::Untracked => 0,
            })
    }
}

/// Inspect `root`: the repository at the root, or those found one or two
/// directories down (skipping hidden directories). Never fails; a
/// missing `git` or no repository yields the empty state.
#[must_use]
pub fn inspect(root: &Path) -> GitState {
    let mut state = GitState::default();
    let repos = if root.join(".git").exists() {
        vec![root.to_path_buf()]
    } else {
        find_repos(root, 2)
    };
    for repo in repos {
        let rel = repo.strip_prefix(root).unwrap_or(&repo).to_path_buf();
        let branch = branch_of(&repo).unwrap_or_default();
        let mut changed = 0;
        for (path, change) in status_of(&repo) {
            changed += 1;
            state.changes.insert(rel.join(path), change);
        }
        state.repos.push(Repo {
            rel,
            branch,
            changed,
        });
    }
    state.repos.sort_by(|a, b| a.rel.cmp(&b.rel));
    state
}

fn find_repos(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    dirs.sort();
    for d in dirs {
        if d.join(".git").exists() {
            found.push(d);
        } else if depth > 1 {
            found.extend(find_repos(&d, depth - 1));
        }
    }
    found
}

/// Drops the caller's own `GIT_*` variables from `cmd`: run from a hook,
/// `GIT_DIR` and `GIT_INDEX_FILE` point at the hook's repository, not
/// the one the command names.
fn without_git_env(cmd: &mut Command) -> &mut Command {
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    cmd
}

/// A `git` command in `repo`, without the caller's own `GIT_*` variables.
fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = without_git_env(&mut Command::new("git"))
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A branch's changes over its base, through `git log` and
/// `git diff --numstat`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitChanges;

impl BranchChanges for GitChanges {
    fn read(&self, dir: &Path, base: &str, head: &str) -> Result<Changes, String> {
        let log = git_or_why(
            dir,
            &[
                "log",
                "--format=%H%x1f%s%x1f%ct",
                &format!("{base}..{head}"),
            ],
        )?;
        let numstat = git_or_why(
            dir,
            &["diff", "--numstat", "-z", "-M", &format!("{base}...{head}")],
        )?;
        Ok(Changes {
            commits: parse_log(&log),
            files: parse_numstat(&numstat),
        })
    }

    fn diff(
        &self,
        dir: &Path,
        base: &str,
        head: &str,
        path: &str,
        old_path: Option<&str>,
    ) -> Result<FileDiff, String> {
        let unified = format!("--unified={DIFF_LINE_CAP}");
        let range = format!("{base}...{head}");
        // The paths come from numstat, so they are literal: a `[id]` or a
        // leading `:` in a name is not a glob or pathspec magic.
        let mut args = vec![
            "--literal-pathspecs",
            "-c",
            "core.quotepath=off",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            &unified,
            &range,
            "--",
            path,
        ];
        args.extend(old_path);
        let out = git_or_why(dir, &args)?;
        if out.len() > DIFF_MAX_BYTES {
            return Ok(FileDiff {
                status: FileStatus::Modified,
                body: DiffBody::TooLarge {
                    lines: out.lines().count(),
                },
            });
        }
        Ok(diff::parse(&out, DIFF_LINE_CAP))
    }
}

/// git's empty tree: the base of a repository with no commit before the
/// session, against which every file reads as added.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The branch a pull request from this checkout would target:
/// `origin`'s default, else the first of the usual names that exists.
fn default_branch(dir: &Path) -> Option<String> {
    if let Some(name) = git(
        dir,
        &["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"],
    ) {
        let name = name.trim();
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    ["origin/main", "origin/master", "main", "master"]
        .into_iter()
        .find(|name| {
            git(
                dir,
                &["rev-parse", "--verify", "-q", &format!("{name}^{{commit}}")],
            )
            .is_some()
        })
        .map(str::to_owned)
}

/// The paths of `paths` git does not track and does not ignore.
fn untracked(top: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let mut args = vec![
        "--literal-pathspecs",
        "ls-files",
        "-z",
        "--others",
        "--exclude-standard",
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    Ok(git_or_why(top, &args)?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect())
}

impl WorkingChanges for GitChanges {
    fn work_base(
        &self,
        dir: &Path,
        since: Option<std::time::SystemTime>,
    ) -> Result<WorkBase, String> {
        let top = PathBuf::from(git_or_why(dir, &["rev-parse", "--show-toplevel"])?.trim());
        let current = git(dir, &["branch", "--show-current"]).unwrap_or_default();
        let current = current.trim();
        if let Some(onto) = default_branch(dir) {
            let short = onto.strip_prefix("origin/").unwrap_or(&onto);
            if !current.is_empty()
                && current != short
                && let Some(sha) = git(dir, &["merge-base", "HEAD", &onto])
            {
                return Ok(WorkBase {
                    sha: sha.trim().to_owned(),
                    kind: BaseKind::Branch(onto),
                    top,
                });
            }
        }
        let sha = match since
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
        {
            Some(secs) => git(
                dir,
                &["rev-list", "-1", &format!("--before={secs}"), "HEAD"],
            )
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| EMPTY_TREE.to_owned()),
            None => git(dir, &["rev-parse", "HEAD"])
                .map_or_else(|| EMPTY_TREE.to_owned(), |s| s.trim().to_owned()),
        };
        Ok(WorkBase {
            sha,
            kind: BaseKind::Session,
            top,
        })
    }

    fn work_stats(
        &self,
        top: &Path,
        base: &str,
        paths: &[String],
    ) -> Result<Vec<FileStat>, String> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut args = vec![
            "--literal-pathspecs",
            "-c",
            "core.quotepath=off",
            "diff",
            "--numstat",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
            base,
            "--",
        ];
        args.extend(paths.iter().map(String::as_str));
        let mut files = parse_numstat(&git_or_why(top, &args)?);
        for path in untracked(top, paths)? {
            let bytes = std::fs::read(top.join(&path)).unwrap_or_default();
            let binary = bytes.contains(&0);
            let lines = if binary {
                0
            } else {
                let text = String::from_utf8_lossy(&bytes);
                u32::try_from(text.lines().count()).unwrap_or(u32::MAX)
            };
            files.push(FileStat {
                path,
                added: lines,
                removed: 0,
                old_path: None,
                binary,
            });
        }
        Ok(files)
    }

    fn work_diff(&self, top: &Path, base: &str, path: &str) -> Result<FileDiff, String> {
        let unified = format!("--unified={DIFF_LINE_CAP}");
        let new = !untracked(top, &[path.to_owned()])?.is_empty();
        let out = if new {
            // `--no-index` exits 1 when the files differ, which they do.
            let out = without_git_env(&mut Command::new("git"))
                .arg("-C")
                .arg(top)
                .args([
                    "diff",
                    "--no-index",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                    &unified,
                    "--",
                    "/dev/null",
                    path,
                ])
                .env("GIT_OPTIONAL_LOCKS", "0")
                .output()
                .map_err(|e| format!("git: {e}"))?;
            if out.status.code() != Some(1) && !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
            }
            String::from_utf8_lossy(&out.stdout).into_owned()
        } else {
            git_or_why(
                top,
                &[
                    "--literal-pathspecs",
                    "-c",
                    "core.quotepath=off",
                    "diff",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                    &unified,
                    base,
                    "--",
                    path,
                ],
            )?
        };
        if out.len() > DIFF_MAX_BYTES {
            return Ok(FileDiff {
                status: if new {
                    FileStatus::Added
                } else {
                    FileStatus::Modified
                },
                body: DiffBody::TooLarge {
                    lines: out.lines().count(),
                },
            });
        }
        Ok(diff::parse(&out, DIFF_LINE_CAP))
    }
}

/// A diff's output past which it is not parsed at all.
const DIFF_MAX_BYTES: usize = 1 << 20;

/// `git` in `repo`, with its own complaint when it fails.
fn git_or_why(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = without_git_env(&mut Command::new("git"))
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// `%H%x1f%s%x1f%ct` lines: sha, subject, committer time in seconds.
fn parse_log(out: &str) -> Vec<Commit> {
    out.lines()
        .filter_map(|line| {
            let mut f = line.split('\x1f');
            let (sha, subject, at) = (f.next()?, f.next()?, f.next()?);
            Some(Commit {
                sha: sha.to_owned(),
                subject: subject.to_owned(),
                at_ms: at.trim().parse::<u64>().unwrap_or(0) * 1000,
            })
        })
        .collect()
}

/// `numstat -z` records: `added<TAB>removed<TAB>path<NUL>`, or for a
/// rename `added<TAB>removed<TAB><NUL>old<NUL>new<NUL>`. `-` for both
/// counts is a binary file, read as 0.
fn parse_numstat(out: &str) -> Vec<FileStat> {
    let mut files = Vec::new();
    let mut fields = out.split('\0');
    while let Some(record) = fields.next() {
        let mut f = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let (path, old_path) = if path.is_empty() {
            let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                break;
            };
            (new.to_owned(), Some(old.to_owned()))
        } else {
            (path.to_owned(), None)
        };
        files.push(FileStat {
            path,
            added: added.parse().unwrap_or(0),
            removed: removed.parse().unwrap_or(0),
            old_path,
            binary: added == "-" && removed == "-",
        });
    }
    files
}

fn branch_of(repo: &Path) -> Option<String> {
    let name = git(repo, &["branch", "--show-current"])?;
    let name = name.trim();
    if !name.is_empty() {
        return Some(name.to_owned());
    }
    let id = git(repo, &["rev-parse", "--short", "HEAD"])?;
    Some(format!("detached {}", id.trim()))
}

/// `git status --porcelain -z`: one entry per changed path, relative to
/// the repository root.
fn status_of(repo: &Path) -> Vec<(PathBuf, Change)> {
    let Some(out) = git(
        repo,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    ) else {
        return Vec::new();
    };
    parse_porcelain(&out)
}

fn parse_porcelain(out: &str) -> Vec<(PathBuf, Change)> {
    let mut result = Vec::new();
    let mut fields = out.split('\0');
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let code = &code[..2];
        // A rename carries the old name in the next field.
        if code.starts_with('R') || code.starts_with('C') {
            fields.next();
        }
        let change = match code {
            "??" => Change::Untracked,
            "UU" | "AA" | "DD" | "AU" | "UA" | "DU" | "UD" => Change::Conflict,
            _ => Change::Modified,
        };
        result.push((PathBuf::from(path), change));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, cmd: &str) {
        let ok = without_git_env(&mut Command::new("sh"))
            .arg("-c")
            .arg(cmd)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("HOME", dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "{cmd}");
    }

    #[test]
    fn parses_porcelain_including_renames() {
        let raw = " M src/a.rs\0?? new.txt\0R  new/name\0old/name\0UU both.rs\0";
        assert_eq!(
            parse_porcelain(raw),
            vec![
                ("src/a.rs".into(), Change::Modified),
                ("new.txt".into(), Change::Untracked),
                ("new/name".into(), Change::Modified),
                ("both.rs".into(), Change::Conflict),
            ]
        );
    }

    #[test]
    fn reads_a_branchs_commits_and_files_over_its_base() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let commit = "git -c user.name=t -c user.email=t@t commit -q";
        sh(
            root,
            &format!("git init -q -b main . && {commit} --allow-empty -m init"),
        );
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        sh(
            root,
            &format!("git add . && {commit} -m base && git branch base"),
        );
        sh(root, "git checkout -q -b work");
        std::fs::write(root.join("a.txt"), "one\nthree\nfour\n").unwrap();
        sh(root, &format!("git add . && {commit} -m 'first change'"));
        std::fs::write(root.join("b.txt"), "new\n").unwrap();
        sh(root, &format!("git add . && {commit} -m 'second change'"));
        let c = GitChanges.read(root, "base", "work").unwrap();
        let subjects: Vec<&str> = c.commits.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects, vec!["second change", "first change"]);
        assert!(c.commits.iter().all(|c| c.sha.len() == 40 && c.at_ms > 0));
        assert_eq!(
            c.files,
            vec![
                FileStat {
                    path: "a.txt".into(),
                    added: 2,
                    removed: 1,
                    old_path: None,
                    binary: false,
                },
                FileStat {
                    path: "b.txt".into(),
                    added: 1,
                    removed: 0,
                    old_path: None,
                    binary: false,
                },
            ]
        );
        assert!(GitChanges.read(root, "nope", "work").is_err());
    }

    #[test]
    fn reads_a_files_diff_over_its_base() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let commit = "git -c user.name=t -c user.email=t@t commit -q";
        sh(
            root,
            &format!("git init -q -b main . && {commit} --allow-empty -m init"),
        );
        let lines =
            "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\n";
        std::fs::write(root.join("m.txt"), lines).unwrap();
        std::fs::write(root.join("old.txt"), lines).unwrap();
        std::fs::write(root.join("bin.dat"), b"a\0b").unwrap();
        std::fs::write(root.join("[m].txt"), "a\n").unwrap();
        sh(
            root,
            &format!("git add . && {commit} -m base && git branch base"),
        );
        sh(root, "git checkout -q -b work && git mv old.txt new.txt");
        std::fs::write(root.join("m.txt"), lines.replace("line 5\n", "line five\n")).unwrap();
        std::fs::write(
            root.join("new.txt"),
            lines.replace("line 9\n", "line nine\n"),
        )
        .unwrap();
        std::fs::write(root.join("bin.dat"), b"a\0c").unwrap();
        std::fs::write(root.join("[m].txt"), "b\n").unwrap();
        sh(root, &format!("git add -A && {commit} -m work"));

        let files = GitChanges.read(root, "base", "work").unwrap().files;
        let find = |p: &str| files.iter().find(|f| f.path == p).unwrap();
        assert!(find("bin.dat").binary);
        assert_eq!(find("new.txt").old_path.as_deref(), Some("old.txt"));
        assert_eq!(
            (find("m.txt").added, find("m.txt").old_path.clone()),
            (1, None)
        );

        let m = GitChanges
            .diff(root, "base", "work", "m.txt", None)
            .unwrap();
        assert_eq!(m.status, FileStatus::Modified);
        let DiffBody::Hunks(h) = &m.body else {
            panic!("{m:?}")
        };
        // The context is as wide as the cap, so the whole file is one hunk.
        assert_eq!((h.len(), h[0].lines.len()), (1, 11));
        let renamed = GitChanges
            .diff(root, "base", "work", "new.txt", Some("old.txt"))
            .unwrap();
        assert_eq!(renamed.status, FileStatus::Renamed);
        assert!(matches!(renamed.body, DiffBody::Hunks(_)));
        let bin = GitChanges
            .diff(root, "base", "work", "bin.dat", None)
            .unwrap();
        assert_eq!(bin.body, DiffBody::Binary);
        // As a glob `[m].txt` would match `m.txt` too, whose diff would
        // then run on into this one's.
        let glob = GitChanges
            .diff(root, "base", "work", "[m].txt", None)
            .unwrap();
        let DiffBody::Hunks(h) = &glob.body else {
            panic!("{glob:?}")
        };
        assert_eq!((h.len(), h[0].lines.len()), (1, 2));
        assert!(
            GitChanges
                .diff(root, "nope", "work", "m.txt", None)
                .is_err()
        );
    }

    #[test]
    fn reads_a_working_copy_against_its_branch_point_or_the_session_start() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let commit = "git -c user.name=t -c user.email=t@t commit -q";
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        std::fs::write(root.join("same.txt"), "same\n").unwrap();
        sh(
            root,
            &format!("git init -q -b main . && git add . && {commit} -m base"),
        );
        sh(root, "git checkout -q -b work");
        std::fs::write(root.join("a.txt"), "one\nthree\nfour\n").unwrap();
        sh(root, &format!("git add . && {commit} -m committed"));
        std::fs::write(root.join("b.txt"), "b\nmore\n").unwrap();
        std::fs::write(root.join("new.txt"), "x\ny\nz\n").unwrap();

        let base = GitChanges.work_base(&root.join("."), None).unwrap();
        assert_eq!(base.kind, BaseKind::Branch("main".into()));
        assert_eq!(
            std::fs::canonicalize(&base.top).unwrap(),
            std::fs::canonicalize(root).unwrap()
        );
        let main = git(root, &["rev-parse", "main"]).unwrap();
        assert_eq!(base.sha, main.trim());

        let paths: Vec<String> = ["a.txt", "b.txt", "new.txt", "same.txt"]
            .map(String::from)
            .to_vec();
        let stats = GitChanges.work_stats(root, &base.sha, &paths).unwrap();
        let counts: Vec<(&str, u32, u32)> = stats
            .iter()
            .map(|f| (f.path.as_str(), f.added, f.removed))
            .collect();
        // Committed and uncommitted edits both count; an unchanged file
        // is left out; a new file counts whole.
        assert_eq!(
            counts,
            vec![("a.txt", 2, 1), ("b.txt", 1, 0), ("new.txt", 3, 0)]
        );

        let new = GitChanges.work_diff(root, &base.sha, "new.txt").unwrap();
        assert_eq!(new.status, FileStatus::Added);
        let DiffBody::Hunks(h) = &new.body else {
            panic!("{new:?}")
        };
        assert_eq!(h[0].lines.len(), 3);
        let a = GitChanges.work_diff(root, &base.sha, "a.txt").unwrap();
        assert_eq!(a.status, FileStatus::Modified);
        assert!(matches!(a.body, DiffBody::Hunks(_)));

        // On the default branch itself the session's start is the base.
        sh(root, "git stash -q -u && git checkout -q main");
        let head = git(root, &["rev-parse", "HEAD"]).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let session = GitChanges.work_base(root, Some(later)).unwrap();
        assert_eq!(
            (session.kind, session.sha.as_str()),
            (BaseKind::Session, head.trim())
        );
        let before = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
        assert_eq!(
            GitChanges.work_base(root, Some(before)).unwrap().sha,
            EMPTY_TREE
        );
    }

    #[test]
    fn inspects_a_repo_at_the_root_and_repos_below() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let init = "git init -q -b main . && git -c user.name=t -c user.email=t@t commit -q --allow-empty -m init";
        // A project that is itself a repository.
        sh(root, init);
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let s = inspect(root);
        assert_eq!(s.repos.len(), 1);
        assert_eq!(s.repos[0].branch, "main");
        assert_eq!(s.changes.get(Path::new("a.txt")), Some(&Change::Untracked));
        assert_eq!(s.status_of(Path::new(""), true), Some(Change::Untracked));

        // A project holding repositories in subdirectories.
        let outer = tempfile::tempdir().unwrap();
        let sub = outer.path().join("clients/acme");
        std::fs::create_dir_all(&sub).unwrap();
        sh(&sub, init);
        std::fs::write(sub.join("tracked.txt"), "1").unwrap();
        sh(
            &sub,
            "git add . && git -c user.name=t -c user.email=t@t commit -q -m add",
        );
        std::fs::write(sub.join("tracked.txt"), "2").unwrap();
        let s = inspect(outer.path());
        assert_eq!(s.repos.len(), 1);
        assert_eq!(s.repos[0].rel, PathBuf::from("clients/acme"));
        assert_eq!(s.repos[0].changed, 1);
        assert_eq!(
            s.status_of(Path::new("clients/acme/tracked.txt"), false),
            Some(Change::Modified)
        );
        assert_eq!(
            s.status_of(Path::new("clients"), true),
            Some(Change::Modified)
        );
        assert_eq!(s.status_of(Path::new("other"), true), None);
    }
}
