//! Project file index: a recursive listing that honors `.gitignore`, a
//! lazily loaded directory view, and fuzzy path matching over the listing.
//!
//! The index reads directory metadata only; the one thing here that
//! opens a file is [`open_pinned`], the remote port's read of a pinned
//! file, confined by descriptors. The walk is delegated to the `ignore`
//! crate (ripgrep's walker) so ignore semantics match what users expect
//! from `rg` and `git`.

use std::cmp::Ordering;
use std::io;
use std::path::{Component, Path, PathBuf};

use ignore::{DirEntry, WalkBuilder};

/// One file or directory under a project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the scanned root, using the platform separator.
    pub rel: PathBuf,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// File size in bytes; always 0 for directories.
    pub size: u64,
}

/// Full recursive listing of a project root.
///
/// Honors `.gitignore`, global git excludes, and `.git/info/exclude`; skips
/// hidden entries (dotfiles and dot-directories, which covers `.git`); never
/// follows symlinks. Sorted by `rel`.
#[derive(Debug, Clone)]
pub struct Listing {
    /// Every entry found, sorted by `rel`.
    pub entries: Vec<Entry>,
    /// True when the walk stopped early because `max_entries` was reached.
    pub truncated: bool,
}

/// One fuzzy-match result borrowing an entry from the searched slice.
///
/// The lifetime `'a` ties each hit to the `entries` slice it came from, so
/// the hits cannot outlive the listing they point into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit<'a> {
    /// The matched entry.
    pub entry: &'a Entry,
    /// Higher is better; only meaningful relative to other hits of one query.
    pub score: i64,
    /// Byte offsets into `entry.rel`'s string form of each matched character,
    /// ascending, for highlighting.
    pub positions: Vec<usize>,
}

/// Recursively list `root`, stopping once `max_entries` entries are collected.
///
/// The root directory itself is not included. Entries that cannot be read
/// (permission errors, races with deletion) are skipped rather than failing
/// the whole scan; only an unreadable or missing root is an error.
/// `shown` names folders under `root` to list even when the root's
/// ignore rules hide them: each is walked as its own tree, honoring its
/// own `.gitignore` but none above it.
pub fn scan(root: &Path, max_entries: usize, shown: &[PathBuf]) -> io::Result<Listing> {
    check_dir(root)?;

    let mut entries = Vec::new();
    let mut truncated = false;
    let mut walks = vec![(walker(root), 1)];
    for dir in shown_dirs(root, shown) {
        let mut builder = walker(&dir);
        builder.parents(false);
        walks.push((builder, 0));
    }
    'walks: for (builder, min_depth) in walks {
        for item in builder.build() {
            if entries.len() >= max_entries {
                truncated = true;
                break 'walks;
            }
            let Some(dent) = readable(item) else { continue };
            if dent.depth() < min_depth {
                continue;
            }
            if let Some(entry) = to_entry(root, &dent) {
                entries.push(entry);
            }
        }
    }
    entries.sort_unstable_by(|a, b| a.rel.cmp(&b.rel));
    // A shown folder the root's rules did not hide was listed twice.
    entries.dedup_by(|a, b| a.rel == b.rel);
    Ok(Listing { entries, truncated })
}

/// The shown folders that exist as directories under `root`, absolute.
fn shown_dirs(root: &Path, shown: &[PathBuf]) -> Vec<PathBuf> {
    shown
        .iter()
        .filter(|p| p.is_relative())
        .map(|p| root.join(p))
        .filter(|p| p.is_dir())
        .collect()
}

