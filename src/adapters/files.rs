//! Project file index: a recursive listing that honors `.gitignore`, a
//! lazily loaded directory view, and fuzzy path matching over the listing.
//!
//! Everything here reads directory metadata only; file contents are never
//! opened. The walk is delegated to the `ignore` crate (ripgrep's walker) so
//! ignore semantics match what users expect from `rg` and `git`.

use std::cmp::Ordering;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// When the scan ran, for staleness checks.
    pub scanned_at: SystemTime,
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
pub fn scan(root: &Path, max_entries: usize) -> io::Result<Listing> {
    let scanned_at = SystemTime::now();
    check_dir(root)?;

    let mut entries = Vec::new();
    let mut truncated = false;
    for item in walker(root).build() {
        if entries.len() >= max_entries {
            truncated = true;
            break;
        }
        let Some(dent) = readable(item) else { continue };
        if dent.depth() == 0 {
            continue;
        }
        if let Some(entry) = to_entry(root, &dent) {
            entries.push(entry);
        }
    }
    entries.sort_unstable_by(|a, b| a.rel.cmp(&b.rel));
    Ok(Listing {
        entries,
        truncated,
        scanned_at,
    })
}

/// Immediate children of `dir`, which must be `root` or somewhere under it.
///
/// Directories come first, then files, each sorted by name ignoring ASCII
/// case. Hidden entries are skipped and `.gitignore` rules from `root` down
/// to `dir` apply, so the lazily loaded tree agrees with [`scan`]. Each
/// entry's `rel` is relative to `root`, not to `dir`.
pub fn children(root: &Path, dir: &Path) -> io::Result<Vec<Entry>> {
    if dir.strip_prefix(root).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not under {}", dir.display(), root.display()),
        ));
    }
    check_dir(dir)?;

    let mut out: Vec<Entry> = walker(dir)
        .max_depth(Some(1))
        .build()
        .filter_map(readable)
        .filter(|dent| dent.depth() == 1)
        .filter_map(|dent| to_entry(root, &dent))
        .collect();
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
        let listing = scan(tmp.path(), 1000).unwrap();
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
    fn scan_truncates_at_cap() {
        let tmp = fixture();
        let listing = scan(tmp.path(), 3).unwrap();
        assert!(listing.truncated);
        assert_eq!(listing.entries.len(), 3);
    }

    #[test]
    fn scan_rejects_missing_root() {
        let tmp = fixture();
        assert!(scan(&tmp.path().join("nope"), 10).is_err());
    }

    #[test]
    fn children_lists_dirs_first_then_files_case_insensitively() {
        let tmp = fixture();
        let root = tmp.path();
        assert_eq!(
            rels(&children(root, root).unwrap()),
            ["docs", "nested", "src"]
        );
        // Ignore rules from the root apply to a nested directory, and the
        // sort ignores case.
        assert_eq!(
            rels(&children(root, &root.join("nested")).unwrap()),
            ["nested/deeper", "nested/Zeta.txt"]
        );
        assert!(children(root, Path::new("/")).is_err());
    }

    #[test]
    fn fuzzy_ranks_file_name_matches_first() {
        let tmp = fixture();
        let listing = scan(tmp.path(), 1000).unwrap();
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
}
