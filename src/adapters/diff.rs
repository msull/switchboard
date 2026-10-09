//! One file's `git diff` read into hunks, with the words that changed
//! between a removed line and the added line that replaced it marked.
//! The flags git runs with are fixed (`GitChanges::diff`), so the format
//! is small: the extended header lines before the first hunk, then
//! hunks. No path is read out of a header; the caller already has it.

use std::ops::Range;

use similar::{ChangeTag, TextDiff};

use crate::ports::changes::{DiffBody, DiffLine, FileDiff, FileStatus, Hunk, LineKind};

/// Body lines past which a diff is not shown. The diff is also read
/// with this much context, so a file under it comes back whole.
pub const DIFF_LINE_CAP: usize = 5000;

/// A line longer than this is not marked word by word.
const MARK_MAX_BYTES: usize = 500;

/// Below this share of equal words a pair reads as a rewrite, and
/// marking it word by word would mark nearly everything.
const MARK_MIN_RATIO: f32 = 0.5;

/// `git diff` output for one file, as hunks numbered on both sides.
#[must_use]
pub fn parse(out: &str, cap: usize) -> FileDiff {
    let mut status = FileStatus::Modified;
    let mut binary = false;
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut body_lines = 0;
    let (mut old_no, mut new_no) = (0, 0);
    for line in out.lines() {
        if let Some(hunk) = hunks.last_mut() {
            if let Some(next) = hunk_header(line) {
                (old_no, new_no) = (next.old_start, next.new_start);
                hunks.push(next);
                continue;
            }
            let (kind, text) = match line.as_bytes().first() {
                // An empty line is an empty context line under
                // `diff.suppressBlankEmpty`.
                None => (LineKind::Context, ""),
                Some(b' ') => (LineKind::Context, &line[1..]),
                Some(b'+') => (LineKind::Added, &line[1..]),
                Some(b'-') => (LineKind::Removed, &line[1..]),
                Some(b'\\') => {
                    if let Some(prev) = hunk.lines.last_mut() {
                        prev.no_newline = true;
                    }
                    continue;
                }
                Some(_) => continue,
            };
            body_lines += 1;
            if body_lines > cap {
                continue;
            }
            let (old, new) = match kind {
                LineKind::Context => (Some(old_no), Some(new_no)),
                LineKind::Removed => (Some(old_no), None),
                LineKind::Added => (None, Some(new_no)),
            };
            old_no += u32::from(old.is_some());
            new_no += u32::from(new.is_some());
            hunk.lines.push(DiffLine {
                kind,
                old_no: old,
                new_no: new,
                text: text.to_owned(),
                ..DiffLine::default()
            });
        } else if let Some(first) = hunk_header(line) {
            (old_no, new_no) = (first.old_start, first.new_start);
            hunks.push(first);
        } else if line.starts_with("new file mode") {
            status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            status = FileStatus::Deleted;
        } else if line.starts_with("rename from") {
            status = FileStatus::Renamed;
        } else if line.starts_with("Binary files") {
            binary = true;
        }
    }
    let body = if binary {
        DiffBody::Binary
    } else if body_lines > cap {
        DiffBody::TooLarge { lines: body_lines }
    } else if hunks.is_empty() {
        DiffBody::Empty
    } else {
        for hunk in &mut hunks {
            mark_words(hunk);
        }
        DiffBody::Hunks(hunks)
    };
    FileDiff { status, body }
}

/// `@@ -a[,b] +c[,d] @@ …` as an empty hunk starting at `a` and `c`.
fn hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let new = rest.strip_prefix('+')?.split_once(' ')?.0;
    let start = |s: &str| s.split(',').next()?.parse::<u32>().ok();
    Some(Hunk {
        old_start: start(old)?,
        new_start: start(new)?,
        lines: Vec::new(),
    })
}

/// Pairs each run of removed lines with the run of added lines right
/// after it, line by line up to the shorter run, and marks the words
/// each pair does not share.
fn mark_words(hunk: &mut Hunk) {
    let lines = &mut hunk.lines;
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Removed {
            i += 1;
            continue;
        }
        let removed = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let added = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        for k in 0..(added - removed).min(i - added) {
            let (old, new) = mark_pair(&lines[removed + k].text, &lines[added + k].text);
            lines[removed + k].marks = old;
            lines[added + k].marks = new;
        }
    }
}