/// Immediate children of `dir`, which must be `root` or somewhere under it.
///
/// Directories come first, then files, each sorted by name ignoring ASCII
/// case. Hidden entries are skipped and `.gitignore` rules from `root` down
/// to `dir` apply, so the lazily loaded tree agrees with [`scan`]. Each
/// entry's `rel` is relative to `root`, not to `dir`.
///
/// `shown` is as for [`scan`]: a shown folder appears among its parent's
/// children whatever the ignore rules say, and inside one only its own
/// rules apply.
pub fn children(root: &Path, dir: &Path, shown: &[PathBuf]) -> io::Result<Vec<Entry>> {
    if dir.strip_prefix(root).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not under {}", dir.display(), root.display()),
        ));
    }
    check_dir(dir)?;

    let shown_dirs = shown_dirs(root, shown);
    // Inside a shown folder the walk starts there, so the rules above it
    // never apply; one level under `dir` is kept.
    let inside = shown_dirs
        .iter()
        .filter(|s| dir.starts_with(s))
        .max_by_key(|s| s.components().count())
        .cloned();
    let mut out: Vec<Entry> = match inside {
        Some(start) => {
            let depth = dir
                .strip_prefix(&start)
                .map_or(0, |r| r.components().count());
            let mut builder = walker(&start);
            builder.parents(false).max_depth(Some(depth + 1));
            builder
                .build()
                .filter_map(readable)
                .filter(|dent| dent.depth() == depth + 1 && dent.path().parent() == Some(dir))
                .filter_map(|dent| to_entry(root, &dent))
                .collect()
        }
        None => walker(dir)
            .max_depth(Some(1))
            .build()
            .filter_map(readable)
            .filter(|dent| dent.depth() == 1)
            .filter_map(|dent| to_entry(root, &dent))
            .collect(),
    };
    for shown_dir in shown_dirs {
        if shown_dir.parent() == Some(dir)
            && let Ok(rel) = shown_dir.strip_prefix(root)
            && !out.iter().any(|e| e.rel == rel)
        {
            out.push(Entry {
                rel: rel.to_path_buf(),
                is_dir: true,
                size: 0,
            });
        }
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| cmp_name_ci(&a.rel, &b.rel))
    });
    Ok(out)
}

/// Rank `entries` against `query` by case-insensitive subsequence match.
///
/// Scoring favors matches at the start of a path segment or after `-`, `_`,
/// `.`; contiguous runs; matches inside the file name rather than its
/// directories; and shorter paths. Returns at most `limit` hits, best first,
/// ties broken by `rel`. Directories are skipped unless `include_dirs`. An
/// empty query yields no hits.
pub fn fuzzy<'a>(
    entries: &'a [Entry],
    query: &str,
    limit: usize,
    include_dirs: bool,
) -> Vec<Hit<'a>> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut hits: Vec<Hit<'a>> = entries
        .iter()
        .filter(|e| include_dirs || !e.is_dir)
        .filter_map(|entry| {
            let text = entry.rel.to_string_lossy();
            let (score, positions) = best_match(&text, &query)?;
            Some(Hit {
                entry,
                score,
                positions,
            })
        })
        .collect();
    hits.sort_unstable_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.entry.rel.cmp(&b.entry.rel))
    });
    hits.truncate(limit);
    hits
}

/// Shared walker configuration for [`scan`] and [`children`].
fn walker(start: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(start);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        // Honor `.gitignore` files in plain directories too, not only inside
        // a checked-out repository.
        .require_git(false)
        .sort_by_file_name(std::cmp::Ord::cmp);
    builder
}

fn check_dir(path: &Path) -> io::Result<()> {
    if std::fs::metadata(path)?.is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is not a directory", path.display()),
        ))
    }
}

/// Unwraps a walk item, logging and dropping entries that could not be read.
fn readable(item: Result<DirEntry, ignore::Error>) -> Option<DirEntry> {
    match item {
        Ok(dent) => Some(dent),
        Err(err) => {
            log::warn!("file index: {err}");
            None
        }
    }
}

/// Converts a walk entry to an [`Entry`], dropping symlinks and anything
/// outside `root`.
fn to_entry(root: &Path, dent: &DirEntry) -> Option<Entry> {
    let file_type = dent.file_type()?;
    if file_type.is_symlink() {
        return None;
    }
    let rel = dent.path().strip_prefix(root).ok()?.to_path_buf();
    let is_dir = file_type.is_dir();
    let size = if is_dir {
        0
    } else {
        dent.metadata().map_or(0, |m| m.len())
    };
    Some(Entry { rel, is_dir, size })
}

