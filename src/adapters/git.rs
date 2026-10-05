//! Git awareness for a project: which repositories it holds (the root
//! itself, or repositories one or two directories down), their branch and
//! how many paths are changed, and per-path status for tree decorations.
//! All through the `git` command; nothing is written.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ports::changes::{BranchChanges, Changes, Commit, FileStat};

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
        let numstat = git_or_why(dir, &["diff", "--numstat", &format!("{base}...{head}")])?;
        Ok(Changes {
            commits: parse_log(&log),
            files: parse_numstat(&numstat),
        })
    }
}

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

/// `added<TAB>removed<TAB>path` lines; `-` (a binary file) reads as 0.
fn parse_numstat(out: &str) -> Vec<FileStat> {
    out.lines()
        .filter_map(|line| {
            let mut f = line.splitn(3, '\t');
            let (added, removed, path) = (f.next()?, f.next()?, f.next()?);
            Some(FileStat {
                path: path.to_owned(),
                added: added.parse().unwrap_or(0),
                removed: removed.parse().unwrap_or(0),
            })
        })
        .collect()
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
                    removed: 1
                },
                FileStat {
                    path: "b.txt".into(),
                    added: 1,
                    removed: 0
                },
            ]
        );
        assert!(GitChanges.read(root, "nope", "work").is_err());
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