/// The byte ranges of `old` and of `new` that the other does not have,
/// or none for a long line or a rewrite.
fn mark_pair(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let none = (Vec::new(), Vec::new());
    if old.len() > MARK_MAX_BYTES || new.len() > MARK_MAX_BYTES {
        return none;
    }
    let diff = TextDiff::from_words(old, new);
    if diff.ratio() < MARK_MIN_RATIO {
        return none;
    }
    let (mut old_marks, mut new_marks) = none;
    let (mut o, mut n) = (0, 0);
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Delete => {
                merge(&mut old_marks, o..o + len);
                o += len;
            }
            ChangeTag::Insert => {
                merge(&mut new_marks, n..n + len);
                n += len;
            }
            ChangeTag::Equal => {
                o += len;
                n += len;
            }
        }
    }
    (old_marks, new_marks)
}

/// Push `r`, joining it onto the last range when they touch.
fn merge(marks: &mut Vec<Range<usize>>, r: Range<usize>) {
    match marks.last_mut() {
        Some(last) if last.end == r.start => last.end = r.end,
        _ => marks.push(r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each fixture is real output of
    // `git -c core.quotepath=off diff --no-color --no-ext-diff --no-textconv -M --unified=N base...work -- path [old_path]`,
    // with N = 3 for `MODIFIED` and 5000 for the rest.

    const MODIFIED: &str = r"diff --git a/m.txt b/m.txt
index c4352f8..bd17d1b 100644
--- a/m.txt
+++ b/m.txt
@@ -1,5 +1,5 @@
 line 1
-line 2
+line two
 line 3
 line 4
 line 5
@@ -15,6 +15,6 @@ line 14
 line 15
 line 16
 line 17
-line 18
+line eighteen
 line 19
 line 20
";

    const ADDED: &str = r"diff --git a/a.txt b/a.txt
new file mode 100644
index 0000000..92d5444
--- /dev/null
+++ b/a.txt
@@ -0,0 +1 @@
+fresh
";

    const DELETED: &str = r"diff --git a/d.txt b/d.txt
deleted file mode 100644
index de98044..0000000
--- a/d.txt
+++ /dev/null
@@ -1,3 +0,0 @@
-a
-b
-c
";

    const RENAMED: &str = r#"diff --git a/r.rs b/s.rs
similarity index 85%
rename from r.rs
rename to s.rs
index 6c7ec21..dc2b8ff 100644
--- a/r.rs
+++ b/s.rs
@@ -1,7 +1,7 @@
 fn main() {
     let a = 0;
     let b = 0;
-    let x = 1;
+    let y = 1;
     let c = 0;
     println!("{a}{b}{c}");
 }
"#;

    const PURE_RENAME: &str = r"diff --git a/p.txt b/q.txt
similarity index 100%
rename from p.txt
rename to q.txt
";

    const BINARY: &str = r"diff --git a/bin.dat b/bin.dat
index 20b5be9..88f3700 100644
Binary files a/bin.dat and b/bin.dat differ
";

    const NO_NEWLINE_OLD: &str = r"diff --git a/nn.txt b/nn.txt
index 9ed40b4..814f4a4 100644
--- a/nn.txt
+++ b/nn.txt
@@ -1,2 +1,2 @@
 one
-two
\ No newline at end of file
+two
";

    const NO_NEWLINE_NEW: &str = r"diff --git a/nn.txt b/nn.txt
index 814f4a4..7279b45 100644
--- a/nn.txt
+++ b/nn.txt
@@ -1,2 +1,2 @@
 one
-two
+three
\ No newline at end of file
";

    // Read with `-c diff.suppressBlankEmpty=true`: the blank context
    // line is written as an empty line, not a lone space.
    const BLANK_CONTEXT: &str = r"diff --git a/e.txt b/e.txt
index f7d4fc4..9faaf1b 100644
--- a/e.txt
+++ b/e.txt
@@ -1,3 +1,3 @@
 x

-y
+z
";

    fn hunks(d: &FileDiff) -> &[Hunk] {
        match &d.body {
            DiffBody::Hunks(h) => h,
            other => panic!("no hunks: {other:?}"),
        }
    }

    fn numbered(h: &Hunk) -> Vec<(LineKind, Option<u32>, Option<u32>, &str)> {
        h.lines
            .iter()
            .map(|l| (l.kind, l.old_no, l.new_no, l.text.as_str()))
            .collect()
    }

    #[test]
    fn a_modified_file_reads_as_numbered_hunks() {
        use LineKind::{Added, Context, Removed};
        let d = parse(MODIFIED, DIFF_LINE_CAP);
        assert_eq!(d.status, FileStatus::Modified);
        let h = hunks(&d);
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].old_start, h[0].new_start), (1, 1));
        assert_eq!(
            numbered(&h[0])[..3],
            [
                (Context, Some(1), Some(1), "line 1"),
                (Removed, Some(2), None, "line 2"),
                (Added, None, Some(2), "line two"),
            ]
        );
        assert_eq!(
            numbered(&h[1])[3..],
            [
                (Removed, Some(18), None, "line 18"),
                (Added, None, Some(18), "line eighteen"),
                (Context, Some(19), Some(19), "line 19"),
                (Context, Some(20), Some(20), "line 20"),
            ]
        );
    }

    #[test]
    fn added_and_deleted_files_read_their_status() {
        let d = parse(ADDED, DIFF_LINE_CAP);
        assert_eq!(d.status, FileStatus::Added);
        assert_eq!(
            numbered(&hunks(&d)[0]),
            [(LineKind::Added, None, Some(1), "fresh")]
        );
        let d = parse(DELETED, DIFF_LINE_CAP);
        assert_eq!(d.status, FileStatus::Deleted);
        let h = &hunks(&d)[0];
        assert_eq!(h.lines.len(), 3);
        assert_eq!(h.lines[2].old_no, Some(3));
        assert!(h.lines.iter().all(|l| l.kind == LineKind::Removed));
    }

    #[test]
    fn renames_read_as_renamed_with_or_without_lines() {
        let d = parse(RENAMED, DIFF_LINE_CAP);
        assert_eq!(d.status, FileStatus::Renamed);
        assert_eq!(hunks(&d)[0].lines.len(), 8);
        let d = parse(PURE_RENAME, DIFF_LINE_CAP);
        assert_eq!(d.status, FileStatus::Renamed);
        assert_eq!(d.body, DiffBody::Empty);
    }

    #[test]
    fn a_binary_file_has_no_lines() {
        assert_eq!(parse(BINARY, DIFF_LINE_CAP).body, DiffBody::Binary);
    }

    #[test]
    fn a_missing_newline_is_marked_on_its_line() {
        let d = parse(NO_NEWLINE_OLD, DIFF_LINE_CAP);
        let lines = &hunks(&d)[0].lines;
        assert_eq!(lines.len(), 3);
        assert!(lines[1].no_newline && lines[1].kind == LineKind::Removed);
        assert!(!lines[2].no_newline);
        let d = parse(NO_NEWLINE_NEW, DIFF_LINE_CAP);
        let lines = &hunks(&d)[0].lines;
        assert!(!lines[1].no_newline);
        assert!(lines[2].no_newline && lines[2].text == "three");
    }

    #[test]
    fn an_empty_line_is_blank_context() {
        let d = parse(BLANK_CONTEXT, DIFF_LINE_CAP);
        assert_eq!(
            numbered(&hunks(&d)[0])[1],
            (LineKind::Context, Some(2), Some(2), "")
        );
        assert_eq!(hunks(&d)[0].lines.len(), 4);
    }

    #[test]
    fn a_diff_over_the_cap_is_too_large() {
        assert_eq!(parse(MODIFIED, 10).body, DiffBody::TooLarge { lines: 13 });
    }

    #[test]
    fn changed_words_are_marked_and_rewrites_are_not() {
        let d = parse(RENAMED, DIFF_LINE_CAP);
        let lines = &hunks(&d)[0].lines;
        let (old, new) = (&lines[3], &lines[4]);
        assert_eq!(old.text, "    let x = 1;");
        assert_eq!(old.marks, vec![8..9]);
        assert_eq!(new.marks, vec![8..9]);
        assert!(lines[0].marks.is_empty());
        assert_eq!(
            mark_pair("alpha beta gamma", "one two three"),
            (Vec::new(), Vec::new())
        );
    }
}