fn cmp_name_ci(a: &Path, b: &Path) -> Ordering {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    };
    name(a).cmp(&name(b)).then_with(|| a.cmp(b))
}

// Score weights. Boundary and contiguity dominate so `dsgn` prefers
// `design.md` over a scattered match; the length penalty only orders
// otherwise-equal candidates.
const BONUS_BOUNDARY: i64 = 12;
const BONUS_CONTIGUOUS: i64 = 8;
const BONUS_IN_NAME: i64 = 4;
const BONUS_QUERY_AT_NAME_START: i64 = 10;
const PENALTY_PER_BYTE: i64 = 1;

/// Tries the match twice, once confined to the file name and once over the
/// whole path, and keeps the better score. A greedy left-to-right
/// subsequence match on the full path would take the first `d` of
/// `docs/design.md`; matching the name alone finds the run users mean.
fn best_match(text: &str, query: &[char]) -> Option<(i64, Vec<usize>)> {
    let name_start = text.rfind('/').map_or(0, |i| i + 1);
    let whole = greedy_match(text, query, 0);
    let name_only = greedy_match(text, query, name_start);
    let positions = match (whole, name_only) {
        (None, None) => return None,
        (Some(w), None) => w,
        (None, Some(n)) => n,
        (Some(w), Some(n)) => {
            if score(text, &n, name_start) >= score(text, &w, name_start) {
                n
            } else {
                w
            }
        }
    };
    let s = score(text, &positions, name_start);
    Some((s, positions))
}

/// Greedy case-insensitive subsequence match of `query` against
/// `text[from..]`, returning byte offsets of the matched characters.
fn greedy_match(text: &str, query: &[char], from: usize) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(query.len());
    let mut chars = text[from..].char_indices().peekable();
    for &q in query {
        let found = chars
            .by_ref()
            .find(|(_, c)| c.to_lowercase().eq(std::iter::once(q)));
        positions.push(from + found?.0);
    }
    Some(positions)
}

fn score(text: &str, positions: &[usize], name_start: usize) -> i64 {
    let bytes = text.as_bytes();
    let mut total = 0;
    let mut prev: Option<usize> = None;
    for &pos in positions {
        let at_boundary = pos == 0 || matches!(bytes[pos - 1], b'/' | b'-' | b'_' | b'.');
        if at_boundary {
            total += BONUS_BOUNDARY;
        }
        if prev.is_some_and(|p| p + 1 == pos) {
            total += BONUS_CONTIGUOUS;
        }
        if pos >= name_start {
            total += BONUS_IN_NAME;
        }
        prev = Some(pos);
    }
    if positions.first() == Some(&name_start) {
        total += BONUS_QUERY_AT_NAME_START;
    }
    total - PENALTY_PER_BYTE * i64::try_from(bytes.len()).unwrap_or(i64::MAX)
}

/// `~/rest` under the home directory; anything else as typed. `None`
/// for a bare `~`, or `~/` with no home directory to put it under.
#[must_use]
pub fn expand_home(typed: &str) -> Option<PathBuf> {
    if let Some(rest) = typed.strip_prefix("~/") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest))
    } else if typed == "~" {
        None
    } else {
        Some(PathBuf::from(typed))
    }
}

/// `p` with `.` dropped and `..` taken back a step, without asking the
/// disk, so a path shows and matches a project root the way it reads.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The existing file `typed` names: an absolute path as it is, `~/`
/// under home, a relative one under `root` and then `cwd`. Directories
/// do not count; the document view cannot show them.
#[must_use]
pub fn existing_file(typed: &str, root: &Path, cwd: &Path) -> Option<PathBuf> {
    let path = expand_home(typed)?;
    let candidates = if path.is_absolute() {
        vec![path]
    } else {
        vec![root.join(&path), cwd.join(&path)]
    };
    candidates
        .into_iter()
        .map(|p| normalize(&p))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    /// Builds the fixture tree used by every test here.
    fn fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join(".gitignore"), "target/\nnested/ignored.txt\n");
        write(&root.join("src/main.rs"), "fn main() {}\n");
        write(&root.join("docs/design.md"), "# design\n");
        write(&root.join("target/debug/x"), "");
        write(&root.join(".hidden/secret"), "");
        write(&root.join("nested/deeper/leaf.txt"), "leaf");
        write(&root.join("nested/ignored.txt"), "");
        write(&root.join("nested/Zeta.txt"), "");
        tmp
    }

    fn rels(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| e.rel.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn scan_honors_gitignore_and_skips_hidden() {
        let tmp = fixture();
        let listing = scan(tmp.path(), 1000, &[]).unwrap();
        assert!(!listing.truncated);
        assert_eq!(
            rels(&listing.entries),
            [
                "docs",
                "docs/design.md",
                "nested",
                "nested/Zeta.txt",
                "nested/deeper",
                "nested/deeper/leaf.txt",
                "src",
                "src/main.rs",
            ]
        );
        let leaf = listing
            .entries
            .iter()
            .find(|e| e.rel.ends_with("leaf.txt"))
            .unwrap();
        assert!(!leaf.is_dir);
        assert_eq!(leaf.size, 4);
        let docs = listing
            .entries
            .iter()
            .find(|e| e.rel == Path::new("docs"))
            .unwrap();
        assert!(docs.is_dir);
        assert_eq!(docs.size, 0);
    }

    #[test]
    fn shown_folders_appear_despite_the_roots_ignore_with_their_own_rules() {
        let tmp = fixture();
        let root = tmp.path();
        write(
            &root.join(".gitignore"),
            "target/\nnested/ignored.txt\nsub/\n",
        );
        write(&root.join("sub/.gitignore"), "build/\n");
        write(&root.join("sub/src/lib.rs"), "");
        write(&root.join("sub/build/out"), "");
        write(&root.join("sub/target/keep"), "");
        assert!(
            !rels(&scan(root, 1000, &[]).unwrap().entries)
                .iter()
                .any(|r| r.starts_with("sub")),
            "hidden without show"
        );
        let shown = [PathBuf::from("sub")];
        let listing = scan(root, 1000, &shown).unwrap();
        let got = rels(&listing.entries);
        assert!(got.contains(&"sub".to_owned()));
        assert!(got.contains(&"sub/src/lib.rs".to_owned()));
        assert!(
            !got.contains(&"sub/build/out".to_owned()),
            "sub's own rules"
        );
        assert!(
            got.contains(&"sub/target/keep".to_owned()),
            "root's rules stop"
        );
        assert_eq!(got.iter().filter(|r| *r == "src").count(), 1);
        // The tree agrees: sub is a child of the root, its children obey
        // only its rules, and deeper levels still list.
        let top = rels(&children(root, root, &shown).unwrap());
        assert!(top.contains(&"sub".to_owned()));
        let inside = rels(&children(root, &root.join("sub"), &shown).unwrap());
        assert_eq!(inside, ["sub/src", "sub/target"]);
        let deeper = rels(&children(root, &root.join("sub/src"), &shown).unwrap());
        assert_eq!(deeper, ["sub/src/lib.rs"]);
        // A shown folder the rules never hid lists once.
        let twice = scan(root, 1000, &[PathBuf::from("docs")]).unwrap();
        assert_eq!(
            rels(&twice.entries).iter().filter(|r| *r == "docs").count(),
            1
        );
        // Missing folders are ignored.
        assert!(scan(root, 1000, &[PathBuf::from("nope")]).is_ok());
    }

    #[test]
    fn scan_truncates_at_cap() {
        let tmp = fixture();
        let listing = scan(tmp.path(), 3, &[]).unwrap();
        assert!(listing.truncated);
        assert_eq!(listing.entries.len(), 3);
    }

    #[test]
    fn scan_rejects_missing_root() {
        let tmp = fixture();
        assert!(scan(&tmp.path().join("nope"), 10, &[]).is_err());
    }

    #[test]
    fn children_lists_dirs_first_then_files_case_insensitively() {
        let tmp = fixture();
        let root = tmp.path();
        assert_eq!(
            rels(&children(root, root, &[]).unwrap()),
            ["docs", "nested", "src"]
        );
        // Ignore rules from the root apply to a nested directory, and the
        // sort ignores case.
        assert_eq!(
            rels(&children(root, &root.join("nested"), &[]).unwrap()),
            ["nested/deeper", "nested/Zeta.txt"]
        );
        assert!(children(root, Path::new("/"), &[]).is_err());
    }

    #[test]
    fn fuzzy_ranks_file_name_matches_first() {
        let tmp = fixture();
        let listing = scan(tmp.path(), 1000, &[]).unwrap();
        let hits = fuzzy(&listing.entries, "dsgn", 10, false);
        assert_eq!(hits[0].entry.rel, Path::new("docs/design.md"));
        assert_eq!(hits[0].positions, [5, 7, 9, 10]);
        assert!(hits.iter().all(|h| !h.entry.is_dir));

        // Directories only appear when asked for, and the limit is respected.
        let hits = fuzzy(&listing.entries, "n", 2, true);
        assert_eq!(hits.len(), 2);
        assert!(
            fuzzy(&listing.entries, "nested", 10, true)
                .iter()
                .any(|h| h.entry.rel == Path::new("nested"))
        );

        assert!(fuzzy(&listing.entries, "", 10, true).is_empty());
        assert!(fuzzy(&listing.entries, "zzzz", 10, true).is_empty());
    }

    #[test]
    fn fuzzy_prefers_shorter_and_boundary_matches() {
        let entry = |rel: &str| Entry {
            rel: PathBuf::from(rel),
            is_dir: false,
            size: 0,
        };
        let entries = [
            entry("src/app_state.rs"),
            entry("src/ui/some_random_thing.rs"),
            entry("as.rs"),
        ];
        let hits = fuzzy(&entries, "as", 10, false);
        assert_eq!(hits[0].entry.rel, Path::new("as.rs"));
        assert_eq!(hits[1].entry.rel, Path::new("src/app_state.rs"));
    }

    #[test]
    fn existing_file_tries_the_root_then_the_cwd_and_wants_a_file() {
        let root = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        fs::write(root.path().join("both.rs"), "").unwrap();
        fs::write(cwd.path().join("both.rs"), "").unwrap();
        fs::write(cwd.path().join("cwd.rs"), "").unwrap();
        fs::create_dir(cwd.path().join("dir.d")).unwrap();
        let at = |p: &str| existing_file(p, root.path(), cwd.path());
        assert_eq!(at("both.rs"), Some(root.path().join("both.rs")));
        assert_eq!(at("./cwd.rs"), Some(cwd.path().join("cwd.rs")));
        assert_eq!(at("gone.rs"), None);
        assert_eq!(at("dir.d"), None);
        let abs = cwd.path().join("cwd.rs");
        assert_eq!(at(&abs.display().to_string()), Some(abs));
        assert_eq!(at("/nonexistent/x.rs"), None);
    }
}

/// Why a pinned file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    /// The path is absolute, empty, or has a `.` or `..` component.
    BadPath,
    /// A dotenv file: those hold the project's secrets.
    Dotenv,
    /// A component is a symlink, or the leaf is not a regular file.
    NotFile(String),
    /// More source bytes than the limit; the size.
    TooLarge(u64),
    /// Not UTF-8.
    NotText,
    Io(String),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadPath => write!(f, "the path must be relative, with no . or .. component"),
            Self::Dotenv => write!(f, "dotenv files are never served"),
            Self::NotFile(why) => write!(f, "not a regular file: {why}"),
            Self::TooLarge(size) => write!(f, "the file is {size} bytes, over the limit"),
            Self::NotText => write!(f, "the file is not UTF-8 text"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

/// A pinned file opened for the remote port: its version, size and
/// modification time from the descriptor the walk ended on, and the
/// descriptor itself for the read.
#[derive(Debug)]
pub struct PinnedFile {
    /// Inode, change time and size hashed: moved by an edit in place,
    /// a same-size rewrite and a replace by rename alike.
    pub version: u64,
    pub size: u64,
    pub modified_ms: Option<u64>,
    fd: std::os::fd::OwnedFd,
}

/// Opens `relative` under `root` one component at a time with `openat`
/// from the previous directory's descriptor, never following a
/// symlink anywhere in the path, so a swap of a directory for a link
/// between two hops is refused at the next hop and a swap after it
/// cannot move the descriptor already held. The leaf is opened
/// non-blocking, so a FIFO with no writer does not park the thread,
/// and must be a regular file. Dotenv names are refused whatever is
/// pinned. The path is checked before anything touches the disk.
pub fn open_pinned(root: &Path, relative: &Path) -> Result<PinnedFile, FileError> {
    use std::hash::{Hash, Hasher};

    use rustix::fs::{Mode, OFlags, openat};
    let components: Vec<&std::ffi::OsStr> = relative
        .components()
        .map(|c| match c {
            Component::Normal(name) if !name.is_empty() => Ok(name),
            _ => Err(FileError::BadPath),
        })
        .collect::<Result<_, _>>()?;
    let Some((leaf, dirs)) = components.split_last() else {
        return Err(FileError::BadPath);
    };
    let leaf_name = leaf.to_string_lossy();
    if leaf_name == ".env" || leaf_name.starts_with(".env.") {
        return Err(FileError::Dotenv);
    }
    let io = |e: rustix::io::Errno| FileError::Io(e.to_string());
    let dir_flags = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut dir = rustix::fs::open(root, dir_flags, Mode::empty()).map_err(io)?;
    for name in dirs {
        // A symlink fails `NOFOLLOW` with `ELOOP` on Linux; with
        // `DIRECTORY` beside it, macOS says `ENOTDIR`.
        dir = openat(&dir, *name, dir_flags, Mode::empty()).map_err(|e| match e {
            rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => {
                FileError::NotFile("a symlink or a file in the path".into())
            }
            other => io(other),
        })?;
    }
    let leaf_flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = openat(&dir, *leaf, leaf_flags, Mode::empty()).map_err(|e| match e {
        rustix::io::Errno::LOOP => FileError::NotFile("a symlink".into()),
        other => io(other),
    })?;
    let stat = rustix::fs::fstat(&fd).map_err(io)?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(FileError::NotFile(
            "a FIFO, socket, device or directory".into(),
        ));
    }
    let size = u64::try_from(stat.st_size).unwrap_or(0);
    let modified_ms = u64::try_from(stat.st_mtime)
        .ok()
        .map(|s| s * 1000 + u64::try_from(stat.st_mtime_nsec).unwrap_or(0) / 1_000_000);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (stat.st_ino, stat.st_ctime, stat.st_ctime_nsec, stat.st_size).hash(&mut h);
    Ok(PinnedFile {
        version: h.finish(),
        size,
        modified_ms,
        fd,
    })
}

impl PinnedFile {
    /// The text, whole, if no more than `limit` bytes: the read takes
    /// one byte past the limit, so growth after the stat cannot pass
    /// it. The non-blocking flag is cleared first.
    pub fn read(self, limit: u64) -> Result<String, FileError> {
        use std::io::Read;
        if self.size > limit {
            return Err(FileError::TooLarge(self.size));
        }
        rustix::fs::fcntl_setfl(&self.fd, rustix::fs::OFlags::empty())
            .map_err(|e| FileError::Io(e.to_string()))?;
        let mut file = std::fs::File::from(self.fd);
        let mut bytes = Vec::with_capacity(usize::try_from(self.size).unwrap_or(0));
        file.by_ref()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| FileError::Io(e.to_string()))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
            return Err(FileError::TooLarge(self.size.max(limit + 1)));
        }
        String::from_utf8(bytes).map_err(|_| FileError::NotText)
    }
}

#[cfg(test)]
mod pinned_tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("sbpin")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/plan.md"), "# Plan\ntoken=abc\n").unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        std::fs::write(dir.path().join(".env.local"), "SECRET=1\n").unwrap();
        dir
    }

    #[test]
    fn a_pinned_file_is_read_whole_with_its_version_and_size() {
        let dir = root();
        let opened = open_pinned(dir.path(), Path::new("docs/plan.md")).unwrap();
        assert_eq!(opened.size, 17);
        assert!(opened.modified_ms.is_some());
        let first = opened.version;
        assert_eq!(opened.read(1024).unwrap(), "# Plan\ntoken=abc\n");
        // A same-size rewrite in place moves the version (ctime).
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.path().join("docs/plan.md"), "# Plan\ntoken=xyz\n").unwrap();
        let again = open_pinned(dir.path(), Path::new("docs/plan.md")).unwrap();
        assert_ne!(again.version, first);
        // A replace by rename is a new inode.
        std::fs::write(dir.path().join("docs/plan.new"), "# Plan\ntoken=xyz\n").unwrap();
        std::fs::rename(
            dir.path().join("docs/plan.new"),
            dir.path().join("docs/plan.md"),
        )
        .unwrap();
        let renamed = open_pinned(dir.path(), Path::new("docs/plan.md")).unwrap();
        assert_ne!(renamed.version, again.version);
    }

    #[test]
    fn the_path_is_checked_before_the_disk_and_dotenv_is_refused() {
        let dir = root();
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "docs/../plan.md",
            "./docs/plan.md",
        ] {
            assert_eq!(
                open_pinned(dir.path(), Path::new(bad)).err(),
                Some(FileError::BadPath),
                "{bad}"
            );
        }
        assert_eq!(
            open_pinned(dir.path(), Path::new(".env")).err(),
            Some(FileError::Dotenv)
        );
        assert_eq!(
            open_pinned(dir.path(), Path::new(".env.local")).err(),
            Some(FileError::Dotenv)
        );
        assert!(matches!(
            open_pinned(dir.path(), Path::new("docs/missing.md")).err(),
            Some(FileError::Io(_))
        ));
    }

    #[test]
    fn no_symlink_is_followed_and_only_a_regular_file_is_read() {
        let dir = root();
        let outside = tempfile::Builder::new()
            .prefix("sbout")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::write(outside.path().join("secret.txt"), "no").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("docs"), dir.path().join("inside")).unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("docs/plan.md"),
            dir.path().join("docs/leaf.md"),
        )
        .unwrap();
        for linked in ["link/secret.txt", "inside/plan.md", "docs/leaf.md"] {
            assert!(
                matches!(
                    open_pinned(dir.path(), Path::new(linked)).err(),
                    Some(FileError::NotFile(_))
                ),
                "{linked}"
            );
        }
        assert!(matches!(
            open_pinned(dir.path(), Path::new("docs")).err(),
            Some(FileError::NotFile(_) | FileError::Io(_))
        ));
        let fifo = dir.path().join("docs/pipe");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if made.is_ok_and(|s| s.success()) {
            let start = std::time::Instant::now();
            assert!(matches!(
                open_pinned(dir.path(), Path::new("docs/pipe")).err(),
                Some(FileError::NotFile(_))
            ));
            assert!(
                start.elapsed() < std::time::Duration::from_secs(1),
                "the open returned at once"
            );
        }
    }

    #[test]
    fn a_file_over_the_limit_is_refused_whole() {
        let dir = root();
        std::fs::write(dir.path().join("docs/big.txt"), "x".repeat(2000)).unwrap();
        let opened = open_pinned(dir.path(), Path::new("docs/big.txt")).unwrap();
        assert_eq!(opened.read(1000).err(), Some(FileError::TooLarge(2000)));
        std::fs::write(dir.path().join("docs/bin"), [0xff, 0xfe, 0x00]).unwrap();
        let opened = open_pinned(dir.path(), Path::new("docs/bin")).unwrap();
        assert_eq!(opened.read(1000).err(), Some(FileError::NotText));
    }
}
